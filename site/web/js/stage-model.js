// stage-model.js: the stage view (?view=stage), pure (site/test/unit.mjs runs it). The DOM half is stage.js.
//
// The stage view is record mode with a person in front of it: the program full screen, read-only, with the firing markers
// and the follow-scroll on, in large high-contrast type, for a projector or a second screen at a gig. It is one mode with
// two entry points, not a third mode:
//
//   ?record=1&cid=<CID> | &example=<name> [&clock=manual]   record mode, for the demo-video renderer: unchanged, byte for byte
//   ?view=stage&cid=<CID> | &example=<name> | #<share link> the stage view: record mode's page, plus a full-screen toggle
//                                                            (button and F), the set's tracklist line, the play button and the
//                                                            "enable sound" banner, with nothing that edits
//
// `?record=1` wins when both are given, so a renderer's URL never changes behaviour. The shared visual language with the
// booth's projector window (an egui viewport on a second display) is STAGE below: one type scale, the lamp colours of
// markers-model.js, and the tracklist line's wording (tracklistLine).

/** What the page is, from its query: {record, stage, manual}. `record` is the chrome-less path both entry points share. */
export function pageMode(q) {
  const get = (k) => (q && typeof q.get === 'function' ? q.get(k) : null);
  const recordParam = get('record') === '1';
  const stage = !recordParam && get('view') === 'stage';
  return { record: recordParam || stage, stage, manual: get('clock') === 'manual' };
}

/** The classes the page's <html> carries for a mode: record mode `record`, the stage view `record stage`. */
export function htmlClasses(mode) {
  if (!mode || !mode.record) return [];
  return mode.stage ? ['record', 'stage'] : ['record'];
}

/**
 * The stage's look, shared with the booth's projector window (keep the two in step: site/docs-src/api.md "Stage view").
 * Type: the code's size follows the screen's width, `width / perPx` px, held between minPx and maxPx, with a line height of
 * `lineRatio` times that (both whole pixels, so the editor's two layers wrap alike). Lamps: markers-model.js COLORS over the
 * stage's ground (dark: #000, light: #fff). The tracklist line: tracklistLine.
 */
export const STAGE = Object.freeze({
  perPx: 60,
  minPx: 18,
  maxPx: 44,
  lineRatio: 1.36,
  gutterRatio: 4.6,
  ground: Object.freeze({ dark: [0x00, 0x00, 0x00], light: [0xff, 0xff, 0xff] }),
  ink: Object.freeze({ dark: [0xf4, 0xf6, 0xfb], light: [0x10, 0x12, 0x18] }),
  hintMs: 3500,
});

/** The stage's type for a viewport width: {fs, lh, gut} in whole pixels. */
export function stageType(width) {
  const w = Number(width) > 0 ? Number(width) : 1280;
  const fs = Math.max(STAGE.minPx, Math.min(STAGE.maxPx, Math.floor(w / STAGE.perPx)));
  return { fs, lh: Math.round(fs * STAGE.lineRatio), gut: Math.round(fs * STAGE.gutterRatio) };
}

/** The lamp palette markers.js paints over-lamps with on the stage: {stage, ink} (null: the editor's own, unchanged). */
export function lampPalette(theme) {
  const t = theme === 'light' ? 'light' : 'dark';
  return { stage: STAGE.ground[t], ink: STAGE.ink[t] };
}

const CID_RE = /^z[1-9A-HJ-NP-Za-km-z]{20,120}$/;
const shortCid = (c) => `${c.slice(0, 6)}…${c.slice(-4)}`;

/**
 * The tracks of a set program, from its text (and the check's notes, which say what a `next` resolved to): one per `play`
 * or `next` line inside `set { }`, in order, [{n, line, label}] (line 0-based). [] for a program that is not a set.
 */
export function setTracks(text, report) {
  const lines = String(text || '').split('\n');
  const notes = new Map();
  for (const d of (report && report.diagnostics) || []) {
    const m = /^next: (.+?) -> /.exec(d && d.message ? d.message : '');
    if (m && d.line != null) notes.set(Number(d.line) - 1, m[1]);
  }
  const out = [];
  let inSet = false;
  let depth = 0;
  for (let i = 0; i < lines.length; i++) {
    const t = lines[i].replace(/#.*$/, '').trim();
    if (!inSet) {
      if (/^set\s*\{/.test(t)) { inSet = true; depth = 1; }
      continue;
    }
    depth += (t.match(/\{/g) || []).length - (t.match(/\}/g) || []).length;
    if (depth <= 0) break;
    let m = /^play\s+(.+?)(?:\s+until\s+.+)?$/.exec(t);
    if (m) {
      let name = m[1].trim().replace(/^"(.*)"$/, '$1');
      if (CID_RE.test(name)) name = shortCid(name);
      out.push({ n: out.length + 1, line: i, label: name });
      continue;
    }
    m = /^next\s+(seed=\S+)/.exec(t);
    if (m) out.push({ n: out.length + 1, line: i, label: notes.get(i) || `next ${m[1]}` });
  }
  return out;
}

/** Which track a set's section is (the set's sections are named `N. <the track's section>` and `mix A→B`): {n} or {n, to}. */
export function trackOfSection(name) {
  const s = String(name || '');
  let m = /^mix (\d+)→(\d+)/.exec(s);
  if (m) return { n: Number(m[1]), to: Number(m[2]) };
  m = /^(\d+)\.\s/.exec(s);
  return m ? { n: Number(m[1]) } : null;
}

/** The section a 1-based song bar is in, from the check's sections ([{name, start, bars}], start 0-based). */
export function sectionAt(sections, songBar) {
  const b = Number(songBar) - 1;
  if (!(b >= 0)) return null;
  for (const s of sections || []) if (b >= s.start && b < s.start + s.bars) return s.name;
  return null;
}

/**
 * Where a set is: {n, total, now, next, to} (to: the track a mix goes into), or null when the program is not a set or
 * nothing plays. `section` is the frame's section name when the runtime gives one, else it is found from `songBar`.
 */
export function setPosition(tracks, { section, songBar, sections } = {}) {
  if (!tracks || !tracks.length) return null;
  const at = trackOfSection(section || sectionAt(sections, songBar));
  if (!at || !(at.n >= 1) || at.n > tracks.length) return null;
  const cur = tracks[at.n - 1];
  const nextT = tracks[(at.to || at.n + 1) - 1] || null;
  return { n: at.n, total: tracks.length, now: cur.label, next: nextT ? nextT.label : null, to: at.to || null, line: cur.line };
}

/** The tracklist line (the booth's projector window says the same): `2/5  now X  ·  next Y`, `mix 2→3  X → Y`, `5/5  now X  ·  last`. */
export function tracklistLine(pos) {
  if (!pos) return '';
  if (pos.to) return `mix ${pos.n}→${pos.to}  ${pos.now} → ${pos.next || ''}`.trim();
  return `${pos.n}/${pos.total}  now ${pos.now}  ·  ${pos.next ? `next ${pos.next}` : 'last'}`;
}

/**
 * The Fullscreen API with its prefixes (Safari's webkit names): {native, active(), request(), exit(), event}. `native`
 * false where the element cannot go full screen (Safari on iPhone has no element full screen): the stage then fills the
 * window itself (the `stage-max` class) and says how to lose the browser's bars.
 */
export function fullscreenOf(doc) {
  const el = doc && doc.documentElement;
  const req = el && (el.requestFullscreen || el.webkitRequestFullscreen);
  const exit = doc && (doc.exitFullscreen || doc.webkitExitFullscreen);
  const enabled = doc && (doc.fullscreenEnabled ?? doc.webkitFullscreenEnabled);
  const native = !!(req && exit && enabled !== false);
  return {
    native,
    event: doc && 'onfullscreenchange' in doc ? 'fullscreenchange' : 'webkitfullscreenchange',
    active: () => !!(doc && (doc.fullscreenElement || doc.webkitFullscreenElement)),
    request: () => (native ? Promise.resolve(req.call(el, { navigationUI: 'hide' })) : Promise.reject(new Error('no element full screen here'))),
    exit: () => (native ? Promise.resolve(exit.call(doc)) : Promise.resolve()),
  };
}

/** What a key does on the stage: 'fullscreen' (F), 'exit' (Escape), 'block' (G, which opens an editing panel), or null. */
export function stageKey(e) {
  if (!e || e.metaKey || e.ctrlKey || e.altKey) return null;
  if (e.key === 'f' || e.key === 'F') return 'fullscreen';
  if (e.key === 'Escape') return 'exit';
  if (e.key === 'g' || e.key === 'G') return 'block';
  return null;
}

/** The hint the stage shows when it goes full screen (and where it cannot, how to get there). */
export function exitHint(native, iosLike) {
  if (native) return 'Full screen: press F or Esc to leave';
  return iosLike ? 'Filling the window. For no browser bars: Share, then Add to Home Screen. Tap ⛶ or press F to leave' : 'Filling the window: press F or Esc to leave';
}
