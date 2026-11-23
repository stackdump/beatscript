// render-model.js: the render mode's arithmetic, pure so Node tests it without the engine (site/test/unit.mjs).
//
// - renderPlan(report, limits): how long one pass of a program is, as btwd render plans it (crates/btwd/src/render.rs
//   `plan`): an arranged song is its sections once (an `end` cuts it shorter), else `end at bar N`, else 16 bars; then
//   capped at the limits the site renders to (btwserve -render-max-bars / -render-max-seconds as deployed: 256, 600).
// - framesOf(bars, bpm, sampleRate): the frames of `bars` bars, rounded as btwd's frame_of.
// - s16(x) and wav(left, right, sampleRate): 16-bit PCM, rounded as btwd's s16 writer (half away from zero), in a WAV.
// - statusAt(seconds, plan, sections): what the position line says at a place in the render, in the runtime's
//   status words (embed-model.js reads them), so the card says "verse 2/3 bar 3/8" in either mode.

export const RENDER_LIMITS = Object.freeze({ maxBars: 256, maxSeconds: 600 });

/**
 * One pass of the program: {bars, programBars, bpm, seconds, loops, cut, notes}. `report` is the check report
 * (btw.check: tempo, arranged, song_bars, end_bar, loops). A cut says which limit made it shorter.
 */
export function renderPlan(report, limits = RENDER_LIMITS) {
  const r = report || {};
  const bpm = Number(r.tempo) > 0 ? Number(r.tempo) : 120;
  const end = Number.isInteger(r.end_bar) && r.end_bar > 0 ? r.end_bar : null;
  let programBars;
  if (r.arranged) {
    const song = Number.isInteger(r.song_bars) && r.song_bars > 0 ? r.song_bars : 16;
    programBars = end ? Math.min(end, song) : song;
  } else {
    programBars = end || 16;
  }
  programBars = Math.max(1, programBars);
  const maxBars = Math.max(1, Math.floor(Number(limits.maxBars) || RENDER_LIMITS.maxBars));
  const maxSeconds = Number(limits.maxSeconds) > 0 ? Number(limits.maxSeconds) : RENDER_LIMITS.maxSeconds;
  // the bars that fit in maxSeconds at this tempo (a bar is four beats)
  const fitBars = Math.max(1, Math.floor((maxSeconds * bpm) / 240 + 1e-9));
  let bars = programBars;
  let cut = null;
  if (bars > maxBars) { bars = maxBars; cut = 'bars'; }
  if (bars > fitBars) { bars = fitBars; cut = 'seconds'; }
  const notes = [];
  if (r.loops && r.arranged) notes.push('the song loops: the render is one pass through its sections');
  if (!r.arranged && !end) notes.push('the program has no end: 16 bars');
  if (cut === 'bars') notes.push(`cut at ${maxBars} bars of the program's ${programBars}`);
  if (cut === 'seconds') notes.push(`cut at ${maxSeconds} s: ${bars} of the program's ${programBars} bars`);
  return { bars, programBars, bpm, seconds: (bars * 240) / bpm, loops: !!(r.loops && r.arranged), cut, notes };
}

/** Frames of `bars` bars at `bpm` and `sampleRate`, as btwd's frame_of: (beat · 60 / bpm · sr) rounded. */
export function framesOf(bars, bpm, sampleRate) {
  return Math.max(1, Math.round(((bars * 4 * 60) / bpm) * sampleRate));
}

/** One sample as 16 bits: clamped, then rounded half away from zero, as btwd's s16 writer (and parity.html). */
export function s16(x) {
  const c = x < -1 ? -1 : x > 1 ? 1 : x !== x ? 0 : x;
  return c < 0 ? -Math.round(-c * 32768) : Math.round(c * 32767);
}

/** Interleaved 16-bit PCM of two channels. */
export function pcm16(left, right) {
  const n = left.length;
  const out = new Int16Array(n * 2);
  for (let i = 0; i < n; i++) {
    out[2 * i] = s16(left[i]);
    out[2 * i + 1] = s16(right[i]);
  }
  return out;
}

/** A stereo 16-bit WAV file (a Uint8Array) of `left` / `right` at `sampleRate`. */
export function wav(left, right, sampleRate) {
  const pcm = pcm16(left, right);
  const data = pcm.byteLength;
  const bytes = new Uint8Array(44 + data);
  const v = new DataView(bytes.buffer);
  const tag = (at, s) => { for (let i = 0; i < 4; i++) bytes[at + i] = s.charCodeAt(i); };
  tag(0, 'RIFF');
  v.setUint32(4, 36 + data, true);
  tag(8, 'WAVE');
  tag(12, 'fmt ');
  v.setUint32(16, 16, true); // PCM fmt chunk
  v.setUint16(20, 1, true); // PCM
  v.setUint16(22, 2, true); // channels
  v.setUint32(24, sampleRate, true);
  v.setUint32(28, sampleRate * 4, true); // bytes a second
  v.setUint16(32, 4, true); // bytes a frame
  v.setUint16(34, 16, true); // bits
  tag(36, 'data');
  v.setUint32(40, data, true);
  // little-endian samples (every engine this runs on is little-endian, as is WAV)
  bytes.set(new Uint8Array(pcm.buffer), 44);
  return bytes;
}

/**
 * The runtime's status words at `seconds` into the render (embed-model.js positionModel reads them): song_bar, and
 * within the section that plays then, section, section_bar and section_bars; else song_bars.
 */
export function statusAt(seconds, plan, sections) {
  const barS = 240 / plan.bpm;
  const songBar = Math.min(plan.bars, Math.floor(Math.max(0, seconds) / barS + 1e-9) + 1);
  const secs = Array.isArray(sections) ? sections : [];
  const s = secs.find((x) => songBar - 1 >= x.start && songBar - 1 < x.start + x.bars);
  if (s) return { song_bar: songBar, section: s.name, section_bar: songBar - s.start, section_bars: s.bars };
  return { song_bar: songBar, song_bars: plan.bars };
}

/** `m:ss` of a number of seconds. */
export function clock(seconds) {
  const t = Math.max(0, Math.floor(seconds + 1e-9));
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`;
}
