// Tests for the studio's in-page MCP, site/studio/src/tool-surface.js (no npm: node:test and node:assert).
//   node --test site/test/studio-tools.test.mjs
// The local tools run on the real wasm where it is built (site/web/wasm, never in git: those tests skip without it)
// and always on the page's JS mock. The capability tools run against fakes of the artifact runtime's namespaces
// (.agents/artifact-capabilities-0.2.98: sample, mcp, db, user, downloads, room).

import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { fileURLToPath, pathToFileURL } from 'node:url';

const site = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const imp = (p) => import(pathToFileURL(path.join(site, p)).href);
const { createToolSurface, LOCAL_TOOLS, CAP_TOOLS, PROTOCOL_VERSION, zipOne, crc32 } = await imp('studio/src/tool-surface.js');
const { makeFacade } = await imp('web/js/wasm.js');
const { mockModule } = await imp('web/js/wasm-mock.js');
const { wav } = await imp('web/js/render-model.js');

const wasmFile = path.join(site, 'web', 'wasm', 'beatscript_bg.wasm');
const noWasm = fs.existsSync(wasmFile) ? false : 'site/web/wasm/beatscript_bg.wasm is not built';
let real = null;
if (!noWasm) {
  const m = await imp('web/wasm/beatscript.js');
  m.initSync({ module: new WebAssembly.Module(fs.readFileSync(wasmFile)) });
  real = makeFacade(m);
}
const mock = makeFacade(mockModule());

const GOOD = 'tempo 124\nkey am\ndeck A: kick x ~ x ~ | hihat ~ x ~ x\n';
const BAD = 'tempo fast\ndeck A: kick x ~ x ~\n';
const STATION = 'station {\n  from gen@2 house seed=sunrise length=long\n}\n';
const fence = (t) => `Here it is.\n\n\`\`\`btw\n${t}\`\`\`\n`;
const ok = (r) => {
  assert.ok(!r.isError, r.isError ? r.content[0].text : '');
  assert.equal(r.content[0].type, 'text');
  return r.structuredContent;
};
const refused = (r, re) => {
  assert.equal(r.isError, true);
  assert.match(r.content[0].text, re);
};

// ---- fakes of the artifact runtime's namespaces ----

function fakeDb() {
  const store = new Map();
  const segs = (p) => p.split('/');
  const snap = (p) => ({ id: segs(p).at(-1), exists: store.has(p), data: () => store.get(p), metadata: { fromCache: false, hasPendingWrites: false } });
  let minted = 0;
  // the same path grammar the real store's builders throw on (db.d.ts)
  const doc = (p) => {
    if (segs(p).length % 2) throw new TypeError(`a document path has an even number of segments: ${p}`);
    return { id: segs(p).at(-1), path: p, get: async () => snap(p), set: async (d) => void store.set(p, structuredClone(d)), delete: async () => void store.delete(p), collection: (sub) => collection(`${p}/${sub}`) };
  };
  const collection = (p) => {
    if (segs(p).length % 2 === 0) throw new TypeError(`a collection path has an odd number of segments: ${p}`);
    const query = (order) => ({
      orderBy: (field, dir = 'asc') => query({ field, dir }),
      limit: () => query(order),
      get: async () => {
        const docs = [...store.keys()].filter((k) => k.startsWith(`${p}/`) && segs(k).length === segs(p).length + 1).map(snap);
        if (order) docs.sort((a, b) => (a.data()[order.field] < b.data()[order.field] ? -1 : 1) * (order.dir === 'desc' ? -1 : 1));
        return { docs, size: docs.length, empty: !docs.length };
      },
    });
    return { path: p, ...query(null), doc: (id = `minted${++minted}`) => doc(`${p}/${id}`) };
  };
  return { doc, collection, store };
}

function fakes({ answers = [], tools = true, uid = 'u_viewer1', mcpFail = null, saveFail = null } = {}) {
  const log = { sample: [], mcp: [], saves: [], emits: [] };
  const sample = async (input, opts) => {
    log.sample.push({ input, opts });
    const a = answers[log.sample.length - 1];
    if (a && typeof a === 'object') throw a; // a SampleError
    return { text: a, truncated: false, modelTierApplied: 'default' };
  };
  sample.limits = async () => (tools ? { maxPromptBytes: 262144, tools: { maxCount: 16 } } : { maxPromptBytes: 262144 });
  const mcp = {
    callTool: async (server, tool, input) => {
      log.mcp.push({ server, tool, input });
      if (mcpFail) throw mcpFail;
      return { content: [{ type: 'text', text: '{}' }], payload: { tool, cid: 'zStoredByTheConnector' } };
    },
  };
  const listeners = new Set();
  const room = {
    emit: async (topic, data) => void log.emits.push({ topic, data }),
    on: (topic, handler) => {
      const l = { topic, handler };
      listeners.add(l);
      return () => listeners.delete(l);
    },
  };
  const downloads = {
    save: async (req) => {
      if (saveFail) throw saveFail;
      log.saves.push(req);
      return { status: 'saved' };
    },
  };
  const db = fakeDb();
  const caps = { sample, mcp, db, user: { id: async () => uid }, downloads, room };
  const hear = (msg) => { for (const l of listeners) if (l.topic === 'program') l.handler({ topic: 'program', isMe: false, sameTab: false, kind: 'viewer', guest: false, by: null, peer: 'p2', ...msg }); };
  return { caps, log, db, hear, listeners };
}

// ---- the real wasm ----

test('real wasm: check, fmt and expand', { skip: noWasm }, async () => {
  const s = createToolSurface({ beatscript: real });
  const c = ok(await s.call('check', { text: GOOD }));
  assert.equal(c.ok, true);
  assert.match(c.cid, /^z[1-9A-HJ-NP-Za-km-z]{20,}$/);
  const bad = ok(await s.call('check', { text: BAD }));
  assert.equal(bad.ok, false);
  assert.equal(bad.diagnostics.find((d) => d.severity === 'error').line, 1);
  const f = ok(await s.call('fmt', { text: 'tempo   124\nkey am\ndeck A:  kick x ~ x ~ | hihat ~ x ~ x  # hi\n' }));
  assert.equal(f.ok, true);
  // fmt upgrades a BeatScript 1 text: a new CID, and the CID it came from
  assert.equal(f.from_cid, c.cid);
  assert.match(f.text, /hihat ~ x ~ x {2}# hi\n$/);
  const e = ok(await s.call('expand', { text: 'tempo 128\narrange from seed(techno, 42)\n', lineage: true }));
  assert.equal(e.ok, true);
  assert.deepEqual(e.refs, ['seed(techno, 42)']);
  assert.ok(e.text.startsWith(`#! forked-from: ${e.from_cid}\n`));
  assert.equal(real.check(e.text).ok, true);
});

test('real wasm: share then open is a round trip on the configured site', { skip: noWasm }, async () => {
  const loads = [];
  const s = createToolSurface({ beatscript: real, onLoad: (t, meta) => loads.push({ t, meta }) });
  const cid = real.check(GOOD).cid;
  const sh = ok(await s.call('share', { text: GOOD }));
  assert.equal(sh.cid, cid);
  assert.ok(sh.url.startsWith(`https://beatscript.app/#${cid}.`), sh.url);
  assert.ok(sh.social_url.startsWith(`https://beatscript.app/c/${cid}.`));
  assert.equal(sh.chars, sh.url.length);
  // only the origin moved: the module's own links, byte for byte after it
  const raw = real.share(GOOD);
  // (the module's own default is beatscript.app since share links moved there; whatever it is, only it is replaced)
  const from = new URL(raw.url).origin;
  assert.equal(sh.url, `https://beatscript.app${raw.url.slice(from.length)}`);
  assert.equal(sh.social_url, `https://beatscript.app${raw.social_url.slice(from.length)}`);
  assert.equal(sh.beats_url, raw.beats_url);
  const o = ok(await s.call('open', { url: sh.url }));
  assert.equal(o.ok, true);
  assert.equal(o.verified, true);
  assert.equal(o.cid, cid);
  assert.equal(o.canonical, GOOD);
  assert.deepEqual(loads, [{ t: GOOD, meta: { cid, via: 'open' } }]);
});

test('real wasm: generate is deterministic and its text checks', { skip: noWasm }, async () => {
  const s = createToolSurface({ beatscript: real });
  const g = ok(await s.call('generate', { genre: 'techno', seed: 3 }));
  assert.equal(g.ok, true);
  assert.equal(real.check(g.text).ok, true);
  assert.equal(real.check(g.text).cid, g.cid);
  assert.equal(ok(await s.call('generate', { genre: 'techno', seed: 3 })).cid, g.cid);
  refused(await s.call('generate', { genre: 'techno', tempo: 120 }), /no argument "tempo"/);
});

test('real wasm: highlight, instruments, cheatsheet and station', { skip: noWasm }, async () => {
  const s = createToolSurface({ beatscript: real });
  const h = ok(await s.call('highlight', { text: GOOD }));
  assert.deepEqual(h.spans[0], [0, 5, 'keyword']);
  assert.ok(h.spans.some(([, , cls]) => cls === 'deck'));
  const inst = ok(await s.call('instruments', {}));
  assert.ok(inst.voices.includes('sub-bass'));
  assert.ok(Array.isArray(inst.kits) && inst.kits.length > 0);
  const sheet = await s.call('cheatsheet');
  assert.ok(sheet.content[0].text.startsWith('# BeatScript cheat sheet'));
  assert.equal(sheet.structuredContent.cheatsheet, sheet.content[0].text);
  const st = s.list().find((t) => t.name === 'station');
  if (real.station) {
    const r = ok(await s.call('station', { station: STATION, n: 2, count: 2 }));
    assert.equal(r.ok, true);
    assert.deepEqual(r.tracks.map((t) => t.n), [2, 3]);
  } else {
    // this build of the module has no beatscript_station: the tool says so
    assert.equal(st.available, false);
    assert.match(st.why, /no station/);
    refused(await s.call('station', { station: STATION }), /no station/);
  }
});

// ---- the JS mock: the surface itself, for CI without the wasm ----

test('list: every tool once, in order, in the MCP shape', () => {
  const s = createToolSurface({ beatscript: mock });
  const list = s.list();
  assert.deepEqual(list.map((t) => t.name), [...LOCAL_TOOLS, ...CAP_TOOLS]);
  for (const t of list) {
    assert.equal(t.inputSchema.type, 'object', t.name);
    assert.ok(t.description.length > 0 && t.description.length <= 1024, t.name);
    assert.ok(JSON.stringify(t.inputSchema).length <= 4096, t.name);
    assert.equal(t.annotations.title, t.title);
    assert.equal(typeof t.annotations.readOnlyHint, 'boolean');
    assert.ok(['wasm', 'sample', 'mcp', 'db', 'downloads', 'room'].includes(t.via));
    assert.equal(t.available, !t.why, t.name);
  }
  // the mock has no expand or generate: listed, unavailable, with a why
  for (const n of ['expand', 'generate']) assert.match(list.find((t) => t.name === n).why, new RegExp(`no ${n}$`));
  assert.equal(list.find((t) => t.name === 'check').available, true);
});

test('mock: check, fmt, share, open, highlight, station', async () => {
  const s = createToolSurface({ beatscript: mock, site: 'https://example.test/' });
  const c = ok(await s.call('check', { text: GOOD }));
  assert.equal(c.ok, true);
  assert.equal(ok(await s.call('check', { text: BAD })).ok, false);
  // fmt upgrades a BeatScript 1 text to BeatScript 2 (a new CID) and names the CID it came from
  const f = ok(await s.call('fmt', { text: GOOD }));
  assert.equal(f.from_cid ?? f.cid, c.cid);
  const sh = ok(await s.call('share', { text: GOOD }));
  assert.ok(sh.url.startsWith(`https://example.test/#${c.cid}.`), sh.url);
  const o = ok(await s.call('open', { url: sh.url }));
  assert.equal(o.verified, true);
  assert.equal(o.canonical, mock.check(GOOD).canonical);
  assert.ok(ok(await s.call('highlight', { text: GOOD })).spans.length > 0);
  const st = ok(await s.call('station', { station: STATION, n: 1, count: 2 }));
  assert.equal(st.ok, true);
  assert.deepEqual(st.tracks.map((t) => t.n), [1, 2]);
});

test('share: only a share link\'s origin moves to the site; path, fragment and anything else stay', async () => {
  const frag = 'zAbc123.cGFja2Vk~bm90ZXM';
  const answer = (over = {}) => ({ ok: true, url: `https://btw.bitwrap.io/#${frag}`, social_url: 'https://btw.bitwrap.io/c/zAbc123.cGFja2Vk', beats_url: `https://beats.bitwrap.io/p#${frag}`, cid: 'zAbc123', chars: 0, ...over });
  const shareWith = async (site, over) => ok(await createToolSurface({ beatscript: { ...mock, share: () => answer(over) }, site }).call('share', { text: GOOD }));
  const sh = await shareWith('https://example.test/studio/?x=1#top');
  assert.equal(sh.url, `https://example.test/#${frag}`);
  assert.equal(sh.social_url, 'https://example.test/c/zAbc123.cGFja2Vk');
  assert.equal(sh.beats_url, `https://beats.bitwrap.io/p#${frag}`);
  assert.equal(sh.chars, sh.url.length);
  assert.equal((await shareWith('http://localhost:8080')).url, `http://localhost:8080/#${frag}`);
  // not a share link: left as the module gave it
  const odd = { url: 'https://btw.bitwrap.io/p/zAbc123', social_url: 'not a url' };
  assert.deepEqual(pickLinks(await shareWith('https://example.test', odd)), odd);
  assert.deepEqual(pickLinks(await shareWith('https://example.test', { url: 'javascript:alert(1)//#x', social_url: 'data:,/c/x' })), { url: 'javascript:alert(1)//#x', social_url: 'data:,/c/x' });
  // a site that is not an http(s) origin moves nothing
  assert.equal((await shareWith('beatscript.app')).url, `https://btw.bitwrap.io/#${frag}`);
  assert.equal((await shareWith('javascript:alert(1)')).url, `https://btw.bitwrap.io/#${frag}`);
  // a failed share is passed through
  const no = ok(await createToolSurface({ beatscript: { ...mock, share: () => ({ ok: false, error: 'the program has errors', url: null }) } }).call('share', { text: BAD }));
  assert.deepEqual(no, { ok: false, error: 'the program has errors', url: null });
});
const pickLinks = (o) => ({ url: o.url, social_url: o.social_url });

test('call: bad arguments and unknown tools are errors, never rejections', async () => {
  const s = createToolSurface({ beatscript: mock });
  refused(await s.call('check', {}), /"text" is required/);
  refused(await s.call('check', { text: 5 }), /"text" is a string/);
  refused(await s.call('check', { text: GOOD, extra: 1 }), /no argument "extra"/);
  refused(await s.call('check', 'tempo 120'), /JSON object/);
  refused(await s.call('nope', {}), /no tool named "nope"/);
  refused(await s.call('toString', {}), /no tool named/);
  refused(await s.call('check', { text: 'x'.repeat(65537) }), /longer than 65536/);
  refused(await s.call(1n, {}), /no tool named bigint/);
  refused(await s.call({ toString: () => 'check' }, { text: GOOD }), /no tool named object/);
  refused(await s.call('check', new Proxy({}, { ownKeys() { throw new Error('a hostile object'); } })), /hostile/);
  refused(await s.call('check', { get text() { throw new Error('a throwing getter'); } }), /throwing getter/);
  // a module that throws, and a caps of null
  refused(await createToolSurface({ beatscript: { ...mock, check: () => { throw new Error('wasm trap'); } } }).call('check', { text: GOOD }), /^wasm trap$/);
  const bare = createToolSurface({ beatscript: mock, caps: null });
  assert.equal(bare.list().find((t) => t.name === 'compose').available, false);
  refused(await bare.call('compose', { prompt: 'x' }), /needs the sample capability/);
  refused(await createToolSurface({}).call('check', { text: GOOD }), /no check$/);
});

// ---- capability tools against fakes ----

test('compose: a broken answer goes back with its problems, the fixed one comes out checked', async () => {
  const f = fakes({ answers: [fence(BAD), fence(GOOD)] });
  const loads = [];
  const s = createToolSurface({ beatscript: mock, caps: f.caps, onLoad: (t, meta) => loads.push({ t, meta }) });
  const r = ok(await s.call('compose', { prompt: 'a techno loop', text: 'tempo 120\n' }));
  assert.equal(r.ok, true);
  assert.equal(r.rounds, 2);
  assert.equal(r.cid, mock.check(GOOD).cid);
  assert.equal(mock.check(r.text).ok, true);
  assert.deepEqual(loads, [{ t: r.text, meta: { cid: r.cid, via: 'compose' } }]);
  // the first call is one self-contained instruction
  const [first, second] = f.log.sample;
  assert.equal(typeof first.input, 'string');
  assert.ok(first.input.includes(mock.cheatsheet()));
  assert.match(first.input, /Voices you may name: wobble-lead, /);
  assert.match(first.input, /The request: a techno loop/);
  assert.match(first.input, /```beatscript\ntempo 120\n```/);
  assert.match(first.input, /one program in a fenced block/);
  // the page's check, fmt and instruments go along as tools
  assert.deepEqual(first.opts.tools.map((t) => t.name), ['check', 'fmt', 'instruments']);
  const checkTool = first.opts.tools[0];
  assert.deepEqual(Object.keys(await checkTool.execute({ text: GOOD })), ['ok', 'cid', 'errors', 'diagnostics']);
  await assert.rejects(checkTool.execute({}), /"text" is required/);
  // the repair round is the exchange so far, ending on check's problems
  assert.ok(Array.isArray(second.input));
  assert.deepEqual(second.input.map((t) => t.role), ['user', 'assistant', 'user']);
  assert.match(second.input[2].content, /check found these problems:[\s\S]*wants a number/);
});

test('compose: at most two repair rounds, no fence asks again, no tools where the view has none', async () => {
  const f = fakes({ answers: [fence(BAD), fence(BAD), fence(BAD), fence(GOOD)] });
  const r = ok(await createToolSurface({ beatscript: mock, caps: f.caps }).call('compose', { prompt: 'x' }));
  assert.equal(r.ok, false);
  assert.equal(r.rounds, 3);
  assert.equal(r.text, BAD);
  assert.ok(r.diagnostics.some((d) => d.severity === 'error'));
  assert.equal(f.log.sample.length, 3);

  const g = fakes({ answers: ['I would rather talk about it.', fence(GOOD)], tools: false });
  const r2 = ok(await createToolSurface({ beatscript: mock, caps: g.caps }).call('compose', { prompt: 'x' }));
  assert.equal(r2.ok, true);
  assert.equal(r2.rounds, 2);
  assert.match(g.log.sample[1].input[2].content, /no fenced program/);
  assert.equal(g.log.sample[0].opts.tools, undefined);

  const h = fakes({ answers: [{ code: 'not_granted', message: 'declined' }] });
  refused(await createToolSurface({ beatscript: mock, caps: h.caps }).call('compose', { prompt: 'x' }), /^Claude is not allowed on this page\.$/);
});

test('compose: the program is found in ```beatscript, ```beatscript, a bare fence, or a reply that is only the program', async () => {
  const OTHER = 'tempo 128\nkey am\ndeck A: kick x ~ x ~\n';
  const replies = [
    `Plan:\n\`\`\`text\nfour on the floor\n\`\`\`\n\n\`\`\`btw\n${GOOD}\`\`\`\n`,
    `\`\`\`BeatScript\r\n${GOOD}\`\`\``,
    `\`\`\`\n${GOOD}\`\`\``,
    GOOD,
    // a marked block wins over a later block of something else
    `\`\`\`btw\n${GOOD}\`\`\`\nTo try it:\n\`\`\`sh\nopen it\n\`\`\`\n`,
    // with tools the answer is every round's text: the last marked block is the program
    `\`\`\`btw\n${BAD}\`\`\`\n\n\`\`\`btw\n${OTHER}\`\`\``,
  ];
  const want = [GOOD, GOOD, GOOD, GOOD, GOOD, OTHER];
  for (const [i, reply] of replies.entries()) {
    const f = fakes({ answers: [reply] });
    const loads = [];
    const r = ok(await createToolSurface({ beatscript: mock, caps: f.caps, onLoad: (t) => loads.push(t) }).call('compose', { prompt: 'x' }));
    assert.equal(r.ok, true, `reply ${i}`);
    assert.equal(r.cid, mock.check(want[i]).cid, `reply ${i}`);
    assert.equal(r.rounds, 1);
    assert.deepEqual(loads, [r.text]);
  }
  // an unclosed fence (a cut-off answer) is no program: it is asked for again, and nothing reaches the editor
  const g = fakes({ answers: [`\`\`\`btw\n${GOOD}`, 'still talking', 'and talking'] });
  const loads = [];
  const r = ok(await createToolSurface({ beatscript: mock, caps: g.caps, onLoad: (t) => loads.push(t) }).call('compose', { prompt: 'x' }));
  assert.deepEqual([r.ok, r.text, r.rounds, loads], [false, null, 3, []]);
  assert.equal(g.log.sample.length, 3);
});

test('compose: never cached, and the page tools go only where limits() says this view runs them', async () => {
  const f = fakes({ answers: [fence(GOOD)] });
  await createToolSurface({ beatscript: mock, caps: f.caps }).call('compose', { prompt: 'x' });
  assert.equal(f.log.sample[0].opts.cache, false);
  assert.equal(f.log.sample[0].opts.tools.length, 3);
  for (const limits of [async () => { throw { code: 'capability_removed' }; }, undefined, async () => ({ maxPromptBytes: 262144 })]) {
    const g = fakes({ answers: [fence(GOOD)] });
    g.caps.sample.limits = limits;
    ok(await createToolSurface({ beatscript: mock, caps: g.caps }).call('compose', { prompt: 'x' }));
    assert.deepEqual(g.log.sample[0].opts, { cache: false });
  }
  const h = fakes({ answers: [fence(GOOD)] });
  h.caps.sample.limits = async () => ({ maxPromptBytes: 262144, tools: { maxCount: 1 } });
  await createToolSurface({ beatscript: mock, caps: h.caps }).call('compose', { prompt: 'x' });
  assert.deepEqual(h.log.sample[0].opts.tools.map((t) => t.name), ['check']);
});

test('sample tools: small plain results; fmt without its second copy; an oversize answer is a thrown error', async () => {
  const f = fakes({ answers: [fence(GOOD)] });
  const s = createToolSurface({ beatscript: mock, caps: f.caps });
  const [, fmtTool] = s.asSampleTools(['check', 'fmt']);
  const out = await fmtTool.execute({ text: GOOD });
  assert.deepEqual(Object.keys(out).sort(), ['cid', 'diagnostics', 'ok', 'text']);
  const long = `${GOOD}${'# a long comment line to make the program large\n'.repeat(800)}`;
  await assert.rejects(fmtTool.execute({ text: long }), /over 32 KB/);
});

test('repair: starts from the text and its problems; a program that checks takes no rounds', async () => {
  const f = fakes({ answers: [fence(GOOD)] });
  const s = createToolSurface({ beatscript: mock, caps: f.caps });
  const r = ok(await s.call('repair', { text: BAD }));
  assert.equal(r.ok, true);
  assert.equal(r.rounds, 1);
  assert.match(f.log.sample[0].input[0].content, /```beatscript\ntempo fast\n/);
  assert.match(f.log.sample[0].input[1].content, /check found these problems/);
  const same = ok(await s.call('repair', { text: GOOD }));
  assert.deepEqual([same.ok, same.rounds, same.text], [true, 0, GOOD]);
  assert.equal(f.log.sample.length, 1);
});

test('store, render and queue call the Beatscript connector with the right tool and input', async () => {
  const f = fakes();
  const s = createToolSurface({ beatscript: mock, caps: f.caps });
  const st = ok(await s.call('store', { text: GOOD, name: 'night drive', license: 'CC-BY-4.0' }));
  assert.deepEqual(st, { tool: 'share', cid: 'zStoredByTheConnector' });
  const cid = 'z4EBG9j4MRUjAVJCAX55SZtBM3NMZJAg3q5CLT19srYMf2yYGkG';
  ok(await s.call('render', { cid, format: 'wav' }));
  ok(await s.call('render', { text: GOOD }));
  ok(await s.call('queue', { action: 'add', items: [{ cid }], session: 'abcdef' }));
  assert.deepEqual(f.log.mcp, [
    { server: 'Beatscript', tool: 'share', input: { text: GOOD, name: 'night drive', license: 'CC-BY-4.0' } },
    { server: 'Beatscript', tool: 'render', input: { cid, format: 'wav' } },
    { server: 'Beatscript', tool: 'render', input: { text: GOOD } },
    { server: 'Beatscript', tool: 'queue', input: { action: 'add', items: [{ cid }], session: 'abcdef' } },
  ]);
  // a program with errors never leaves the page
  refused(await s.call('store', { text: BAD }), /the program has errors: 1:/);
  refused(await s.call('render', {}), /give text or cid/);
  assert.equal(f.log.mcp.length, 4);
  // the connector's codes each say what to do
  const g = fakes({ mcpFail: { code: 'server_not_connected', message: 'no' } });
  refused(await createToolSurface({ beatscript: mock, caps: g.caps, server: 'My BeatScript' }).call('store', { text: GOOD }), /^Add the My BeatScript connector/);
  assert.equal(g.log.mcp[0].server, 'My BeatScript');
  const h = fakes({ mcpFail: { code: 'tool_error', message: 'sign in first' } });
  refused(await createToolSurface({ beatscript: mock, caps: h.caps }).call('store', { text: GOOD }), /^Beatscript share: sign in first$/);
});

test('library: save, list, get and delete in the viewer\'s private subtree', async () => {
  const f = fakes();
  const loads = [];
  const s = createToolSurface({ beatscript: mock, caps: f.caps, onLoad: (t, meta) => loads.push({ t, meta }) });
  const saved = ok(await s.call('library_save', { text: `#! name: Night Drive\n${GOOD}` }));
  assert.equal(saved.name, 'Night Drive');
  assert.equal(saved.cid, mock.check(`#! name: Night Drive\n${GOOD}`).cid);
  assert.equal(saved.id, saved.cid);
  const draft = ok(await s.call('library_save', { text: BAD, name: 'draft' }));
  assert.equal(draft.cid, null);
  assert.ok(draft.id);
  assert.ok([...f.db.store.keys()].every((k) => k.startsWith('data/users/u_viewer1/library/programs/')));
  const body = f.db.store.get(`data/users/u_viewer1/library/programs/${saved.id}`);
  assert.deepEqual(Object.keys(body), ['name', 'text', 'cid', 'savedAt']);
  const list = ok(await s.call('library_list', {}));
  assert.deepEqual(list.programs.map((p) => p.id).sort(), [saved.id, draft.id].sort());
  assert.ok(list.programs.every((p) => !('text' in p)));
  const got = ok(await s.call('library_get', { id: saved.id }));
  assert.equal(got.text, `#! name: Night Drive\n${GOOD}`);
  assert.equal(loads.at(-1).meta.via, 'library');
  assert.deepEqual(ok(await s.call('library_delete', { id: saved.id })), { deleted: saved.id });
  assert.deepEqual(ok(await s.call('library_list', {})).programs.map((p) => p.id), [draft.id]);
  refused(await s.call('library_get', { id: saved.id }), /no program with that id/);
  refused(await s.call('library_get', { id: '../other' }), /no program with that id/);
  // no viewer id: no private library
  const g = fakes({ uid: null });
  refused(await createToolSurface({ beatscript: mock, caps: g.caps }).call('library_list', {}), /signed in/);
});

test('export: the .beatscript text, and the WAV in a zip, through downloads.save', async () => {
  const f = fakes();
  const left = Float32Array.from([0, 0.5, -0.5, 1]);
  const right = Float32Array.from([0, -0.25, 0.25, -1]);
  const render = async (t) => {
    assert.equal(t, GOOD);
    return { left, right, sampleRate: 8000 };
  };
  const s = createToolSurface({ beatscript: mock, caps: f.caps, render, encodeWav: wav });
  assert.deepEqual(s.list().find((t) => t.name === 'export').inputSchema.properties.format.enum, ['beatscript', 'wav']);
  const named = `#! name: Night Drive\n${GOOD}`;
  assert.deepEqual(ok(await s.call('export', { format: 'beatscript', text: named })), { saved: 'Night-Drive.beatscript.txt', bytes: named.length });
  ok(await s.call('export', { format: 'beatscript', text: GOOD, filename: 'my mix.beatscript' }));
  assert.deepEqual(f.log.saves.slice(0, 2), [{ filename: 'Night-Drive.beatscript.txt', data: named }, { filename: 'my-mix.beatscript.txt', data: GOOD }]);

  const w = ok(await s.call('export', { format: 'wav', text: GOOD, filename: 'loop' }));
  assert.equal(w.saved, 'loop.wav.zip');
  const { filename, data } = f.log.saves[2];
  assert.equal(filename, 'loop.wav.zip');
  const want = wav(left, right, 8000);
  const v = new DataView(data.buffer, data.byteOffset, data.byteLength);
  assert.equal(v.getUint32(0, true), 0x04034b50);
  assert.equal(v.getUint16(8, true), 0); // stored
  const nameLen = v.getUint16(26, true);
  assert.equal(new TextDecoder().decode(data.subarray(30, 30 + nameLen)), 'loop.wav');
  assert.deepEqual(data.subarray(30 + nameLen, 30 + nameLen + want.length), want);
  assert.equal(v.getUint32(14, true), crc32(want));
  if (zlib.crc32) assert.equal(crc32(want), zlib.crc32(want));
  assert.equal(v.getUint32(data.length - 22, true), 0x06054b50);
  assert.deepEqual(zipOne('loop.wav', want), data);

  refused(await s.call('export', { format: 'mp3', text: GOOD }), /beatscript or wav/);
  const g = fakes({ saveFail: { code: 'declined', message: 'no' } });
  refused(await createToolSurface({ beatscript: mock, caps: g.caps }).call('export', { format: 'beatscript', text: GOOD }), /^You declined the download\.$/);
});

test('export: without render or encodeWav, wav is not offered and says why; beatscript still saves', async () => {
  const render = async () => ({ left: new Float32Array(1), right: new Float32Array(1), sampleRate: 8000 });
  for (const [opts, why] of [[{}, /^A WAV needs the render engine/], [{ render }, /^A WAV needs the WAV encoder/], [{ encodeWav: wav }, /^A WAV needs the render engine/]]) {
    const f = fakes();
    const s = createToolSurface({ beatscript: mock, caps: f.caps, ...opts });
    const ex = s.list().find((t) => t.name === 'export');
    assert.equal(ex.available, true);
    assert.deepEqual(ex.inputSchema.properties.format.enum, ['beatscript']);
    assert.doesNotMatch(ex.description, /wav/);
    refused(await s.call('export', { format: 'wav', text: GOOD }), why);
    refused(await s.call('export', { format: 'mp3', text: GOOD }), /"format" is beatscript$/);
    ok(await s.call('export', { format: 'beatscript', text: GOOD }));
    assert.deepEqual(f.log.saves.map((x) => x.filename), ['beatscript.beatscript.txt']);
  }
});

test('tool-surface.js imports nothing, so it runs where it is copied', () => {
  const src = fs.readFileSync(path.join(site, 'studio', 'src', 'tool-surface.js'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
  assert.doesNotMatch(src, /\bimport\s*[\s{('"*]|\bfrom\s+'/);
});

test('jam: send emits a checked program; a received one reaches onJam only if it checks', async () => {
  const f = fakes();
  const jams = [];
  const s = createToolSurface({ beatscript: mock, caps: f.caps, onJam: (m) => jams.push(m) });
  const cid = mock.check(GOOD).cid;
  assert.deepEqual(ok(await s.call('jam_send', { text: GOOD })), { sent: true, cid });
  assert.deepEqual(f.log.emits, [{ topic: 'program', data: { text: GOOD, cid } }]);
  refused(await s.call('jam_send', { text: BAD }), /the program has errors/);
  refused(await s.call('jam_send', { text: `${GOOD}# ${'x'.repeat(5000)}\n` }), /4 KiB/);
  assert.equal(f.log.emits.length, 1);

  f.hear({ data: { text: BAD, cid: 'zLies' } });
  f.hear({ data: { text: 42 } });
  f.hear({ data: { text: GOOD }, sameTab: true, isMe: true }); // our own send, echoed
  assert.deepEqual(jams, []);
  f.hear({ data: { text: GOOD, cid: 'zClaimedButNotChecked1234567' }, peer: 'p9' });
  assert.equal(jams.length, 1);
  assert.equal(jams[0].text, GOOD);
  assert.equal(jams[0].cid, cid);
  assert.equal(jams[0].peer, 'p9');
  s.close();
  assert.equal(f.listeners.size, 0);
  // no onJam: nothing subscribes
  const g = fakes();
  createToolSurface({ beatscript: mock, caps: g.caps });
  assert.equal(g.listeners.size, 0);
  // a received program is only handed on, never sent again, and a throwing onJam does not reach the room
  const h = fakes();
  const was = globalThis.reportError;
  const reported = [];
  globalThis.reportError = (e) => reported.push(e);
  try {
    createToolSurface({ beatscript: mock, caps: h.caps, onJam: () => { throw new Error('editor busy'); } });
    assert.doesNotThrow(() => h.hear({ data: { text: GOOD } }));
    h.hear({ data: { text: `${GOOD}${'#'.repeat(65537)}` } });
  } finally {
    globalThis.reportError = was;
  }
  assert.deepEqual(reported.map((e) => e.message), ['editor busy']);
  assert.deepEqual(h.log.emits, []);
});

test('an absent capability: listed with available false and a why, and call() is an error', async () => {
  const s = createToolSurface({ beatscript: mock });
  const list = s.list();
  const why = { compose: 'sample', repair: 'sample', store: 'mcp', render: 'mcp', queue: 'mcp', library_list: 'db', export: 'downloads', jam_send: 'room' };
  for (const [name, ns] of Object.entries(why)) {
    const t = list.find((x) => x.name === name);
    assert.equal(t.available, false, name);
    assert.equal(t.why, `needs the ${ns} capability: a Claude artifact that declares it`);
    refused(await s.call(name, { text: GOOD, prompt: 'x', action: 'list', format: 'beatscript' }), new RegExp(`^needs the ${ns} capability`));
  }
  // the library needs user as well as db
  const lib = createToolSurface({ beatscript: mock, caps: { db: fakeDb() } }).list().find((t) => t.name === 'library_save');
  assert.equal(lib.why, 'needs the user capability: a Claude artifact that declares it');
});

// ---- JSON-RPC ----

test('handle: initialize, ping, tools/list, tools/call, errors and notifications', async () => {
  const s = createToolSurface({ beatscript: mock });
  const init = await s.handle({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: PROTOCOL_VERSION, capabilities: {}, clientInfo: { name: 't', version: '0' } } });
  assert.deepEqual(init, { jsonrpc: '2.0', id: 1, result: { protocolVersion: '2025-06-18', capabilities: { tools: {} }, serverInfo: { name: 'beatscript-studio', version: '0.1.0' } } });
  assert.deepEqual(await s.handle({ jsonrpc: '2.0', id: 'p', method: 'ping' }), { jsonrpc: '2.0', id: 'p', result: {} });
  assert.equal(await s.handle({ jsonrpc: '2.0', method: 'notifications/initialized' }), null);
  assert.equal(await s.handle({ jsonrpc: '2.0', method: 'no/such/notification' }), null);

  const { result: { tools } } = await s.handle({ jsonrpc: '2.0', id: 2, method: 'tools/list' });
  assert.deepEqual(tools.map((t) => t.name), s.list().filter((t) => t.available).map((t) => t.name));
  assert.ok(!tools.some((t) => t.name === 'compose' || t.name === 'expand'));
  assert.deepEqual(Object.keys(tools[0]), ['name', 'title', 'description', 'inputSchema', 'annotations']);

  const call = await s.handle({ jsonrpc: '2.0', id: 3, method: 'tools/call', params: { name: 'check', arguments: { text: GOOD } } });
  assert.equal(call.id, 3);
  assert.equal(call.result.structuredContent.ok, true);
  assert.deepEqual(JSON.parse(call.result.content[0].text), call.result.structuredContent);
  const unavailable = await s.handle({ jsonrpc: '2.0', id: 4, method: 'tools/call', params: { name: 'compose', arguments: { prompt: 'x' } } });
  assert.equal(unavailable.result.isError, true);
  assert.equal((await s.handle({ jsonrpc: '2.0', id: 5, method: 'tools/call', params: { name: 'nope' } })).error.code, -32602);
  assert.deepEqual((await s.handle({ jsonrpc: '2.0', id: 6, method: 'resources/list' })).error.code, -32601);
  assert.equal((await s.handle({ id: 7, method: 'ping' })).error.code, -32600);
  // every error carries a code and a message, and an id only of a type JSON-RPC allows
  assert.deepEqual(await s.handle({ jsonrpc: '2.0', id: { x: 1 }, method: 'ping' }), { jsonrpc: '2.0', id: null, error: { code: -32600, message: 'the id is a string or a number' } });
  assert.deepEqual(await s.handle('ping'), { jsonrpc: '2.0', id: null, error: { code: -32600, message: 'not a JSON-RPC 2.0 request' } });
  assert.deepEqual(await s.handle([{ jsonrpc: '2.0', id: 1, method: 'ping' }]), { jsonrpc: '2.0', id: null, error: { code: -32600, message: 'not a JSON-RPC 2.0 request' } });
  assert.deepEqual(await s.handle({ jsonrpc: '2.0', id: null, method: 'ping' }), { jsonrpc: '2.0', id: null, result: {} });
  for (const params of [undefined, {}, { name: 5 }, { name: 'toString' }]) {
    const r = await s.handle({ jsonrpc: '2.0', id: 'c', method: 'tools/call', params });
    assert.equal(r.id, 'c');
    assert.equal(r.error.code, -32602);
    assert.equal(typeof r.error.message, 'string');
  }
  const bad = await s.handle({ jsonrpc: '2.0', id: 8, method: 'tools/call', params: { name: 'check', arguments: 'tempo 120' } });
  assert.deepEqual([bad.id, bad.result.isError], [8, true]);
});

test('serve: answers JSON-RPC over postMessage to the sender, at its origin', async () => {
  const s = createToolSurface({ beatscript: mock });
  const win = { listeners: new Set(), addEventListener(t, f) { if (t === 'message') this.listeners.add(f); }, removeEventListener(t, f) { this.listeners.delete(f); } };
  const post = (data, origin) => {
    const sent = [];
    for (const f of win.listeners) f({ data, origin, source: { postMessage: (m, o) => sent.push([m, o]) } });
    return new Promise((r) => setTimeout(() => r(sent), 10));
  };
  const off = s.serve(win);
  assert.deepEqual(await post({ jsonrpc: '2.0', id: 1, method: 'ping' }, 'https://parent.test'), [[{ jsonrpc: '2.0', id: 1, result: {} }, 'https://parent.test']]);
  assert.deepEqual((await post({ jsonrpc: '2.0', id: 2, method: 'ping' }, 'null'))[0][1], '*');
  assert.deepEqual(await post({ jsonrpc: '2.0', id: 3, result: {} }, 'https://parent.test'), []); // a response is not answered
  assert.deepEqual(await post('hello', 'https://parent.test'), []);
  off();
  assert.equal(win.listeners.size, 0);

  const off2 = s.serve(win, { origins: ['https://parent.test'] });
  assert.deepEqual(await post({ jsonrpc: '2.0', id: 4, method: 'ping' }, 'https://other.test'), []);
  assert.deepEqual(await post({ jsonrpc: '2.0', id: 5, method: 'ping' }, 'https://parent.test'), [[{ jsonrpc: '2.0', id: 5, result: {} }, 'https://parent.test']]);
  // with a list, a sandboxed sender (origin "null") is never answered, so nothing is ever posted to '*'
  assert.deepEqual(await post({ jsonrpc: '2.0', id: 6, method: 'ping' }, 'null'), []);
  off2();
  s.serve(win, { origins: ['null', 'https://parent.test'] });
  assert.deepEqual(await post({ jsonrpc: '2.0', id: 7, method: 'ping' }, 'null'), []);
  win.listeners.clear();
  // one origin as a string is that origin, not a substring test
  s.serve(win, { origins: 'https://parent.test' });
  assert.deepEqual(await post({ jsonrpc: '2.0', id: 8, method: 'ping' }, 'https://parent.tes'), []);
  assert.equal((await post({ jsonrpc: '2.0', id: 9, method: 'ping' }, 'https://parent.test')).length, 1);
  win.listeners.clear();
  // a notification has no answer, and a sender that cannot be posted to is not an unhandled rejection
  s.serve(win);
  assert.deepEqual(await post({ jsonrpc: '2.0', method: 'notifications/initialized' }, 'https://parent.test'), []);
  const reported = [];
  const was = globalThis.reportError;
  globalThis.reportError = (e) => reported.push(e);
  try {
    for (const f of win.listeners) f({ data: { jsonrpc: '2.0', id: 10, method: 'ping' }, origin: 'https://parent.test', source: { postMessage() { throw new Error('gone'); } } });
    await new Promise((r) => setTimeout(r, 10));
  } finally {
    globalThis.reportError = was;
  }
  assert.deepEqual(reported.map((e) => e.message), ['gone']);
});

test('serve: two surfaces bridged to each other do not echo: a reply is never answered', async () => {
  const a = createToolSurface({ beatscript: mock });
  const b = createToolSurface({ beatscript: mock });
  const mk = () => ({ listeners: new Set(), addEventListener(t, f) { this.listeners.add(f); }, removeEventListener(t, f) { this.listeners.delete(f); } });
  const wa = mk();
  const wb = mk();
  let posts = 0;
  const deliver = (to, from, data) => setTimeout(() => { for (const f of to.listeners) f({ data, origin: 'https://peer.test', source: { postMessage: (m) => { posts++; deliver(from, to, m); } } }); }, 0);
  a.serve(wa);
  b.serve(wb);
  const got = [];
  wb.listeners.add((e) => { if (e.data.result) got.push(e.data); });
  deliver(wa, wb, { jsonrpc: '2.0', id: 1, method: 'ping' });
  await new Promise((r) => setTimeout(r, 50));
  assert.equal(posts, 1);
  assert.deepEqual(got, [{ jsonrpc: '2.0', id: 1, result: {} }]);
});
