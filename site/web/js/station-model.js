// The station model, page side (BeatScript radio, stations v1). A station is a BeatScript program: a `station { }` block
// (`from` the first track's generator call, `next` the step from one track to the next, `forms` the rotation), and its
// `tempo` and `key`. Position N of a station is always the same track. Listeners do not steer a station: they tune it,
// turning one knob a notch (to a neighbouring station that already exists) or to a value, and they lock knobs; a retune
// keeps the position N.
//
// The server (beatscript-api::station, POST /api/station, and later the page wasm's beatscript_station) is where a station's answer is
// made: its canonical text, CIDs, tracks and segues. This module holds what the page needs to ask it and to show its
// answer: the knobs and their notch rules, locks, the position N, now playing and up next, the request JSON, applying an
// answer, scan (a neighbour picked from the station's CID and N, never Math.random) and the share URL. It also writes the
// walk and the tracks the way the spec says (seedAt, trackOf, segueOf): the page's JS mock answers with them, and the
// tests pin them.
//
// Pure: no DOM, no clock, no network. A CID is whatever `cidOf(text)` the caller passes (the module's check, or the mock's).

import { CID_RE, MAX_LOAD_TEXT } from './util.js';

export const VERSION = 'stations/1';
/** The seeded generator's genres (beatscript-api::generate::genres, beats' catalogue): the genre knob's values. */
export const SEEDED_GENRES = ['techno', 'house', 'jazz', 'ambient', 'dnb', 'edm', 'speedcore', 'dubstep', 'country', 'blues', 'synthwave', 'trance', 'lofi', 'reggae', 'funk', 'bossa', 'trap', 'garage', 'metal'];
/** The knobs, in the order the page shows them. */
export const KNOBS = ['genre', 'seed', 'step', 'forms', 'tempo', 'key', 'length'];
/** `seeded` is the generator's own arrangement; the other six are the seed-taking forms. */
export const FORMS = ['seeded', 'passacaglia', 'canon', 'rondo', 'fugue', 'theme-and-variations', 'cross'];
export const LENGTHS = ['short', 'standard', 'long'];
export const TEMPO_MIN = 40;
export const TEMPO_MAX = 300;
export const TEMPO_NOTCH = 2;
/** `next seed=~` is O(N): the server walks at most this far (a position past it is refused). */
export const MAX_N_WALK = 10000;
/** `next seed=+n` is O(1): any 32-bit position. */
export const MAX_N = 0xffffffff;
/** The phrasebook lines stations v1 pins (a track's phrase is line s_N mod this; the server's list has 46). */
export const PHRASES = 46;
/** The genres that play no arp or harmony on the bar's grid: their rondo and cross bindings say part=melody. */
export const SPARSE_GENRES = ['ambient', 'bossa', 'jazz', 'lofi'];
/** How many tracks the page asks for at once: now playing and up next. */
export const WINDOW = 2;

export const DEFAULT_STATION = `#! name: techno radio
#! license: CC0-1.0
tempo 128
key am
station {
  from gen@2 techno seed=42
  next seed=~
  forms seeded passacaglia rondo fugue
}
`;

const U32 = 2n ** 32n;
const enc = new TextEncoder();

/** FNV-1a 32 over the UTF-8 bytes of `s`. */
export function fnv1a32(s) {
  let h = 0x811c9dc5;
  for (const b of enc.encode(String(s))) h = Math.imul(h ^ b, 0x01000193) >>> 0;
  return h >>> 0;
}

/** A seed as written -> the generator's 32-bit seed: a whole number is itself (mod 2^32), a word its FNV-1a of the lowercased text. */
export function seedValue(seed) {
  const s = String(seed ?? '').trim();
  if (/^\d+$/.test(s)) return Number(BigInt(s) % U32);
  return fnv1a32(s.toLowerCase());
}

const SEED_RE = /^(\d{1,20}|[A-Za-z][A-Za-z0-9_-]{0,63})$/;
const GENRE_RE = /^[a-z][a-z0-9-]{0,31}$/;
const STEP_RE = /^(\+[1-9]\d{0,9}|~)$/;

// ---- keys: one notch is one step round the circle of fifths, in the key's mode --------------------------------------------

const PC = { c: 0, d: 2, e: 4, f: 5, g: 7, a: 9, b: 11 };
const MAJOR_SPELL = ['c', 'db', 'd', 'eb', 'e', 'f', 'f#', 'g', 'ab', 'a', 'bb', 'b'];
const MINOR_SPELL = ['c', 'c#', 'd', 'eb', 'e', 'f', 'f#', 'g', 'g#', 'a', 'bb', 'b'];
const MODES = ['major', 'minor', 'dorian', 'mixolydian', 'phrygian', 'harmonic', 'pent', 'minpent', 'blues'];
const MINORISH = new Set(['m', 'minor', 'dorian', 'phrygian', 'harmonic', 'minpent', 'blues']);

/** `am`, `c`, `f#m`, `bb`, `d:dorian` -> {pc, mode} (mode '' major, 'm' minor, else the `:mode`), or null. */
export function parseKey(k) {
  const s = String(k ?? '').trim().toLowerCase();
  let m = /^([a-g])([#b]?):([a-z]+)$/.exec(s);
  if (m) return MODES.includes(m[3]) ? { pc: (PC[m[1]] + (m[2] === '#' ? 1 : m[2] === 'b' ? 11 : 0)) % 12, mode: m[3] === 'major' ? '' : m[3] === 'minor' ? 'm' : m[3] } : null;
  m = /^([a-g])([#b]?)(m?)$/.exec(s);
  if (!m) return null;
  return { pc: (PC[m[1]] + (m[2] === '#' ? 1 : m[2] === 'b' ? 11 : 0)) % 12, mode: m[3] };
}

function keyText({ pc, mode }) {
  const name = (MINORISH.has(mode) ? MINOR_SPELL : MAJOR_SPELL)[((pc % 12) + 12) % 12];
  return mode === '' ? name : mode === 'm' ? `${name}m` : `${name}:${mode}`;
}

// ---- notches ------------------------------------------------------------------------------------------------------------

/**
 * One notch of a knob from `value` in direction `dir` (+1 or -1): the neighbour's value, or null when there is none
 * (genre: never a notch in phase 1; a tempo past 40..300; a length past short or long; forms of one name).
 *   seed +-1 (mod 2^32; a word seed counts from its hash), tempo +-2, key one step round the circle of fifths in its mode
 *   (am -> em / dm), step +1 <-> ~ (either direction), forms rotated by one (left for +1, right for -1; a station of one
 *   form steps through the seven in FORMS order, round), length one step (short standard long).
 */
export function notch(knob, value, dir) {
  const d = dir < 0 ? -1 : 1;
  switch (knob) {
    case 'seed': return String((seedValue(value) + d + 0x100000000) % 0x100000000);
    case 'tempo': {
      const t = Number(value);
      if (!Number.isFinite(t)) return null;
      const v = t + d * TEMPO_NOTCH;
      return v >= TEMPO_MIN && v <= TEMPO_MAX ? v : null;
    }
    case 'key': {
      const k = parseKey(value);
      return k ? keyText({ pc: (k.pc + (d > 0 ? 7 : 5)) % 12, mode: k.mode }) : null;
    }
    case 'step': return String(value) === '~' ? '+1' : '~';
    case 'forms': {
      const f = Array.isArray(value) ? value : [];
      if (!f.length) return null;
      if (f.length === 1) {
        const i = FORMS.indexOf(f[0]);
        return i < 0 ? null : [FORMS[(i + d + FORMS.length) % FORMS.length]];
      }
      return d > 0 ? [...f.slice(1), f[0]] : [f[f.length - 1], ...f.slice(0, -1)];
    }
    case 'length': {
      const i = LENGTHS.indexOf(String(value || 'standard'));
      const j = i + d;
      return i >= 0 && j >= 0 && j < LENGTHS.length ? LENGTHS[j] : null;
    }
    default: return null; // genre: an explicit value only
  }
}

const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/**
 * The one-notch neighbours along each unlocked knob: [{knob, dir: '+1'|'-1', value}], never a locked knob and never genre.
 * A notch that lands where another of the same knob does (step's toggle, forms of two names) is listed once.
 */
export function neighbourOptions(knobs, locks = []) {
  const held = new Set(locks);
  const out = [];
  for (const knob of KNOBS) {
    if (knob === 'genre' || held.has(knob)) continue;
    for (const dir of [1, -1]) {
      const value = notch(knob, knobs[knob], dir);
      if (value == null || out.some((o) => o.knob === knob && same(o.value, value))) continue;
      out.push({ knob, dir: dir > 0 ? '+1' : '-1', value });
    }
  }
  return out;
}

// ---- the station program -------------------------------------------------------------------------------------------------

/** The code of a line: before a `#` that starts the line or follows a space. */
function codeOf(raw) {
  for (let j = 0; j < raw.length; j++) if (raw[j] === '#' && (j === 0 || /\s/.test(raw[j - 1]))) return raw.slice(0, j);
  return raw;
}

/** A generator call's words (`gen@2 techno seed=42 length=long`) -> {generator, genre, seed, length} or {error}. */
export function parseGenCall(words) {
  const [g, genre, ...args] = words;
  if (!g || !/^gen@\d+$/.test(g)) return { error: `a station's \`from\` is a generator call, \`gen@2 GENRE seed=N\`: found \`${g || 'nothing'}\`` };
  if (!genre || !GENRE_RE.test(genre)) return { error: `\`${g}\` wants a genre (\`${g} techno seed=42\`)` };
  const out = { generator: g, genre, seed: null, length: null };
  for (const a of args) {
    const m = /^([a-z]+)=(.*)$/.exec(a);
    if (!m) return { error: `\`${a}\` is not \`name=value\`` };
    const [, k, v] = m;
    if (['section', 'part', 'bpm', 'key'].includes(k)) return { error: `a station's \`from\` takes no \`${k}=\`: ${k === 'bpm' || k === 'key' ? "a station's tempo and key are its `tempo` and `key` lines" : 'a station plays whole tracks'}` };
    if (k === 'seed') {
      if (out.seed != null) return { error: '`seed=` twice' };
      if (!SEED_RE.test(v)) return { error: `\`seed=${v}\`: a seed is a whole number or a word` };
      out.seed = v;
    } else if (k === 'length') {
      if (out.length != null) return { error: '`length=` twice' };
      if (!LENGTHS.includes(v)) return { error: `\`length=${v}\`: short, standard or long` };
      out.length = v;
    } else return { error: `a station's \`from\` takes seed= and length=, not \`${k}=\`` };
  }
  if (out.seed == null) return { error: `\`${g} ${genre}\` needs a seed (\`seed=42\`)` };
  return out;
}

/**
 * A station program -> {ok, knobs, generator, header, said, errors}. `knobs` are the program's values with the defaults
 * filled in (step +1, forms seeded, length standard); `tempo` and `key` are null when the program does not say them (they
 * are then the from call's own, which only the generator knows: the server's answer says them). `header` keeps the `#!`
 * lines, which are not in the CID. errors: [{severity, message, line, col}].
 */
export function parseStation(text) {
  const errors = [];
  const err = (line, col, message) => errors.push({ severity: 'error', message, line, col });
  const header = [];
  let tempo = null;
  let key = null;
  let from = null;
  let step = null;
  let forms = null;
  let block = 0; // 0 before, 1 inside, 2 after
  let blockLine = 0;
  const src = String(text ?? '');
  if (src.length > MAX_LOAD_TEXT) return { ok: false, errors: [{ severity: 'error', message: `a station program is at most ${MAX_LOAD_TEXT} characters`, line: 1, col: 1 }] };
  src.split('\n').forEach((raw, i) => {
    const line = i + 1;
    const t = raw.trim();
    if (t.startsWith('#!')) {
      header.push(t);
      const m = /^#!\s*stations:\s*(.*)$/.exec(t);
      if (m && m[1].trim() !== 'v1') err(line, 1, `\`#! stations: ${m[1].trim()}\`: v1 is the only version`);
      return;
    }
    const code = codeOf(raw);
    const c = code.trim();
    if (!c) return;
    const col = code.length - code.trimStart().length + 1;
    const w = c.split(/\s+/);
    if (block === 1) {
      if (c === '}') { block = 2; return; }
      if (w[0] === 'from') {
        if (from) return err(line, col, 'one `from` a station');
        const g = parseGenCall(w.slice(1));
        if (g.error) return err(line, col + 5, g.error);
        from = g;
      } else if (w[0] === 'next') {
        if (step) return err(line, col, 'one `next` a station');
        const m = w.length === 2 && /^seed=(.*)$/.exec(w[1]);
        if (!m || !STEP_RE.test(m[1])) return err(line, col + 5, '`next` is `next seed=+N` (N at least 1) or `next seed=~`');
        step = m[1];
      } else if (w[0] === 'forms') {
        if (forms) return err(line, col, 'one `forms` a station');
        const names = w.slice(1);
        if (!names.length) return err(line, col, '`forms` names at least one form');
        const bad = names.find((n) => !FORMS.includes(n));
        if (bad) return err(line, col + c.indexOf(bad), `\`${bad}\` is not a form: ${FORMS.join(', ')}`);
        forms = names;
      } else err(line, col, `a station's block holds \`from\`, \`next\` and \`forms\` lines: found \`${w[0]}\``);
      return;
    }
    if (/^station\s*\{$/.test(c)) {
      if (block) return err(line, col, 'one `station { }` block a program');
      block = 1;
      blockLine = line;
      return;
    }
    if (w[0] === 'tempo') {
      const v = Number(w[1]);
      if (w.length !== 2 || !Number.isFinite(v) || v < TEMPO_MIN || v > TEMPO_MAX) return err(line, col, `\`tempo\` is a number from ${TEMPO_MIN} to ${TEMPO_MAX}`);
      if (tempo != null) return err(line, col, 'one `tempo` a station');
      tempo = v;
      return;
    }
    if (w[0] === 'key') {
      if (w.length !== 2 || !parseKey(w[1])) return err(line, col, '`key` is a key: `am`, `c`, `f#m`, `d:dorian`');
      if (key != null) return err(line, col, 'one `key` a station');
      key = w[1].toLowerCase();
      return;
    }
    err(line, col, `a station program holds its \`station { }\` block, \`tempo\`, \`key\` and comments, nothing else: found \`${w[0]}\``);
  });
  if (block === 1) err(blockLine, 1, 'the `station {` block is never closed: add a `}`');
  if (!block) err(1, 1, 'a station program has a `station { }` block');
  else if (!from && block === 2 && !errors.length) err(blockLine, 1, 'a station needs a `from` line: `from gen@2 techno seed=42`');
  if (errors.length) return { ok: false, errors, header };
  const knobs = { genre: from.genre, seed: from.seed, step: step || '+1', forms: forms || ['seeded'], tempo, key, length: from.length || 'standard' };
  return { ok: true, errors, header, generator: from.generator, knobs, said: { tempo: tempo != null, key: key != null, length: from.length != null } };
}

/** The canonical station program for `knobs` (tempo/key written only when not null), after the `#!` header lines. */
export function stationText(knobs, { generator = 'gen@2', header = [] } = {}) {
  const k = knobs;
  const lines = [...header];
  if (k.tempo != null) lines.push(`tempo ${k.tempo}`);
  if (k.key) lines.push(`key ${k.key}`);
  lines.push('station {');
  lines.push(`  from ${genCall(generator, k.genre, k.seed, k.length)}`);
  if (k.step && k.step !== '+1') lines.push(`  next seed=${k.step}`);
  if (k.forms && !same(k.forms, ['seeded'])) lines.push(`  forms ${k.forms.join(' ')}`);
  lines.push('}');
  return `${lines.join('\n')}\n`;
}

/** `gen@2 techno seed=42` (with ` length=L` unless it is standard, and any extra words). */
export function genCall(generator, genre, seed, length, extra = '') {
  return `${generator} ${genre} seed=${seed}${length && length !== 'standard' ? ` length=${length}` : ''}${extra ? ` ${extra}` : ''}`;
}

// ---- retune and locks -------------------------------------------------------------------------------------------------------

/** Whether `v` is a value knob `knob` may be retuned to (an explicit value, not a notch). */
export function validValue(knob, v) {
  switch (knob) {
    case 'genre': return typeof v === 'string' && SEEDED_GENRES.includes(v); // the generator's genres (the server refuses any other)
    case 'seed': return SEED_RE.test(String(v));
    case 'step': return STEP_RE.test(String(v));
    case 'forms': return Array.isArray(v) && v.length > 0 && v.length <= 32 && v.every((f) => FORMS.includes(f));
    case 'tempo': return typeof v === 'number' && Number.isFinite(v) && v >= TEMPO_MIN && v <= TEMPO_MAX;
    case 'key': return !!parseKey(v);
    case 'length': return LENGTHS.includes(v);
    default: return false;
  }
}

const isNotch = (v) => v === '+1' || v === '-1';

/**
 * How an explicit step is asked for in a retune: `seed=+N` or `~` (a bare "+1" / "-1" is a notch: on a `+3` station "+1"
 * toggles to `~`, it does not set +1).
 */
export const stepRetuneValue = (step) => (String(step) === '~' ? '~' : `seed=${step}`);

/**
 * Set knobs: `changes` is {knob: value | "+1" | "-1"}. `knobs` are the station's values (tempo/key as the answer resolved them
 * when the text does not say them). Returns {ok: true, knobs, retuned} or {ok: false, error, knob}: a locked knob, an
 * unknown knob, a value the knob cannot take, a notch that has no neighbour, or genre by a notch, are refused.
 * The position N is not here: a retune keeps it (stationRequest sends the same n).
 */
export function retune(knobs, changes, locks = []) {
  const held = new Set(locks);
  const out = { ...knobs, forms: [...(knobs.forms || [])] };
  const retuned = [];
  for (const [knob, raw] of Object.entries(changes || {})) {
    if (!KNOBS.includes(knob)) return { ok: false, knob, error: `\`${knob}\` is not a knob: ${KNOBS.join(', ')}` };
    if (held.has(knob)) return { ok: false, knob, error: `${knob} is locked: unlock it to retune it` };
    let v = raw;
    // "+1" / "-1" is a notch on every knob (for step too: a value is `seed=+N` or `~`, stepRetuneValue)
    if (isNotch(v)) {
      if (knob === 'genre') return { ok: false, knob, error: 'genre retunes only by a value (its neighbourhoods are phase 2)' };
      const n = notch(knob, knobs[knob], Number(v));
      if (n == null) return { ok: false, knob, error: `${knob} has no neighbour ${Number(v) > 0 ? 'up' : 'down'} from ${fmtValue(knob, knobs[knob])}` };
      v = n;
    } else {
      if (knob === 'forms' && typeof v === 'string') v = v.trim().split(/\s+/).filter(Boolean);
      if (knob === 'tempo' && typeof v === 'string' && v.trim() !== '') v = Number(v);
      if (knob === 'seed' && typeof v === 'number') v = String(v);
      if (knob === 'key' && typeof v === 'string') v = v.trim().toLowerCase();
      if (knob === 'step') {
        const m = /^seed=(.*)$/.exec(String(v));
        if (m) v = m[1];
        else if (v !== '~') return { ok: false, knob, error: `step is set as \`seed=+N\` or \`~\` ("+1" and "-1" turn it a notch): not ${JSON.stringify(raw)}` };
      }
      if (!validValue(knob, v)) return { ok: false, knob, error: `${knob} cannot be ${JSON.stringify(raw)}` };
    }
    out[knob] = v;
    if (!retuned.includes(knob)) retuned.push(knob);
  }
  return { ok: true, knobs: out, retuned };
}

/** A knob's value in words: forms joined, everything else as it is. */
export function fmtValue(knob, v) {
  if (v == null || v === '') return knob === 'tempo' || knob === 'key' ? "the genre's" : '';
  return Array.isArray(v) ? v.join(' ') : String(v);
}

/** Toggle a lock: the new list of locked knobs (in KNOBS order). */
export function toggleLock(locks, knob) {
  if (!KNOBS.includes(knob)) return [...locks];
  const s = new Set(locks);
  if (s.has(knob)) s.delete(knob);
  else s.add(knob);
  return KNOBS.filter((k) => s.has(k));
}

// ---- the walk, the tracks and the segues (the spec's rules, for the mock and the tests) -----------------------------------------

/** The step rule: {kind: '+', n} or {kind: '~'}. */
export function stepRule(step) {
  const s = String(step || '+1');
  return s === '~' ? { kind: '~' } : { kind: '+', n: Number(s.slice(1)) || 1 };
}

/** The farthest position a station reaches: `~` walks, so it is capped. */
export const maxN = (step) => (stepRule(step).kind === '~' ? MAX_N_WALK : MAX_N);

/**
 * The seed at position N, as written in the track (track 0 keeps a word seed's word; every other is a number). For
 * `next seed=~`, cidOf(text) is the CID of a program's text and `memo` (an array) keeps the walk between calls.
 */
export function seedAt(knobs, generator, N, cidOf, memo = null) {
  if (N === 0) return String(knobs.seed);
  const rule = stepRule(knobs.step);
  if (rule.kind === '+') return String(Number((BigInt(seedValue(knobs.seed)) + BigInt(rule.n) * BigInt(N)) % U32));
  if (N > MAX_N_WALK) throw new Error(`a \`next seed=~\` station reaches position ${MAX_N_WALK} at most`);
  const walk = memo || [];
  if (!walk.length) walk.push(String(knobs.seed));
  while (walk.length <= N) {
    const prev = walk[walk.length - 1];
    const cid = cidOf(`arrange from ${genCall(generator, knobs.genre, prev, knobs.length)}\n`);
    if (!cid) throw new Error('the walk needs the CID of a generator call');
    walk.push(String(fnv1a32(cid)));
  }
  return walk[N];
}

/** The phrasebook line a seed picks (0-based; stations v1 pins the list of PHRASES ids). */
export const phraseIndex = (s) => seedValue(s) % PHRASES;

const sparse = (genre) => SPARSE_GENRES.includes(genre);

/** The `arrange from` call of form `form` with theme seed `t` and fresh seed `s`, phrase `@p`. */
export function formCall(form, { generator, genre, length }, t, s, phrase) {
  const g = (seed, extra = '') => genCall(generator, genre, seed, length, extra);
  const melody = sparse(genre) ? 'part=melody' : '';
  switch (form) {
    case 'seeded': return g(s);
    case 'passacaglia': return `template(passacaglia, ${phrase}, ground=${g(t)}, upper=${g(s)})`;
    case 'canon': return `template(canon, ${phrase}, subject=${g(s)}, ground=${g(t)})`;
    case 'rondo': return `template(rondo, ${phrase}, refrain=${g(t)}, episodes=${g(s, sparse(genre) ? 'part=melody' : 'part=arp')})`;
    case 'fugue': return `template(fugue, ${phrase}, subject=${g(t)}, counter=${g(s)})`;
    case 'theme-and-variations': return `template(theme-and-variations, ${phrase}, theme=${g(t)})`;
    case 'cross': return `template(cross, ${phrase}, a=${g(t, melody)}, b=${g(s, melody)})`;
    default: throw new Error(`\`${form}\` is not a form`);
  }
}

/**
 * Track N of a station: {n, seed, theme, form, phrase, text} (text without its CID; `parent` is the caller's: the CID of
 * track N-1). `env` = {generator, cidOf, phraseId(k), tempo, key (resolved), stationCid, parentCid, memo}.
 */
export function trackOf(knobs, N, env) {
  const R = knobs.forms.length;
  const form = knobs.forms[N % R];
  const s = seedAt(knobs, env.generator, N, env.cidOf, env.memo);
  const t = seedAt(knobs, env.generator, Math.floor(N / R) * R, env.cidOf, env.memo);
  const phrase = env.phraseId(phraseIndex(s));
  const call = formCall(form, { generator: env.generator, genre: knobs.genre, length: knobs.length }, t, s, phrase);
  const head = [`#! station: ${env.stationCid} track ${N}`];
  if (N >= 1 && env.parentCid) head.push(`#! from: next(${env.parentCid}) seed=${knobs.step}`);
  const text = `${head.join('\n')}\ntempo ${env.tempo}\nkey ${env.key}\narrange from ${call}\n`;
  return { n: N, seed: numOrWord(s), theme: numOrWord(t), form, phrase, text };
}

/** The segue from track N to N+1: a cross of their two generator calls, its phrase line (s_N + s_{N+1}) mod PHRASES. */
export function segueOf(knobs, N, env) {
  const a = seedAt(knobs, env.generator, N, env.cidOf, env.memo);
  const b = seedAt(knobs, env.generator, N + 1, env.cidOf, env.memo);
  const phrase = env.phraseId((seedValue(a) + seedValue(b)) % PHRASES);
  const melody = sparse(knobs.genre) ? 'part=melody' : '';
  const g = (seed) => genCall(env.generator, knobs.genre, seed, knobs.length, melody);
  const text = `#! station: ${env.stationCid} segue ${N} to ${N + 1}\ntempo ${env.tempo}\nkey ${env.key}\narrange from template(cross, ${phrase}, a=${g(a)}, b=${g(b)})\n`;
  return { from: N, to: N + 1, form: 'cross', phrase, text };
}

const numOrWord = (s) => (/^\d+$/.test(String(s)) ? Number(s) : String(s));

// ---- the page's state: position, now playing, up next ---------------------------------------------------------------------------

/** A fresh state for a station text at position n (nothing answered yet). */
export function initialState({ text = DEFAULT_STATION, n = 0, segues = true, locks = [] } = {}) {
  const p = parseStation(text);
  const knobs = p.ok ? p.knobs : null;
  return { text, cid: null, station: null, knobs, n: clampN(n, knobs && knobs.step), segues: !!segues, locks: KNOBS.filter((k) => locks.includes(k)), now: null, next: null, segue: null, neighbours: [], error: null };
}

function clampN(n, step) {
  const v = Math.floor(Number(n));
  if (!Number.isFinite(v) || v < 0) return 0;
  return Math.min(v, maxN(step));
}

/**
 * The request JSON for the station answer: {station, n, count, segue, check, retune?, locks?, neighbours?}. A retune sends
 * the same n: the new station's track N is what plays next. `count` never asks past the station's reach (a `~` station's
 * last position is MAX_N_WALK); `step` is the station's step after the retune, when it changes it.
 */
export function stationRequest(state, { retune: changes = null, n = state.n, count = WINDOW, check = false, neighbours = false, step = state.knobs && state.knobs.step } = {}) {
  const req = { station: state.text, n, count: Math.max(1, Math.min(count, maxN(step) - n + 1)), segue: !!state.segues, check: !!check };
  if (changes && Object.keys(changes).length) req.retune = { ...changes };
  if (state.locks && state.locks.length) req.locks = [...state.locks];
  if (neighbours) req.neighbours = true;
  return req;
}

/**
 * A retune, checked here first (a locked knob is refused before anything is asked): {ok, request} or {ok: false, error}.
 * `changes` is {knob: value | "+1" | "-1"}; what is sent is what was asked (a notch as "+1"/"-1": the server's notch).
 */
export function retuneRequest(state, changes, opts = {}) {
  if (!state.knobs) return { ok: false, error: 'the station has not been answered yet' };
  const r = retune(state.knobs, changes, state.locks);
  if (!r.ok) return r;
  return { ok: true, request: stationRequest(state, { ...opts, retune: changes, step: r.knobs.step }), knobs: r.knobs };
}

/** The knobs of an answer's station object (seed as a string, as written). */
export function knobsOf(st) {
  return { genre: st.genre, seed: String(st.seed), step: String(st.step || '+1'), forms: Array.isArray(st.forms) && st.forms.length ? [...st.forms] : ['seeded'], tempo: st.tempo ?? null, key: st.key ?? null, length: st.length || 'standard' };
}

/**
 * Apply an answer: {state, error}. An error answer ({ok: false, error, diagnostics}) leaves the state as it was, with
 * `error` = {message, diagnostics}. Otherwise the station is the answer's (its text, CID and knobs), N is the answer's n
 * (a retune's is the n it was asked at), now playing is track n, up next track n+1, and the segue the one from n. The
 * locks stay the page's own: the answer echoes the ones it was asked with, and a lock toggled since then must hold.
 */
export function applyAnswer(state, ans) {
  if (!ans || typeof ans !== 'object' || ans.ok !== true || !ans.station) {
    const message = (ans && typeof ans.error === 'string' && ans.error) || 'the station answer is not one';
    const error = { message, diagnostics: (ans && Array.isArray(ans.diagnostics)) ? ans.diagnostics : [] };
    return { state: { ...state, error }, error };
  }
  const n = Number.isInteger(ans.n) ? ans.n : state.n;
  const tracks = Array.isArray(ans.tracks) ? ans.tracks : [];
  const segues = Array.isArray(ans.segues) ? ans.segues : [];
  const next = {
    ...state,
    text: ans.station.text,
    cid: ans.station.cid || null,
    station: ans.station,
    knobs: knobsOf(ans.station),
    n,
    locks: state.locks,
    now: tracks.find((t) => t.n === n) || null,
    next: tracks.find((t) => t.n === n + 1) || null,
    segue: segues.find((s) => s.from === n) || null,
    neighbours: Array.isArray(ans.neighbours) ? ans.neighbours : [],
    retuned: Array.isArray(ans.retuned) ? ans.retuned : [],
    error: null,
  };
  return { state: next, error: null };
}

/** Move to position N (clamped to 0 .. the station's reach). What plays there is asked for again. */
export function stepTo(state, N) {
  const n = clampN(N, state.knobs && state.knobs.step);
  if (n === state.n) return state;
  return { ...state, n, now: null, next: null, segue: null };
}

/** One position forward (+1) or back (-1). */
export const stepBy = (state, d) => stepTo(state, state.n + (d < 0 ? -1 : 1));

/** Track N ended (and its segue, if any): N+1 is now playing; what is up next is asked for. */
export function advance(state) {
  const n = clampN(state.n + 1, state.knobs && state.knobs.step);
  if (n === state.n) return state;
  return { ...state, n, now: state.next && state.next.n === n ? state.next : null, next: null, segue: null };
}

/**
 * The play order from here, for a player of the station (the /radio page and the player package's <beatscript-radio>): the
 * queue's items first, else the segue after the station track that plays, then the station's next tracks (a few of them:
 * one that fails to resolve is passed over for the one after it). `current` is what plays ({kind: 'track' | 'segue' |
 * 'queue' | ..., n}) or null; `ahead` the answer's tracks after N; `started` whether a station track has played. Entries are
 * {kind: 'queue', item} | {kind: 'segue', segue} | {kind: 'track', track}.
 */
export function playOrder(state, { current = null, ahead = [], queue = [], started = false, depth = 3 } = {}) {
  const out = [];
  for (const item of queue || []) out.push({ kind: 'queue', item });
  const seg = state.segue;
  if (!out.length && current && current.kind === 'track' && state.segues && seg && seg.from === current.n && current.n === state.n) out.push({ kind: 'segue', segue: seg });
  // the station goes on after the track it is at (or plays track N when nothing of it has played yet)
  const played = current && (current.kind === 'track' || current.kind === 'segue');
  const nextN = played || started ? state.n + 1 : state.n;
  const pool = [state.now, state.next, ...(ahead || [])].filter(Boolean);
  for (let n = nextN; n < nextN + depth; n++) {
    const track = pool.find((x) => x.n === n);
    if (track) out.push({ kind: 'track', track });
  }
  return out;
}

/** A station track at position n started playing: the state moves there (one on is an advance; anything else a step). */
export function trackStarted(state, n) {
  if (n === state.n) return state;
  return n === state.n + 1 ? advance(state) : stepTo(state, n);
}

// ---- scan ------------------------------------------------------------------------------------------------------------------------

/**
 * Scan: a one-notch neighbour along an unlocked knob, picked from the station's CID and N (FNV-1a of "cid#N"), so the same
 * station at the same position always scans the same way. null when every knob is locked (or the station has no CID yet).
 */
export function scanPick(cid, n, options) {
  if (!cid || !options || !options.length) return null;
  return options[fnv1a32(`${cid}#${n}`) % options.length];
}

/** The retune a scan asks for: {ok, pick, request} or {ok: false, error}. */
export function scanRequest(state, opts = {}) {
  if (!state.knobs || !state.cid) return { ok: false, error: 'the station has not been answered yet' };
  const pick = scanPick(state.cid, state.n, neighbourOptions(state.knobs, state.locks));
  if (!pick) return { ok: false, error: 'every knob that turns is locked: unlock one to scan' };
  const r = retuneRequest(state, { [pick.knob]: pick.dir }, opts);
  return r.ok ? { ...r, pick } : r;
}

// ---- the share URL ---------------------------------------------------------------------------------------------------------------

/** `/radio?s=<the station text, or its CID>&n=N`. The text form carries the station; the CID form needs it stored. */
export function shareUrl(state, { byCid = false } = {}) {
  const s = byCid && state.cid ? state.cid : state.text;
  return `/radio?s=${encodeURIComponent(s)}&n=${state.n}`;
}

/** A share URL's query (a string or URLSearchParams) -> {text} | {cid} | {} with n (0 when it is not a whole number). */
export function parseShare(search) {
  const q = typeof search === 'string' ? new URLSearchParams(search.startsWith('?') ? search.slice(1) : search) : search;
  const out = {};
  const s = q.get('s');
  if (s != null && s !== '') {
    if (CID_RE.test(s.trim())) out.cid = s.trim();
    else if (s.length <= MAX_LOAD_TEXT) out.text = s;
  }
  const n = q.get('n');
  out.n = n != null && /^\d{1,10}$/.test(n) ? Math.min(Number(n), MAX_N) : 0;
  return out;
}
