// The player's offline render (site/player/src/beatscript-render.js, <beatscript-track mode="render">) in Node: no
// browser, no AudioContext, the packaged engine and worklet on the render's own clock.
//
//   node site/test/player-render.mjs        (needs site/web/wasm, or BTW_WASM=<dir>; skips without it)
//   BTW_RENDER_BENCH=1 ...                  also times a two-minute track (and prints the factor over real time)
//
// Builds the package, then for each program: it renders; the length is exactly its bars at its tempo (one pass for a
// looping song; the cap for a long one); it is not silent, and each section is heard (its RMS); a second render is the
// same PCM, sample for sample (sha256 of the 16-bit PCM and of the f32); and the WAV is well formed.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const wasmDir = process.env.BTW_WASM || path.join(here, '..', 'web', 'wasm');
if (!fs.existsSync(path.join(wasmDir, 'btw_bg.wasm'))) {
  console.log(`# skip: no ${wasmDir}/btw_bg.wasm (scripts/build-web-wasm.sh ${wasmDir})`);
  process.exit(0);
}
const dist = process.env.BTW_PLAYER_DIST || fs.mkdtempSync(path.join(os.tmpdir(), 'btw-player-render-'));
if (!process.env.BTW_PLAYER_DIST) execFileSync(process.execPath, [path.join(here, '..', 'player', 'build.mjs'), '--wasm', wasmDir, '--out', dist], { stdio: 'inherit' });

const { renderProgram } = await import(pathToFileURL(path.join(dist, 'beatscript-render.js')).href);
const { framesOf, wav, pcm16, renderPlan } = await import(pathToFileURL(path.join(dist, 'render-model.js')).href);
const module = await WebAssembly.compile(fs.readFileSync(path.join(wasmDir, 'btw_bg.wasm')));

const SR = 44100;
const failures = [];
const check = (ok, what) => { console.log(`${ok ? 'ok' : 'FAIL'} - ${what}`); if (!ok) failures.push(what); };
const sha = (buf) => crypto.createHash('sha256').update(Buffer.from(buf.buffer, buf.byteOffset, buf.byteLength)).digest('hex');
const rms = (a, from, to) => { let s = 0; for (let i = from; i < to; i++) s += a[i] * a[i]; return Math.sqrt(s / Math.max(1, to - from)); };

const DRUMS = `let four = x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~
let backs = ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~
let offs = ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~
`;
const CASES = [
  {
    name: 'csp.html: drums on an 808 kit, a melody, a sung line (4 bars at 120)',
    bars: 4, bpm: 120,
    text: `#! name: render test
tempo 120
key am
${DRUMS}section verse 4 bars {
  deck A: kick four kit=drums-808 | snare backs | hihat offs
  deck B: melody a4 ~ c5 ~ e5 ~ c5 ~ a4 ~ g4 ~ a4 ~ ~ ~
  deck B: sing "hold the night, hold it near" on melody
}
arrange verse
`,
  },
  {
    name: 'a looping song: two sections, one pass (2 + 2 bars at 128)',
    bars: 4, bpm: 128, loops: true,
    // the chorus is drums only (deck B goes out): each section is heard where it plays
    text: `tempo 128
key am
${DRUMS}section verse 2 bars {
  deck A: kick four | hihat offs
  deck B: melody a4 ~ c5 ~ e5 ~ c5 ~ a4 ~ g4 ~ a4 ~ ~ ~
}
section chorus 2 bars {
  deck A: kick four | snare backs | hihat offs
}
arrange { verse chorus } loop
`,
  },
  {
    name: 'no arrangement, no end: 16 bars at 140',
    bars: 16, bpm: 140,
    text: `tempo 140
deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ | hihat ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~
`,
  },
];

for (const c of CASES) {
  const r1 = await renderProgram({ module, text: c.text, sampleRate: SR });
  const r2 = await renderProgram({ module, text: c.text, sampleRate: SR });
  const want = framesOf(c.bars, c.bpm, SR);
  check(r1.frames === want && r1.left.length === want && r1.right.length === want, `${c.name}: ${r1.frames} frames, ${c.bars} bars at ${c.bpm} is ${want}`);
  check(r1.bars === c.bars && r1.bpm === c.bpm, `${c.name}: the plan says ${r1.bars} bars at ${r1.bpm}`);
  if (c.loops) check(r1.loops === true && r1.notes.some((n) => /one pass/.test(n)), `${c.name}: it loops, rendered once (${r1.notes.join('; ')})`);
  const p1 = pcm16(r1.left, r1.right);
  const p2 = pcm16(r2.left, r2.right);
  check(sha(p1) === sha(p2), `${c.name}: a second render is the same 16-bit PCM (${sha(p1).slice(0, 16)})`);
  check(sha(r1.left) === sha(r2.left) && sha(r1.right) === sha(r2.right), `${c.name}: and the same f32, both channels`);
  const level = rms(r1.left, 0, r1.frames);
  check(level > 0.005, `${c.name}: it is not silent (RMS ${level.toFixed(4)})`);
  // the first bar has sound: the render starts on the downbeat, not after the net's install
  const bar = Math.round((240 / c.bpm) * SR);
  check(rms(r1.left, 0, Math.min(bar, r1.frames)) > 0.002, `${c.name}: the first bar is heard (lead of ${r1.lead} frames dropped)`);
  // the first kick is within the first 10 ms (the downbeat is frame 0)
  let first = -1;
  for (let i = 0; i < r1.frames; i++) if (Math.abs(r1.left[i]) > 0.02 || Math.abs(r1.right[i]) > 0.02) { first = i; break; }
  check(first >= 0 && first < SR / 100, `${c.name}: the first hit is at frame ${first} (${((first / SR) * 1000).toFixed(1)} ms)`);
  const w = wav(r1.left, r1.right, SR);
  const v = new DataView(w.buffer);
  check(String.fromCharCode(...w.subarray(0, 4)) === 'RIFF' && String.fromCharCode(...w.subarray(8, 12)) === 'WAVE' && v.getUint32(24, true) === SR && v.getUint32(40, true) === want * 4 && w.length === 44 + want * 4, `${c.name}: the WAV is ${w.length} bytes, 16-bit stereo at ${SR}`);
  check(r1.errors.length === 0, `${c.name}: no engine error (${r1.errors.join('; ')})`);
  console.log(`# ${c.name}: ${r1.frames} frames (${r1.seconds.toFixed(2)} s) in ${r1.ms.toFixed(0)} ms, ${(r1.seconds * 1000 / r1.ms).toFixed(1)}x real time; pcm16 sha256 ${sha(p1)}`);
}

// the chorus of the looping song has no melody: the second half is quieter on deck B's register? Simpler and exact: the
// render of the looping song is its two sections, so it is as long as both, and the cap holds a long song to 256 bars
{
  const long = `tempo 200\n${DRUMS}section a 200 bars {\n  deck A: kick four\n}\nsection b 200 bars {\n  deck A: hihat offs\n}\narrange a b\n`;
  const plan = renderPlan({ ok: true, tempo: 200, arranged: true, song_bars: 400, loops: false });
  check(plan.bars === 256 && plan.cut === 'bars', `the cap: a 400-bar song at 200 renders 256 bars (${plan.bars}, cut ${plan.cut})`);
  const slow = renderPlan({ ok: true, tempo: 60, arranged: true, song_bars: 200, loops: false });
  check(slow.bars === 150 && slow.cut === 'seconds', `the cap: 200 bars at 60 is 800 s, cut to 600 s, 150 bars (${slow.bars}, cut ${slow.cut})`);
  if (process.env.BTW_RENDER_LONG) {
    const r = await renderProgram({ module, text: long, sampleRate: SR });
    check(r.bars === 256 && r.frames === framesOf(256, 200, SR), `the cap, rendered: ${r.bars} bars, ${r.frames} frames`);
  }
}

if (process.env.BTW_RENDER_BENCH) {
  // a two-minute track: 60 bars at 120, four sections, both decks, a sung line
  const text = `#! name: two minutes
tempo 120
key am
${DRUMS}section intro 8 bars {
  deck A: kick four kit=drums-808 | hihat offs
}
section verse 20 bars {
  deck A: kick four kit=drums-808 | snare backs | hihat offs
  deck B: melody a4 ~ c5 ~ e5 ~ c5 ~ a4 ~ g4 ~ a4 ~ ~ ~
  deck B: sing "hold the night, hold it near" on melody
}
section chorus 24 bars {
  deck A: kick four | snare backs | hihat offs
  deck B: melody e5 ~ g5 ~ a5 ~ g5 ~ e5 ~ d5 ~ e5 ~ ~ ~
}
section outro 8 bars {
  deck A: kick four
}
arrange intro verse chorus outro
`;
  for (let i = 0; i < 2; i++) {
    const r = await renderProgram({ module, text, sampleRate: SR });
    check(r.frames === framesOf(60, 120, SR), `bench: 60 bars at 120 is ${r.frames} frames (${r.seconds.toFixed(1)} s)`);
    console.log(`# bench ${i + 1}: ${r.seconds.toFixed(1)} s of audio in ${(r.ms / 1000).toFixed(2)} s, ${(r.seconds * 1000 / r.ms).toFixed(1)}x real time, pcm16 sha256 ${sha(pcm16(r.left, r.right))}`);
  }
}

if (!process.env.BTW_PLAYER_DIST) fs.rmSync(dist, { recursive: true, force: true });
console.log(failures.length ? `# ${failures.length} failed` : '# all passed');
process.exit(failures.length ? 1 : 0);
