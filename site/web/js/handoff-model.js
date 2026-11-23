// handoff-model.js: what plays next, and never dead air. Pure (site/test/unit.mjs runs it on a fake clock and a fake sink).
//
// The radio player plays one item after another: station tracks, the segue between two, queue items. Whatever comes
// next is resolved while the current item plays (its text fetched and compiled into a run of its own, or its render
// found ready), so the change at the boundary is a swap, not a wait. When the next item is not ready at the boundary
// (the render is pending or failed, it does not compile, the network stalls, the queue answers late), the player does
// not stop: it plays a BRIDGE, a small loop fitted to the outgoing track's tempo and key, hot-reloaded into the running
// program so it lands on the next bar line with no gap, and loops on the bar grid until the next item is ready; then
// the next item comes in on a bar line. A next item that fails is passed over for the one after it, the bridge playing on.
//
// The fallback order, at a boundary:
//   1. the next item, prepared (its render when the mode and the render say so, else its compiled run);
//   2. the bridge, while the next item is still resolving; the row shows "bridging";
//   3. a next item that failed (no text, no compile, render failed in render mode): the item after it, still bridging;
//   4. bridging longer than `maxBridgeMs` with nothing ready: stop, and say why (a network that is down is not radio).
// A presence pause (nobody listening: queue-model.js pauseReason) is the one intended stop, and it is not a gap.
//
// The sink is the page's sound: play(prepared) starts an item fresh, bridge(text) hot-reloads a loop into what plays
// (it lands on the next bar line), stop() ends it. A provider says what is next: upcoming() -> a descriptor
// {key, kind, prepare(): Promise<prepared>} or null, after(key) -> the one after a failed one, commit(desc) -> the
// provider's own bookkeeping when desc starts (the station's N moves on, the queue's item is taken).

import { fnv1a32 } from './station-model.js';

export const HANDOFF = Object.freeze({ maxBridgeMs: 10 * 60 * 1000 });

/** The sparse genres play the quiet bridge (no kick). */
const QUIET = new Set(['ambient', 'bossa', 'jazz', 'lofi']);

/** The bridges: small CC0 loops. %T is the tempo, %C the chord (am, c, f#m), %R the bass root (a2, f#2). */
export const BRIDGES = Object.freeze({
  pulse: [
    'deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ | hihat ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ vel=45..70',
    'deck B: bass %R ~ ~ ~ ~ ~ ~ ~ %R ~ ~ ~ ~ ~ ~ ~ inst=sub-bass | harmony chord(%C) inst=warm-pad',
  ],
  drift: [
    'deck A: hihat ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ vel=30..50',
    'deck B: harmony chord(%C) inst=warm-pad',
  ],
  break: [
    'deck A: kick x ~ ~ ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ ~ ~ | snare ~ ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ | hihat x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~ vel=35..60',
    'deck B: harmony chord(%C) inst=warm-pad',
  ],
});

const MINORISH = /^(dorian|phrygian|aeolian|minor|locrian)$/;

/** The chord and the bass root of a key line's value ("am", "c", "f#m", "d:dorian"); am when there is none. */
export function keyParts(key) {
  const k = String(key || 'am').trim().toLowerCase();
  const m = /^([a-g])([#b]?)(m?)(?::([a-z]+))?$/.exec(k);
  if (!m) return { chord: 'am', root: 'a2' };
  const minor = m[3] === 'm' || (m[4] && MINORISH.test(m[4]));
  return { chord: `${m[1]}${m[2]}${minor ? 'm' : ''}`, root: `${m[1]}${m[2]}2` };
}

/**
 * The bridge for an outgoing track: {name, text}. Picked by the outgoing CID (the same track always bridges the same way),
 * the quiet one for a sparse genre; at the outgoing tempo and key. A loop with no sections and no end, so a hot reload
 * of it lands on the next bar and it plays until something replaces it.
 */
export function bridgeFor({ cid = '', tempo = 120, key = 'am', genre = '' } = {}) {
  const names = Object.keys(BRIDGES);
  const name = QUIET.has(String(genre)) ? 'drift' : names[fnv1a32(String(cid || 'bridge')) % names.length];
  const t = Math.round(Math.max(40, Math.min(300, Number(tempo) || 120)));
  const { chord, root } = keyParts(key);
  const lines = BRIDGES[name].map((l) => l.replaceAll('%C', chord).replaceAll('%R', root));
  const text = ['#! beatscript 2', `#! name: bridge (${name})`, '#! license: CC0-1.0', `tempo ${t}`, ...lines, ''].join('\n');
  return { name, text };
}

/**
 * The sequencer: what plays, what is next (resolving, ready or failed), and whether it is bridging. The page calls
 * start(desc) for the first item, frame({bar, ended, target}) on every frame, and boundary() when an item is over.
 */
export class Handoff {
  constructor({ provider, sink, now = () => Date.now(), verdict = () => '', onChange = null, cfg = HANDOFF } = {}) {
    this.provider = provider;
    this.sink = sink;
    this.now = now;
    this.verdict = verdict; // '' to go on, else the presence pause's reason
    this.onChange = onChange;
    this.cfg = { ...HANDOFF, ...cfg };
    this.current = null; // {desc, prepared}
    this.next = null; // {desc, state: 'resolving' | 'ready' | 'failed', prepared, error, seq}
    this.bridging = null; // {since, from, name}
    this.paused = '';
    this.seq = 0;
    this.lastBar = null;
  }

  get state() {
    if (this.paused) return 'paused';
    if (this.bridging) return 'bridging';
    return this.current ? 'playing' : 'idle';
  }

  changed() { if (this.onChange) this.onChange(this); }

  /** Resolve a descriptor (its prepare()), keeping what it answers; a later resolve of another one wins. */
  resolve(desc) {
    if (!desc) { this.next = null; return; }
    const seq = ++this.seq;
    const n = { desc, state: 'resolving', prepared: null, error: null, seq };
    this.next = n;
    let p;
    try { p = Promise.resolve(desc.prepare()); } catch (e) { p = Promise.reject(e); }
    p.then((prepared) => {
      if (this.next !== n) { if (this.sink.discard) this.sink.discard(prepared); return; }
      n.state = 'ready';
      n.prepared = prepared;
      this.changed();
    }, (e) => {
      if (this.next !== n) return;
      n.state = 'failed';
      n.error = e;
      this.changed();
      // bridging to it: the one after it, at once
      if (this.bridging) this.resolve(this.provider.after(desc.key));
    });
    this.changed();
  }

  /** Look again at what is next (the queue changed, the station was retuned): resolve it unless it is the same. */
  refresh() {
    // nothing plays: nothing is resolved ahead (the engine starts only on a click)
    if (!this.current) { this.next = null; return; }
    const d = this.provider.upcoming();
    if (!d) { this.next = null; this.changed(); return; }
    if (this.next && this.next.desc.key === d.key && this.next.state !== 'failed') return;
    this.resolve(d);
  }

  /** Start with an item now (Play, a double-click on a row): it is prepared first, the bridge covering when something plays. */
  async start(desc) {
    this.paused = '';
    if (!desc) return false;
    let prepared;
    try {
      prepared = await desc.prepare();
    } catch (e) {
      if (this.current) { this.toBridge(); this.resolve(this.provider.after(desc.key)); return false; }
      throw e;
    }
    this.play(desc, prepared);
    return true;
  }

  play(desc, prepared) {
    this.sink.play(prepared, desc);
    this.current = { desc, prepared };
    this.bridging = null;
    this.lastBar = null;
    if (this.provider.commit) this.provider.commit(desc);
    this.resolve(this.provider.upcoming());
  }

  /** The current item is over (ended, or its play length reached on a bar line). */
  boundary() {
    if (!this.current || this.bridging) return;
    const why = this.verdict();
    if (why) {
      this.paused = why;
      this.sink.stop();
      this.changed();
      return;
    }
    const n = this.next;
    if (n && n.state === 'ready') { this.play(n.desc, n.prepared); return; }
    if (!n) this.resolve(this.provider.upcoming());
    else if (n.state === 'failed') this.resolve(this.provider.after(n.desc.key));
    this.toBridge();
  }

  toBridge() {
    const from = (this.current && this.current.desc) || {};
    const b = bridgeFor({ cid: from.cid, tempo: from.tempo, key: from.musicKey, genre: from.genre });
    this.sink.bridge(b.text, b);
    this.bridging = { since: this.now(), from: from.key || null, name: b.name };
    this.lastBar = null;
    this.changed();
  }

  /** One frame of what plays: {bar} (whole bars since it started). While bridging, the next item comes in on a bar line. */
  frame({ bar = 0 } = {}) {
    if (!this.bridging) return;
    const crossed = this.lastBar != null && bar !== this.lastBar;
    this.lastBar = bar;
    const n = this.next;
    if (n && n.state === 'ready' && crossed) { this.play(n.desc, n.prepared); return; }
    if (this.now() - this.bridging.since > this.cfg.maxBridgeMs) {
      this.bridging = null;
      this.current = null;
      this.paused = 'bridge_timeout';
      this.sink.stop();
      this.changed();
    }
  }

  /** The person stopped: nothing plays, nothing is bridged; what is next stays resolved. */
  stop() {
    this.bridging = null;
    this.paused = '';
    this.current = null;
    this.sink.stop();
    this.changed();
  }
}
