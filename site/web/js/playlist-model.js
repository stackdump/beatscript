// playlist-model.js: the radio player's playlist, pure (site/test/unit.mjs runs it on a fake clock).
//
// The playlist is one list of what will play, in order: the "up next" queue first (the items a person or an MCP caller
// queued, which play before the station goes on), then the station's own upcoming tracks, N+1, N+2, ... (seed-derived:
// position N of a station is always the same track). Each row carries what the list shows (title, genre, BPM, duration),
// its render status (none | queued | rendering | ready | failed), and where it will play from: the render (an Ogg Opus
// file the server made) or the live engine in this tab.
//
// RenderTracker keeps those statuses fresh without hammering the server: one GET /api/render/<cid>/status per CID when a
// row first shows, then only rows that are rendering are asked again (2 s, growing to 15 s), never more than two at a
// time, never more than `perMinute` a minute, and nothing while the tab is hidden. ready and failed are final for a CID.
//
// renderToStart picks the one render worth starting: the next row that would play from a render, and only while someone
// is listening (the queue's presence rule: no render is paid for when nobody is there to hear it).

/** A row's render states. `queued`: this page asked for it and the server has not started it yet. */
export const RENDER_STATES = Object.freeze(['none', 'queued', 'rendering', 'ready', 'failed']);

/** Where a row plays from, by the playback mode (auto is the default). */
export const PLAYBACK_MODES = Object.freeze(['auto', 'render', 'live']);

export const POLL = Object.freeze({
  firstMs: 2000, // a rendering CID is asked again after this
  maxMs: 15000, // ... growing by `grow` each time up to this
  grow: 1.5,
  noneMs: 60000, // a CID with no render is asked again after this (someone else may render it), only while it shows
  failedMs: 0, // never again
  inFlight: 2,
  perMinute: 30,
});

const CID_RE = /^z[1-9A-HJ-NP-Za-km-z]{20,120}$/;

/** `m:ss` of seconds; `--:--` when unknown. */
export function fmtTime(s) {
  if (s == null || s === '' || !(Number(s) >= 0) || !Number.isFinite(Number(s))) return '--:--';
  const t = Math.floor(Number(s) + 1e-9);
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`;
}

/** The `#! name:` header line of a program, or ''. */
export function nameOf(text) {
  const m = /^[ \t]*#![ \t]*name:[ \t]*(.+?)[ \t]*$/m.exec(String(text || ''));
  return m ? m[1] : '';
}

/** A program's genre: its `#! genre:` header, else the genre of its first generator call (`gen@2 techno ...`), else ''. */
export function genreOf(text) {
  const t = String(text || '');
  const h = /^[ \t]*#![ \t]*genre:[ \t]*([\w-]+)/m.exec(t);
  if (h) return h[1].toLowerCase();
  const g = /\bgen@\d+[ \t]+([a-z][\w-]*)/.exec(t);
  return g ? g[1] : '';
}

/** A program's `tempo` line, or null. */
export function tempoOf(text) {
  const m = /^[ \t]*tempo[ \t]+(\d+(?:\.\d+)?)/m.exec(String(text || ''));
  return m ? Number(m[1]) : null;
}

/** A program's `key` line, or ''. */
export function keyOf(text) {
  const m = /^[ \t]*key[ \t]+([a-g][#b]?m?(?::[a-z]+)?)\b/im.exec(String(text || ''));
  return m ? m[1].toLowerCase() : '';
}

/**
 * Where a row plays from: 'render' (its Opus render), 'live' (this tab's engine) or 'wait' (render mode and the render
 * is not ready yet: the player bridges until it is, or plays live when it fails).
 */
export function playSource(mode, render) {
  const st = (render && render.state) || 'none';
  if (mode === 'live') return 'live';
  if (st === 'ready' && render.url) return 'render';
  if (mode === 'render' && st !== 'failed') return 'wait';
  return 'live';
}

/** The words for a row's render status (a short cell). */
export function renderLabel(r) {
  const st = (r && r.state) || 'none';
  if (st === 'rendering') {
    const p = r.seconds > 0 && r.secondsRendered >= 0 ? Math.min(99, Math.floor((100 * r.secondsRendered) / r.seconds)) : null;
    return p == null ? 'rendering' : `rendering ${p}%`;
  }
  return st;
}

/**
 * The status answer (GET /api/render/<cid>/status) as a row's render: {state, url, seconds, secondsRendered, bytes,
 * error}. A restricted program has no render ('failed', with why).
 */
export function renderFromStatus(ans, format = 'opus') {
  if (!ans || typeof ans !== 'object') return { state: 'none' };
  if (ans.restricted) return { state: 'failed', error: 'the program is private or uses assets: it plays live only' };
  const f = (ans.formats && ans.formats[format]) || {};
  const state = f.state === 'ready' ? 'ready' : f.state === 'rendering' ? 'rendering' : 'none';
  const out = { state, seconds: Number(f.seconds || ans.seconds) || null };
  if (f.url) out.url = f.url;
  if (f.bytes) out.bytes = f.bytes;
  if (f.seconds_rendered != null) out.secondsRendered = Number(f.seconds_rendered);
  return out;
}

/**
 * Per-CID render statuses and when to ask for each again. `now` is ms on the page's clock. The page calls want() with
 * the CIDs its rows show, due() for the ones to ask now (it marks them in flight), and settle() with each answer.
 */
export class RenderTracker {
  constructor({ poll = POLL } = {}) {
    this.poll = { ...POLL, ...poll };
    this.byCid = new Map(); // cid -> {render, nextAt, delay, inFlight}
    this.shown = new Set();
    this.asked = []; // the times of the last minute's asks
  }

  get(cid) {
    const e = this.byCid.get(cid);
    return e ? e.render : { state: 'none' };
  }

  /** The CIDs the playlist shows now: new ones are asked at once; ones no longer shown are not asked again. */
  want(cids) {
    this.shown = new Set((cids || []).filter((c) => CID_RE.test(String(c || ''))));
    for (const c of this.shown) if (!this.byCid.has(c)) this.byCid.set(c, { render: { state: 'none' }, nextAt: 0, delay: 0, inFlight: false, fresh: true });
  }

  /** What this page did itself: asked for a render (queued), saw it start (rendering), finish (ready) or fail. */
  mark(cid, render) {
    const e = this.byCid.get(cid) || { render: { state: 'none' }, nextAt: 0, delay: 0, inFlight: false };
    e.render = { ...e.render, ...render };
    e.fresh = false;
    if (render.state === 'rendering') { e.delay = this.poll.firstMs; e.nextAt = 0; }
    this.byCid.set(cid, e);
  }

  /** The CIDs to ask now (at most the free in-flight slots and what the minute's budget allows). Marks them in flight. */
  due(now, { hidden = false } = {}) {
    if (hidden) return [];
    this.asked = this.asked.filter((t) => now - t < 60000);
    let slots = Math.min(this.poll.inFlight - [...this.byCid.values()].filter((e) => e.inFlight).length, this.poll.perMinute - this.asked.length);
    const out = [];
    // first asks first, then rendering ones (the ones whose answer moves), then the rest, each by how overdue
    const order = [...this.shown].map((cid) => [cid, this.byCid.get(cid)]).filter(([, e]) => e && !e.inFlight && e.nextAt != null && e.nextAt <= now)
      .sort((a, b) => rank(a[1]) - rank(b[1]) || a[1].nextAt - b[1].nextAt);
    for (const [cid, e] of order) {
      if (slots <= 0) break;
      e.inFlight = true;
      this.asked.push(now);
      out.push(cid);
      slots--;
    }
    return out;
  }

  /** An answer (the status JSON) or an error ({status}) for a CID asked: its render, and when to ask again (null: never). */
  settle(cid, now, { answer = null, error = null } = {}) {
    const e = this.byCid.get(cid);
    if (!e) return;
    e.inFlight = false;
    e.fresh = false;
    if (error) {
      // 404: not stored here (a station track nobody rendered): no render, asked again only as a 'none' is
      if (error.status === 404) { e.render = { ...e.render, state: e.render.state === 'queued' ? 'queued' : 'none' }; e.nextAt = now + this.poll.noneMs; return; }
      if (error.status === 422) { e.render = { state: 'failed', error: error.message || 'refused' }; e.nextAt = null; return; }
      e.delay = Math.min(this.poll.maxMs, Math.max(this.poll.firstMs, (e.delay || this.poll.firstMs) * this.poll.grow));
      e.nextAt = now + e.delay; // a network error or a 429: later, and slower
      return;
    }
    const r = renderFromStatus(answer);
    // this page's own ask stays 'queued' until the server says it is rendering (or ready)
    if (r.state === 'none' && e.render.state === 'queued') r.state = 'queued';
    e.render = r;
    if (r.state === 'ready' || r.state === 'failed') { e.nextAt = null; return; }
    if (r.state === 'rendering' || r.state === 'queued') {
      e.delay = e.delay ? Math.min(this.poll.maxMs, e.delay * this.poll.grow) : this.poll.firstMs;
      e.nextAt = now + e.delay;
      return;
    }
    e.delay = 0;
    e.nextAt = now + this.poll.noneMs;
  }
}

function rank(e) {
  if (e.fresh) return 0;
  const st = e.render.state;
  return st === 'rendering' || st === 'queued' ? 1 : 2;
}

/**
 * The rows of the playlist, in play order. `now`: what plays ({kind, title, ...} or null). `queue`: the queue's up_next
 * items. `station`: the station's upcoming tracks ([{n, cid, text, form, seed}], after the one playing). `meta`: per CID
 * what is known of a program ({title, genre, bpm, seconds}). `renders`: a RenderTracker (or anything with get(cid)).
 * `bridging`: the row the player is bridging to (its key), when it is.
 */
export function playlistRows({ now = null, queue = [], station = [], stationInfo = {}, meta = new Map(), renders = null, mode = 'auto', bridging = null } = {}) {
  const rows = [];
  const m = (cid) => (cid && meta.get(cid)) || {};
  const r = (cid) => (cid && renders ? renders.get(cid) : { state: 'none' });
  const push = (row) => {
    const render = row.render || r(row.cid);
    rows.push({ ...row, render, source: playSource(mode, render), bridging: bridging != null && row.key === bridging });
  };
  if (now) {
    const mm = m(now.cid);
    push({ key: now.key || 'now', kind: now.kind || 'track', current: true, cid: now.cid || null, n: now.n, id: now.id || null,
      title: now.title || mm.title || '', genre: now.genre || mm.genre || '', bpm: now.bpm || mm.bpm || null, seconds: now.seconds || mm.seconds || null,
      render: now.source === 'render' ? { state: 'ready', url: now.url } : undefined });
  }
  for (const it of queue || []) {
    const mm = m(it.cid);
    push({ key: `q:${it.id}`, kind: 'queue', current: false, cid: it.cid || null, id: it.id, title: it.title || mm.title || shortCidTitle(it.cid), genre: mm.genre || '', bpm: mm.bpm || null,
      seconds: mm.seconds || (it.minutes ? it.minutes * 60 : null), movable: true, removable: true });
  }
  for (const t of station || []) {
    const mm = m(t.cid);
    push({ key: `s:${t.n}`, kind: 'station', current: false, cid: t.cid || null, n: t.n,
      title: mm.title || stationTitle(stationInfo, t), genre: stationInfo.genre || mm.genre || '', bpm: stationInfo.tempo || mm.bpm || null, seconds: mm.seconds || null });
  }
  return rows;
}

/** A station track's title: the station's name, its position and form ("techno radio #7 · fugue"). */
export function stationTitle(info, t) {
  const name = (info && info.name) || 'station';
  return `${name} #${t.n}${t.form && t.form !== 'seeded' ? ` · ${t.form}` : ''}`;
}

function shortCidTitle(cid) {
  return cid ? `${String(cid).slice(0, 8)}…${String(cid).slice(-4)}` : 'untitled';
}

/** The total of the rows' known durations, and how many are unknown (the playlist's footer: "12:40 + 2 unknown"). */
export function totalTime(rows) {
  let s = 0;
  let unknown = 0;
  for (const r of rows || []) {
    if (r.seconds > 0) s += r.seconds;
    else unknown++;
  }
  return { seconds: s, unknown };
}

/**
 * The one render worth starting now, or null: the first upcoming row (not the one playing) that would play from a render
 * in this mode and has none, and only while a listener is present and nothing this page asked for is still going.
 * `lookahead` is how many upcoming rows count (default 1: the next one).
 */
export function renderToStart(rows, { mode = 'auto', listening = false, lookahead = 1 } = {}) {
  if (!listening || mode === 'live') return null;
  const upcoming = (rows || []).filter((r) => !r.current).slice(0, Math.max(1, lookahead));
  if ((rows || []).some((r) => r.render && (r.render.state === 'queued' || r.render.state === 'rendering') && r.render.mine)) return null;
  const r = upcoming.find((x) => x.cid && x.render && x.render.state === 'none');
  return r || null;
}

// ---- the playlist's own edits (optimistic, page side: the server's answer replaces them) -------------------------------------

/** Move the item with `id` to index `to` (past the end: last), as the server does. */
export function moveItem(items, id, to) {
  const list = [...(items || [])];
  const i = list.findIndex((x) => x.id === id || x.cid === id);
  if (i < 0) return list;
  const [it] = list.splice(i, 1);
  list.splice(Math.max(0, Math.min(to, list.length)), 0, it);
  return list;
}

/** Take the item with `id` (or CID) out. */
export function removeItem(items, id) {
  const list = [...(items || [])];
  const i = list.findIndex((x) => x.id === id || x.cid === id);
  if (i >= 0) list.splice(i, 1);
  return list;
}

/**
 * What a person typed into "add": a CID, a /p/<CID> link (any host), a self-sealing share link (`#<CID>.<text>` or
 * `p#<CID>.<text>`), or an example's name. {cid} | {cid, sealed: true, link} | {example} | {error}.
 */
export function parseAdd(input) {
  const s = String(input || '').trim();
  if (!s) return { error: 'paste a CID or a share link' };
  if (CID_RE.test(s)) return { cid: s };
  const sealed = /(?:^|[#/])p?#?(z[1-9A-HJ-NP-Za-km-z]{20,120})\.[A-Za-z0-9_-]+/.exec(s);
  if (sealed && /#/.test(s)) return { cid: sealed[1], sealed: true, link: s };
  const p = /\/(?:p|c|embed)\/(z[1-9A-HJ-NP-Za-km-z]{20,120})(?:[.?#/]|$)/.exec(s);
  if (p) return { cid: p[1] };
  if (/^[a-z0-9][a-z0-9-]{0,63}$/.test(s)) return { example: s };
  return { error: 'that is not a CID, a share link or an example name' };
}
