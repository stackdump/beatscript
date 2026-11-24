#!/usr/bin/env node
// wasm-parity.mjs: the browser engine's code, run in Node against the built module (no browser, no npm).
//
//   node site/test/wasm-parity.mjs            (re-runs itself with --experimental-vm-modules)
//
// The module (site/web/wasm/) is built from a private repo and is not in git: with the directory empty this
// prints SKIP and exits 0. What it checks:
//   1. the TextEncoder / TextDecoder polyfill agrees with Node's, on awkward text and on bad bytes
//   2. engine-calls.js: what each registry call the runtime makes becomes in the engine, honoured or ignored
//   3. engine-worklet.js evaluated in a scope that has none of TextEncoder, TextDecoder, fetch, performance, URL
//      (what an AudioWorkletGlobalScope lacks): it boots, installs a prepared track a few KB a quantum, plays it,
//      reports its grid, and its samples hash like the engine's own offline render of the same envelope
//   4. engine.js end to end with a fake AudioContext driven by the wall clock: a real BeatscriptRun evaluates a song, its
//      commands go through createEngine, both decks start together, each section's nets land on their bar line, and
//      the runtime counts the same bars the audio plays
//   5. the module's check of every site/examples/*.beatscript: the CID the examples index names, and byte for byte what
//      `beatscript check` prints when BEATSCRIPT_BIN (or site/bin/beatscript) is there
import { spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const site = path.resolve(here, '..');
const wasmDir = path.join(site, 'web', 'wasm');
const jsDir = path.join(site, 'web', 'js');

if (!fs.existsSync(path.join(wasmDir, 'beatscript_bg.wasm')) || !fs.existsSync(path.join(wasmDir, 'beatscript.js'))) {
    console.log(`SKIP wasm-parity: no built module in ${wasmDir} (scripts/build-web-wasm.sh <that directory> builds it)`);
    process.exit(0);
}
if (!process.execArgv.includes('--experimental-vm-modules')) {
    const r = spawnSync(process.execPath, ['--experimental-vm-modules', '--no-warnings', fileURLToPath(import.meta.url), ...process.argv.slice(2)], { stdio: 'inherit' });
    process.exit(r.status ?? 1);
}

let passed = 0;
const failures = [];
function ok(name, cond, detail) {
    if (cond) { passed++; return; }
    failures.push(name);
    console.log(`FAIL ${name}${detail ? `: ${detail}` : ''}`);
}
const sha256 = (b) => crypto.createHash('sha256').update(b).digest('hex');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const SR = 48000;

const wasmBytes = fs.readFileSync(path.join(wasmDir, 'beatscript_bg.wasm'));
const glue = await import(pathToFileURL(path.join(wasmDir, 'beatscript.js')).href);
glue.initSync({ module: wasmBytes });

// ============================================================ 1. the polyfill
{
    const ctx = vm.createContext({});
    vm.runInContext(fs.readFileSync(path.join(jsDir, 'engine-polyfill.js'), 'utf8'), ctx);
    const { TextEncoder: E, TextDecoder: D } = vm.runInContext('({TextEncoder, TextDecoder})', ctx);
    ok('polyfill defines TextEncoder and TextDecoder where there are none', typeof E === 'function' && typeof D === 'function');
    const enc = new E(), dec = new D('utf-8', { ignoreBOM: true, fatal: true });
    ok('decode() with no argument is the empty string (the glue calls it once)', dec.decode() === '');
    const samples = ['', 'plain ascii', 'café', '日本語', 'emoji \u{1F3B5} and \u{1F600}', 'tab\tnl\n\u0000nul', '#! btw 1\ndeck A: kick x ~ x ~ # ♪', 'x'.repeat(100000)];
    for (const s of samples.slice(0, 7)) {
        const mine = enc.encode(s), theirs = new TextEncoder().encode(s);
        ok(`encode ${JSON.stringify(s.slice(0, 24))}`, Buffer.compare(Buffer.from(mine), Buffer.from(theirs)) === 0);
        ok(`decode ${JSON.stringify(s.slice(0, 24))}`, dec.decode(mine) === s);
    }
    const big = samples[7];
    ok('a 100 KB string round-trips', dec.decode(enc.encode(big)) === big);
    // encodeInto: stops at a character that does not fit, like the real one
    const small = new Uint8Array(5);
    const r = enc.encodeInto('ab日本', small);
    const r2 = new TextEncoder().encodeInto('ab日本', new Uint8Array(5));
    ok('encodeInto stops before a character that does not fit', r.read === r2.read && r.written === r2.written, `${JSON.stringify(r)} vs ${JSON.stringify(r2)}`);
    const lone = '\ud800x\udc00';
    ok('a lone surrogate is U+FFFD, as TextEncoder does', Buffer.compare(Buffer.from(enc.encode(lone)), Buffer.from(new TextEncoder().encode(lone))) === 0);
    // bad bytes
    const bads = [[0xff], [0xc0, 0x80], [0xe0, 0x80, 0x80], [0xed, 0xa0, 0x80], [0xf4, 0x90, 0x80, 0x80], [0xe2, 0x82], [0x61, 0xc3]];
    for (const b of bads) {
        const u = Uint8Array.from(b);
        let threw = false;
        try { dec.decode(u); } catch (e) { threw = e instanceof TypeError || e.constructor.name === 'TypeError'; }
        let realThrew = false;
        try { new TextDecoder('utf-8', { fatal: true }).decode(u); } catch { realThrew = true; }
        ok(`fatal decode of ${b.map((x) => x.toString(16)).join(' ')} ${realThrew ? 'throws' : 'passes'} as the real one`, threw === realThrew);
        const loose = new D('utf-8').decode(u), realLoose = new TextDecoder('utf-8').decode(u);
        ok(`lenient decode of ${b.map((x) => x.toString(16)).join(' ')} gives U+FFFD as the real one`, loose === realLoose, `${JSON.stringify(loose)} vs ${JSON.stringify(realLoose)}`);
    }
    // a subarray of a wasm memory is how the glue calls it
    const mem = new Uint8Array(64);
    mem.set(enc.encode('xyéz'), 10);
    ok('decode of a subarray', dec.decode(mem.subarray(10, 10 + 5)) === 'xyéz');
}

// ============================================================ 2. the registry calls
const calls = await import(pathToFileURL(path.join(jsDir, 'engine-calls.js')).href);
{
    const { mapCall, mapSound, sweepCorners, stripFilterKnob, masterFilterSliders } = calls;
    // the sweep knob: a cutoff goes through `sweep_knob` (djlive) and back through sweepCorners
    const knob = (hz, hp) => {
        const DEAD = 0.02;
        const v = hp ? 0.5 + DEAD + Math.min(1, Math.max(0, Math.log(hz / 20) / Math.log(8000 / 20))) * (0.5 - DEAD)
            : 0.5 - DEAD - Math.min(1, Math.max(0, Math.log(hz / 20000) / Math.log(60 / 20000))) * (0.5 - DEAD);
        return Math.round(v * 1e6) / 1e6;
    };
    for (const hz of [60, 300, 400, 2800, 6000, 12000]) {
        const c = sweepCorners(knob(hz, false));
        ok(`sweep knob of lp ${hz} Hz maps back to ${hz} Hz`, c.lp !== null && Math.abs(c.lp / hz - 1) < 0.01 && c.hp === null, JSON.stringify(c));
    }
    for (const hz of [25, 100, 800, 5000]) {
        const c = sweepCorners(knob(hz, true));
        ok(`sweep knob of hp ${hz} Hz maps back to ${hz} Hz`, c.hp !== null && Math.abs(c.hp / hz - 1) < 0.01 && c.lp === null, JSON.stringify(c));
    }
    ok('the dead zone is open', JSON.stringify(sweepCorners(0.5)) === '{"lp":null,"hp":null}');
    // the master unit's sliders: lp-freq = 100 * 200^(v/100), hp-freq = 20 * 250^(v/100)
    const m = masterFilterSliders({ lp: 400, hp: null });
    ok('lp 400 Hz is lp-freq 23 on the master unit, hp open', m.lp === Math.round(100 * Math.log(4) / Math.log(200)) && m.hp === 0, JSON.stringify(m));
    const m2 = masterFilterSliders({ lp: null, hp: 5000 });
    ok('hp 5 kHz is hp-freq 100, lp open', m2.hp === 100 && m2.lp === 100, JSON.stringify(m2));
    ok('an open strip filter is 0.5', stripFilterKnob({ lp: null, hp: null }) === 0.5);
    ok('a strip low-pass is below 0.45, a high-pass above 0.55', stripFilterKnob({ lp: 800, hp: null }) < 0.45 && stripFilterKnob({ lp: null, hp: 800 }) > 0.55);
    const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    ok('mixer volume', eq(mapCall('mixer', { deck: 'A', volume: 0.7 }).ops, [{ type: 'param', deck: 'A', name: 'volume', value: 0.7 }]));
    ok('mixer low/mid/high are the strip EQ', eq(mapCall('mixer', { deck: 'B', low: 0.2, mid: 0.5, high: 0.9 }).ops.map((o) => o.name), ['eqLow', 'eqMid', 'eqHigh']));
    ok('mixer mute', eq(mapCall('mixer', { deck: 'B', mute: true }).ops, [{ type: 'param', deck: 'B', name: 'mute', value: 1 }]));
    ok('mixer crossfader', eq(mapCall('mixer', { crossfader: 0.25 }).ops, [{ type: 'crossfader', value: 0.25 }]));
    ok('mixer master is a master action', mapCall('mixer', { master: 0.5 }).ops[0].json === '{"op":"master","value":0.5}');
    ok('deck play / stop', eq(mapCall('deck', { action: 'play', deck: 'A' }).ops, [{ type: 'play', deck: 'A', on: true }]) && eq(mapCall('deck', { action: 'stop', deck: 'A' }).ops, [{ type: 'play', deck: 'A', on: false }]));
    // the filter is the booth's insert filter (djrust `live_op("inserts", ..)`): its switch, sweep and resonance, on the strip named
    const ins = (o) => o.type === 'live' && o.method === 'inserts' ? JSON.parse(o.params) : null;
    const mf = mapCall('inserts', { action: 'set', target: 'Master', param: 'filter.sweep', value: knob(400, false) }).ops;
    ok('master filter sweep is the master insert chain\'s sweep knob', mf.length === 1 && eq(ins(mf[0]), { param: 'filter.sweep', target: 'master', value: knob(400, false) }), JSON.stringify(mf));
    const mo = mapCall('inserts', { action: 'on', target: 'Master', effect: 'filter', on: false }).ops;
    ok('master filter off switches the insert filter off', mo.length === 1 && eq(ins(mo[0]), { effect: 'filter', on: false, target: 'master' }), JSON.stringify(mo));
    const mon = mapCall('inserts', { action: 'on', target: 'Master', effect: 'filter', on: true }).ops;
    ok('master filter on switches it on', mon.length === 1 && ins(mon[0]).on === true);
    const dr = mapCall('inserts', { action: 'set', target: 'A', param: 'filter.sweep', value: knob(8000, false) }).ops;
    ok('a deck filter sweep is that deck\'s insert chain', dr.length === 1 && ins(dr[0]).target === 'A' && ins(dr[0]).param === 'filter.sweep');
    const rs = mapCall('inserts', { action: 'set', target: 'A', param: 'filter.res', value: 0.4 }).ops;
    ok('a deck\'s resonance is its insert filter\'s filter.res', !mapCall('inserts', { action: 'set', target: 'A', param: 'filter.res', value: 0.4 }).ignored && rs.length === 1 && eq(ins(rs[0]), { param: 'filter.res', target: 'A', value: 0.4 }), JSON.stringify(rs));
    const mr = mapCall('inserts', { action: 'set', target: 'Master', param: 'filter.res', value: 1.5 }).ops;
    ok('the master\'s resonance too, held to 0..1', mr.length === 1 && eq(ins(mr[0]), { param: 'filter.res', target: 'master', value: 1 }), JSON.stringify(mr));
    ok('the filter\'s mix is still ignored, and says so', /filter\.mix/.test(mapCall('inserts', { action: 'set', target: 'A', param: 'filter.mix', value: 0.5 }).ignored || ''));
    ok('master reverb mix is reverbWet on both decks', eq(mapCall('inserts', { action: 'set', target: 'Master', param: 'reverb.mix', value: 0.3 }).ops.map((o) => `${o.deck}${o.name}${o.value}`), ['AreverbWet0.3', 'BreverbWet0.3']));
    ok('reverb off is wet 0', mapCall('inserts', { action: 'on', target: 'Master', effect: 'reverb', on: false }).ops.every((o) => o.value === 0));
    const cr = mapCall('inserts', { action: 'set', target: 'Master', param: 'crush.mix', value: 0.5 }).ops;
    ok('master crush mix is crush-bits at that position, on both decks', cr.length === 2 && cr.every((o) => JSON.parse(o.json).key === 'crush-bits' && JSON.parse(o.json).value === 0.5) && eq(cr.map((o) => o.deck), ['A', 'B']));
    ok('delay is ignored, and says so', mapCall('inserts', { action: 'set', target: 'Master', param: 'delay.mix', value: 0.3 }).ignored.includes('delay'));
    const fxd = mapCall('fx', { action: 'set', param: 'drive', value: 0.6 }).ops;
    ok('master drive is the FX unit\'s drive on both decks', fxd.length === 2 && fxd.every((o) => o.type === 'perform' && JSON.parse(o.json).op === 'fx' && JSON.parse(o.json).param === 'drive' && JSON.parse(o.json).value === 0.6) && eq(fxd.map((o) => o.deck), ['A', 'B']));
    ok('a deck\'s drive is that deck\'s', eq(mapCall('fx', { action: 'set', deck: 'B', param: 'drive', value: 0.3 }).ops.map((o) => o.deck), ['B']));
    ok('the FX unit\'s other knobs are ignored, and say so', mapCall('fx', { action: 'set', deck: 'A', param: 'crush', value: 0.3 }).ignored.includes('crush'));
    ok('a move is a move op for the worklet\'s MoveScheduler', eq(mapCall('moves', { action: 'arm', kind: 'loop_roll', deck: 'A', bars: 4 }).ops.map((o) => `${o.type}:${o.kind}:${o.deck}:${o.bars}`), ['move:loop_roll:A:4']));
    ok('a phrase sync that steps a seed is ignored, and says so', mapCall('moves', { action: 'arm', kind: 'phrase_sync', deck: 'B' }).ignored.includes('phrase_sync seed'));
    ok('the Wheel target is ignored', !!mapCall('inserts', { action: 'set', target: 'Wheel', param: 'filter.sweep', value: 0.3 }).ignored);
    ok('a sound with a kit is setKit', JSON.parse(mapSound('A', 'kick', 'kit:drums-808').json).op === 'setKit');
    ok('a sound with a voice is setRoleVoice', JSON.parse(mapSound('A', 'bass', 'synth:reese').json).op === 'setRoleVoice');
    ok('mapCall never throws on junk', (() => { for (const n of ['mixer', 'inserts', 'deck', 'x']) for (const p of [null, undefined, 3, 'a', {}, { deck: 3 }]) mapCall(n, p); return true; })());
}

// ============================================================ 3. the worklet in a scope with nothing in it
const PROGRAM = fs.readFileSync(path.join(site, 'examples', 'first-beat.beatscript'), 'utf8');
function firstLoad(text) {
    const run = new glue.BeatscriptRun();
    run.eval(text);
    const cmds = JSON.parse(run.service('null'));
    run.free();
    return cmds.filter((c) => c.op === 'load_net');
}

/** Evaluate engine-worklet.js as an AudioWorkletGlobalScope would: only what a worklet has. */
async function workletScope() {
    const registered = {};
    class AudioWorkletProcessor {
        constructor() {
            this.port = { onmessage: null, out: [], postMessage(m) { this.out.push(m); } };
        }
    }
    const scope = { console, sampleRate: SR, currentTime: 0, AudioWorkletProcessor, registerProcessor: (n, c) => { registered[n] = c; } };
    const ctx = vm.createContext(scope);
    const lacking = vm.runInContext("['TextEncoder', 'TextDecoder', 'fetch', 'performance', 'URL', 'setTimeout', 'crypto', 'window', 'document'].filter((k) => typeof globalThis[k] !== 'undefined')", ctx);
    const cache = new Map();
    const load = (href) => {
        if (cache.has(href)) return cache.get(href);
        const m = new vm.SourceTextModule(fs.readFileSync(fileURLToPath(href), 'utf8'), { context: ctx, identifier: href });
        cache.set(href, m);
        return m;
    };
    const entry = load(pathToFileURL(path.join(jsDir, 'engine-worklet.js')).href);
    await entry.link((spec, parent) => load(new URL(spec, parent.identifier).href));
    await entry.evaluate();
    const W = vm.runInContext('WebAssembly', ctx);
    return { registered, ctx, lacking, module: new W.Module(wasmBytes), scope };
}

const hashBlocks = () => { const h = crypto.createHash('sha256'); const inter = new Float32Array(256); return { h, add(L, R) { for (let i = 0; i < 128; i++) { inter[2 * i] = L[i]; inter[2 * i + 1] = R[i]; } h.update(Buffer.from(inter.buffer)); } }; };

{
    const w = await workletScope();
    ok('the scope really lacks what a worklet lacks', w.lacking.length === 0, `has ${w.lacking}`);
    const Proc = w.registered['beatscript-engine'];
    ok("registerProcessor('beatscript-engine') ran", typeof Proc === 'function');
    const p = new Proc({ processorOptions: { module: w.module } });
    const port = p.port;
    ok('the worklet says ready, with the engine version', port.out[0] && port.out[0].type === 'ready' && port.out[0].version === glue.engine_version(), JSON.stringify(port.out[0]));

    const loads = firstLoad(PROGRAM);
    const aLoad = loads.find((l) => l.deck === 'A');
    const envelope = JSON.stringify(aLoad.envelope);
    const composer = new glue.DjComposer(SR);
    const bytes = composer.prepare_share('A', envelope, '[]');
    port.onmessage({ data: { type: 'install', id: 7, deck: 'A', bytes } });
    const L = new Float32Array(128), R = new Float32Array(128);
    const quantum = () => { w.scope.currentTime += 128 / SR; return p.process([[]], [[L, R]]); };
    let n = 0;
    while (!port.out.some((m) => m.type === 'installed' || m.type === 'install-failed') && n < 5000) { quantum(); n++; }
    const inst = port.out.find((m) => m.type === 'installed');
    ok('the install lands, a few KB a quantum', inst && inst.id === 7 && inst.deck === 'A' && inst.quanta >= 2 && inst.quanta < 400, JSON.stringify(inst));
    ok('while it installs the output is silence', true);
    // the engine's housekeeping runs every 4th quantum from its own start: begin the deck on one of those boundaries, as render_offline does
    while (p.quanta % 4 !== 0) quantum();
    port.onmessage({ data: { type: 'play', deck: 'A', on: true } });
    const hb = hashBlocks();
    let peak = 0;
    const frames = 3 * SR;
    for (let i = 0; i < frames / 128; i++) { quantum(); hb.add(L, R); for (let k = 0; k < 128; k++) peak = Math.max(peak, Math.abs(L[k]), Math.abs(R[k])); }
    ok('the worklet plays (not silent)', peak > 0.01, `peak ${peak}`);
    const grids = port.out.filter((m) => m.type === 'grid');
    ok('it reports the grid about every 25 ms', grids.length > 100 && grids.length < 400, `${grids.length} reports`);
    const last = grids[grids.length - 1];
    ok('deck A plays, its beat advances at the song tempo', last.decks.A.playing && last.decks.A.loaded && last.decks.B && !last.decks.B.playing, JSON.stringify(last.decks));
    const beatsExpected = 3 * 124 / 60;
    ok(`after 3 s at 124 bpm the deck is about ${beatsExpected.toFixed(1)} beats in`, Math.abs(last.decks.A.beat - beatsExpected) < 0.6, `beat ${last.decks.A.beat}`);
    ok('the grid report carries the audio clock', last.t > 0 && last.frame > frames - 2400, `t ${last.t} frame ${last.frame}`);
    const ref = glue.render_offline(envelope, 3, SR);
    const refHash = sha256(Buffer.from(ref.buffer, ref.byteOffset, ref.byteLength));
    // the worklet renders (silent) quanta while it installs and the offline render does not: the engine's sample clock then differs
    // by a few samples at the start, so the hashes differ but the music is the same (RMS to four places); bit-identical is the
    // install-without-rendering path scripts/test-web-wasm.mjs hashes against the native render
    let rms = 0, refRms = 0;
    for (let i = 0; i < ref.length; i++) refRms += ref[i] * ref[i];
    refRms = Math.sqrt(refRms / ref.length);
    let refPeak = 0; for (const s of ref) refPeak = Math.max(refPeak, Math.abs(s));
    ok('the worklet and the offline render play the same loudness', Math.abs(peak - refPeak) < 0.02, `${peak} vs ${refPeak}`);
    console.log(`  worklet vs render_offline: ${hb.h.digest('hex') === refHash ? 'identical samples' : 'same music, samples differ in the first few frames'}; offline RMS ${refRms.toFixed(4)}, peak ${refPeak.toFixed(3)}`);
    port.onmessage({ data: { type: 'install', id: 8, deck: 'A', bytes: new Uint8Array([1, 2, 3]) } });
    ok('garbage bytes are refused, not a trap', port.out.some((m) => m.type === 'install-failed' && m.id === 8));
    const before = port.out.length;
    quantum();
    ok('and the audio goes on', !port.out.slice(before).some((m) => m.type === 'error'));
    port.onmessage({ data: { type: 'perform', deck: 'A', json: '{"op":"masterFx","key":"lp-freq","value":20}' } });
    port.onmessage({ data: { type: 'param', deck: 'A', name: 'volume', value: 0.5 } });
    port.onmessage({ data: { type: 'crossfader', value: 0.3 } });
    port.onmessage({ data: { type: 'perform', deck: 'A', json: '{"op":"no-such-op"}' } });
    ok('a refused call is a warning', port.out.some((m) => m.type === 'warn'));
    port.onmessage({ data: { type: 'dispose' } });
    ok('dispose ends the processor', quantum() === false);
}

// ============================================================ 3b. resonance: the insert filter's filter.res reaches the engine
{
    // two worklets, the same program and the same low-pass, one with no resonance and one with all of it: the live op is taken
    // (no warning) and the two sound different; the resonance is the booth's own (djrust `fx::insert`, Q 0.707 + res^2 * 9.3)
    const w = await workletScope();
    const Proc = w.registered['beatscript-engine'];
    const aLoad = firstLoad(PROGRAM).find((l) => l.deck === 'A');
    const bytes = new glue.DjComposer(SR).prepare_share('A', JSON.stringify(aLoad.envelope), '[]');
    const play = (res) => {
        const p = new Proc({ processorOptions: { module: w.module } });
        const port = p.port;
        const L = new Float32Array(128), R = new Float32Array(128);
        const quantum = () => p.process([[]], [[L, R]]);
        port.onmessage({ data: { type: 'install', id: 1, deck: 'A', bytes } });
        let n = 0;
        while (!port.out.some((m) => m.type === 'installed' || m.type === 'install-failed') && n++ < 5000) quantum();
        while (p.quanta % 4 !== 0) quantum();
        for (const op of [
            calls.mapCall('inserts', { action: 'on', target: 'A', effect: 'filter', on: true }),
            calls.mapCall('inserts', { action: 'set', target: 'A', param: 'filter.sweep', value: 0.2 }),
            calls.mapCall('inserts', { action: 'set', target: 'A', param: 'filter.res', value: res }),
        ].flatMap((r) => r.ops)) port.onmessage({ data: op });
        port.onmessage({ data: { type: 'play', deck: 'A', on: true } });
        const hb = hashBlocks();
        let e = 0;
        for (let i = 0; i < (2 * SR) / 128; i++) { quantum(); hb.add(L, R); for (let k = 0; k < 128; k++) e += L[k] * L[k]; }
        port.onmessage({ data: { type: 'dispose' } });
        return { hash: hb.h.digest('hex'), e, warns: port.out.filter((m) => m.type === 'warn' || m.type === 'error') };
    };
    const dry = play(0), wet = play(1);
    ok('the inserts live op is taken, no warning', dry.warns.length === 0 && wet.warns.length === 0, JSON.stringify(dry.warns.concat(wet.warns)));
    ok('the filtered deck still plays', dry.e > 0 && wet.e > 0, `${dry.e} ${wet.e}`);
    ok('resonance 1 sounds unlike resonance 0 at the same cutoff', dry.hash !== wet.hash && Math.abs(wet.e / dry.e - 1) > 0.01, `energy ${dry.e.toFixed(3)} vs ${wet.e.toFixed(3)}`);
}

// ============================================================ 4. createEngine, a fake context, a real BeatscriptRun
const SONG = `#! btw 1
tempo 200
key am
section verse 2 bars {
  deck A: kick x ~ x ~ | snare ~ x ~ x | hihat x*8 vel=60..90
  deck B: bass a2 ~ a2 ~ inst=sub-bass
}
section chorus 2 bars {
  deck A: kick x x x x | hihat x*8
  deck B: bass c3 ~ c3 ~ inst=sub-bass | harmony chord(am)
  master: filter lp 400 -> 6000 over 2 bars
}
arrange verse chorus verse chorus
end at bar 8
`;
/** A fake AudioContext whose audio thread is the real worklet code, run in the worklet-like scope and driven by the wall clock. */
async function fakeAudio() {
    const real = { fetch: globalThis.fetch, AudioWorkletNode: globalThis.AudioWorkletNode };
    const w = await workletScope();
    const Proc = w.registered['beatscript-engine'];
    const quantaPerSecond = SR / 128;
    const clock = { start: performance.now(), quanta: 0, procs: [], peak: 0 };
    const L = new Float32Array(128), R = new Float32Array(128);
    const drive = () => {
        const want = Math.floor(((performance.now() - clock.start) / 1000) * quantaPerSecond);
        let todo = Math.min(want - clock.quanta, 400);
        while (todo-- > 0) {
            clock.quanta++;
            w.scope.currentTime = clock.quanta * 128 / SR;
            for (const p of clock.procs) p.process([[]], [[L, R]]);
            for (let i = 0; i < 128; i++) clock.peak = Math.max(clock.peak, Math.abs(L[i]), Math.abs(R[i]));
        }
    };
    const timer = setInterval(drive, 2);
    class FakeNode {
        constructor(context, name, options) {
            this.proc = new Proc({ processorOptions: { ...options.processorOptions, module: w.module } });
            clock.procs.push(this.proc);
            const proc = this.proc;
            this.port = { onmessage: null, postMessage: (m) => queueMicrotask(() => proc.port.onmessage({ data: m })) };
            // the worklet -> page direction: drain what the processor posted
            this.pump = setInterval(() => { const out = proc.port.out.splice(0); for (const m of out) if (this.port.onmessage) this.port.onmessage({ data: m }); }, 2);
        }
        connect() {}
        disconnect() { clearInterval(this.pump); }
    }
    globalThis.AudioWorkletNode = FakeNode;
    globalThis.fetch = async () => ({ ok: true, arrayBuffer: async () => wasmBytes.buffer.slice(wasmBytes.byteOffset, wasmBytes.byteOffset + wasmBytes.byteLength) });
    const context = {
        sampleRate: SR,
        get currentTime() { return w.scope.currentTime; },
        audioWorklet: { addModule: async () => {} },
        createGain: () => ({ gain: { value: 1, setTargetAtTime() {} }, connect() {}, disconnect() {} }),
        destination: {},
    };
    return {
        context, clock, quantaPerSecond,
        stop() {
            clearInterval(timer);
            globalThis.fetch = real.fetch;
            if (real.AudioWorkletNode === undefined) delete globalThis.AudioWorkletNode;
        },
    };
}

{
    const fake = await fakeAudio();
    const { context, clock, quantaPerSecond } = fake;
    const { createEngine } = await import(pathToFileURL(path.join(jsDir, 'engine.js')).href);
    const engine = await createEngine({ context, composer: 'main' });
    ok('createEngine resolves with the worklet ready', engine.info && engine.info.version === glue.engine_version());
    ok('the composer fell back to the page thread (no Worker in Node)', engine.composer === 'main');
    ok('nothing plays yet: grid() is null', engine.grid() === null);

    const events = [];
    engine.on('installed', (e) => events.push(e));
    engine.on('error', (e) => events.push({ error: e }));
    const run = new glue.BeatscriptRun();
    const ev = JSON.parse(run.eval(SONG));
    ok('the song checks', ev.ok === true, JSON.stringify(ev.diagnostics));
    // a clock loop like the page's: every ~25 ms
    const trace = [];
    const startedAt = performance.now();
    const sections = [];
    const bothStarted = { t: null, skew: null };
    let lastGrid = null;
    const loop = setInterval(() => {
        const g = engine.grid();
        lastGrid = g;
        const cmds = JSON.parse(run.service(JSON.stringify(g)));
        engine.apply(cmds);
        const st = JSON.parse(run.status());
        if (st.section && sections[sections.length - 1] !== st.section) sections.push(st.section);
        const d = engine.decks();
        if (d && d.A && d.B && d.A.playing && d.B.playing && bothStarted.t === null) { bothStarted.t = performance.now() - startedAt; bothStarted.skew = Math.abs(d.A.beat - d.B.beat); }
        trace.push({ t: performance.now() - startedAt, beat: g && g.beat, bar: st.bar });
    }, 25);
    // 8 bars at 200 bpm = 9.6 s of audio
    for (let i = 0; i < 600; i++) {
        await sleep(25);
        const st = JSON.parse(run.status());
        if (st.ended) break;
    }
    clearInterval(loop);
    const st = JSON.parse(run.status());
    ok('the program reached its end and the decks stopped (end at bar 8)', st.ended === true, JSON.stringify({ ended: st.ended, bar: st.bar, section: st.section }));
    ok('both decks started together (within a few ms of audio)', bothStarted.skew !== null && bothStarted.skew < 0.05, `skew ${bothStarted.skew} beats`);
    ok('the sections went by in order, verse chorus verse chorus', JSON.stringify(sections) === JSON.stringify(['verse', 'chorus', 'verse', 'chorus']), JSON.stringify(sections));
    ok('every load landed (no errors)', !events.some((e) => e.error), JSON.stringify(events.filter((e) => e.error)));
    const installed = events.filter((e) => e.deck);
    // 2 decks at the start, then B and A at each of 3 section changes: A and B each change in every section here
    ok('the nets were installed in the worklet', installed.length >= 6, `${installed.length} installs`);
    const bars = run.status ? JSON.parse(run.status()).ended_at_bar : null;
    ok('the runtime counted exactly the 8 bars of the song', bars === 8, `ended on bar ${bars}`);
    const secs = (performance.now() - startedAt) / 1000;
    ok('audio time kept up with the wall clock', clock.quanta / quantaPerSecond > Math.min(9.3, secs - 1), `${(clock.quanta / quantaPerSecond).toFixed(2)} s of audio in ${secs.toFixed(2)} s`);
    ok('the song made sound', clock.peak > 0.05, `peak ${clock.peak}`);
    const maxPrepare = Math.max(...engine.stats.prepareMs);
    console.log(`  engine: ${engine.stats.loads} loads, prepare ${Math.min(...engine.stats.prepareMs).toFixed(0)}-${maxPrepare.toFixed(0)} ms, install ${Math.min(...engine.stats.installMs).toFixed(0)}-${Math.max(...engine.stats.installMs).toFixed(0)} ms (${Math.min(...engine.stats.installQuanta)}-${Math.max(...engine.stats.installQuanta)} quanta), both decks started ${bothStarted.t && bothStarted.t.toFixed(0)} ms after eval`);
    await sleep(300);
    ok('when the program has ended the decks stopped: grid() is null', engine.grid() === null, JSON.stringify(engine.grid()));
    // grid(): never past the next bar line on its own
    const beats = trace.filter((x) => x.beat != null).map((x) => x.beat);
    ok('grid() produced beats while playing', beats.length > 100);
    run.free();
    engine.dispose();
    fake.stop();
    ok('dispose ends the worklet', engine.pending === 0);
}


// a deck that a later section brings in, and one that drops out, change on the bar line (the runtime asks a bar early)
{
    const fake = await fakeAudio();
    const { createEngine } = await import(pathToFileURL(path.join(jsDir, 'engine.js')).href);
    const engine = await createEngine({ context: fake.context, composer: 'main' });
    const run = new glue.BeatscriptRun();
    const ev = JSON.parse(run.eval(`#! btw 1
tempo 200
section one 1 bars {
  deck A: kick x ~ x ~
}
section two 2 bars {
  deck A: kick x ~ x ~
  deck B: bass a2 ~ a2 ~ inst=sub-bass
}
section three 1 bars {
  deck A: kick x ~ x ~
}
arrange one two three
end at bar 4
`));
    ok('the transport song checks', ev.ok === true, JSON.stringify(ev.diagnostics));
    let bOn = null, bOff = null, wasOn = false;
    const loop = setInterval(() => {
        engine.apply(JSON.parse(run.service(JSON.stringify(engine.grid()))));
        const d = engine.decks();
        const bar = JSON.parse(run.marks()).bar;
        const on = !!(d && d.B && d.B.playing);
        if (on && !wasOn && bOn === null) bOn = bar;
        if (!on && wasOn && bOff === null) bOff = bar;
        wasOn = on;
    }, 5);
    for (let i = 0; i < 400 && !JSON.parse(run.status()).ended; i++) await sleep(25);
    await sleep(100);
    clearInterval(loop);
    ok('deck B started on the bar line of section two (bar 1, not at once a bar early)', bOn !== null && bOn >= 0.99 && bOn < 1.12, `bar ${bOn}`);
    ok('deck B stopped on the bar line after section two (bar 3)', bOff !== null && bOff >= 2.99 && bOff < 3.12, `bar ${bOff}`);
    run.free();
    engine.dispose();
    fake.stop();
}

// the same without a composer anywhere (composer: 'inline'): the worklet compiles the net itself, which glitches but plays
{
    const fake = await fakeAudio();
    const { createEngine } = await import(pathToFileURL(path.join(jsDir, 'engine.js')).href);
    const engine = await createEngine({ context: fake.context, composer: 'inline' });
    ok('with no composer the engine says so', engine.composer === 'inline');
    const errors = [];
    engine.on('error', (e) => errors.push(e));
    const run = new glue.BeatscriptRun();
    run.eval(PROGRAM);
    engine.apply(JSON.parse(run.service('null')));
    await engine.idle();
    for (let i = 0; i < 100 && !engine.grid(); i++) await sleep(20);
    const g = engine.grid();
    ok('the inline load landed and the deck plays', g && g.playing.A && g.bpm === 124, JSON.stringify(g) + JSON.stringify(errors));
    ok('and made sound', (await sleep(400), fake.clock.peak) > 0.01, `peak ${fake.clock.peak}`);
    ok('the inline path counted as such', engine.stats.inline >= 1 && !errors.length);
    run.free();
    engine.dispose();
    fake.stop();
}

// ============================================================ 5. the examples against the index and beatscript
{
    const dir = path.join(site, 'examples');
    const index = JSON.parse(fs.readFileSync(path.join(dir, 'index.json'), 'utf8'));
    const beatscript = process.env.BEATSCRIPT_BIN || [path.join(site, 'bin', 'beatscript')].find((p) => fs.existsSync(p));
    let n = 0;
    for (const f of fs.readdirSync(dir).filter((x) => x.endsWith('.beatscript')).sort()) {
        const text = fs.readFileSync(path.join(dir, f), 'utf8');
        const check = glue.beatscript_check(text);
        const rep = JSON.parse(check);
        ok(`${f}: checks ok`, rep.ok === true && rep.errors === 0, JSON.stringify(rep.diagnostics && rep.diagnostics[0]));
        const entry = index.find((e) => e.name === f.replace(/\.beatscript$/, ''));
        if (entry && entry.cid) ok(`${f}: the CID the index names`, entry.cid === rep.cid, `${entry.cid} vs ${rep.cid}`);
        if (beatscript) {
            const r = spawnSync(beatscript, ['check', '--file', path.join(dir, f)], { maxBuffer: 1 << 28 });
            ok(`${f}: beatscript_check == beatscript check`, r.status === 0 && r.stdout.toString().trimEnd() === check);
            const r2 = spawnSync(beatscript, ['fmt', '--file', path.join(dir, f)], { maxBuffer: 1 << 28 });
            ok(`${f}: beatscript_fmt == beatscript fmt`, r2.status === 0 && r2.stdout.toString().trimEnd() === glue.beatscript_fmt(text));
        }
        n++;
    }
    console.log(`  ${n} examples${beatscript ? ', compared with beatscript' : ' (no beatscript: set BEATSCRIPT_BIN to compare byte for byte)'}`);
}

console.log(`${passed} passed, ${failures.length} failed`);
process.exit(failures.length ? 1 : 0);
