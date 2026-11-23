// A program's references in the editor: show or hide a reference's expansion, every reference's, copy one, eject one
// (docs/beatscriptls.md "References: show, hide, copy, eject"). This is the pure side (assist.js draws it): no DOM, so
// test/unit.mjs pins it.
//
// A reference is what the language service's `beatscript/expand` answers for one (a `gen@2 ...` call, a `seed(...)`, a stored
// program's `<cid>.drop.bass`, a `template(...)`): {range, replaced, kind, half, call, resolved, place, name, text, from, ...},
// `text` being what eject writes in place of `replaced`, under the `from` line. Which expansions are shown is the page's
// state, as it is the VS Code extension's (editors/vscode/extension.js): the service answers what a reference stands for and
// keeps nothing.

import { offsetOf } from './assist-model.js';

/** The service's commands (`executeCommandProvider.commands`): its code lenses and code actions name them. */
export const TOGGLE = 'beatscript.expand.toggle';
export const COPY = 'beatscript.expand.copy';
export const EJECT = 'beatscript.eject';

/** How much of an expansion the decoration's hover shows; a click on the decoration shows all of it. */
export const HOVER_LINES = 12;
export const HOVER_CHARS = 1200;

/** Columns between a line's last character and its decoration: the lightbulb sits in the first ones. */
export const GAP = 4;

/** A reference's key in the shown set: its kind, call, place and name, so an edit that moves it keeps it shown (VS Code's). */
export const refKey = (r) => `${r.kind}|${r.call}|${r.place}|${r.name || ''}`;

/** A long CID as its first character, `…` and its last 8 (djengine::rights::short_cid). */
export function shortCid(c) {
  const cs = [...String(c || '')];
  return cs.length > 12 ? `${cs[0]}…${cs.slice(-8).join('')}` : cs.join('');
}

const CID = /^z[1-9A-HJ-NP-Za-km-z]{20,80}$/;

/** A call as a title says it: a stored reference's CID shortened (`z…8 chars.drop.bass`), anything else as it is. */
export function shortCall(call) {
  const s = String(call || '');
  const head = s.split(/[. ]/)[0];
  return CID.test(head) ? `${shortCid(head)}${s.slice(head.length)}` : s;
}

/** Show the reference's expansion, or hide it when it is shown. True: it is shown now. */
export function toggleOne(shown, ref) {
  const k = refKey(ref);
  if (shown.has(k)) {
    shown.delete(k);
    return false;
  }
  shown.add(k);
  return true;
}

/** Every reference: hide them all when any of them is shown, else show them all. True: they are shown now. */
export function toggleAll(shown, refs) {
  if (refs.some((r) => shown.has(refKey(r)))) {
    shown.clear();
    return false;
  }
  for (const r of refs) shown.add(refKey(r));
  return refs.length > 0;
}

/**
 * Where the decorations go: one group per line, at the end of the line a reference ends on (a part line's own params may
 * follow the call), in text order. A group holds the shown references of its line and, for `lens` (the reference at the
 * caret when its expansion is hidden), a lens: [{line, end (UTF-16 offset of the line's end), items: [{ref, lens}]}].
 */
export function decorations(text, refs, shown, lens = null) {
  const byLine = new Map();
  const add = (ref, isLens) => {
    const line = ref.range.end.line;
    if (!byLine.has(line)) byLine.set(line, []);
    byLine.get(line).push({ ref, lens: isLens });
  };
  for (const r of refs) if (shown.has(refKey(r))) add(r, false);
  if (lens && !shown.has(refKey(lens))) add(lens, true);
  return [...byLine.keys()].sort((a, b) => a - b).map((line) => ({
    line,
    end: offsetOf(text, { line, character: Number.MAX_SAFE_INTEGER }),
    items: byLine.get(line).sort((a, b) => a.ref.range.start.character - b.ref.range.start.character),
  }));
}

const width = (s) => [...s].length;

/** `s` cut to `n` columns, the last one `…` when it was cut. */
function cut(s, n) {
  const cs = [...s];
  return cs.length <= n ? s : n <= 1 ? '…'.slice(0, n) : `${cs.slice(0, n - 1).join('')}…`;
}

/**
 * What a shown expansion's decoration says in at most `cols` columns: `⇒ <its first line>  … N more lines  [z…cid]`. When
 * that is too wide the first line is cut first, then dropped for the count (`⇒ 5 lines  [z…cid]`), then the CID goes.
 */
export function label(ref, cols = Infinity) {
  const lines = String(ref.text || '').split('\n');
  const n = lines.length;
  const more = n > 1 ? `  … ${n - 1} more line${n === 2 ? '' : 's'}` : '';
  const cid = ref.resolved ? `  [${shortCid(ref.resolved)}]` : '';
  const first = cut(lines[0].trim(), 72);
  const full = `⇒ ${first}${more}${cid}`;
  if (width(full) <= cols) return full;
  const room = cols - width(`⇒ ${more}${cid}`);
  if (room >= 12) return `⇒ ${cut(first, room)}${more}${cid}`;
  const count = `⇒ ${n} line${n === 1 ? '' : 's'}`;
  if (width(count + cid) <= cols) return count + cid;
  if (width(count) <= cols) return count;
  return '⇒';
}

/** What the lens over a reference whose expansion is hidden says (the decoration's place, until it is shown). */
export function lensLabel(cols = Infinity) {
  const full = '⇒ show expansion';
  return width(full) <= cols ? full : '⇒';
}

/** The first `lines` lines of `text`, at most `chars` characters (one line at least), and how many lines were left out. */
export function truncate(text, lines, chars) {
  const all = String(text).split('\n');
  const out = [];
  let len = 0;
  for (const l of all) {
    if (out.length >= lines || (out.length && len + l.length > chars)) break;
    out.push(l);
    len += l.length + 1;
  }
  return { text: out.join('\n'), left: all.length - out.length, total: all.length };
}

/** The head of an expansion's markdown: the call, what it resolves to, its kind and half. */
function head(ref) {
  return `\`${ref.call}\`${ref.resolved ? ` → \`${ref.resolved}\`` : ''} (${ref.kind}, ${ref.half})`;
}

/** The decoration's hover: the call, its CID and the expansion under its `# from:` line, cut short with a count of the rest. */
export function hoverMarkdown(ref, lines = HOVER_LINES, chars = HOVER_CHARS) {
  const t = truncate(`${ref.from}\n${ref.text}`, lines, chars);
  const rest = t.left ? `\n\n… ${t.left} more line${t.left === 1 ? '' : 's'} (${t.total} in all): click it to see the whole` : '';
  return `${head(ref)}\n\n\`\`\`\n${t.text}\n\`\`\`${rest}\n\nClick: the whole expansion, copy it, eject it, hide it (Alt+E).`;
}

/** The whole expansion (the popup a click on the decoration opens): what eject would write, its `# from:` line first. */
export function wholeMarkdown(ref) {
  return `${head(ref)}\n\n\`\`\`\n${ref.from}\n${ref.text}\n\`\`\``;
}

/** The text's length in UTF-8 bytes: what the service compares a command's `length` with (an eject of a changed text is refused). */
export const utf8Length = (text) => new TextEncoder().encode(String(text)).length;

/** Does the LSP range hold the LSP position (both ends included)? */
export function rangeHolds(range, pos) {
  const { start: a, end: b } = range;
  const after = pos.line > a.line || (pos.line === a.line && pos.character >= a.character);
  const before = pos.line < b.line || (pos.line === b.line && pos.character <= b.character);
  return after && before;
}

/**
 * Is UTF-16 offset `at` inside a quoted phrase or a comment of its line? There Alt+E is left to type (Option+E is é, € or ę on
 * a Mac). A comment's `#` starts the line or follows a space (in a word it is a sharp: `c#4`).
 */
export function inPhraseOrComment(text, at) {
  const t = String(text);
  let i = Math.max(t.lastIndexOf('\n', at - 1), t.lastIndexOf('\r', at - 1)) + 1;
  let quoted = false;
  for (; i < at && i < t.length; i++) {
    const c = t[i];
    if (c === '"') quoted = !quoted;
    else if (c === '#' && !quoted && (i === 0 || /[\s]/.test(t[i - 1]))) return true;
  }
  return quoted;
}

/** What copying an expansion puts on the clipboard: what eject would write (the `# from:` line, the text), as VS Code copies it. */
export const copyText = (ref) => `${ref.from}\n${ref.text}\n`;

/** One sentence for a screen reader when an expansion is shown. */
export function spoken(ref) {
  const lines = String(ref.text || '').split('\n');
  return `Expansion of ${shortCall(ref.call)} shown: ${lines[0].trim()}${lines.length > 1 ? `, and ${lines.length - 1} more line${lines.length === 2 ? '' : 's'}` : ''}.`;
}

/** One sentence for a screen reader when the whole expansion opens under the reference (the popup is not focused). */
export function spokenOpen(ref, rows) {
  const n = String(ref.text || '').split('\n').length;
  return `The whole expansion of ${shortCall(ref.call)} is open, ${n} line${n === 1 ? '' : 's'}, first: ${String(ref.text || '').split('\n')[0].trim()}. Up and Down pick ${rows.map((r) => r.label.split(' ')[0].toLowerCase()).join(', ')}; Enter runs it; Escape closes it.`;
}

/**
 * What a command the service named (a lens's or a code action's) is for the page, as the VS Code extension's middleware
 * routes it: show/hide and copy are the page's own (it asks `beatscript/expand`); eject goes to the service, which makes the edit
 * and proves it. {run: 'toggle' | 'copy' | 'eject' | null, at}: `at` the command's argument ({uri, position, length?}), else
 * `caret` ({uri, position}).
 */
export function route(command, caret) {
  const name = command && command.command;
  const run = name === TOGGLE ? 'toggle' : name === COPY ? 'copy' : name === EJECT ? 'eject' : null;
  const arg = command && Array.isArray(command.arguments) ? command.arguments[0] : null;
  return { run, at: run ? (arg && arg.uri ? arg : caret) : null };
}

/**
 * The page's own rows in the Alt+Enter list on a reference, after the service's (*Eject* and *Copy the expansion*): show or
 * hide this expansion, every reference's, and open the whole. [{id: 'toggle' | 'all' | 'open', label, detail}]
 */
export function referenceRows(ref, shown, refs) {
  const c = shortCall(ref.call);
  const on = shown.has(refKey(ref));
  const any = refs.some((r) => shown.has(refKey(r)));
  return [
    { id: 'toggle', label: `${on ? 'Hide' : 'Show'} the expansion of \`${c}\``, detail: 'Alt+E' },
    ...(refs.length > 1 ? [{ id: 'all', label: `${any ? 'Hide' : 'Show'} every reference's expansion`, detail: 'Alt+Shift+E' }] : []),
    { id: 'open', label: `Open the whole expansion of \`${c}\``, detail: `${String(ref.text || '').split('\n').length} lines` },
  ];
}

/** The rows of the popup with the whole expansion under them (a click on a decoration, or *Open*). [{id, label, detail}] */
export function popupRows(ref, shown) {
  return [
    { id: 'toggle', label: `${shown.has(refKey(ref)) ? 'Hide' : 'Show'} the expansion`, detail: 'Alt+E' },
    { id: 'copy', label: 'Copy the expansion', detail: 'what eject writes' },
    { id: 'eject', label: `Eject \`${shortCall(ref.call)}\` into local text`, detail: 'proved' },
  ];
}
