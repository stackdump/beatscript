// The editor's assistant: completion, hover, quick-fixes and refactors (the lightbulb), the outline and go to definition,
// from the language service in the wasm module (BeatscriptLs: djrust's crates/beatscriptls, the server `beatscript lsp` runs for VS Code and
// IntelliJ). The editor keeps its own keys; these are added:
//
//   Ctrl+Space          complete at the caret (also opens by itself after = : $ . | and ( )
//   ↑ ↓  Enter / Tab    pick in a list; Esc closes it (and only it: a second Esc stops the program, as before)
//   Alt+Enter           the fixes and refactors at the caret (the 💡 beside a line that has some)
//   Mod+Shift+O         the outline: type to filter, Enter to go
//   F12                 go to the definition of the name at the caret
//   Alt+E               show or hide the expansion of the reference at the caret (a `gen@2` call, a `seed(...)`, a stored
//                       program's `<cid>.drop.bass`, a `template(...)`): a read-only note at the end of its line, its first
//                       line, how many more and its CID. Hover the note for more of it; click it for the whole, copy, eject
//                       and hide. While the caret is on a reference whose expansion is hidden, `⇒ show expansion` stands there.
//                       Taken only on a reference: elsewhere, and in a quoted phrase or a comment, it types (é on a Mac).
//   Alt+Shift+E         every reference's expansion: shown, or hidden when any is (when the program has a reference)
//
// On a reference the Alt+Enter list also has *Eject* (the service makes the edit and proves it), *Copy the expansion*, and
// the page's show/hide, every one's, and *Open the whole expansion*. Which expansions are shown is the page's state
// (expansion-model.js), as it is the VS Code extension's; the service only answers `beatscript/expand` (docs/beatscriptls.md).
//
// The squiggles stay the checker's (beatscript_check): the service adds to the editor, it does not replace what the page had.

import { lspPos, offsetOf, offsetOfVcol, expandSnippet, mergeEdits, mdToHtml, flattenSymbols, filterRows, kindLetter, TRIGGERS } from './assist-model.js';
import { EJECT, GAP, refKey, shortCall, toggleOne, toggleAll, decorations, label, lensLabel, hoverMarkdown, wholeMarkdown, copyText, spoken, spokenOpen, route, referenceRows, popupRows, utf8Length, rangeHolds, inPhraseOrComment } from './expansion-model.js';
import { lineStarts, lineOf, visualCol } from './geometry.js';
import { h } from './util.js';
import { cellXY } from './wrap-model.js';

const URI = 'main';
const VISIBLE = 10;
const NO_REF = 'No reference here: a gen@2 call, a seed(...), a stored program\'s part or a template(...) has an expansion.';
const isEject = (a) => !!(a && a.data && a.data.refactor === 'eject');

export class Assist {
  /** facade: the wasm facade (its `ls()` makes a BeatscriptLs, or null in the mock). toast(msg, kind): the page's. */
  constructor(editor, facade, toast = () => {}) {
    this.ed = editor;
    this.ls = facade && facade.ls ? facade.ls() : null;
    this.toast = toast;
    this.synced = null;
    this.list = null; // {kind: 'complete'|'actions'|'outline'|'expansion', rows, sel, ...}
    this.pop = h('div', { class: 'as-pop', role: 'listbox', hidden: true });
    this.bulb = h('button', { type: 'button', class: 'as-bulb', title: 'Fixes and refactors (Alt+Enter)', 'aria-label': 'Fixes and refactors', hidden: true }, '💡');
    this.bulb.addEventListener('mousedown', (e) => { e.preventDefault(); this.openActions(); });
    // references: which expansions are shown (refKey), the text's references once per text, the caret's reference (its
    // lens while its expansion is hidden), and what each drawn note is
    this.shown = new Set();
    this.refsOf = { text: null, refs: [], errors: [] };
    this.caretRef = null;
    this.caretFor = null;
    this.expItems = [];
    this.expFor = null;
    this.exps = h('div', { class: 'as-exps', 'aria-hidden': 'true', hidden: true });
    this.exps.addEventListener('mousedown', (e) => {
      const el = e.target.closest('[data-x]');
      const it = el && this.expItems[+el.dataset.x];
      if (!it) return;
      e.preventDefault(); // the caret stays where it was, and the keys reach the popup
      this.ed.ta.focus({ preventScroll: true });
      if (it.lens) this.toggleRef(it.ref);
      else this.openExpansion(it.ref);
    });
    this.live = h('div', { class: 'as-live', 'aria-live': 'polite', 'aria-atomic': 'true' });
    editor.code.append(this.pop, this.bulb, this.exps);
    editor.root.append(this.live);
    this.pop.addEventListener('mousedown', (e) => {
      const row = e.target.closest('[data-i]');
      if (!row || !this.list) return;
      e.preventDefault();
      this.list.sel = +row.dataset.i;
      this.accept();
    });
  }

  get on() {
    return !!this.ls;
  }

  // ---- the service ---------------------------------------------------------------------------------------

  sync() {
    const t = this.ed.text;
    if (this.synced !== t) {
      this.ls.change(URI, t);
      this.synced = t;
    }
    return t;
  }

  req(method, params) {
    const t = this.sync();
    let r;
    try {
      r = JSON.parse(this.ls.request(method, JSON.stringify({ textDocument: { uri: URI }, ...params })));
    } catch (e) {
      console.warn('beatscript assist:', method, e);
      return null;
    }
    if (r && r.error && Object.keys(r).length === 1) return { error: r.error.message, text: t };
    return r;
  }

  caretPos() {
    return lspPos(this.ed.text, this.ed.ta.selectionStart);
  }

  // ---- editor hooks --------------------------------------------------------------------------------------

  /** The editor's keydown, first. True: the key was the assistant's. */
  onKey(e) {
    if (!this.on) return false;
    const mod = e.metaKey || e.ctrlKey;
    if (this.list) {
      const n = this.list.rows.length;
      const move = (d) => { this.list.sel = (this.list.sel + d + n) % Math.max(n, 1); this.paint(); };
      if (e.key === 'ArrowDown') { e.preventDefault(); move(1); return true; }
      if (e.key === 'ArrowUp') { e.preventDefault(); move(-1); return true; }
      if (e.key === 'PageDown') { e.preventDefault(); move(VISIBLE); return true; }
      if (e.key === 'PageUp') { e.preventDefault(); move(-VISIBLE); return true; }
      if ((e.key === 'Enter' || e.key === 'Tab') && !mod && n) { e.preventDefault(); this.accept(); return true; }
      if (e.key === 'Escape') { e.preventDefault(); this.close(); return true; }
      if (this.list.kind === 'outline') {
        if (e.key === 'Backspace') { e.preventDefault(); this.list.q = this.list.q.slice(0, -1); this.refilter(); return true; }
        if (e.key.length === 1 && !mod && !e.altKey) { e.preventDefault(); this.list.q += e.key; this.refilter(); return true; }
      }
      if (this.list.kind !== 'complete') { this.close(); }
    }
    if (mod && e.key === ' ') { e.preventDefault(); this.complete(); return true; }
    if (e.altKey && e.key === 'Enter') { e.preventDefault(); this.openActions(); return true; }
    // Alt+E (e.code: Option+E is a dead key on a Mac): the expansion at the caret; with Shift, every reference's. Taken only
    // when there is a reference for it: elsewhere, and in a quoted phrase or a comment, the key types (é, €, ę on a Mac)
    if (e.altKey && !mod && (e.code === 'KeyE' || e.key === 'e' || e.key === 'E')) {
      if (!this.expansionKey(e.shiftKey)) return false;
      e.preventDefault();
      return true;
    }
    if (mod && e.shiftKey && (e.key === 'o' || e.key === 'O')) { e.preventDefault(); this.outline(); return true; }
    if (e.key === 'F12') { e.preventDefault(); this.definition(); return true; }
    return false;
  }

  /** After the text changed by typing: keep an open completion list current, or open one after a trigger character. */
  onInput(programmatic) {
    // the edit may have moved the references: their notes go until they are drawn again for the new text (once it rests)
    if (this.on && !this.exps.hidden && this.expFor !== this.ed.text) this.exps.hidden = true;
    if (!this.on || programmatic) { this.close(); return; }
    // a paste with the mouse, a drop: a list made for the old text (the fixes, the outline, an expansion) goes
    if (this.list && this.list.kind !== 'complete') this.close();
    const t = this.ed.text;
    const at = this.ed.ta.selectionStart;
    const ch = t[at - 1];
    if (this.list && this.list.kind === 'complete') {
      if (!ch || /\s/.test(ch) && !this.list.space) this.close();
      else this.complete(true);
    } else if (ch && TRIGGERS.has(ch)) this.complete(true);
  }

  /** The caret moved: the lightbulb where the caret's line has fixes or refactors. */
  onCursor() {
    if (!this.on) return;
    clearTimeout(this._bulbT);
    this._bulbT = setTimeout(() => this.updateBulb(), 120);
  }

  /** Hover text for (line0, vcol) when no diagnostic is there: {title, html} or null. */
  hoverAt(line0, vcol) {
    if (!this.on) return null;
    const t = this.ed.text;
    const starts = lineStarts(t);
    if (line0 >= starts.length) return null;
    const at = offsetOfVcol(t, starts[line0], vcol);
    const r = this.req('textDocument/hover', { position: lspPos(t, at) });
    if (!r || r.error || !r.contents) return null;
    return { html: mdToHtml(r.contents.value) };
  }

  /** The hover of one of the notes this draws (a shown expansion, a lens), asked before the text's: {html} or null. */
  hoverTarget(el) {
    const x = this.on && el && el.closest ? el.closest('.as-exp') : null;
    const it = x && this.expItems[+x.dataset.x];
    if (!it) return null;
    if (it.lens) return { html: null }; // its title says it
    return { html: mdToHtml(hoverMarkdown(it.ref)) };
  }

  /** The page checked the text (it changed, or a program it draws on arrived): the notes for that text. */
  onChecked() {
    if (!this.on) return;
    if (this.knownSince) {
      // a program was told to the module: the caret's reference and its actions, asked again of the reopened document
      this.knownSince = false;
      this.updateBulb();
    }
    if (this.shown.size || this.caretRef) this.paintExpansions();
  }

  /**
   * The module was told a program (the page fetched one that a stored reference names). The service works a text's
   * references out once and keeps them, so what it answered for this text may have lacked that program: the document is
   * opened afresh at the next request, and the caret's reference is asked again after the check that follows.
   */
  onKnown() {
    this.synced = null;
    this.refsOf = { text: null, refs: [], errors: [] };
    this.knownSince = true;
  }

  /** A program was loaded in place of the text (an example, a link, an assistant's eval): what was shown was the old one's. */
  onLoad() {
    if (!this.on) return;
    this.close();
    this.shown.clear();
    this.caretRef = null;
    this.caretFor = null;
    this.paintExpansions();
  }

  /** The editor's width or font changed (the text wraps again): the notes move with it. */
  onLayout() {
    if (this.on && (this.shown.size || this.caretRef)) this.paintExpansions();
  }

  // ---- lists -----------------------------------------------------------------------------------------------

  /** Where the popup goes: under the caret, in the code layer's coordinates. */
  place() {
    const t = this.ed.text;
    const starts = lineStarts(t);
    const at = this.list && this.list.anchor != null ? this.list.anchor : this.ed.ta.selectionStart;
    const l = lineOf(starts, at);
    const p = cellXY(this.ed.wrap, l, visualCol(t, starts[l], at)); // the text soft-wraps: ask the layout
    this.pop.style.left = `${p.x}px`;
    this.pop.style.top = `${p.y + this.ed.metrics.lineH + 2}px`;
  }

  paint() {
    const L = this.list;
    if (!L || !L.rows.length) {
      this.pop.hidden = true;
      return;
    }
    const first = Math.max(0, Math.min(L.sel - Math.floor(VISIBLE / 2), L.rows.length - VISIBLE));
    const shown = L.rows.slice(first, first + VISIBLE);
    const head = L.kind === 'outline' ? [h('div', { class: 'as-q' }, `outline ${L.q ? `· ${L.q}` : '· type to filter'}`)] : [];
    this.pop.replaceChildren(...head, ...shown.map((r, k) => {
      const i = first + k;
      return h('div', { class: `as-row${i === L.sel ? ' sel' : ''}`, role: 'option', 'aria-selected': String(i === L.sel), 'data-i': String(i) },
        h('i', { class: 'as-k' }, r.icon || ''),
        h('span', { class: 'as-l', style: r.depth ? `padding-left:${r.depth * 12}px` : null }, r.label),
        r.detail ? h('span', { class: 'as-d' }, r.detail) : null);
    }), ...(L.rows[L.sel] && L.rows[L.sel].doc ? [h('div', { class: L.kind === 'expansion' ? 'as-doc as-whole' : 'as-doc' })] : []));
    const doc = this.pop.querySelector('.as-doc');
    if (doc) doc.innerHTML = mdToHtml(L.rows[L.sel].doc);
    this.pop.hidden = false;
    this.place();
  }

  close() {
    this.list = null;
    this.pop.hidden = true;
  }

  accept() {
    const L = this.list;
    if (!L) return;
    const r = L.rows[L.sel];
    this.close();
    if (r && r.go) r.go();
  }

  // ---- completion ------------------------------------------------------------------------------------------

  complete(auto = false) {
    const res = this.req('textDocument/completion', { position: this.caretPos() });
    const items = (res && res.items) || [];
    if (!items.length) {
      this.close();
      return;
    }
    const t = this.ed.text;
    const rows = items.map((it) => ({
      label: it.label,
      detail: it.detail || '',
      doc: it.documentation ? it.documentation.value : '',
      icon: kindLetter(it.kind),
      go: () => this.insert(it),
    }));
    const range = items[0].textEdit && items[0].textEdit.range;
    const anchor = range ? offsetOf(t, range.start) : this.ed.ta.selectionStart;
    const keep = this.list && this.list.kind === 'complete' ? rows.findIndex((r) => r.label === this.list.rows[this.list.sel]?.label) : -1;
    this.list = { kind: 'complete', rows, sel: Math.max(0, keep), anchor, auto };
    this.paint();
  }

  insert(item) {
    const t = this.ed.text;
    const te = item.textEdit;
    const from = te ? offsetOf(t, te.range.start) : this.ed.ta.selectionStart;
    const to = te ? Math.max(offsetOf(t, te.range.end), this.ed.ta.selectionStart) : this.ed.ta.selectionStart;
    const raw = te ? te.newText : item.insertText || item.label;
    const { text, sel } = item.insertTextFormat === 2 ? expandSnippet(raw) : { text: raw, sel: [raw.length, raw.length] };
    // keep the line's indentation on the lines a snippet adds
    const ls = t.lastIndexOf('\n', from - 1) + 1;
    const lead = /^[ \t]*/.exec(t.slice(ls, from))[0];
    const body = text.replace(/\n/g, `\n${lead}`);
    const shift = (k) => k + (text.slice(0, k).split('\n').length - 1) * lead.length;
    this.ed.applyEdit(from, to, body);
    this.ed.ta.setSelectionRange(from + shift(sel[0]), from + shift(sel[1]));
  }

  // ---- code actions ------------------------------------------------------------------------------------------

  actionsAt() {
    const ta = this.ed.ta;
    const t = this.ed.text;
    const res = this.req('textDocument/codeAction', { range: { start: lspPos(t, ta.selectionStart), end: lspPos(t, ta.selectionEnd) }, context: { diagnostics: [] } });
    return Array.isArray(res) ? res : [];
  }

  updateBulb() {
    const acts = this.actionsAt();
    // the reference at the caret (the service offers to eject one there): its lens stands at its line's end while its
    // expansion is hidden
    const t = this.ed.text;
    const ref = acts.some(isEject) ? this.referenceAt().ref : null;
    const was = this.caretRef ? refKey(this.caretRef) : null;
    this.caretRef = ref;
    this.caretFor = t;
    if ((ref ? refKey(ref) : null) !== was || (this.expFor !== t && (this.shown.size || ref))) this.paintExpansions();
    if (!acts.length) {
      this.bulb.hidden = true;
      return;
    }
    const starts = lineStarts(t);
    const l = lineOf(starts, this.ed.ta.selectionStart);
    const end = l + 1 < starts.length ? starts[l + 1] - 1 : t.length;
    const p = cellXY(this.ed.wrap, l, visualCol(t, starts[l], end) + 1);
    this.bulb.style.left = `${p.x}px`;
    this.bulb.style.top = `${p.y}px`;
    this.bulb.hidden = false;
  }

  openActions() {
    const acts = this.actionsAt();
    if (!acts.length) {
      this.toast('No fixes or refactors here.', 'ok');
      return;
    }
    const rows = acts.map((a) => ({ label: a.title, detail: a.kind === 'quickfix' ? (a.isPreferred ? 'fix' : 'maybe') : a.kind.replace(/^refactor\./, ''), icon: a.kind === 'quickfix' ? '✓' : '↻', go: () => this.run(a) }));
    // on a reference, after the service's eject and copy: the page's own show or hide, every one's, and the whole
    const ref = acts.some(isEject) ? this.referenceAt().ref : null;
    if (ref) {
      const go = { toggle: () => this.toggleRef(ref), all: () => this.toggleAllExpansions(), open: () => this.openExpansion(ref) };
      for (const r of referenceRows(ref, this.shown, this.references())) rows.push({ label: r.label, detail: r.detail, icon: '⇒', go: go[r.id] });
    }
    this.list = { kind: 'actions', rows, sel: 0, anchor: this.ed.ta.selectionStart };
    this.paint();
  }

  run(action) {
    let a = action;
    // a command and no edit (*Copy the expansion of ...*): the page carries it out, as the VS Code extension does
    if (a.command && !a.edit && !(a.data && a.data.refactor)) {
      this.runCommand(a.command);
      return;
    }
    if (!a.edit) {
      // a refactor: made and proved by the service (the nets must not change)
      let r;
      try { r = JSON.parse(this.ls.request('codeAction/resolve', JSON.stringify(a))); } catch { r = null; }
      if (!r || r.error) {
        this.toast(r && r.error ? r.error.message : 'The refactor could not be made.', 'warn');
        return;
      }
      a = r;
    }
    this.applyWorkspaceEdit(a.edit);
  }

  /** A WorkspaceEdit on this document as one undoable edit. False when it has none. */
  applyWorkspaceEdit(edit) {
    const edits = edit && edit.changes && edit.changes[URI];
    if (!edits || !edits.length) return false;
    const m = mergeEdits(this.ed.text, edits);
    this.ed.applyEdit(m.from, m.to, m.insert);
    this.bulb.hidden = true;
    return true;
  }

  // ---- references: show or hide an expansion, every one's, copy one, eject one (expansion-model.js) --------------------

  /** Every reference of the text (`beatscript/expand`), asked once per text. */
  references() {
    const t = this.ed.text;
    if (this.refsOf.text !== t) {
      const res = this.req('beatscript/expand', {});
      this.refsOf = { text: t, refs: (res && res.references) || [], errors: (res && res.errors) || [] };
    }
    return this.refsOf.refs;
  }

  /** The reference at an LSP position (the caret's): {ref}, or {why} to tell the user and `here`, a problem of a reference there. */
  referenceAt(position = this.caretPos()) {
    const res = this.req('beatscript/expand', { position });
    const ref = res && res.references && res.references[0];
    if (ref) return { ref };
    const errs = (res && res.errors) || [];
    const here = errs.find((d) => d.range && rangeHolds(d.range, position));
    const err = here || errs[0];
    return { ref: null, here: !!here, why: err ? String(err.message).split('\n')[0] : res && res.error ? res.error : NO_REF };
  }

  /**
   * Alt+E (`all`: Alt+Shift+E) when there is something for it: a reference at the caret, or one of its problems there (said);
   * for every one's, any reference. False: nothing, so the key is left to type.
   */
  expansionKey(all) {
    if (inPhraseOrComment(this.ed.text, this.ed.ta.selectionStart)) return false;
    const position = this.caretPos();
    if (all) {
      const refs = this.references();
      const here = this.refsOf.errors.find((d) => d.range && rangeHolds(d.range, position));
      if (!refs.length && !here) return false;
      if (refs.length) this.toggleAllExpansions();
      else this.toast(String(here.message).split('\n')[0], 'ok');
      return true;
    }
    const { ref, why, here } = this.referenceAt(position);
    if (!ref && !here) return false;
    if (ref) this.toggleRef(ref);
    else this.toast(why, 'ok');
    return true;
  }

  /** Alt+E: show or hide the expansion of the reference at `position` (the caret's). The reference, or null. */
  toggleExpansion(position) {
    const { ref, why } = this.referenceAt(position);
    if (!ref) {
      this.toast(why, 'ok');
      return null;
    }
    this.toggleRef(ref);
    return ref;
  }

  toggleRef(ref) {
    const on = toggleOne(this.shown, ref);
    this.say(on ? spoken(ref) : `Expansion of ${shortCall(ref.call)} hidden.`);
    this.paintExpansions();
  }

  /** Alt+Shift+E: every reference's expansion shown, or all hidden when any is. */
  toggleAllExpansions() {
    const refs = this.references();
    if (!refs.length) {
      const err = this.refsOf.errors[0];
      this.toast(err ? String(err.message).split('\n')[0] : 'No references in this program: nothing to expand.', 'ok');
      return;
    }
    const on = toggleAll(this.shown, refs);
    this.say(on ? `${refs.length} expansion${refs.length === 1 ? '' : 's'} shown.` : 'Expansions hidden.');
    this.paintExpansions();
  }

  /** Copy what eject would write for the reference at `position`; where the clipboard is shut, the whole opens to copy by hand. */
  copyExpansion(position) {
    const { ref, why } = this.referenceAt(position);
    if (!ref) {
      this.toast(why, 'ok');
      return;
    }
    const fail = () => {
      this.openExpansion(ref);
      this.toast('Could not copy automatically: the expansion is open under the reference; select it and copy it.', 'warn');
    };
    try {
      navigator.clipboard.writeText(copyText(ref)).then(() => this.toast(`Copied the expansion of ${shortCall(ref.call)}.`, 'ok'), fail);
    } catch {
      fail();
    }
  }

  /** Eject the reference at `at` ({uri, position, length?}) as VS Code's lens does: the service makes the edit and proves it. */
  eject(at) {
    const r = this.req('workspace/executeCommand', { command: EJECT, arguments: [at] });
    if (!r || r.error || !r.edit) {
      this.toast(r && r.error ? `Not ejected: ${r.error}` : 'The reference could not be ejected.', 'warn');
      return false;
    }
    return this.applyWorkspaceEdit(r.edit);
  }

  /** A command the service named (a code action's), routed as the VS Code extension's middleware routes it. */
  runCommand(command) {
    const { run, at } = route(command, { uri: URI, position: this.caretPos(), length: utf8Length(this.ed.text) });
    if (run === 'toggle') this.toggleExpansion(at.position);
    else if (run === 'copy') this.copyExpansion(at.position);
    else if (run === 'eject') this.eject(at);
    else this.toast(`The page has no command ${command && command.command}.`, 'warn');
  }

  /** The whole expansion of `ref` in the popup under it, with show or hide, copy and eject. */
  openExpansion(ref) {
    const doc = wholeMarkdown(ref);
    const position = ref.range.start;
    // the eject says which text it was made for (its UTF-8 length): the service refuses it for another
    const at = { uri: URI, position, length: utf8Length(this.ed.text) };
    const go = { toggle: () => this.toggleRef(ref), copy: () => this.copyExpansion(position), eject: () => this.eject(at) };
    const made = popupRows(ref, this.shown);
    const rows = made.map((r) => ({ label: r.label, detail: r.detail, icon: '⇒', doc, go: go[r.id] }));
    this.list = { kind: 'expansion', rows, sel: 0, anchor: offsetOf(this.ed.text, position) };
    this.paint();
    this.say(spokenOpen(ref, made));
  }

  /** Draw the notes: each shown expansion, and the lens of the caret's reference, at the end of its line. */
  paintExpansions() {
    const lay = this.exps;
    const t = this.ed.text;
    const lens = this.caretFor === t ? this.caretRef : null;
    this.expItems = [];
    this.expFor = t;
    if (!this.shown.size && !lens) {
      lay.replaceChildren();
      lay.hidden = true;
      return;
    }
    const groups = decorations(t, this.shown.size ? this.references() : [], this.shown, lens);
    const starts = lineStarts(t);
    const { charW, padLeft } = this.ed.metrics;
    const right = this.ed.code.clientWidth - padLeft; // where the text's rows end
    const out = [];
    for (const g of groups) {
      if (g.line >= starts.length) continue;
      // after the line's last character, on its last row; never over the text: a line that leaves fewer than three columns
      // gets no note (the hover, Alt+Enter and the popup still reach the expansion)
      const end = cellXY(this.ed.wrap, g.line, visualCol(t, starts[g.line], g.end));
      const room = Math.floor((right - end.x) / charW + 1e-6);
      if (room < 3) continue;
      const gap = Math.max(1, Math.min(GAP, room - 2));
      const x = end.x + gap * charW;
      const p = { y: end.y };
      const cols = room - gap;
      const each = Math.max(1, Math.floor((cols - 2 * (g.items.length - 1)) / g.items.length));
      const kids = [];
      for (const it of g.items) {
        if (kids.length) kids.push('  ');
        const i = this.expItems.push(it) - 1;
        kids.push(h('span', { class: it.lens ? 'as-exp as-lens' : 'as-exp', 'data-x': String(i), title: it.lens ? `Show the expansion of ${shortCall(it.ref.call)} here (Alt+E); Alt+Enter: copy it, or eject it into local text` : null }, it.lens ? lensLabel(each) : label(it.ref, each)));
      }
      const row = h('div', { class: 'as-exp-line' }, ...kids);
      row.style.left = `${x}px`;
      row.style.top = `${p.y}px`;
      row.style.maxWidth = `${Math.max(charW, right - x)}px`;
      out.push(row);
    }
    lay.replaceChildren(...out);
    lay.hidden = !out.length;
  }

  /** Tell a screen reader (the notes are drawn, not read). */
  say(msg) {
    this.live.textContent = msg;
  }

  // ---- outline and definition ------------------------------------------------------------------------------

  outline() {
    const res = this.req('textDocument/documentSymbol', {});
    const all = flattenSymbols(Array.isArray(res) ? res : []);
    if (!all.length) {
      this.toast('Nothing to outline yet.', 'ok');
      return;
    }
    this.list = { kind: 'outline', all, q: '', rows: [], sel: 0, anchor: this.ed.ta.selectionStart };
    this.refilter();
  }

  refilter() {
    const L = this.list;
    const t = this.ed.text;
    L.rows = filterRows(L.all, L.q).map((r) => ({ label: r.name, detail: r.detail, icon: kindLetter(r.kind, true), depth: L.q ? 0 : r.depth, go: () => this.goTo(offsetOf(t, r)) }));
    L.sel = 0;
    this.paint();
  }

  goTo(at) {
    const t = this.ed.text;
    const starts = lineStarts(t);
    const l = lineOf(starts, at);
    this.ed.ta.focus({ preventScroll: true });
    this.ed.ta.setSelectionRange(at, at);
    this.ed.revealLine(l, true);
  }

  definition() {
    const r = this.req('textDocument/definition', { position: this.caretPos() });
    if (!r || r.error || !r.range) {
      this.toast('No definition here.', 'ok');
      return;
    }
    this.goTo(offsetOf(this.ed.text, r.range.start));
  }
}
