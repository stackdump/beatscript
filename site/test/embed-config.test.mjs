// Tests for site/embed/embed-config.js (no npm: node:test and node:assert).
//   node --test site/test/embed-config.test.mjs
// The cases in site/testdata/embed-config/cases.json are the contract btwserve's Rust mirror is held to too, so they are
// compared as JSON text: the key order is part of the output.

import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const site = path.join(here, '..');
const M = await import(pathToFileURL(path.join(site, 'embed', 'embed-config.js')).href);
const { SCHEMA, CAPABILITIES, normalize, fromElement, toAttributes, artifactCapabilities, detectHost, resolve, NO_WASM, NO_WASM_STUDIO } = M;

const schemaFile = JSON.parse(fs.readFileSync(path.join(site, 'web', 'embed-config.schema.json'), 'utf8'));
const cases = JSON.parse(fs.readFileSync(path.join(site, 'testdata', 'embed-config', 'cases.json'), 'utf8'));

const CID = 'z4EBG9izEDKtK6ErJfgHcqL5VTiaPVyz3BvCzju4znbxpQjCNDi';
const same = (got, want, msg) => assert.equal(JSON.stringify(got), JSON.stringify(want), msg);

/** An element as fromElement sees it: attributes, an optional config script and an optional program script. */
function fakeEl({ tag = 'beatscript-track', attrs = {}, json = null, script = null } = {}) {
  return {
    tagName: tag.toUpperCase(),
    getAttribute: (n) => (Object.hasOwn(attrs, n) ? attrs[n] : null),
    hasAttribute: (n) => Object.hasOwn(attrs, n),
    querySelector: (sel) => {
      if (sel.includes('beatscript-config')) return json == null ? null : { textContent: json };
      if (sel.includes('text/beatscript')) return script == null ? null : { textContent: script };
      return null;
    },
  };
}

const parse = (raw) => {
  const r = normalize(raw);
  assert.ok(r.ok, JSON.stringify(r.errors));
  return r.config;
};

test('SCHEMA is the JSON file, key for key and in the same order', () => {
  assert.deepStrictEqual(SCHEMA, schemaFile);
  assert.equal(JSON.stringify(SCHEMA), JSON.stringify(schemaFile));
  assert.ok(Object.isFrozen(SCHEMA.properties.kind.enum));
});

test('defaultsFor gives each kind its audio, features and capabilities, in schema order', () => {
  same(M.defaultsFor('player'), { audio: 'live', features: ['share'], capabilities: ['downloads'] });
  same(M.defaultsFor('studio'), { audio: 'live', features: ['highlight', 'editor', 'download', 'share', 'compose'], capabilities: ['sample', 'mcp', 'db', 'downloads'] });
  same(M.defaultsFor('card'), { audio: 'url', features: ['share'], capabilities: [] });
  same(M.defaultsFor('audio'), { audio: 'inline', features: ['download'], capabilities: [] });
  assert.equal(M.DEFAULT_SITE, 'https://beatscript.app');
});

test('the cases file has 14 to 20 cases, valid and invalid', () => {
  assert.ok(cases.length >= 14 && cases.length <= 20, String(cases.length));
  assert.ok(cases.some((c) => c.errors) && cases.some((c) => c.normalized));
});

for (const c of cases) {
  test(`case: ${c.name}`, () => {
    const r = normalize(c.input);
    if (c.errors) {
      assert.equal(r.ok, false);
      assert.equal(r.config, null);
      same(r.errors.map((e) => e.path), c.errors);
      for (const e of r.errors) assert.ok(e.message, e.path);
      return;
    }
    assert.ok(r.ok, JSON.stringify(r.errors));
    same(r.config, c.normalized, 'normalized');
    same(toAttributes(r.config), c.element, 'element');
    same(artifactCapabilities(r.config), c.capabilities, 'capabilities');
    // the same config as a JSON string
    if (c.input) same(normalize(JSON.stringify(c.input)).config, c.normalized, 'from a JSON string');
    // the element reads back to the same config
    if (c.element.tag) {
      const attrs = Object.fromEntries(c.element.attrs);
      const el = fakeEl({ tag: c.element.tag, attrs, script: c.element.text });
      same(parse(fromElement(el)), c.normalized, 'fromElement(toAttributes(config))');
    }
  });
}

test('normalize never throws, and says where a config is wrong', () => {
  for (const bad of ['{', '[1]', 7, 'null', [], { source: 5 }, { width: 10.5 }, { pin: { version: '1.2' } }, { site: 'http://x.org' }]) {
    const r = normalize(bad);
    if (bad === 'null') { assert.ok(r.ok); continue; }
    assert.equal(r.ok, false, JSON.stringify(bad));
    assert.ok(r.errors.length > 0);
  }
  same(normalize('{').errors, [{ path: '', message: 'is not valid JSON' }]);
  same(normalize({ site: 'http://x.org' }).errors, [{ path: '/site', message: 'must start with https://' }]);
  same(normalize({ 'a/b~': 1 }).errors.map((e) => e.path), ['/a~1b~0']);
  // a value of the wrong type is only that error, not also too long or out of range
  same(normalize({ source: { station: 3 } }).errors, [{ path: '/source/station', message: 'must be a string' }]);
});

test('normalize reads an object as its JSON: nothing inherited, no holes, no undefined, and never throws', () => {
  // a prototype's keys are not the config's (they used to pass the check unseen and land in the config)
  const r = normalize(Object.create({ kind: 'widget', width: 'huge' }));
  assert.ok(r.ok);
  same([r.config.kind, r.config.width], ['player', undefined]);
  // a hole is null, as in the JSON string; an undefined value is no key
  same(normalize({ features: [, 'share'] }).errors.map((e) => e.path), ['/features/0']); // eslint-disable-line no-sparse-arrays
  assert.ok(normalize({ title: undefined }).ok);
  // what JSON cannot say, a getter that throws, a cycle
  const cycle = {}; cycle.self = cycle;
  const getter = { get kind() { throw new Error('boom'); } };
  for (const bad of [{ features: [1n] }, { features: [cycle] }, cycle, getter, () => {}, Symbol('x')]) {
    const n = normalize(bad);
    assert.equal(n.ok, false);
    same(n.errors, [{ path: '', message: 'is not valid JSON' }]);
  }
  // a huge string is counted only as far as the limit
  const huge = 'x'.repeat(20_000_000);
  const t0 = Date.now();
  same(normalize({ title: huge, source: { text: huge } }).errors.map((e) => e.path), ['/source/text', '/title']);
  assert.ok(Date.now() - t0 < 5000, 'a huge string is cheap');
  // characters, not UTF-16 units: 80 emoji are a title of 80
  assert.ok(normalize({ title: '🎵'.repeat(80) }).ok);
  same(normalize({ title: '🎵'.repeat(81) }).errors.map((e) => e.path), ['/title']);
});

test('normalize and fromElement: a __proto__ key is a key, not a prototype', () => {
  const before = Object.getOwnPropertyNames(Object.prototype).join();
  const payload = '{"__proto__":{"polluted":1},"constructor":{"prototype":{"polluted":1}},"source":{"__proto__":{"text":"x"}}}';
  // (the source also lacks a program; fromElement's cid gives it one)
  same(normalize(payload).errors.map((e) => e.path), ['/__proto__', '/constructor', '/source', '/source/__proto__']);
  const raw = fromElement(fakeEl({ attrs: { config: payload, cid: CID, highlight: '' } }));
  same(normalize(raw).errors.map((e) => e.path), ['/__proto__', '/constructor', '/source/__proto__']);
  assert.equal(Object.getOwnPropertyNames(Object.prototype).join(), before);
  assert.equal({}.polluted, undefined);
  // a key the config lacks is not read from a polluted Object.prototype
  Object.prototype.site = 'http://evil.example'; // eslint-disable-line no-extend-native
  try { assert.equal(normalize({}).config.site, 'https://beatscript.app'); } finally { delete Object.prototype.site; }
});

test('a kind named after an Object member is an unknown kind, not a crash', () => {
  for (const kind of ['constructor', '__proto__', 'toString', 'hasOwnProperty']) {
    same(M.defaultsFor(kind), M.defaultsFor('player'), kind);
    const raw = fromElement(fakeEl({ attrs: { config: JSON.stringify({ kind }), highlight: '' } }));
    same(raw.features, ['share', 'highlight'], kind);
    same(normalize(raw).errors.map((e) => e.path), ['/kind'], kind);
    same(toAttributes({ ...parse(null), kind }), { tag: null, attrs: [], text: null }, kind);
  }
  assert.equal(fromElement(fakeEl({ tag: 'constructor' })).kind, undefined);
});

test('error paths sort by code point, as Rust sorts strings', () => {
  // UTF-16 units would put the emoji (U+1F3B5) before U+FF01
  same(normalize({ '🎵': 1, '！': 1, b: 1 }).errors.map((e) => e.path), ['/b', '/！', '/🎵']);
});

test('normalize warns about settings that do nothing', () => {
  same(normalize({ source: { text: 'tempo 90\n', n: 2 } }).warnings, ['n and count apply only to a station']);
  same(normalize({ source: { text: 'tempo 90\n' }, bridge: true }).warnings, ['bridge applies only to the studio']);
  same(normalize({ kind: 'audio', source: { text: 'tempo 90\n' } }).warnings, ['inline audio needs a stored program: a cid alone']);
  same(normalize({ kind: 'studio' }).warnings, []);
});

test('fromElement: the config JSON, then the attributes over it', () => {
  const el = fakeEl({
    attrs: { config: JSON.stringify({ theme: 'light', source: { cid: CID }, title: 'From JSON', features: ['share'] }), theme: 'auto', title: 'From the attribute', width: '640' },
  });
  const raw = fromElement(el);
  same(raw, { theme: 'auto', source: { cid: CID }, title: 'From the attribute', features: ['share'], kind: 'player', width: 640 });
  // the config attribute wins over a child config script; the script is read when there is no attribute
  same(fromElement(fakeEl({ attrs: { config: '{"theme":"light"}' }, json: '{"theme":"dark"}' })).theme, 'light');
  same(fromElement(fakeEl({ json: '{"compact":true}' })).compact, true);
  // lists, aliases and numbers
  const r = fromElement(fakeEl({ tag: 'beatscript-studio', attrs: { features: 'mixer, editor  share', caps: 'room,sample', station: 'lofi', n: '4', count: 'many' } }));
  same(r, { kind: 'studio', source: { station: 'lofi', n: 4, count: 'many' }, features: ['mixer', 'editor', 'share'], capabilities: ['room', 'sample'] });
  same(normalize(r).errors.map((e) => e.path), ['/source/count']);
  same(fromElement(fakeEl({ attrs: { capabilities: '' } })).capabilities, []);
  // JSON that does not parse comes back for normalize to report
  assert.equal(fromElement(fakeEl({ attrs: { config: '{oops' } })), '{oops');
  assert.equal(normalize(fromElement(fakeEl({ attrs: { config: '{oops' } }))).errors[0].path, '');
});

test('fromElement: the program from a child script, dedented as the player does', () => {
  const script = '\n      #! name: Indented\n      tempo 120\n        deck A: kick x ~ x ~\n    ';
  same(fromElement(fakeEl({ script })).source, { text: '#! name: Indented\ntempo 120\n  deck A: kick x ~ x ~\n' });
  // a script too long for the schema still comes back for normalize to refuse (Math.min(...lines) overflowed the stack)
  const long = fromElement(fakeEl({ script: '  x\n'.repeat(300_000) }));
  assert.equal(long.source.text.length, 600_000);
  same(normalize(long).errors.map((e) => e.path), ['/source/text']);
  // the text attribute is taken as it is, before any child script
  same(fromElement(fakeEl({ attrs: { text: '  tempo 90' }, script })).source, { text: '  tempo 90' });
  // a cid attribute goes with a child script's text
  same(fromElement(fakeEl({ attrs: { cid: CID }, script: 'tempo 90' })).source, { text: 'tempo 90\n', cid: CID });
});

test('fromElement: mode is an alias of audio', () => {
  assert.equal(fromElement(fakeEl({ attrs: { mode: 'render' } })).audio, 'render');
  assert.equal(fromElement(fakeEl({ attrs: { mode: 'render', audio: 'live' } })).audio, 'live');
  assert.equal(parse(fromElement(fakeEl({ attrs: { mode: 'render' } }))).audio, 'render');
});

test('fromElement: the highlight boolean adds highlight', () => {
  same(parse(fromElement(fakeEl({ attrs: { highlight: '' } }))).features, ['highlight', 'share']);
  same(parse(fromElement(fakeEl({ attrs: { highlight: '', features: 'download' } }))).features, ['highlight', 'download']);
  same(parse(fromElement(fakeEl({ attrs: { highlight: 'false' } }))).features, ['share']);
  // a studio has it already
  assert.equal(fromElement(fakeEl({ tag: 'beatscript-studio', attrs: { highlight: '' } })).features, undefined);
});

test('fromElement: compact="false" is false, a bare compact is true', () => {
  assert.equal(fromElement(fakeEl({ attrs: { compact: 'false' } })).compact, false);
  assert.equal(fromElement(fakeEl({ attrs: { compact: '' } })).compact, true);
  assert.equal(fromElement(fakeEl({ attrs: { compact: 'compact' } })).compact, true);
  assert.equal(fromElement(fakeEl({ attrs: { bridge: '' } })).bridge, true);
  assert.equal(fromElement(fakeEl()).compact, undefined);
});

test('artifactCapabilities names the connector server it is given', () => {
  const config = parse({ kind: 'studio', capabilities: ['mcp', 'dictation'] });
  same(artifactCapabilities(config, { server: 'BeatScript (staging)' }), { mcp: { servers: [{ server: 'BeatScript (staging)', tools: ['share', 'render', 'queue'] }] }, dictation: {} });
});

test('artifactCapabilities opens the room topic the jam sends on to Contributors', () => {
  // room.d.ts: an event topic the declaration does not list is admin-only (Editor and above)
  same(artifactCapabilities(parse({ kind: 'studio', capabilities: ['room'] })), { room: { topics: { program: 'interact' } } });
  // each call gets its own object
  const a = artifactCapabilities(parse({ kind: 'studio', capabilities: ['room'] }));
  a.room.topics.chat = 'interact';
  same(artifactCapabilities(parse({ kind: 'studio', capabilities: ['room'] })).room, { topics: { program: 'interact' } });
});

test('toAttributes: theme is an attribute only for light, the one value player 0.2.0 styles', () => {
  const light = toAttributes(parse({ source: { text: 'tempo 90\n' }, theme: 'light' }));
  same(light.attrs, [['theme', 'light']]);
  // auto goes in the config, which the player resolves from prefers-color-scheme
  const auto = toAttributes(parse({ source: { text: 'tempo 90\n' }, theme: 'auto' }));
  same(auto.attrs, [['config', '{"theme":"auto"}']]);
  const back = parse(fromElement(fakeEl({ attrs: Object.fromEntries(auto.attrs), script: auto.text })));
  assert.equal(back.theme, 'auto');
});

test('detectHost: a page that is not an artifact', async () => {
  const host = await detectHost({ WebAssembly });
  same(host, { artifact: false, wasm: true, worklet: false, offline: false, clipboard: false, caps: Object.fromEntries(CAPABILITIES.map((c) => [c, null])) });
  const rich = await detectHost({
    WebAssembly, AudioWorkletNode: function AudioWorkletNode() {}, OfflineAudioContext: function OfflineAudioContext() {},
    navigator: { clipboard: { writeText: async () => {} } },
  }, { want: [] });
  same(rich, { artifact: false, wasm: true, worklet: true, offline: true, clipboard: true, caps: {} });
});

test('detectHost: a CSP without wasm-unsafe-eval', async () => {
  const csp = { compile: () => Promise.reject(new WebAssembly.CompileError('Refused to compile: unsafe-eval is not allowed')) };
  assert.equal((await detectHost({ WebAssembly: csp }, { want: [] })).wasm, false);
  assert.equal((await detectHost({}, { want: [] })).wasm, false);
});

test('detectHost: an artifact, with claude.use', async () => {
  const sample = Object.freeze({ ask: () => {} });
  const asked = [];
  const win = {
    WebAssembly,
    claude: {
      use(name) {
        asked.push(name);
        if (name === 'sample') return Promise.resolve(sample);
        if (name === 'db') return new Promise(() => {}); // a host that never answers
        if (name === 'tts') throw new Error('broken');
        if (name === 'room') return Promise.reject(new Error('no'));
        return Promise.resolve(null);
      },
    },
  };
  const t0 = Date.now();
  const host = await detectHost(win, { timeoutMs: 50 });
  assert.ok(Date.now() - t0 < 2000, 'the timeout gives null');
  assert.equal(host.artifact, true);
  assert.equal(host.caps.sample, sample);
  same(Object.keys(host.caps), CAPABILITIES);
  for (const c of CAPABILITIES.filter((c) => c !== 'sample')) assert.equal(host.caps[c], null, c);
  same(asked, CAPABILITIES);
  // only what is wanted, user included when asked for
  same(Object.keys((await detectHost(win, { timeoutMs: 50, want: ['sample', 'user'] })).caps), ['sample', 'user']);
});

const ALL = { wasm: true, caps: Object.fromEntries([...CAPABILITIES, 'user'].map((c) => [c, {}])) };
const notesOf = (raw, host) => resolve(parse(raw), host).notes;

test('resolve: a host with everything changes nothing', () => {
  const config = parse({ kind: 'studio', source: { text: 'tempo 90\n' }, features: ['highlight', 'editor', 'download', 'share', 'compose'] });
  const r = resolve(config, ALL);
  same(r, { config, notes: [] });
});

test('resolve: no wasm', () => {
  const noWasm = { ...ALL, wasm: false };
  const player = resolve(parse({ source: { text: 'tempo 90\n' }, features: ['highlight', 'share'] }), noWasm);
  same(player.notes, [{ what: 'kind', status: 'unsupported', message: NO_WASM }]);
  assert.equal(NO_WASM, "this page does not allow WebAssembly ('wasm-unsafe-eval'), so it cannot play here");
  // the text still shows; the share link needs the engine to pack it
  same([player.config.features, player.config.capabilities], [['highlight'], []]);
  // the studio can show, edit and save the text, and says it cannot check, play or share it
  const studio = resolve(parse({ kind: 'studio', source: { text: 'tempo 90\n' }, features: [...M.FEATURES] }), noWasm);
  same(studio.notes, [{ what: 'kind', status: 'unsupported', message: NO_WASM_STUDIO }]);
  assert.equal(NO_WASM_STUDIO, "this page does not allow WebAssembly ('wasm-unsafe-eval'), so the studio can show and save the text but cannot check, play or share it");
  same([studio.config.features, studio.config.capabilities], [['highlight', 'editor', 'download'], []]);
  // a stored program does not play the site's render instead: neither element fetches it
  for (const raw of [{ kind: 'studio', source: { cid: CID } }, { source: { cid: CID }, audio: 'url' }, { source: { cid: CID }, audio: 'inline' }]) {
    const r = resolve(parse(raw), noWasm);
    same(r.notes.map((n) => [n.what, n.status]), [['kind', 'unsupported']], JSON.stringify(raw));
    assert.equal(r.config.audio, parse(raw).audio);
  }
  // card and audio need no wasm
  const engineNotes = (raw) => notesOf(raw, noWasm).filter((n) => n.what === 'kind' || n.what === 'audio');
  same(engineNotes({ kind: 'card', source: { text: 'tempo 90\n' } }), []);
  same(engineNotes({ kind: 'audio', source: { cid: CID } }), []);
});

test('resolve: no sample drops compose, and each missing capability gets its note', () => {
  const r = resolve(parse({ kind: 'studio' }), { wasm: true, caps: { mcp: {}, db: {}, downloads: {}, sample: null } });
  same(r.config.features, ['highlight', 'editor', 'download', 'share']);
  same(r.config.capabilities, ['mcp', 'db', 'downloads']);
  same(r.notes, [
    { what: 'compose', status: 'unsupported', message: 'compose needs Claude: the sample capability in a Claude artifact' },
    { what: 'sample', status: 'unsupported', message: 'the sample capability is not available here' },
  ]);
  // an ordinary page: every capability dropped, one note each
  const page = resolve(parse({ kind: 'studio' }), { wasm: true, caps: {} });
  same(page.config.capabilities, []);
  same(page.notes.map((n) => n.what), ['download', 'compose', 'sample', 'mcp', 'db', 'downloads']);
});

test('resolve: no downloads keeps download as a plain browser download', () => {
  const r = resolve(parse({ kind: 'studio' }), { ...ALL, caps: { ...ALL.caps, downloads: null } });
  assert.ok(r.config.features.includes('download'));
  same(r.notes, [
    { what: 'download', status: 'degraded', message: 'download falls back to a plain browser download' },
    { what: 'downloads', status: 'unsupported', message: 'the downloads capability is not available here' },
  ]);
});

test('resolve: a player asked for an editor, a mixer, knobs or compose', () => {
  const r = resolve(parse({ source: { text: 'tempo 90\n' }, features: ['editor', 'mixer', 'knobs', 'share', 'compose'], capabilities: ['sample'] }), ALL);
  same(r.config.features, ['share']);
  same(r.notes, [
    { what: 'editor', status: 'unsupported', message: 'editor needs kind studio' },
    { what: 'mixer', status: 'unsupported', message: 'mixer needs kind studio' },
    { what: 'knobs', status: 'unsupported', message: 'knobs needs kind studio' },
    { what: 'compose', status: 'unsupported', message: 'compose needs kind studio' },
  ]);
  // on a studio neither the mixer nor the knobs are built yet
  const s = resolve(parse({ kind: 'studio', features: ['mixer', 'knobs', 'share'] }), ALL);
  same(s.config.features, ['share']);
  same(s.notes, [
    { what: 'mixer', status: 'unsupported', message: 'there is no mixer yet' },
    { what: 'knobs', status: 'unsupported', message: 'there are no knobs yet' },
  ]);
});

test('resolve: a set on a player plays its first item; a station does not play', () => {
  same(notesOf({ source: { set: [{ cid: CID }, { text: 'tempo 90\n' }] } }, ALL), [{ what: 'source', status: 'degraded', message: 'the player plays only the first item of the set' }]);
  same(notesOf({ source: { set: [{ text: 'tempo 90\n' }] } }, ALL), []);
  same(notesOf({ source: { station: 'lofi' } }, ALL), [{ what: 'source', status: 'unsupported', message: 'the player cannot play a station' }]);
  same(notesOf({ kind: 'studio', source: { station: 'lofi' } }, ALL), []);
});
