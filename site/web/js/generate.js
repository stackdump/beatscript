// The ✦generate panel: the seeded generator (no model, no sign-in) on its own toolbar button. Genre, seed, length ->
// a whole program, the same every time. Generate writes it into the editor and plays it on the current source; the
// seed's -1/+1 (and [ ], or Up/Down in the seed box) step to the neighbouring seed while it plays, like turning a knob. "Insert as
// reference" puts the `gen@2 ...` call for one section's part at the caret instead.
//
// The pure parts (the request, the call, stepping, the structure and the roles of a generated program) are in
// generate-model.js; this file is the DOM. The panel's state lives on the Panels object and in localStorage, so closing
// and reopening keeps the genre and seed.

import { h, shortCid } from './util.js';
import { describeError } from './errors.js';
import { SEEDED_GENRES } from './station-model.js';
import { LENGTHS, genRequest, genCall, stepSeed, randomSeed, structureOf, rolesIn, loadGenState, saveGenState } from './generate-model.js';

/** The panel's body. `panels` keeps the state across renders (panels.gen). */
export function generateBody(panels) {
  const app = panels.app;
  const st = panels.gen || (panels.gen = loadGenState(safeStore(), SEEDED_GENRES));
  const save = () => saveGenState(safeStore(), st);

  const genre = h('select', { id: 'gen-genre', 'aria-label': 'Genre' }, ...SEEDED_GENRES.map((g) => h('option', { value: g }, g)));
  genre.value = st.genre;
  const seed = h('input', { type: 'text', id: 'gen-seed', inputmode: 'numeric', autocomplete: 'off', spellcheck: 'false', 'aria-label': 'Seed', 'aria-describedby': 'gen-seed-help', placeholder: 'a number or a word' });
  seed.value = st.seed;
  const prev = h('button', { type: 'button', class: 'tb', id: 'gen-prev', title: 'The previous seed ([, or Down in the seed box)', 'aria-label': 'Previous seed' }, '−1');
  const next = h('button', { type: 'button', class: 'tb', id: 'gen-next', title: 'The next seed (], or Up in the seed box)', 'aria-label': 'Next seed' }, '+1');
  const dice = h('button', { type: 'button', class: 'tb', id: 'gen-random', title: 'A random seed', 'aria-label': 'Random seed' }, '🎲');
  const len = h('select', { id: 'gen-length', 'aria-label': 'Length' }, ...LENGTHS.map((v) => h('option', { value: v }, v)));
  len.value = st.length;
  const structure = h('p', { class: 'note', id: 'gen-structure', 'aria-live': 'polite' });
  const go = h('button', { type: 'button', class: 'tb primary', id: 'gen-go', title: 'Write the program into the editor and play it (Enter)' }, 'Generate');
  const status = h('div', { role: 'status', id: 'gen-status' });
  const section = h('select', { id: 'gen-section', 'aria-label': 'Section' });
  const part = h('select', { id: 'gen-part', 'aria-label': 'Part' });
  const call = h('code', { id: 'gen-call' });
  const insert = h('button', { type: 'button', class: 'tb', id: 'gen-insert', title: 'Put this gen@2 call at the editor\'s caret' }, 'Insert as reference');

  let preview = null; // the generated answer for the current inputs (not yet in the editor)

  const req = () => genRequest({ genre: genre.value, seed: seed.value, length: len.value });

  /** Generate for the current inputs without touching the editor: the structure line and the section and part lists. */
  const refresh = () => {
    st.genre = genre.value; st.seed = seed.value.trim() || '1'; st.length = len.value;
    save();
    preview = null;
    try { preview = app.btw && app.btw.generate ? app.btw.generate(req()) : null; } catch (e) { console.warn('btw: generate preview', e); }
    if (!preview) {
      structure.textContent = app.btw ? 'The structure shows once generated (this module generates on the server).' : 'Loading the language module...';
    } else if (!preview.ok && !preview.text) {
      structure.textContent = preview.error || 'the generator refused these inputs';
    } else {
      const s = structureOf(preview.check);
      structure.textContent = s.sections.length ? `Structure: ${s.sections.map((x) => `${x.name} ${x.bars}`).join(' · ')} (${s.bars} bars)` : '';
    }
    const secs = preview && preview.check ? [...new Set(structureOf(preview.check).sections.map((x) => x.name))] : [];
    const keepSec = section.value || st.section;
    section.replaceChildren(...secs.map((n) => h('option', { value: n }, n)));
    if (secs.includes(keepSec)) section.value = keepSec;
    else { const d = secs.find((n) => /drop|chorus/.test(n)); if (d) section.value = d; }
    refreshParts();
  };
  const refreshParts = () => {
    const roles = preview && preview.text ? rolesIn(preview.text, section.value) : [];
    const keep = part.value || st.part;
    part.replaceChildren(...roles.map((r) => h('option', { value: r }, r)));
    if (roles.includes(keep)) part.value = keep;
    st.section = section.value; st.part = part.value;
    save();
    const c = genCall({ genre: genre.value, seed: seed.value, length: len.value, section: section.value, part: part.value });
    call.textContent = c;
    insert.disabled = !section.value || !part.value;
  };

  /** Write the program for the current inputs into the editor and play it. `stepping`: a seed step while it plays. */
  const generate = async ({ stepping = false } = {}) => {
    const r0 = req();
    app.ensureAudio();
    go.disabled = true;
    try {
      let r = preview && preview.text ? preview : app.btw && app.btw.generate ? app.btw.generate(r0) : null;
      if (!r) r = await app.api.generate(r0);
      if (!r.ok && !r.text) throw new Error(r.error || 'the generator refused the request');
      app.setProgram(r.text, { name: r.name || `${r0.genre} ${r0.seed}`, cid: r.cid });
      status.replaceChildren(h('div', { class: r.ok ? 'okbox' : 'err' },
        h('div', {}, h('b', {}, r.ok ? 'Generated. ' : 'Generated, but it does not check. '), `${r.generator || 'seeded'} · ${r0.genre} · seed ${r.seed}`),
        h('div', { class: 'note' }, 'CID ', h('code', { id: 'gen-cid', title: r.cid || '' }, shortCid(r.cid || '', 8, 8)), ' · CC0-1.0 · share stores it')));
      // a step while it plays is an edit (on the engine it lands on the next bar); the button is a load, like opening a program
      if (r.ok) await app.play(stepping && app.isPlaying() ? 'eval' : 'load');
    } catch (e) {
      const d = describeError(e);
      status.replaceChildren(h('div', { class: 'err' }, d.title, ...(d.lines.length ? [h('pre', {}, d.lines.join('\n'))] : [])));
    } finally {
      go.disabled = false;
    }
  };

  const step = (d) => {
    seed.value = stepSeed(seed.value, d);
    refresh();
    // the knob: while something plays, the neighbouring seed takes over at once
    if (app.isPlaying()) generate({ stepping: true });
  };

  genre.addEventListener('change', refresh);
  len.addEventListener('change', refresh);
  seed.addEventListener('change', refresh);
  seed.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') { e.preventDefault(); step(e.key === 'ArrowUp' ? 1 : -1); }
    else if (e.key === 'Enter') { e.preventDefault(); refresh(); generate(); }
  });
  prev.addEventListener('click', () => step(-1));
  next.addEventListener('click', () => step(1));
  dice.addEventListener('click', () => { seed.value = randomSeed(); refresh(); if (app.isPlaying()) generate({ stepping: true }); });
  go.addEventListener('click', () => { refresh(); generate(); });
  section.addEventListener('change', refreshParts);
  part.addEventListener('change', refreshParts);
  insert.addEventListener('click', () => {
    const ed = app.editor;
    const ta = ed.ta;
    const text = call.textContent;
    if (!text) return;
    ed.applyEdit(ta.selectionStart, ta.selectionEnd, text);
    app.toast(`Inserted ${text}`, 'ok');
  });

  const body = h('div', { class: 'gen', id: 'gen' },
    h('p', { class: 'note' }, 'The seeded generator writes a whole track from a seed, with the genre\'s rhythms, chords and song structure. No model and no sign-in: the same genre, seed and length always give the same program and CID.'),
    h('fieldset', { class: 'plain' },
      h('div', { class: 'row' }, h('label', {}, 'Genre', genre), h('label', {}, 'Length', len)),
      h('div', { class: 'row gen-seed' }, h('label', {}, 'Seed', seed), prev, next, dice),
      h('p', { class: 'note', id: 'gen-seed-help' }, 'Step the seed while it plays: −1 / +1, the [ and ] keys, or Up / Down in the seed box.'),
      structure,
      go),
    status,
    h('h3', {}, 'Insert as reference'),
    h('p', { class: 'note' }, 'One part of one section, as a call your program keeps: it is generated in your program\'s key and tempo, and pinned by its CID.'),
    h('fieldset', { class: 'plain' },
      h('div', { class: 'row' }, h('label', {}, 'Section', section), h('label', {}, 'Part', part)),
      h('p', { class: 'note' }, call),
      insert));
  // [ and ] step the seed from anywhere in the panel but a text box
  body.addEventListener('keydown', (e) => {
    const t = e.target;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA') || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === '[' || e.key === ']') { e.preventDefault(); step(e.key === ']' ? 1 : -1); }
  });
  refresh();
  return [body];
}

function safeStore() {
  try { return window.localStorage; } catch { return null; }
}
