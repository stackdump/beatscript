// Unit tests for the ✦generate panel's pure parts (site/web/js/generate-model.js). No npm: node:test.
//   node --test site/test/generate-unit.mjs
// Where the real wasm is built (site/web/wasm) the roles and the structure are also read off real generated programs.

import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const web = path.join(here, '..', 'web');
const M = await import(pathToFileURL(path.join(web, 'js', 'generate-model.js')).href);

let wasm = null;
{
  const glue = path.join(web, 'wasm', 'beatscript.js');
  if (fs.existsSync(glue) && !process.env.BEATSCRIPT_MOCK_WASM) {
    const m = await import(pathToFileURL(glue).href);
    m.initSync({ module: fs.readFileSync(path.join(web, 'wasm', 'beatscript_bg.wasm')) });
    if (typeof m.beatscript_generate === 'function') wasm = m;
  }
}

test('generate: a seed value is the number itself, or the FNV-1a hash of the word (beatscript-api::generate::seed_value)', () => {
  assert.equal(M.seedValue('42'), 42);
  assert.equal(M.seedValue(' 42 '), 42);
  assert.equal(M.seedValue('4294967296'), 0); // the low 32 bits, as the Rust masks a u64
  assert.equal(M.seedValue('4294967338'), 42);
  // FNV-1a 32 of "a" is 0xe40c292c; a word is trimmed and lower-cased first
  assert.equal(M.seedValue('a'), 0xe40c292c);
  assert.equal(M.seedValue(' A '), 0xe40c292c);
  assert.equal(M.seedValue('sunrise'), M.seedValue('SunRise'));
});

test('generate: the seed steps to its neighbour, wrapping in 32 bits; a word steps from its hash', () => {
  assert.equal(M.stepSeed('42', 1), '43');
  assert.equal(M.stepSeed('42', -1), '41');
  assert.equal(M.stepSeed('0', -1), '4294967295');
  assert.equal(M.stepSeed('4294967295', 1), '0');
  assert.equal(M.stepSeed('a', 1), String(0xe40c292c + 1));
  assert.equal(M.stepSeed('', 1), '2'); // an empty seed is the generator's default, 1
});

test('generate: a random seed is a whole number under 100000', () => {
  assert.equal(M.randomSeed(() => 0), '0');
  assert.equal(M.randomSeed(() => 0.999999), '99999');
  assert.match(M.randomSeed(), /^\d{1,5}$/);
});

test('generate: the request and the call say the same inputs; length is written only when not standard', () => {
  assert.deepEqual(M.genRequest({ genre: 'house', seed: ' 7 ', length: 'long' }), { genre: 'house', seed: '7', length: 'long' });
  assert.deepEqual(M.genRequest({ genre: '', seed: '', length: 'huge' }), { genre: 'techno', seed: '1', length: 'standard' });
  // the ending: `end` is the default and is not sent (the request, and so the program and its CID, are the old ones); `loop` is
  assert.deepEqual(M.genRequest({ genre: 'house', seed: '7', length: 'long', ending: 'end' }), { genre: 'house', seed: '7', length: 'long' });
  assert.deepEqual(M.genRequest({ genre: 'house', seed: '7', length: 'long', ending: 'loop' }), { genre: 'house', seed: '7', length: 'long', ending: 'loop' });
  assert.deepEqual(M.genRequest({ ending: 'forever' }), { genre: 'techno', seed: '1', length: 'standard' });
  assert.equal(M.genCall({ genre: 'techno', seed: '42', length: 'standard', section: 'drop', part: 'bass' }), 'gen@2 techno seed=42 section=drop part=bass');
  assert.equal(M.genCall({ genre: 'house', seed: '7', length: 'short', section: 'intro', part: 'kick' }), 'gen@2 house seed=7 length=short section=intro part=kick');
  // a NAME seed is written as it is; any other word as its number (the same track)
  assert.equal(M.genCall({ genre: 'techno', seed: 'sunrise' }), 'gen@2 techno seed=sunrise');
  assert.equal(M.genCall({ genre: 'techno', seed: 'two words' }), `gen@2 techno seed=${M.seedValue('two words')}`);
});

test('generate: the roles of a section come from its part lines and the defs it calls', () => {
  const text = [
    '#! generator: seeded@2',
    'def groove(k, h) {',
    '  deck A: kick $k | snare every(4, [fill], backbeat) | hihat $h vel=70..100',
    '}',
    'def band(m) {',
    '  deck B: bass bassline | harmony chords voice-lead=on | melody $m',
    '}',
    'section intro 8 bars {',
    '  master: filter lp 400 -> 2000 over 8 bars exp',
    '  deck A: kick pulse | hihat degrade(hats, 0.4, 3) vel=60..90',
    '}',
    'section drop 16 bars {',
    '  groove(pulse, hats)',
    '  band(lead)',
    '  evolve every 4 bars {',
    '    deck B: arp arps',
    '  }',
    '}',
    'section break 8 bars {',
    '  deck B: harmony chords | arp arps inst=pluck',
    '}',
  ].join('\n');
  assert.deepEqual(M.rolesIn(text, 'intro'), ['kick', 'hihat']);
  assert.deepEqual(M.rolesIn(text, 'drop'), ['kick', 'snare', 'hihat', 'bass', 'melody', 'harmony']);
  assert.deepEqual(M.rolesIn(text, 'break'), ['harmony', 'arp']);
  assert.deepEqual(M.rolesIn(text, 'nowhere'), []);
});

test('generate: the structure is the check report\'s sections in play order and its bars', () => {
  const s = M.structureOf({ sections: [{ name: 'intro', start: 0, bars: 8 }, { name: 'drop', start: 8, bars: 16 }], song_bars: 24 });
  assert.deepEqual(s, { sections: [{ name: 'intro', bars: 8 }, { name: 'drop', bars: 16 }], bars: 24 });
  assert.deepEqual(M.structureOf(null), { sections: [], bars: 0 });
});

test('generate: the remembered inputs are checked, a bad one takes the default', () => {
  const mem = (v) => ({ getItem: () => v, setItem() {} });
  const g = ['techno', 'house'];
  assert.deepEqual(M.loadGenState(mem(JSON.stringify({ genre: 'house', seed: '9', length: 'long', section: 'drop', part: 'bass' })), g), { genre: 'house', seed: '9', length: 'long', ending: 'end', section: 'drop', part: 'bass' });
  assert.equal(M.loadGenState(mem(JSON.stringify({ ending: 'loop' })), g).ending, 'loop');
  const bad = M.loadGenState(mem('{"genre":"polka","seed":"","length":"x","part":"tuba"}'), g);
  assert.equal(bad.genre, 'techno');
  assert.equal(bad.length, 'standard');
  assert.equal(bad.ending, 'end');
  assert.equal(bad.part, '');
  assert.match(bad.seed, /^\d+$/);
  assert.equal(M.loadGenState(mem('not json'), g).genre, 'techno');
  let saved = null;
  M.saveGenState({ setItem: (k, v) => { saved = [k, JSON.parse(v)]; } }, { genre: 'house', seed: '3', length: 'short', section: 'intro', part: 'kick' });
  assert.deepEqual(saved, ['beatscript.gen', { genre: 'house', seed: '3', length: 'short', section: 'intro', part: 'kick' }]);
});

test('generate: against the real generator, every section\'s roles make a gen@2 call that checks', { skip: !wasm }, () => {
  for (const [genre, seed, length] of [['techno', '42', 'standard'], ['house', '7', 'short'], ['dnb', 'sunrise', 'long']]) {
    const r = JSON.parse(wasm.beatscript_generate(JSON.stringify(M.genRequest({ genre, seed, length }))));
    assert.ok(r.ok, `${genre} ${seed}: ${r.error || ''}`);
    const { sections } = M.structureOf(r.check);
    assert.ok(sections.length >= 2, `${genre} ${seed} has sections`);
    // the step to the neighbour is a different program
    const n = JSON.parse(wasm.beatscript_generate(JSON.stringify(M.genRequest({ genre, seed: M.stepSeed(seed, 1), length }))));
    assert.notEqual(n.cid, r.cid);
    let calls = 0;
    for (const name of new Set(sections.map((x) => x.name))) {
      const roles = M.rolesIn(r.text, name);
      for (const part of roles) {
        const call = M.genCall({ genre, seed, length, section: name, part });
        const prog = `tempo ${r.bpm}\nkey ${r.key}\nlet borrowed = ${call}\ndeck A: kick borrowed\n`;
        const c = JSON.parse(wasm.beatscript_check(prog));
        const errs = (c.diagnostics || []).filter((d) => d.severity === 'error');
        assert.equal(errs.length, 0, `${call}: ${errs.map((d) => d.message).join('; ')}`);
        calls++;
      }
    }
    assert.ok(calls > 0, `${genre} ${seed}: some part to reference`);
  }
});
