#!/usr/bin/env node
// test-web-wasm.mjs: the browser module against the native code. Node >= 22.7 (ESM detection), no npm.
//
//   scripts/build-web-wasm.sh <dir>                       # build the module first
//   node scripts/test-web-wasm.mjs [--wasm <dir>] [--seconds 3] [--passes 640] [--native FILE]
//
// <dir> defaults to $BEATSCRIPT_WASM_DIR, else site/web/wasm. The native side and a built beatscript are looked for under
// $CARGO_TARGET_DIR (default target/).
//
// Also: the page finds every feature it looks for in the module (site/web/js/wasm.js FEATURES), the legacy shim btw.js
// built beside it answers the old names (btw_check, BtwRun, ...) with the module's own exports and is in hashes.json, fmt
// upgrades a BeatScript 1 text and check says the version. (rename-to-beatscript: keep: btw.js is the shim's name.)
//
// For every program (crates/beatscript-api/tests/{corpus,extra}/*.beatscript, examples/*.beatscript, docs/beatscript/*.beatscript):
//   - beatscript_check / beatscript_fmt / beatscript_share / beatscript_open / beatscript_highlight equal, byte for byte, what beatscript-api answers natively
//     (`cargo run --example native_dump`, which is beatscript-api itself) and, when a built beatscript is found
//     (target/debug/beatscript or $BEATSCRIPT_CLI), what `beatscript check` / `beatscript fmt` print; the program, net and song_net CIDs are
//     named in the failure when they differ
//   - open(share(text)) gives the same canonical text and CID back, verified; a forged link is refused
//   - BeatscriptRun on a fake clock (the quarter-beat clock of crates/beatscript-api/tests/run.rs) gives the same commands on every
//     pass and the same marks every bar as the native `Run`
//   - the engine: the first deck-A net of the program, through DjComposer.prepare_share -> Dj.begin_install ->
//     install_step -> set_playing -> render in 128-frame blocks with tick() every 4th (the path the AudioWorklet
//     takes), hashes to the same samples as the native `render_offline` of the same envelope; so does wasm
//     `render_offline` (wasm and native engine agree) -- the determinism proof that the browser plays the program
//   - the install is taken in steps small enough for a render quantum (the slowest step is reported)
//   - the Wheel: the program's `wheel` calls through WheelWinder -> WheelVoice.install / WheelVoice.call -> render_add in
//     128-frame quanta (the composer's and the worklet's path) give the native core's samples (native_dump `wheel_render`,
//     which crates/djweb/tests/wheel_parity.rs proves is the booth's, bit for bit); bit for bit on the programs here, and
//     the check allows rounding (90 dB under the peak), since windwheel calls std's exp / sin / tanh / powf, the
//     platform's libm natively and Rust's own on wasm32
import { spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..');
const argv = process.argv.slice(2);
const opt = (name, dflt) => { const i = argv.indexOf(name); return i >= 0 ? argv[i + 1] : dflt; };
const SECONDS = Number(opt('--seconds', 3));
const PASSES = Number(opt('--passes', 640));
const SR = 48000;
const BUDGET = 12 * 1024; // what the worklet takes of an install per quantum (engine-worklet.js DEFAULT_BUDGET)

const wasmDir = path.resolve(opt('--wasm', process.env.BEATSCRIPT_WASM_DIR || path.join(root, 'site', 'web', 'wasm')));
const targetDir = process.env.CARGO_TARGET_DIR ? path.resolve(process.env.CARGO_TARGET_DIR) : path.join(root, 'target');
if (!fs.existsSync(path.join(wasmDir, 'beatscript_bg.wasm')) || !fs.existsSync(path.join(wasmDir, 'beatscript.js'))) {
    console.error(`test-web-wasm: no built module in ${wasmDir}: run scripts/build-web-wasm.sh ${wasmDir}`);
    process.exit(2);
}

let passed = 0;
const failures = [];
function ok(name, cond, detail) {
    if (cond) { passed++; return; }
    failures.push(`${name}${detail ? `: ${detail}` : ''}`);
    console.log(`FAIL ${name}${detail ? `: ${detail}` : ''}`);
}
const sha256 = (data) => crypto.createHash('sha256').update(data).digest('hex');

// ---- the programs
function programs() {
    const dirs = ['crates/beatscript-api/tests/corpus', 'crates/beatscript-api/tests/extra', 'examples', 'docs/beatscript'];
    const out = [];
    for (const d of dirs) {
        const full = path.join(root, d);
        if (!fs.existsSync(full)) continue;
        for (const f of fs.readdirSync(full).sort()) if (f.endsWith('.beatscript')) out.push(path.join(full, f));
    }
    return out;
}
const files = programs();

// ---- the native side
let native;
const nativeFile = opt('--native');
if (nativeFile) {
    native = JSON.parse(fs.readFileSync(nativeFile, 'utf8'));
} else {
    const env = { ...process.env, CARGO_PROFILE_DEV_DEBUG: '0', CARGO_PROFILE_TEST_DEBUG: '0', CARGO_INCREMENTAL: '0' };
    const build = spawnSync('cargo', ['build', '-q', '--profile', 'test', '-p', 'djweb', '--example', 'native_dump'], { cwd: root, env, stdio: ['ignore', 'inherit', 'inherit'] });
    if (build.status !== 0) { console.error('test-web-wasm: could not build the native side (cargo build --profile test -p djweb --example native_dump)'); process.exit(2); }
    const bin = path.join(targetDir, 'debug', 'examples', 'native_dump');
    const r = spawnSync(bin, ['--seconds', String(SECONDS), '--passes', String(PASSES), ...files], { cwd: root, env, maxBuffer: 1 << 30 });
    if (r.status !== 0) { console.error(`test-web-wasm: native_dump failed:\n${r.stderr}`); process.exit(2); }
    native = JSON.parse(r.stdout.toString());
}

// ---- the module
const glue = await import(pathToFileURL(path.join(wasmDir, 'beatscript.js')).href);
const t0 = performance.now();
const exports = glue.initSync({ module: fs.readFileSync(path.join(wasmDir, 'beatscript_bg.wasm')) });
const initMs = performance.now() - t0;
const memInit = exports.memory.buffer.byteLength;
const t1 = performance.now();
const probe = new glue.Dj(SR);
const djMs = performance.now() - t1;
const memDj = exports.memory.buffer.byteLength;
probe.free();
const probeComposer = new glue.DjComposer(SR);
const memComposer = exports.memory.buffer.byteLength;
probeComposer.free();
const wasmBytes = fs.statSync(path.join(wasmDir, 'beatscript_bg.wasm')).size;

// ---- beatscript, when it is built
const beatscript = process.env.BEATSCRIPT_CLI || ['debug', 'release'].map((p) => path.join(targetDir, p, 'beatscript')).find((p) => fs.existsSync(p));
function beatscriptCliRun(args, input) {
    const r = spawnSync(beatscript, args, { input, maxBuffer: 1 << 28 });
    return r.status === 0 ? r.stdout.toString() : null;
}

// ---- the fake clock, the same as native_dump's
function simulate(text) {
    const run = new glue.BeatscriptRun();
    const evalAnswer = run.eval(text);
    let beat = 0;
    const playing = { A: false, B: false };
    const commands = [];
    const marks = [];
    let firstA = null;
    const wheel = [];
    for (let pass = 0; pass < PASSES; pass++) {
        if (playing.A || playing.B) beat += 0.25;
        const d = playing.A ? 'A' : playing.B ? 'B' : null;
        const grid = d === null ? 'null' : `{"deck":"${d}","beat":${beat},"bpm":140,"playing":{"A":${playing.A},"B":${playing.B}}}`;
        const out = run.service(grid);
        for (const c of JSON.parse(out)) {
            if (c.op === 'play') {
                if (c.on && !(playing.A || playing.B)) beat = 0;
                playing[c.deck === 'B' ? 'B' : 'A'] = c.on;
            } else if (c.op === 'call' && c.noun === 'wheel') {
                wheel.push(JSON.stringify(c.params));
            } else if (c.op === 'load_net' && c.deck === 'A' && firstA === null) {
                firstA = JSON.stringify(c.envelope);
            }
        }
        commands.push(sha256(out));
        if (pass % 16 === 15) marks.push(sha256(run.marks()));
    }
    const status = run.status();
    run.free();
    return { evalAnswer, commands, marks, status, firstA, wheel };
}

// ---- the Wheel through the page's path (native_dump `wheel_render`: the same calls, the same quanta)
const WHEEL_CALLS = 24;
const WHEEL_FRAMES = 4800;
function wheelRender(calls) {
    const winder = new glue.WheelWinder();
    const voice = new glue.WheelVoice(SR);
    const L = new Float32Array(128), R = new Float32Array(128);
    const inter = new Float32Array(256);
    const h = crypto.createHash('sha256');
    const samples = [];
    const wounds = [];
    let peak = 0;
    for (const c of calls.slice(0, WHEEL_CALLS)) {
        try {
            const bytes = winder.call(c);
            if (bytes.length) { wounds.push(sha256(Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength))); voice.install(bytes); }
        } catch { /* refused, as natively */ }
        try { voice.call(c, 140, 140); } catch { /* refused, as natively */ }
        for (let q = 0; q < Math.floor(WHEEL_FRAMES / 128); q++) {
            L.fill(0); R.fill(0);
            voice.render_add(L, R);
            for (let i = 0; i < 128; i++) { inter[2 * i] = L[i]; inter[2 * i + 1] = R[i]; samples.push(L[i], R[i]); peak = Math.max(peak, Math.abs(L[i]), Math.abs(R[i])); }
            h.update(Buffer.from(inter.buffer, 0, 1024));
        }
    }
    winder.free();
    voice.free();
    return { sha: h.digest('hex'), peak, samples, wounds };
}

// ---- the engine through the worklet's path
function djRender(envelopeText, frames, { interleave = false } = {}) {
    const composer = new glue.DjComposer(SR);
    const tp = performance.now();
    const bytes = composer.prepare_share('A', envelopeText, '[]');
    const prepareMs = performance.now() - tp;
    composer.free();
    const dj = new glue.Dj(SR);
    dj.begin_install('A', bytes);
    const L = new Float32Array(128), R = new Float32Array(128);
    const h = crypto.createHash('sha256');
    let steps = 0, slowest = 0, quanta = 0, rendered = 0, landed = null;
    const inter = new Float32Array(256);
    const block = () => {
        dj.render(L, R);
        const take = Math.min(128, frames - rendered);
        for (let i = 0; i < take; i++) { inter[2 * i] = L[i]; inter[2 * i + 1] = R[i]; }
        h.update(Buffer.from(inter.buffer, 0, take * 8));
        rendered += take;
        quanta++;
        if (quanta % 4 === 0) dj.tick();
    };
    // install: the worklet does one step per render quantum
    while (dj.installing()) {
        const s0 = performance.now();
        const r = dj.install_step(BUDGET);
        slowest = Math.max(slowest, performance.now() - s0);
        steps++;
        if (r) { landed = JSON.parse(r); break; }
        if (interleave) { dj.render(L, R); quanta++; if (quanta % 4 === 0) dj.tick(); }
    }
    dj.set_playing('A', true);
    let peak = 0;
    while (rendered < frames) { block(); for (let i = 0; i < 128; i++) peak = Math.max(peak, Math.abs(L[i]), Math.abs(R[i])); }
    const state = JSON.parse(dj.state('A'));
    dj.free();
    return { sha: h.digest('hex'), peak, steps, slowest, prepareMs, landed, state, bytes: bytes.length };
}

function wasmOfflineHash(envelopeText, seconds) {
    const s = glue.render_offline(envelopeText, seconds, SR);
    return { sha: sha256(Buffer.from(s.buffer, s.byteOffset, s.byteLength)), frames: s.length / 2 };
}

// ---- go
const timing = { check: [], prepare: [], slowestStep: 0, bytes: [], steps: [] };
let checked = 0, simulated = 0, rendered = 0;
for (const f of files) {
    const name = path.relative(root, f);
    const text = fs.readFileSync(f, 'utf8');
    const nat = native[f];
    if (!nat) { ok(`${name}: native dump has it`, false); continue; }

    // check / fmt / share / open / highlight byte for byte
    const tc = performance.now();
    const check = glue.beatscript_check(text);
    timing.check.push(performance.now() - tc);
    const fmt = glue.beatscript_fmt(text);
    ok(`${name}: beatscript_check == native`, check === nat.check, check === nat.check ? '' : cidDiff(check, nat.check));
    ok(`${name}: beatscript_fmt == native`, fmt === nat.fmt);
    ok(`${name}: beatscript_highlight == native`, glue.beatscript_highlight(text) === nat.highlight);
    const share = glue.beatscript_share(text, null, false, null);
    ok(`${name}: beatscript_share == native`, share === nat.share);
    const rep = JSON.parse(check);
    if (beatscript) {
        const bc = beatscriptCliRun(['check', '--file', f]);
        ok(`${name}: beatscript_check == beatscript check`, bc !== null && bc.trimEnd() === check, bc === null ? 'beatscript failed' : cidDiff(check, bc.trimEnd()));
        const bf = beatscriptCliRun(['fmt', '--file', f]);
        ok(`${name}: beatscript_fmt == beatscript fmt`, bf !== null && bf.trimEnd() === fmt);
    }
    checked++;

    // open / share round trip, and a forged link
    const sh = JSON.parse(share);
    if (rep.ok && !rep.restricted) {
        ok(`${name}: share ok`, sh.ok === true && typeof sh.url === 'string', JSON.stringify(sh).slice(0, 200));
        const op = JSON.parse(glue.beatscript_open(sh.url));
        ok(`${name}: open(share) gives the program back`, op.ok && op.verified && op.cid === rep.cid && op.canonical === rep.canonical, JSON.stringify(op).slice(0, 200));
        ok(`${name}: open == native`, glue.beatscript_open(sh.url) === nat.open);
        const at = sh.url.length - 8;
        const forged = sh.url.slice(0, at) + (sh.url[at] === 'A' ? 'B' : 'A') + sh.url.slice(at + 1);
        const bad = JSON.parse(glue.beatscript_open(forged));
        ok(`${name}: a forged link is refused`, !bad.ok || !bad.verified, JSON.stringify(bad).slice(0, 200));
    }

    // the runtime on the fake clock
    if (nat.run) {
        const s = simulate(text);
        ok(`${name}: BeatscriptRun eval == native`, s.evalAnswer === nat.run.eval);
        let firstBad = -1;
        for (let i = 0; i < s.commands.length; i++) if (s.commands[i] !== nat.run.commands[i]) { firstBad = i; break; }
        ok(`${name}: BeatscriptRun commands == native on all ${PASSES} passes`, firstBad < 0 && s.commands.length === nat.run.commands.length, `first difference at pass ${firstBad}`);
        const markBad = s.marks.findIndex((m, i) => m !== nat.run.marks[i]);
        ok(`${name}: BeatscriptRun marks == native at each bar (${s.marks.length})`, markBad < 0 && s.marks.length === nat.run.marks.length, `first difference at bar ${markBad}`);
        ok(`${name}: BeatscriptRun status == native`, s.status === nat.run.status);
        simulated++;

        // the Wheel
        ok(`${name}: the same wheel calls as native`, (nat.wheel ? nat.wheel.calls : 0) === Math.min(s.wheel.length, WHEEL_CALLS));
        if (nat.wheel) {
            const w = wheelRender(s.wheel);
            ok(`${name}: the Wheel's instruments wind to the same bytes as native (${w.wounds.length})`, JSON.stringify(w.wounds) === JSON.stringify(nat.wheel.wounds || []), `${w.wounds.map((x) => x.slice(0, 8))} vs ${(nat.wheel.wounds || []).map((x) => x.slice(0, 8))}`);
            let worst = 0, energy = 0;
            const ns = nat.wheel.samples || [];
            for (let i = 0; i < ns.length; i++) { worst = Math.max(worst, Math.abs(w.samples[i] - ns[i])); energy = Math.max(energy, Math.abs(ns[i])); }
            const db = worst > 0 ? 20 * Math.log10(worst / Math.max(energy, 1e-12)) : -Infinity;
            console.log(`${name}: the Wheel, ${nat.wheel.calls} calls: ${w.sha === nat.wheel.sha256 ? 'bit for bit with native' : `within ${db.toFixed(0)} dB of native's peak`}`);
            ok(`${name}: the Wheel (composer and worklet path) == native to rounding, ${nat.wheel.calls} calls (worst ${db.toFixed(0)} dB below the peak${w.sha === nat.wheel.sha256 ? ', bit for bit' : ''})`, ns.length === w.samples.length && db < -90, `worst difference ${worst} of ${energy}, ${w.samples.length} vs ${ns.length} samples`);
            ok(`${name}: the Wheel's peak agrees`, Math.abs(w.peak - nat.wheel.peak) < 1e-6, `${w.peak} vs ${nat.wheel.peak}`);
        }

        // the engine
        if (s.firstA && nat.render && nat.render.sha256) {
            const frames = Math.round(SECONDS * SR);
            ok(`${name}: envelope is the one native rendered`, sha256(s.firstA) === nat.render.envelope_sha256);
            const dj = djRender(s.firstA, frames);
            ok(`${name}: Dj (worklet path) renders ${frames} frames == native render_offline`, dj.sha === nat.render.sha256, `wasm ${dj.sha.slice(0, 12)} native ${nat.render.sha256.slice(0, 12)}`);
            ok(`${name}: the install landed on the deck`, dj.landed && dj.landed.deck === 'A' && !dj.landed.error, JSON.stringify(dj.landed));
            ok(`${name}: it is not silent`, dj.peak > 0.01, `peak ${dj.peak}`);
            ok(`${name}: native peak agrees`, Math.abs(dj.peak - nat.render.peak) < 1e-6, `${dj.peak} vs ${nat.render.peak}`);
            const off = wasmOfflineHash(s.firstA, SECONDS);
            ok(`${name}: wasm render_offline == native render_offline`, off.sha === nat.render.sha256 && off.frames === nat.render.frames);
            // with the audio running while it installs (the worklet): not bit-identical, but it plays
            const live = djRender(s.firstA, frames, { interleave: true });
            ok(`${name}: with renders between install steps it still plays`, live.peak > 0.01);
            timing.prepare.push(dj.prepareMs);
            timing.slowestStep = Math.max(timing.slowestStep, dj.slowest);
            timing.bytes.push(dj.bytes);
            timing.steps.push(dj.steps);
            rendered++;
        }
    }
}

// ---- things that are not per program
const instruments = glue.beatscript_instruments();
const cheat = glue.beatscript_cheatsheet();
ok('beatscript_instruments is JSON with voices', Array.isArray(JSON.parse(instruments).voices) && JSON.parse(instruments).voices.length > 10);
ok('beatscript_cheatsheet is markdown of a sensible size', cheat.length > 4000 && cheat.length < 30000 && cheat.startsWith('#'), `${cheat.length} chars`);
if (beatscript) {
    ok('beatscript_instruments == beatscript instruments', (beatscriptCliRun(['instruments']) || '').trimEnd() === instruments);
    ok('beatscript_cheatsheet == beatscript cheatsheet', (beatscriptCliRun(['cheatsheet']) || '').trimEnd() === cheat.trimEnd());
}
const gen = JSON.parse(glue.beatscript_generation(fs.readFileSync(path.join(root, 'crates/beatscript-api/tests/corpus/markers-demo.beatscript'), 'utf8'), null, 2));
ok('beatscript_generation(markers-demo, 2) answers a CID', gen.ok === true && /^z/.test(gen.cid), JSON.stringify(gen));
// radio (docs/beatscript/radio.md): beatscript_station answers what the MCP station tool and POST /api/station do (crates/djweb/tests/drift.rs
// holds the native exports to station_json byte for byte; this is the module itself)
const station = JSON.parse(glue.beatscript_station(JSON.stringify({ station: 'tempo 128\nkey am\nstation {\n  from gen@2 techno seed=42\n  next seed=~\n  forms seeded passacaglia\n}\n', n: 3 })));
ok('beatscript_station(n 3) answers track 3 and the segue to 4', station.ok === true && station.tracks[0].n === 3 && /^z/.test(station.tracks[0].cid) && station.segues[0].to === 4 && /^z/.test(station.segues[0].cid), JSON.stringify(station).slice(0, 400));
ok('engine_version is the engine crate', glue.engine_version() === JSON.parse(glue.beatscript_build_info()).engine);
const bad = JSON.parse(glue.beatscript_check('deck A: kick x ~ y ~\n'));
ok('an error has line, col, start16 and a render form', bad.ok === false && bad.diagnostics[0].line === 1 && 'start16' in bad.diagnostics[0] && bad.diagnostics[0].render.includes('-->'), JSON.stringify(bad.diagnostics && bad.diagnostics[0]).slice(0, 200));

// ---- the names after the rename, and BeatScript 2
const facadeJs = await import(pathToFileURL(path.join(root, 'site', 'web', 'js', 'wasm.js')).href);
const feats = facadeJs.featuresOf(glue);
ok('the page finds every feature it looks for in the module (all but the station, which djweb does not have)', JSON.stringify(feats.missing) === '["station"]', JSON.stringify(feats.missing));
for (const old of Object.values(facadeJs.LEGACY)) ok(`the module has no ${old} of its own (the shim answers it)`, glue[old] === undefined);
const shimFile = path.join(wasmDir, 'btw.js');
ok('the legacy shim btw.js is built beside the module', fs.existsSync(shimFile));
if (fs.existsSync(shimFile)) {
    const shim = await import(pathToFileURL(shimFile).href);
    for (const [now, old] of Object.entries(facadeJs.LEGACY)) {
        if (glue[now] === undefined) continue;
        ok(`btw.js ${old} is the module's ${now}`, typeof shim[old] === 'function' && shim[old] === glue[now]);
    }
    for (const n of ['Dj', 'DjComposer', 'engine_version', 'initSync']) ok(`btw.js passes ${n} through`, typeof shim[n] === 'function');
    ok('a facade over btw.js finds what one over beatscript.js does', JSON.stringify(facadeJs.featuresOf(shim).found) === JSON.stringify(feats.found));
    let hashes = null;
    try { hashes = JSON.parse(fs.readFileSync(path.join(wasmDir, 'hashes.json'), 'utf8')); } catch { /* none */ }
    const h = hashes && hashes.files && hashes.files['btw.js'];
    ok('hashes.json lists btw.js with its sha256', !!h && h.sha256 === sha256(fs.readFileSync(shimFile)), JSON.stringify(h));
}
{
    const v1 = 'tempo 120\ndeck A: kick x ~ x ~ | snare ~ x ~ x\n';
    const c = JSON.parse(glue.beatscript_check(v1));
    const f = JSON.parse(glue.beatscript_fmt(v1));
    ok('beatscript_check says version 1 for a text with no version line, and its BeatScript 2 CID', c.version === 1 && /^z/.test(c.renamed_cid) && c.renamed_cid !== c.cid, JSON.stringify([c.version, c.renamed_cid]));
    ok('beatscript_fmt upgrades it: version 2, the version line first, from_cid the CID it had, cid its renamed_cid', f.version === 2 && f.text.startsWith('#! beatscript 2\n') && f.from_cid === c.cid && f.cid === c.renamed_cid, JSON.stringify(f).slice(0, 300));
    const c2 = JSON.parse(glue.beatscript_check(f.text));
    ok('the upgraded text checks as BeatScript 2 to that CID', c2.version === 2 && c2.cid === f.cid && c2.renamed_cid === c.cid);
    const again = JSON.parse(glue.beatscript_fmt(f.text));
    ok('a BeatScript 2 text formats to its own CID, no from_cid', again.cid === f.cid && again.from_cid === undefined && again.text === f.text);
}

// ---- report
const med = (a) => { const s = [...a].sort((x, y) => x - y); return s.length ? s[Math.floor(s.length / 2)] : 0; };
console.log(`\nmodule: ${path.join(wasmDir, 'beatscript_bg.wasm')} ${(wasmBytes / 1048576).toFixed(2)} MB, init ${initMs.toFixed(1)} ms (${(memInit / 1048576).toFixed(1)} MB of memory), new Dj ${djMs.toFixed(0)} ms (memory then ${(memDj / 1048576).toFixed(1)} MB), + DjComposer ${(memComposer / 1048576).toFixed(1)} MB`);
console.log(`programs: ${files.length} (${checked} checked, ${simulated} on the fake clock, ${rendered} rendered ${SECONDS} s); beatscript compared: ${beatscript ? 'yes' : 'no (not built)'}`);
console.log(`beatscript_check median ${med(timing.check).toFixed(1)} ms; prepare_share median ${med(timing.prepare).toFixed(0)} ms (max ${Math.max(0, ...timing.prepare).toFixed(0)}); prepared track median ${(med(timing.bytes) / 1024).toFixed(0)} KB in ${med(timing.steps)} install steps of ${BUDGET / 1024} KB; slowest install step ${timing.slowestStep.toFixed(2)} ms (a render quantum is 2.67 ms at 48 kHz)`);
console.log(`${passed} passed, ${failures.length} failed`);
process.exit(failures.length ? 1 : 0);

// which CIDs differ, for a failure message
function cidDiff(a, b) {
    try {
        const x = JSON.parse(a), y = JSON.parse(b);
        const out = [];
        for (const k of ['cid', 'nets', 'song_nets']) if (JSON.stringify(x[k]) !== JSON.stringify(y[k])) out.push(`${k}: ${JSON.stringify(x[k])} vs ${JSON.stringify(y[k])}`);
        return out.join('; ') || 'CIDs equal, other fields differ';
    } catch { return 'not JSON'; }
}
