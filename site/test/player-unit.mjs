// Unit tests for the radio player's pure models (no npm: node:test and node:assert).
//   node --test site/test/player-unit.mjs
// playlist-model.js: the rows, the render statuses and how often they are asked for, what plays from a render.
// handoff-model.js: the next item resolved while one plays, and never dead air (the bridge) when it is late or fails.

import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const J = (f) => import(pathToFileURL(path.join(here, '..', 'web', 'js', f)).href);
const P = await J('playlist-model.js');
const H = await J('handoff-model.js');

// the real module when it is built (site/web/wasm, never in git): the bridges are checked by the real checker
let real = null;
{
  const glue = path.join(here, '..', 'web', 'wasm', 'beatscript.js');
  if (fs.existsSync(glue) && !process.env.BEATSCRIPT_MOCK_WASM) {
    const m = await import(pathToFileURL(glue).href);
    m.initSync({ module: fs.readFileSync(path.join(here, '..', 'web', 'wasm', 'beatscript_bg.wasm')) });
    real = (await J('wasm.js')).makeFacade(m);
  }
}

const CID = (c) => `z${'Q'.repeat(40)}${c}`;
const status = (state, extra = {}) => ({ cid: 'x', restricted: false, formats: { opus: { state, url: '/api/render/x?format=opus', available: true, ...extra } } });

// ---- playlist-model -------------------------------------------------------------------------------------------------------------

test('playlist: fmtTime, nameOf, genreOf, tempoOf, keyOf read what the list shows', () => {
  assert.equal(P.fmtTime(0), '0:00');
  assert.equal(P.fmtTime(252.9), '4:12');
  assert.equal(P.fmtTime(null), '--:--');
  assert.equal(P.fmtTime(NaN), '--:--');
  const t = '#! beatscript 2\n#! name: night drive\ntempo 124\nkey f#m\narrange from gen@2 synthwave seed=7\n';
  assert.equal(P.nameOf(t), 'night drive');
  assert.equal(P.genreOf(t), 'synthwave');
  assert.equal(P.genreOf('#! genre: House\ntempo 120\n'), 'house');
  assert.equal(P.tempoOf(t), 124);
  assert.equal(P.keyOf(t), 'f#m');
  assert.equal(P.nameOf('tempo 120'), '');
});

test('playlist: a row plays from its render only when the render is ready (auto), waits for it (render), or never (live)', () => {
  const ready = { state: 'ready', url: '/r' };
  assert.equal(P.playSource('auto', ready), 'render');
  assert.equal(P.playSource('auto', { state: 'rendering' }), 'live');
  assert.equal(P.playSource('auto', { state: 'none' }), 'live');
  assert.equal(P.playSource('render', { state: 'rendering' }), 'wait');
  assert.equal(P.playSource('render', { state: 'none' }), 'wait');
  assert.equal(P.playSource('render', { state: 'failed' }), 'live');
  assert.equal(P.playSource('live', ready), 'live');
  assert.equal(P.renderLabel({ state: 'rendering', seconds: 200, secondsRendered: 50 }), 'rendering 25%');
  assert.equal(P.renderLabel({ state: 'ready' }), 'ready');
});

test('playlist: the status answer becomes a render; restricted is failed with why', () => {
  assert.deepEqual(P.renderFromStatus(status('ready', { seconds: 120, bytes: 9 })), { state: 'ready', seconds: 120, url: '/api/render/x?format=opus', bytes: 9 });
  assert.equal(P.renderFromStatus(status('rendering', { seconds_rendered: 3, seconds: 10 })).secondsRendered, 3);
  assert.equal(P.renderFromStatus(status('none')).state, 'none');
  assert.equal(P.renderFromStatus({ restricted: true }).state, 'failed');
});

test('render status: a CID is asked once when it shows, again only while rendering (2 s growing to 15 s), never once ready', () => {
  const r = new P.RenderTracker();
  const a = CID('a');
  r.want([a]);
  assert.deepEqual(r.due(0), [a]);
  assert.deepEqual(r.due(0), [], 'in flight: not asked twice');
  r.settle(a, 0, { answer: status('rendering', { seconds: 100, seconds_rendered: 10 }) });
  assert.equal(r.get(a).state, 'rendering');
  assert.deepEqual(r.due(1999), []);
  assert.deepEqual(r.due(2000), [a]);
  r.settle(a, 2000, { answer: status('rendering') });
  assert.deepEqual(r.due(4999), [], 'the second wait is 1.5x the first');
  assert.deepEqual(r.due(5000), [a]);
  r.settle(a, 5000, { answer: status('ready') });
  assert.equal(r.get(a).state, 'ready');
  for (const t of [6000, 60000, 3600000]) assert.deepEqual(r.due(t), [], 'ready is final');
  // the waits grow by half each time and never pass 15 s
  const b = CID('b');
  const q = new P.RenderTracker();
  q.want([b]);
  let t = 0;
  const waits = [];
  for (let i = 0; i < 8; i++) {
    while (!q.due(t).length) t += 250;
    q.settle(b, t, { answer: status('rendering') });
    waits.push(q.byCid.get(b).nextAt - t);
  }
  assert.deepEqual(waits, [2000, 3000, 4500, 6750, 10125, 15000, 15000, 15000]);
});

test('render status: at most two in flight, at most 30 a minute, nothing while hidden, only the rows that show', () => {
  const r = new P.RenderTracker();
  const cids = 'abcdefgh'.split('').map(CID);
  r.want(cids);
  assert.deepEqual(r.due(0, { hidden: true }), []);
  const first = r.due(0);
  assert.equal(first.length, 2);
  assert.equal(r.due(0).length, 0);
  for (const c of first) r.settle(c, 0, { answer: status('none') });
  assert.equal(r.due(1).length, 2);
  // a row that no longer shows is not asked again
  r.want([cids[7]]);
  const d = r.due(2);
  assert.ok(d.every((c) => c === cids[7]));
  // the minute's budget
  const q = new P.RenderTracker({ poll: { inFlight: 100, perMinute: 30 } });
  q.want(Array.from({ length: 50 }, (_, i) => CID(String.fromCharCode(97 + (i % 26)) + i)));
  assert.equal(q.due(0).length, 30);
  assert.equal(q.due(30000).length, 0);
  assert.ok(q.due(60001).length > 0);
});

test('render status: a 404 is no render (asked again in a minute), a 422 is failed for good, a network error backs off', () => {
  const r = new P.RenderTracker();
  const [a, b, c] = ['a', 'b', 'c'].map(CID);
  r.want([a, b, c]);
  r.due(0);
  r.settle(a, 0, { error: { status: 404 } });
  r.settle(b, 0, { error: { status: 422, message: 'restricted' } });
  assert.equal(r.get(a).state, 'none');
  assert.equal(r.get(b).state, 'failed');
  r.due(0);
  r.settle(c, 0, { error: { status: 0 } });
  assert.equal(r.get(c).state, 'none');
  assert.ok(!r.due(1000).includes(c));
  assert.ok(r.due(3000).includes(c));
  assert.ok(!r.due(59999).includes(a));
  assert.ok(r.due(60000).includes(a) || r.byCid.get(a).inFlight);
  assert.ok(!r.due(10 ** 9).includes(b));
});

test('render status: what this page asked for stays queued until the server says rendering', () => {
  const r = new P.RenderTracker();
  const a = CID('a');
  r.want([a]);
  r.mark(a, { state: 'queued', mine: true });
  r.due(0);
  r.settle(a, 0, { answer: status('none') });
  assert.equal(r.get(a).state, 'queued');
  r.due(2000);
  r.settle(a, 2000, { answer: status('rendering') });
  assert.equal(r.get(a).state, 'rendering');
});

test('playlist: rows are now playing, then the queue, then the station; each says its render and where it plays from', () => {
  const r = new P.RenderTracker();
  const [a, b, s1, s2] = ['a', 'b', 'c', 'd'].map(CID);
  r.want([a, b, s1, s2]);
  r.mark(a, { state: 'ready', url: '/ra' });
  r.mark(b, { state: 'rendering' });
  r.mark(s2, { state: 'failed' });
  const meta = new Map([[a, { title: 'Alpha', genre: 'house', bpm: 124, seconds: 200 }]]);
  const rows = P.playlistRows({
    now: { key: 's:7', kind: 'track', n: 7, cid: CID('n'), title: 'radio #7', seconds: 240 },
    queue: [{ id: 'q1', cid: a }, { id: 'q2', cid: b, title: 'Beta', minutes: 2 }],
    station: [{ n: 8, cid: s1, form: 'fugue' }, { n: 9, cid: s2, form: 'seeded' }],
    stationInfo: { name: 'techno radio', genre: 'techno', tempo: 128 },
    meta, renders: r, mode: 'auto', bridging: 'q:q1',
  });
  assert.deepEqual(rows.map((x) => x.key), ['s:7', 'q:q1', 'q:q2', 's:8', 's:9']);
  assert.ok(rows[0].current);
  assert.deepEqual([rows[1].title, rows[1].genre, rows[1].bpm, rows[1].seconds], ['Alpha', 'house', 124, 200]);
  assert.equal(rows[1].source, 'render');
  assert.ok(rows[1].bridging);
  assert.equal(rows[2].seconds, 120);
  assert.equal(rows[2].source, 'live');
  assert.equal(rows[3].title, 'techno radio #8 · fugue');
  assert.deepEqual([rows[3].genre, rows[3].bpm], ['techno', 128]);
  assert.equal(rows[4].title, 'techno radio #9');
  assert.equal(rows[4].render.state, 'failed');
  assert.ok(rows[1].movable && rows[1].removable && !rows[3].movable);
  assert.deepEqual(P.totalTime(rows), { seconds: 240 + 200 + 120, unknown: 2 });
});

test('playlist: a render is started for the next row only, only while someone listens, one at a time, never in live mode', () => {
  const rows = [
    { key: 'now', current: true, cid: CID('n'), render: { state: 'none' } },
    { key: 'q:q1', cid: CID('a'), render: { state: 'none' } },
    { key: 's:8', cid: CID('b'), render: { state: 'none' } },
  ];
  assert.equal(P.renderToStart(rows, { listening: false }), null);
  assert.equal(P.renderToStart(rows, { listening: true, mode: 'live' }), null);
  assert.equal(P.renderToStart(rows, { listening: true }).key, 'q:q1');
  rows[1].render = { state: 'ready' };
  assert.equal(P.renderToStart(rows, { listening: true }), null, 'the next one is ready: nothing further ahead');
  assert.equal(P.renderToStart(rows, { listening: true, lookahead: 2 }).key, 's:8');
  rows[2].render = { state: 'rendering', mine: true };
  rows[1].render = { state: 'none' };
  assert.equal(P.renderToStart(rows, { listening: true }), null, 'one of ours is going');
});

test('playlist: move and remove as the server does them; add takes a CID, a /p/ link, a sealed link or an example', () => {
  const items = [{ id: 'q1' }, { id: 'q2' }, { id: 'q3', cid: CID('c') }];
  assert.deepEqual(P.moveItem(items, 'q3', 0).map((x) => x.id), ['q3', 'q1', 'q2']);
  assert.deepEqual(P.moveItem(items, 'q1', 99).map((x) => x.id), ['q2', 'q3', 'q1']);
  assert.deepEqual(P.moveItem(items, CID('c'), 1).map((x) => x.id), ['q1', 'q3', 'q2']);
  assert.deepEqual(P.removeItem(items, 'q2').map((x) => x.id), ['q1', 'q3']);
  assert.deepEqual(P.moveItem(items, 'nope', 0), items);
  const c = CID('x');
  assert.deepEqual(P.parseAdd(c), { cid: c });
  assert.deepEqual(P.parseAdd(`https://beatscript.app/p/${c}`), { cid: c });
  assert.deepEqual(P.parseAdd(`https://beatscript.app/p/${c}.beatscript`), { cid: c });
  assert.deepEqual(P.parseAdd(`https://beatscript.app/#${c}.eJxLzs8tyM8rSc0rAQAUcwPh`), { cid: c, sealed: true, link: `https://beatscript.app/#${c}.eJxLzs8tyM8rSc0rAQAUcwPh` });
  assert.deepEqual(P.parseAdd('first-beat'), { example: 'first-beat' });
  assert.ok(P.parseAdd('').error);
  assert.ok(P.parseAdd('not a thing!').error);
});

// ---- handoff-model: never dead air ------------------------------------------------------------------------------------------

/**
 * A fake clock, a fake sink that records what sounds, and a provider of items whose prepare() resolves (or fails) when
 * the test says. Time moves in 25 ms frames (the page's service loop); a bar is 2 s (120 BPM). `buffer` is one render
 * quantum of the worklet at 44.1 kHz (128 frames, 2.9 ms): no gap may be longer.
 */
function rig({ items }) {
  let t = 0;
  const frameMs = 25;
  const barMs = 2000;
  const log = []; // [t, what] each time what sounds changes
  const sink = {
    sounding: null,
    startedAt: 0,
    play(prepared) { this.sounding = prepared.name; this.startedAt = t; log.push([t, prepared.name]); },
    bridge(text, b) { assert.match(text, /^tempo \d+$/m); this.sounding = `bridge:${b.name}`; log.push([t, this.sounding]); },
    stop() { this.sounding = null; log.push([t, null]); },
    discard() {},
  };
  const pending = new Map(); // key -> {resolve, reject}
  let idx = 0;
  const desc = (i) => {
    const it = items[i];
    if (!it) return null;
    return {
      key: it.name, kind: 'track', cid: CID(String(i)), tempo: 120, musicKey: 'am',
      prepare: () => (it.at == null ? Promise.resolve({ name: it.name }) : new Promise((res, rej) => pending.set(it.name, { res: () => res({ name: it.name }), rej, at: it.at, fail: it.fail }))),
    };
  };
  const provider = {
    upcoming: () => desc(idx + 1),
    after: (key) => { const i = items.findIndex((x) => x.name === key); return desc(i + 1); },
    commit: (d) => { idx = items.findIndex((x) => x.name === d.key); },
  };
  const h = new H.Handoff({ provider, sink, now: () => t });
  const step = async () => {
    t += frameMs;
    for (const [k, p] of pending) if (t >= p.at) { pending.delete(k); if (p.fail) p.rej(new Error('failed')); else p.res(); }
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    const bar = Math.floor((t - sink.startedAt) / barMs);
    h.frame({ bar });
    // an item plays 4 bars (8 s), then its boundary (a bridge has none: it loops)
    if (h.current && !h.bridging && !h.paused && t - sink.startedAt >= 4 * barMs) h.boundary();
  };
  /** The longest silence between the first sound and `until`. */
  const longestGap = (until) => {
    let gap = 0;
    let silentSince = null;
    for (const [at, what] of log) {
      if (what == null && silentSince == null) silentSince = at;
      if (what != null && silentSince != null) { gap = Math.max(gap, at - silentSince); silentSince = null; }
    }
    if (silentSince != null) gap = Math.max(gap, until - silentSince);
    return gap;
  };
  return { h, sink, log, step, longestGap, desc, now: () => t, buffer: (128 / 44100) * 1000 };
}

test('handoff: a next item ready in time is a swap at the boundary, never a bridge', async () => {
  const r = rig({ items: [{ name: 'A' }, { name: 'B' }, { name: 'C' }] });
  await r.h.start(r.desc(0));
  while (r.now() < 20000) await r.step();
  assert.deepEqual(r.log.map((x) => x[1]), ['A', 'B', 'C']);
  assert.ok(r.longestGap(r.now()) <= r.buffer);
});

test('handoff: a slow next item is bridged on the bar grid until it is ready, and the audio never stops', async () => {
  // B takes 13 s to resolve (a slow network and a long compile): A ends at 8 s, the bridge plays, B comes in on a bar line
  const r = rig({ items: [{ name: 'A' }, { name: 'B', at: 13000 }, { name: 'C' }] });
  await r.h.start(r.desc(0));
  const states = new Set();
  while (r.now() < 30000) { await r.step(); states.add(r.h.state); assert.ok(r.sink.sounding, `silent at ${r.now()} ms`); }
  const seq = r.log.map((x) => x[1]);
  assert.equal(seq[0], 'A');
  assert.match(seq[1], /^bridge:/);
  assert.equal(seq[2], 'B');
  const bAt = r.log[2][0];
  assert.ok(bAt >= 13000, 'B only when it was ready');
  assert.ok(states.has('bridging'));
  assert.ok(r.longestGap(r.now()) <= r.buffer);
});

test('handoff: a next item that fails is passed over for the one after it, the bridge playing on; never silent', async () => {
  const r = rig({ items: [{ name: 'A' }, { name: 'B', at: 9000, fail: true }, { name: 'C', at: 12000 }, { name: 'D' }] });
  await r.h.start(r.desc(0));
  while (r.now() < 40000) { await r.step(); assert.ok(r.sink.sounding, `silent at ${r.now()} ms`); }
  const seq = r.log.map((x) => x[1]);
  assert.deepEqual([seq[0], seq[2]], ['A', 'C']);
  assert.match(seq[1], /^bridge:/);
  assert.ok(!seq.includes('B'));
  assert.ok(r.longestGap(r.now()) <= r.buffer);
});

test('handoff: a next item that never comes is bridged up to the limit, then the player stops and says why', async () => {
  const r = rig({ items: [{ name: 'A' }, { name: 'B', at: 10 ** 9 }] });
  r.h.cfg.maxBridgeMs = 6000;
  await r.h.start(r.desc(0));
  while (r.now() < 13900) { await r.step(); assert.ok(r.sink.sounding, `silent at ${r.now()} ms`); }
  while (r.now() < 16000) await r.step();
  assert.equal(r.h.state, 'paused');
  assert.equal(r.h.paused, 'bridge_timeout');
  assert.equal(r.sink.sounding, null);
});

test('handoff: nobody listening is the one intended stop (a presence pause), not a gap to bridge', async () => {
  const r = rig({ items: [{ name: 'A' }, { name: 'B' }] });
  let why = '';
  r.h.verdict = () => why;
  await r.h.start(r.desc(0));
  why = 'idle_hidden';
  while (r.now() < 10000) await r.step();
  assert.equal(r.h.state, 'paused');
  assert.deepEqual(r.log.map((x) => x[1]), ['A', null]);
});

test('bridge: a loop fitted to the outgoing tempo and key, the same for the same track, the quiet one for a sparse genre', () => {
  const a = H.bridgeFor({ cid: CID('a'), tempo: 128, key: 'f#m', genre: 'techno' });
  assert.match(a.text, /^#! beatscript 2$/m);
  assert.match(a.text, /^#! license: CC0-1.0$/m);
  assert.match(a.text, /^tempo 128$/m);
  assert.match(a.text, /chord\(f#m\)/);
  assert.doesNotMatch(a.text, /%[CR]/);
  assert.doesNotMatch(a.text, /^(section|arrange|end)\b/m, 'a loop: no sections, no end');
  assert.deepEqual(H.bridgeFor({ cid: CID('a'), tempo: 128, key: 'f#m', genre: 'techno' }), a);
  assert.equal(H.bridgeFor({ cid: CID('a'), genre: 'ambient' }).name, 'drift');
  assert.deepEqual(H.keyParts('d:dorian'), { chord: 'dm', root: 'd2' });
  assert.deepEqual(H.keyParts('eb'), { chord: 'eb', root: 'eb2' });
  assert.deepEqual(H.keyParts('nonsense'), { chord: 'am', root: 'a2' });
  assert.match(H.bridgeFor({ tempo: 9999 }).text, /^tempo 300$/m);
});

test('bridge: every bridge, in every kind of key, checks clean with the real checker (BeatScript 2, no warnings)', { skip: !real && 'site/web/wasm is not built' }, () => {
  for (const [tempo, key] of [[128, 'am'], [124, 'f#m'], [90, 'd:dorian'], [140, 'eb'], [170, 'c'], [100, 'bbm']]) {
    for (const name of Object.keys(H.BRIDGES)) {
      // find a CID that picks this bridge (the pick is a hash of the outgoing CID)
      let b = null;
      for (let i = 0; i < 200 && (!b || b.name !== name); i++) b = H.bridgeFor({ cid: `z${i}`, tempo, key, genre: name === 'drift' ? 'ambient' : 'techno' });
      assert.equal(b.name, name);
      const r = real.check(b.text);
      assert.equal(r.ok, true, `${name} ${key}: ${JSON.stringify(r.diagnostics)}`);
      assert.equal(r.warnings, 0, `${name} ${key}: ${JSON.stringify(r.diagnostics)}`);
      assert.equal(r.tempo, tempo);
      assert.equal(r.arranged, false, 'a bridge is a loop');
    }
  }
});

test('handoff: a ready next item that is passed over (the order changed) is let go, never the one that plays', async () => {
  // what the sink was given to discard, and what it played: a prepared item is one or the other, never both
  const discarded = [];
  let order = ['B'];
  const played = [];
  const make = (name) => ({ key: name, kind: 'track', cid: CID(name), tempo: 120, musicKey: 'am', prepare: () => Promise.resolve({ name }) });
  const h = new H.Handoff({
    provider: { upcoming: () => (order.length ? make(order[0]) : null), after: () => null, commit: () => {} },
    sink: { play: (p) => played.push(p.name), bridge: () => {}, stop: () => {}, discard: (p) => discarded.push(p.name) },
    now: () => 0,
  });
  await h.start(make('A'));
  await Promise.resolve(); await Promise.resolve();
  assert.equal(h.next.state, 'ready');
  // a segue comes into the order before B (the station's answer arrived): B was ready, and is let go
  order = ['G', 'B'];
  h.refresh();
  await Promise.resolve(); await Promise.resolve();
  assert.deepEqual(discarded, ['B']);
  assert.equal(h.next.desc.key, 'G');
  // G plays: it is not discarded, and it is not next any more
  h.boundary();
  assert.deepEqual(played, ['A', 'G']);
  assert.ok(!discarded.includes('G'));
  // nothing is next: the one resolved for after G (the order still says G, then B) is dropped when the order empties
  await Promise.resolve(); await Promise.resolve();
  order = [];
  h.refresh();
  assert.equal(h.next, null);
  assert.ok(discarded.length >= 2 && !discarded.includes('A'));
});

// ---- station-model: the play order the /radio page and <beatscript-radio> share --------------------------------------------------

test('station: playOrder is the segue after the track that plays, then the next tracks; trackStarted moves N there', async () => {
  const S = await J('station-model.js');
  const t = (n) => ({ n, cid: CID(`t${n}`), text: `track ${n}` });
  const base = { ...S.initialState({ n: 3 }), now: t(3), next: t(4), segue: { from: 3, to: 4, cid: CID('g3'), text: 'segue' } };
  // nothing has played: track N first
  assert.deepEqual(S.playOrder(base, { ahead: [t(4), t(5)] }).map((e) => e.kind === 'track' ? e.track.n : e.kind), [3, 4, 5]);
  // track N plays: its segue, then N+1 and on
  const after = S.playOrder(base, { current: { kind: 'track', n: 3 }, ahead: [t(4), t(5), t(6)] });
  assert.deepEqual(after.map((e) => e.kind === 'track' ? e.track.n : e.kind), ['segue', 4, 5, 6]);
  // segues off: straight to N+1
  assert.deepEqual(S.playOrder({ ...base, segues: false }, { current: { kind: 'track', n: 3 }, ahead: [t(4)] }).map((e) => e.track && e.track.n), [4]);
  // the segue plays: N+1 next (not the segue again)
  assert.deepEqual(S.playOrder(base, { current: { kind: 'segue', n: 3 }, ahead: [t(4), t(5)] }).map((e) => e.kind === 'track' ? e.track.n : e.kind), [4, 5]);
  // the queue goes first, and then no segue
  assert.deepEqual(S.playOrder(base, { current: { kind: 'track', n: 3 }, queue: [{ id: 'q1' }], ahead: [t(4)] }).map((e) => e.kind), ['queue', 'track']);
  // depth
  assert.equal(S.playOrder(base, { started: true, ahead: [t(4), t(5), t(6), t(7)], depth: 2 }).length, 2);
  // trackStarted: one on is an advance (the next track becomes now), anything else a step
  const a = S.trackStarted(base, 4);
  assert.equal(a.n, 4);
  assert.equal(a.now && a.now.n, 4);
  assert.equal(a.segue, null);
  assert.equal(S.trackStarted(base, 3), base);
  const b = S.trackStarted(base, 9);
  assert.equal(b.n, 9);
  assert.equal(b.now, null);
});
