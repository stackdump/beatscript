// The studio package (site/studio, @stackdump/beatscript-studio) under a Claude-artifact-like CSP, in headless Chrome.
//
//   node site/test/studio-csp.mjs        (needs site/web/wasm, or BTW_WASM=<dir>; skips without it)
//
// On a machine whose sound device is busy or absent the audio clock does not move and nothing installs (player-csp.mjs
// fails the same way): BTW_CHROME_ARGS=--disable-audio-output gives Chrome a null sink.
//
// Builds the package into a temp dir, serves it, and opens test/csp.html (two studios: the defaults, and one with
// audio=render, every feature and every capability). Checks:
//   - no securitypolicyviolation, no console error, nothing fetched but the page's own files
//   - the element checks its program (report.ok, report.cid), highlights it with the engine's colours, and the editor
//     works inside the shadow root: typing an error lists it at its line:col, a click on it moves the caret there
//   - outside an artifact, the notes say compose, store and library are not available, and their buttons are hidden
//   - tools.call('check') and tools.call('fmt') work from page script
//   - Play installs both decks (the composer exists, as player-csp.mjs checks), and Stop stops
//   - with a stub window.claude installed before the module loads, whose use('sample') resolves to a fake namespace,
//     the compose button appears, and Compose puts the stub's fenced program in the editor
//   - Render (audio=render) renders one pass and plays it (Web Audio: the CSP refuses blob: media, the one refusal
//     allowed), the context running; the WAV and .btw exports fall back to an <a download> link to a blob
//   - share gives a link on the configured site, and open(link) puts the same program (CID) back; generate loads a
//     program that checks; a studio with no program holds STARTER, which checks
//   - a set: the titles are listed, picking item 2 loads its text, a CID-only item says why, and when item 1 ends
//     item 2 plays
//   - taken off the page, it stops, closes its context and the bridge; put back, the bridge answers once and Play plays
//   - the bridge: a parent page on another origin frames a studio with `bridge` and gets tools/list and tools/call
//     answered over postMessage (only the bridged studio answers)
//   - with the stub's downloads, .btw and WAV call downloads.save (a .btw.txt, a .wav.zip), and no link is offered
//   - a studio that lists dictation and tts: without window.claude it has neither the mic nor the speaker; with the
//     stub's, the mic is beside the compose prompt (a click calls start, onFinal's text lands in the prompt, a second
//     click stops) and the speaker on the status line speaks the problems, naming their count

import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { launch, sleep } from './cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const wasm = process.env.BTW_WASM || path.join(here, '..', 'web', 'wasm');
if (!fs.existsSync(path.join(wasm, 'btw_bg.wasm'))) {
  console.log(`# skip: no ${wasm}/btw_bg.wasm (scripts/build-web-wasm.sh ${wasm})`);
  process.exit(0);
}
const dist = fs.mkdtempSync(path.join(os.tmpdir(), 'btw-studio-'));
execFileSync(process.execPath, [path.join(here, '..', 'studio', 'build.mjs'), '--wasm', wasm, '--out', dist], { stdio: 'inherit' });

const CSP = "default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval' cdn.jsdelivr.net unpkg.com; connect-src 'self'";
const TYPES = { '.js': 'text/javascript', '.html': 'text/html; charset=utf-8', '.json': 'application/json' };
// a stand-in for a Claude artifact's runtime: only `use`, and only sample (a function, as sample.d.ts's), downloads,
// dictation and tts resolve. Its sample answers a fenced program; its downloads records each save
const STUB = `<script>
  window.__used = []; window.__sampled = []; window.__saved = [];
  const sample = Object.freeze(Object.assign(async (input) => { window.__sampled.push(typeof input === 'string' ? input.length : input.length + ' turns'); return { text: 'Here it is.\\n\\n\\x60\\x60\\x60btw\\n#! name: from the stub\\ntempo 120\\ndeck A: kick x ~ x ~\\n\\x60\\x60\\x60\\n', truncated: false }; }, { json: async () => ({}) }));
  const downloads = Object.freeze({ save: async ({ filename, data }) => { window.__saved.push({ filename, text: typeof data === 'string' ? data : null, bytes: typeof data === 'string' ? data.length : data.byteLength, head: data instanceof Uint8Array ? String.fromCharCode(data[0], data[1]) : null }); return { status: 'saved' }; } });
  // dictation and tts as their d.ts shapes them: each call is recorded in __voice, and window.__fire(event, e) calls the
  // page's handlers (dictation.partial, dictation.final, dictation.state, tts.state), as the platform would
  window.__voice = []; window.__spoken = [];
  const subs = {};
  window.__fire = (k, e) => (subs[k] || []).slice().forEach((fn) => fn(e));
  const on = (k) => (fn) => { (subs[k] = subs[k] || []).push(fn); return () => { subs[k] = subs[k].filter((x) => x !== fn); }; };
  let heard = 'idle', said = 'idle', reading = null;
  const dictation = Object.freeze({
    start: async () => { window.__voice.push('dictation.start'); heard = 'listening'; window.__fire('dictation.state', { state: 'listening' }); },
    stop: async () => { window.__voice.push('dictation.stop'); if (heard === 'idle') return; heard = 'idle'; window.__fire('dictation.state', { state: 'idle', reason: 'stopped' }); },
    active: () => heard !== 'idle',
    onPartial: on('dictation.partial'), onFinal: on('dictation.final'), onState: on('dictation.state'),
  });
  // a reading lasts until stop(), which rejects it "stopped" as the platform does
  const tts = Object.freeze({
    speak: (text) => new Promise((resolve, reject) => { window.__voice.push('tts.speak'); window.__spoken.push(text); said = 'speaking'; reading = reject; window.__fire('tts.state', { state: 'speaking' }); }),
    stop: async () => { window.__voice.push('tts.stop'); if (said === 'idle') return; said = 'idle'; reading({ code: 'stopped', reason: 'stopped', message: 'stopped' }); window.__fire('tts.state', { state: 'idle', reason: 'stopped' }); },
    active: () => said !== 'idle',
    onState: on('tts.state'),
  });
  const granted = { sample, downloads, dictation, tts };
  window.claude = Object.freeze({ use: async (name) => { window.__used.push(name); return granted[name] || null; } });
</script>`;

const page = fs.readFileSync(path.join(dist, 'test', 'csp.html'), 'utf8');
// a page that frames the bridged studio from the other origin and keeps every JSON-RPC answer it is sent (it stands for
// the host around an artifact, so it has no artifact CSP of its own)
const PARENT = (child) => `<!doctype html><title>parent</title><script>
  window.__answers = [];
  addEventListener('message', (e) => { if (e.data && e.data.jsonrpc === '2.0' && !e.data.method) window.__answers.push({ origin: e.origin, data: e.data }); });
</script><iframe id="f" src="${child}/test/bridge.html" width="880" height="700"></iframe>`;
let other = null; // the second origin, for the bridge
const serve = () => new Promise((resolve) => {
  const s = http.createServer((req, res) => {
    const u = new URL(req.url, 'http://x');
    if (u.pathname === '/favicon.ico') { res.writeHead(204); res.end(); return; }
    if (u.pathname === '/test/claude.html') {
      res.writeHead(200, { 'Content-Type': TYPES['.html'], 'Content-Security-Policy': CSP });
      res.end(page.replace('<script type="module"', `${STUB}\n<script type="module"`));
      return;
    }
    if (u.pathname === '/test/bridge.html') {
      res.writeHead(200, { 'Content-Type': TYPES['.html'], 'Content-Security-Policy': CSP });
      res.end(page.replace('<beatscript-studio id="studio">', '<beatscript-studio id="studio" bridge>'));
      return;
    }
    if (u.pathname === '/test/parent.html') {
      res.writeHead(200, { 'Content-Type': TYPES['.html'] });
      res.end(PARENT(other.origin));
      return;
    }
    const p = path.join(dist, decodeURIComponent(u.pathname));
    if (!p.startsWith(dist) || !fs.existsSync(p) || fs.statSync(p).isDirectory()) { res.writeHead(404); res.end(); return; }
    res.writeHead(200, { 'Content-Type': TYPES[path.extname(p)] || 'application/octet-stream', 'Cache-Control': 'no-store', 'Content-Security-Policy': CSP });
    res.end(fs.readFileSync(p));
  });
  s.listen(0, '127.0.0.1', () => resolve({ s, origin: `http://127.0.0.1:${s.address().port}` }));
});
const site = await serve();
other = await serve();
const dlDir = fs.mkdtempSync(path.join(os.tmpdir(), 'btw-studio-dl-'));

const failures = [];
const check = (ok, what) => { console.log(`${ok ? 'ok' : 'FAIL'} - ${what}`); if (!ok) failures.push(what); };
const el = (id) => `document.getElementById(${JSON.stringify(id)})`;
const report = (b, id = 'studio') => b.eval(`${el(id)}.report`);
/** A click on a thing in the element's shadow root (its middle). */
async function clickIn(b, id, sel) {
  const r = await b.eval(`(()=>{const e=${el(id)}.shadowRoot.querySelector(${JSON.stringify(sel)}); if(!e) return null; e.scrollIntoView({block:'center'}); const r=e.getBoundingClientRect(); return {x:r.x+r.width/2,y:r.y+r.height/2}})()`);
  if (!r) throw new Error(`no ${sel} in #${id}`);
  for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) await b.cdp('Input.dispatchMouseEvent', { type, x: r.x, y: r.y, button: 'left', clickCount: 1 });
}
/** The element's tools are made once the engine and the host have answered. */
const ready = (b, id) => b.waitFor(`customElements.get('beatscript-studio') && ${el(id)}.report.tools.length > 0`, 30000);
/** Add a <beatscript-studio id=id> with these attributes to the page. */
const addStudio = (b, id, attrs = {}) => b.eval(`(() => { const s = document.createElement('beatscript-studio'); s.id = ${JSON.stringify(id)}; for (const [k, v] of Object.entries(${JSON.stringify(attrs)})) s.setAttribute(k, v); document.body.append(s); })()`);
/** Post a JSON-RPC ping to this window and count the answers to it within ms (a bridged studio answers its own page). */
const pings = (b, id, ms = 600) => b.eval(`new Promise((done) => { let n = 0; const on = (e) => { if (e.data && e.data.id === ${JSON.stringify(id)} && e.data.result) n++; }; addEventListener('message', on); postMessage({ jsonrpc: '2.0', id: ${JSON.stringify(id)}, method: 'ping' }, '*'); setTimeout(() => { removeEventListener('message', on); done(n); }, ${ms}); })`, true);
const shadow = (id, sel) => `${el(id)}.shadowRoot.querySelector(${JSON.stringify(sel)})`;
/** The bytes a download link saved into dlDir, once they are there (Chrome renames the file when it is complete). */
async function downloaded(name, ms = 5000) {
  const f = path.join(dlDir, name);
  for (let t = 0; t < ms; t += 100) { if (fs.existsSync(f)) return fs.readFileSync(f); await sleep(100); }
  return null;
}

const b = await launch({ width: 900, height: 1000 });
try {
  // a download link's file lands in dlDir, where the test reads it
  await b.cdp('Page.setDownloadBehavior', { behavior: 'allow', downloadPath: dlDir });
  await b.goto(`${site.origin}/test/csp.html`);
  await ready(b, 'studio');
  await ready(b, 'full');
  const r = await report(b);
  console.log(`# studio: ${JSON.stringify({ ...r, text: undefined, config: undefined })}`);
  check(r.ok && /^z[1-9A-HJ-NP-Za-km-z]{20,120}$/.test(r.cid || ''), `the element checks its program (ok ${r.ok}, ${r.cid})`);
  check(r.problems.length === 0, `no problems (${JSON.stringify(r.problems)})`);
  check(/^#! name: artifact test/.test(r.text), 'the text is the child script\'s, dedented');

  // the editor in the shadow root: highlighted by the engine, typing lands, a problem is listed and a click jumps to it
  const hl = await b.eval(`(() => { const p = ${el('studio')}.shadowRoot.querySelector('.ly-hl'); const cls = {}; for (const s of p.querySelectorAll('span')) cls[s.className] = (cls[s.className] || 0) + 1; return { same: p.textContent.trimEnd() === ${el('studio')}.text.trimEnd(), cls }; })()`);
  check(r.highlighted === 'engine' && hl.same && hl.cls['c-keyword'] > 0 && hl.cls['c-deck'] > 0, `the editor's layer is the program in the engine's colours (${JSON.stringify(hl.cls)})`);
  await b.eval(`(() => { const ta = ${el('studio')}.shadowRoot.querySelector('textarea'); ta.focus(); ta.setSelectionRange(ta.value.length, ta.value.length); })()`);
  await b.type('deck C: kick nonsense(\n');
  await b.waitFor(`${el('studio')}.report.problems.length > 0`, 5000).catch(() => false);
  const bad = await report(b);
  check(!bad.ok && bad.problems.length > 0 && bad.text.endsWith('deck C: kick nonsense(\n'), `typed into the shadow root's textarea, the error is listed (${JSON.stringify(bad.problems[0])})`);
  const row = await b.eval(`${el('studio')}.shadowRoot.querySelector('.problems li button .pos').textContent`);
  check(bad.problems[0] && row === `${bad.problems[0].line}:${bad.problems[0].col}`, `the problems list says line:col (${row})`);
  await clickIn(b, 'studio', '.problems li button');
  const caret = await b.eval(`(() => { const ta = ${el('studio')}.shadowRoot.querySelector('textarea'); return ta.value.slice(0, ta.selectionStart).split('\\n').length; })()`);
  check(bad.problems[0] && caret === bad.problems[0].line, `a click on the problem moves the caret to its line (${caret})`);
  const sq = await b.eval(`${el('studio')}.shadowRoot.querySelectorAll('.ly-diag .sq-error').length`);
  check(sq > 0, `the error is underlined in the editor (${sq})`);
  await b.eval(`${el('studio')}.load(${JSON.stringify(r.text)})`);
  check((await report(b)).ok, 'the program put back checks again');

  // outside an artifact: compose, store and library are not here, and the notes say so
  const f = await report(b, 'full');
  console.log(`# full: ${JSON.stringify({ notes: f.notes, buttons: f.buttons, tools: f.tools })}`);
  const said = (re) => f.notes.some((n) => n.status !== 'works' && re.test(`${n.what} ${n.message}`));
  check(said(/sample|compose/i), 'the notes say compose (sample) is not available outside an artifact');
  check(said(/mcp|store|connector/i), 'the notes say store (mcp) is not available');
  check(said(/\bdb\b|library/i), 'the notes say the library (db) is not available');
  check(!['compose', 'store', 'library', 'jam'].some((x) => f.buttons.includes(x)), `their buttons are hidden (${f.buttons.join(' ')})`);
  check(f.buttons.includes('render') && f.buttons.includes('exportWav'), 'audio=render shows Render, and download shows the exports');
  const notesLine = await b.eval(`${el('full')}.shadowRoot.querySelector('.notes[part="notes"]').textContent`);
  check(notesLine.length > 0, `the notes line under the editor says it (${notesLine.slice(0, 100)}…)`);

  // the tools from page script
  const calls = await b.eval(`(async () => { const t = ${el('studio')}.tools; const text = ${el('studio')}.text; return { check: await t.call('check', { text }), fmt: await t.call('fmt', { text }) }; })()`, true);
  const ct = JSON.stringify(calls.check.structuredContent || calls.check.content);
  check(!calls.check.isError && ct.includes(r.cid), `tools.call('check') answers the program's CID (${ct.slice(0, 80)}…)`);
  check(!calls.fmt.isError && (calls.fmt.content || []).length > 0, `tools.call('fmt') answers (${JSON.stringify(calls.fmt).slice(0, 80)}…)`);

  // Play installs both decks
  await clickIn(b, 'studio', 'button.play');
  await b.waitFor(`${el('studio')}.report.playing`, 30000).catch(() => false);
  await b.waitFor(`${el('studio')}.report.installed.length === 2`, 15000).catch(() => false);
  const p = await report(b);
  const ctx = await b.eval(`(() => { const t = ${el('studio')}; return { ctx: t.ctx && t.ctx.state, time: t.ctx && t.ctx.currentTime, grid: t.engine && t.engine.grid() }; })()`);
  console.log(`# play: ${JSON.stringify({ playing: p.playing, sec: p.sec, bar: p.bar, state: p.state, composer: p.composer, installed: p.installed, ...ctx })}`);
  check(p.playing && JSON.stringify(p.installed) === '["A","B"]', `Play installs both decks (${JSON.stringify(p.installed)})`);
  check(!!p.composer, `the engine's composer runs (${p.composer})`);
  check(/^verse 1\/1$/.test(p.sec) && /^bar [1-4]\/4$/.test(p.bar), `the position line says the section and bar (${p.sec} ${p.bar})`);
  await clickIn(b, 'studio', 'button.play');
  await sleep(100);
  check(!(await report(b)).playing, 'Stop stops');

  const v = await b.eval('window.__violations');
  check(v.length === 0, `no blocked request (${JSON.stringify(v)})`);
  const fetched = await b.eval(`performance.getEntriesByType('resource').map((e) => e.name)`);
  check(fetched.every((u) => u.startsWith(site.origin)), `nothing fetched but the page's own files (${fetched.filter((u) => !u.startsWith(site.origin)).join(', ')})`);
  const errors = b.errors().map((e) => e.text);
  check(errors.length === 0, `no console error (${errors.join('; ')})`);

  // share: a link on the configured site (beatscript.app here) that carries the program
  await clickIn(b, 'studio', 'button[part="share"]');
  await b.waitFor(`${el('studio')}.report.shared`, 5000).catch(() => false);
  const sh = (await report(b)).shared;
  const url = sh ? sh.url : '';
  check(url.startsWith('https://beatscript.app/') && url.includes(r.cid), `share gives a link on the site that names the CID (${url.slice(0, 70)}…, copied ${sh && sh.copied})`);
  // open: another program in the editor, then the link through the open panel puts the shared one back
  await b.eval(`${el('studio')}.load('tempo 90\\ndeck A: kick x ~ ~ ~\\n')`);
  const moved = await report(b);
  await clickIn(b, 'studio', 'button[part="open"]');
  await b.type(url);
  await clickIn(b, 'studio', '.panel button.tb');
  const op = await report(b);
  check(moved.cid !== r.cid && op.ok && op.cid === r.cid, `open(link) through the open panel puts the shared program back: the same CID (${op.cid})`);
  const ot = await b.eval(`${el('studio')}.tools.call('open', { url: ${JSON.stringify(url)} })`, true);
  check(ot.structuredContent && ot.structuredContent.verified === true && ot.structuredContent.cid === r.cid, `tools.call('open') verifies the link against its CID (${JSON.stringify(ot.structuredContent || ot.content).slice(0, 60)}…)`);

  // generate: the panel's genre and seed give a program that checks; so does the tool, into the editor
  await clickIn(b, 'studio', 'button[part="generate"]');
  await b.eval(`${shadow('studio', '.panel input')}.value = '7'`);
  await clickIn(b, 'studio', '.panel button.tb');
  const g = await report(b);
  const gmsg = await b.eval(`(${shadow('studio', '.panel .msg')} || {}).textContent || ''`);
  check(g.ok && g.cid !== r.cid && /seed 7\b/.test(gmsg), `generate loads a program that checks (${gmsg} ${g.cid})`);
  const gt = await b.eval(`${el('studio')}.tools.call('generate', { genre: 'house', seed: 3 })`, true);
  const gr = await report(b);
  check(gt.structuredContent && gt.structuredContent.ok && gr.ok && gr.text === gt.structuredContent.text, `tools.call('generate') gives a program that checks, and it goes into the editor (${gr.cid})`);

  // .btw with no downloads capability: an <a download> link to a blob, which saves the program
  await b.eval(`${el('studio')}.load(${JSON.stringify(r.text)})`);
  await clickIn(b, 'studio', 'button[part="exportBtw"]');
  const linkOf = (id) => b.eval(`(() => { const a = ${shadow(id, 'a.dl')}; return { hidden: a.hidden, href: a.href, name: a.download, label: a.textContent }; })()`);
  const dl = await linkOf('studio');
  check(!dl.hidden && dl.href.startsWith('blob:') && dl.name === 'artifact-test-drums-and-a-melody.btw', `.btw without the downloads capability offers a download link to a blob (${dl.name}, ${dl.href.slice(0, 30)}…)`);
  const file = await downloaded(dl.name);
  check(file && file.toString('utf8') === r.text, `the link saves the program (${file ? `${file.length} bytes` : 'no file'})`);

  // Render (audio=render): one pass, played through Web Audio (the CSP refuses the blob: WAV), the context running
  const v0 = (await b.eval('window.__violations')).length;
  const e0 = b.errors().length;
  const t0 = Date.now();
  await clickIn(b, 'full', 'button[part="render"]');
  await b.waitFor(`${el('full')}.report.render && ${el('full')}.report.render.via`, 30000).catch(() => false);
  await sleep(400);
  const rr = await report(b, 'full');
  const out = await b.eval(`(() => { const t = ${el('full')}; return { ctx: t.ctx && t.ctx.state, source: !!t.bufSrc, time: t.ctx && t.ctx.currentTime }; })()`);
  console.log(`# render: ${JSON.stringify({ render: rr.render, state: rr.state, ...out, ms: Date.now() - t0 })}`);
  check(rr.render && rr.render.frames > 0 && rr.render.seconds > 0, `Render renders one pass (${rr.render && `${rr.render.frames} frames, ${rr.render.bars} bars`})`);
  check(rr.render && rr.render.via === 'webaudio' && out.source && out.ctx === 'running', `under the artifact CSP it plays through Web Audio, the context running (${rr.render && rr.render.via}, ${out.ctx})`);
  await b.eval(`${el('full')}.stop()`);
  check(!(await b.eval(`!!${el('full')}.bufSrc`)), 'stop() stops the render');
  // WAV with no downloads capability: rendered, then a link to a blob WAV
  await clickIn(b, 'full', 'button[part="exportWav"]');
  await b.waitFor(`${shadow('full', 'a.dl')}.download.endsWith('.wav')`, 30000).catch(() => false);
  const wl = await linkOf('full');
  const wf = await downloaded(wl.name);
  check(!wl.hidden && wl.href.startsWith('blob:') && wl.name === 'a-light-one.wav', `WAV without the downloads capability offers a download link to a blob (${wl.name})`);
  check(wf && wf.subarray(0, 4).toString('latin1') === 'RIFF' && wf.length > 44, `the link saves a WAV (${wf ? `${wf.length} bytes` : 'no file'})`);
  const v1 = (await b.eval('window.__violations')).slice(v0);
  check(v1.every((x) => /media-src|default-src/.test(x.directive) && /^blob/.test(x.blocked)), `the one refusal is the blob: media probe (${JSON.stringify(v1.map((x) => `${x.directive} ${x.blocked}`))})`);
  const e1 = b.errors().slice(e0).map((e) => e.text).filter((t) => !/blob:/.test(t));
  check(e1.length === 0, `render and export: no other console error (${e1.join('; ')})`);

  // a studio with no program holds STARTER, which checks; with `bridge` and its own site
  await addStudio(b, 'starter', { bridge: '', site: 'https://btw.example.org' });
  await ready(b, 'starter');
  const st = await report(b, 'starter');
  check(st.ok && /^#! name: a first beat/.test(st.text) && st.problems.length === 0, `a studio with no program holds the starter, which checks (${st.cid})`);
  await b.eval(`${el('starter')}.share()`, true);
  const ss = (await report(b, 'starter')).shared;
  const stt = await b.eval(`${el('starter')}.tools.call('share', { text: ${el('starter')}.text })`, true);
  const tu = (stt.structuredContent && stt.structuredContent.url) || '';
  check(ss && ss.url.startsWith('https://btw.example.org/') && ss.url.includes(st.cid) && tu.startsWith('https://btw.example.org/'), `with site set, the share link (button and tool) is on that site (${ss && ss.url.slice(0, 50)}…)`);
  // taken off the page and put back: it stops, closes its context and the bridge; put back, the bridge answers once
  check((await pings(b, 'p1')) === 1, 'with bridge, the studio answers a ping posted to its page, once');
  await clickIn(b, 'starter', 'button.play');
  await b.waitFor(`${el('starter')}.report.playing`, 30000).catch(() => false);
  const off = await b.eval(`(async () => { const t = ${el('starter')}; window.__starter = t; const ctx = t.ctx; const was = t.report.playing; t.remove(); await new Promise((r) => setTimeout(r, 50)); return { was, ctx: ctx && ctx.state, playing: t.report.playing, timer: t.timer }; })()`, true);
  check(off.was && off.ctx === 'closed' && !off.playing && !off.timer, `taken off the page, it stops and closes its context (${JSON.stringify(off)})`);
  check((await pings(b, 'p2')) === 0, 'taken off the page, its bridge is closed');
  await b.eval('document.body.append(window.__starter)');
  check((await pings(b, 'p3')) === 1, 'put back, the bridge answers again, once');
  await clickIn(b, 'starter', 'button.play');
  await b.waitFor(`${el('starter')}.report.playing`, 30000).catch(() => false);
  check((await report(b, 'starter')).playing, 'put back, Play plays');
  await b.eval(`${el('starter')}.stop()`);

  // a set: the titles, picking an item, a CID-only item, and the next item playing when one ends
  const T1 = '#! name: first\ntempo 240\nsection intro 1 bars {\n  deck A: kick x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~\n}\narrange intro\n';
  const T2 = '#! name: second one\ntempo 120\ndeck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\n';
  await addStudio(b, 'set', { config: JSON.stringify({ source: { set: [{ title: 'Opener', text: T1 }, { text: T2 }, { cid: `z${'A'.repeat(40)}` }] } }) });
  await ready(b, 'set');
  const pickIn = (i) => b.eval(`(() => { const s = ${shadow('set', 'select.set')}; s.value = '${i}'; s.dispatchEvent(new Event('change')); })()`);
  const s0 = await report(b, 'set');
  const titles = await b.eval(`[...${shadow('set', 'select.set')}.options].map((o) => o.textContent)`);
  check(JSON.stringify(titles) === '["Opener","second one","item 3"]' && s0.text === T1 && s0.ok, `a set lists its items' titles (${titles.join(' | ')}), the first in the editor (ok ${s0.ok})`);
  await pickIn(1);
  const s1 = await report(b, 'set');
  check(s1.text === T2 && s1.set.at === 1 && s1.ok, `picking item 2 loads its text (at ${s1.set.at}, ok ${s1.ok})`);
  await pickIn(2);
  const s2 = await report(b, 'set');
  check(s2.text === T2 && /only a CID/.test(s2.state), `a CID-only item says why and leaves the editor as it was (${s2.state})`);
  await pickIn(0);
  await clickIn(b, 'set', 'button.play');
  await b.waitFor(`${el('set')}.report.set.at === 1 && ${el('set')}.report.playing`, 20000).catch(() => false);
  const s3 = await report(b, 'set');
  check(s3.set.at === 1 && s3.playing && s3.text === T2, `when item 1 ends, item 2 plays (at ${s3.set.at}, playing ${s3.playing}, ${s3.state || 'no message'})`);
  await b.eval(`${el('set')}.stop()`);
  // no window.claude: a studio that lists dictation and tts makes neither the mic nor the speaker
  await addStudio(b, 'novoice', { capabilities: 'sample dictation tts' });
  await ready(b, 'novoice');
  const nv = await report(b, 'novoice');
  const nvEls = await b.eval(`!!(${shadow('novoice', 'button[part="dictate"]')} || ${shadow('novoice', 'button[part="speak"]')})`);
  check(!nvEls && !nv.buttons.includes('dictate') && !nv.buttons.includes('speak'), `without the stubs, neither the mic nor the speaker exists (${nv.buttons.join(' ')})`);
  const v3 = (await b.eval('window.__violations')).slice(v0).filter((x) => !/^blob/.test(x.blocked));
  check(v3.length === 0, `starter and set: no blocked request (${JSON.stringify(v3)})`);
  const e3 = b.errors().slice(e0).map((e) => e.text).filter((t) => !/blob:/.test(t));
  check(e3.length === 0, `starter and set: no console error (${e3.join('; ')})`);

  // the bridge, framed: a parent page on another origin posts JSON-RPC to the studio page and reads the answers
  await b.goto(`${site.origin}/test/parent.html`);
  const post = (msg) => b.eval(`document.getElementById('f').contentWindow.postMessage(${JSON.stringify(msg)}, '*')`);
  let up = false;
  for (let i = 0; i < 150 && !up; i++) {
    await post({ jsonrpc: '2.0', id: `ping${i}`, method: 'ping' });
    await sleep(200);
    up = await b.eval('window.__answers.length > 0');
  }
  check(up, 'the framed studio answers its parent\'s ping');
  await b.eval('window.__answers = []');
  await post({ jsonrpc: '2.0', id: 1, method: 'tools/list' });
  await post({ jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'check', arguments: { text: r.text } } });
  await b.waitFor('window.__answers.filter((a) => a.data.id === 1 || a.data.id === 2).length >= 2', 10000).catch(() => false);
  await sleep(300);
  const ans = await b.eval('window.__answers');
  const a1 = ans.filter((a) => a.data.id === 1);
  const a2 = ans.filter((a) => a.data.id === 2);
  const names = a1[0] && a1[0].data.result ? a1[0].data.result.tools.map((t) => t.name) : [];
  check(a1.length === 1 && a1[0].origin === other.origin && names.includes('check') && !names.includes('compose'), `tools/list over postMessage, from the framed origin, answered once by the bridged studio only (${names.join(' ')})`);
  const res2 = a2[0] && a2[0].data.result;
  check(a2.length === 1 && res2 && !res2.isError && res2.structuredContent && res2.structuredContent.cid === r.cid, `tools/call check over postMessage answers the program's CID (${res2 && JSON.stringify(res2.structuredContent).slice(0, 60)}…)`);

  // a stub window.claude: sample resolves, so compose appears (on both: compose is a default feature of a studio)
  await b.goto(`${site.origin}/test/claude.html`);
  await ready(b, 'studio');
  await ready(b, 'full');
  const c = await report(b, 'full');
  const used = await b.eval('window.__used');
  console.log(`# claude: ${JSON.stringify({ buttons: c.buttons, tools: c.tools, used })}`);
  check(used.includes('sample'), `the element asked claude.use('sample') (${used.join(' ')})`);
  check(c.buttons.includes('compose') && !c.buttons.includes('store') && !c.buttons.includes('library'), `with sample, the compose button appears, and only it (${c.buttons.join(' ')})`);
  check((await report(b)).buttons.includes('compose'), 'the default studio shows compose too');
  check(c.tools.includes('compose'), `compose is an available tool (${c.tools.join(' ')})`);
  const v2 = await b.eval('window.__violations');
  check(v2.length === 0, `with the stub: no blocked request (${JSON.stringify(v2)})`);
  const e2 = b.errors().map((e) => e.text).filter((t) => !/blob:/.test(t));
  check(e2.length === 0, `with the stub: no console error (${e2.join('; ')})`);

  // compose: the stub's fenced program reaches the editor, from one sample call
  await clickIn(b, 'studio', 'button[part="compose"]');
  await b.type('a short beat');
  await clickIn(b, 'studio', '.panel button.tb');
  await b.waitFor(`${el('studio')}.report.text.includes('from the stub')`, 10000).catch(() => false);
  const cr = await report(b);
  const sampled = await b.eval('window.__sampled');
  check(cr.ok && cr.text.includes('#! name: from the stub') && sampled.length === 1, `Compose asks the stub's sample once and puts its program in the editor (${sampled.join(' ')} chars; ok ${cr.ok})`);
  // downloads: .btw and WAV go through downloads.save, and no link is offered
  await b.eval(`${el('studio')}.load(${JSON.stringify(r.text)})`);
  await clickIn(b, 'studio', 'button[part="exportBtw"]');
  await b.waitFor('window.__saved.length >= 1', 5000).catch(() => false);
  await clickIn(b, 'studio', 'button[part="exportWav"]');
  await b.waitFor('window.__saved.length >= 2', 30000).catch(() => false);
  const saved = await b.eval('window.__saved');
  const after = await report(b);
  const linked = await b.eval(`!${shadow('studio', 'a.dl')}.hidden`);
  console.log(`# saved: ${JSON.stringify(saved.map((x) => ({ ...x, text: x.text && `${x.text.length} chars` })))}`);
  check(saved[0] && saved[0].filename === 'artifact-test-drums-and-a-melody.btw.txt' && saved[0].text === r.text, `.btw calls downloads.save with the program (${saved[0] && saved[0].filename})`);
  check(saved[1] && saved[1].filename === 'artifact-test-drums-and-a-melody.wav.zip' && saved[1].head === 'PK' && saved[1].bytes > 1000, `WAV renders and calls downloads.save with a zip (${saved[1] && `${saved[1].filename}, ${saved[1].bytes} bytes`})`);
  check(!linked && after.state === 'saved', `with downloads, no link is offered and it says saved (${after.state})`);
  const e4 = b.errors().map((e) => e.text).filter((t) => !/blob:/.test(t));
  check(e4.length === 0, `compose and downloads: no console error (${e4.join('; ')})`);

  // dictation and tts, with the stub's: the mic beside the compose prompt, the speaker on the status line
  await addStudio(b, 'voice', { capabilities: 'sample dictation tts' });
  await ready(b, 'voice');
  const vr = await report(b, 'voice');
  const usedV = await b.eval('window.__used');
  check(usedV.includes('dictation') && usedV.includes('tts'), `the element asked claude.use('dictation') and use('tts') (${usedV.join(' ')})`);
  check(vr.buttons.includes('dictate') && vr.buttons.includes('speak'), `with the stubs, the mic and the speaker appear (${vr.buttons.join(' ')})`);
  await clickIn(b, 'voice', 'button[part="compose"]');
  const beside = await b.eval(`(() => { const m = ${shadow('voice', '.panel button[part="dictate"]')}; return m && !m.hidden ? m.previousElementSibling.tagName : null; })()`);
  check(beside === 'TEXTAREA', `the mic is beside the compose prompt (${beside})`);
  await clickIn(b, 'voice', 'button[part="dictate"]');
  await b.waitFor(`window.__voice.includes('dictation.start')`, 5000).catch(() => false);
  const micOn = await b.eval(`${shadow('voice', 'button[part="dictate"]')}.getAttribute('aria-pressed')`);
  check((await b.eval('window.__voice')).includes('dictation.start') && micOn === 'true', `a click on the mic calls start, and listening presses it (aria-pressed ${micOn})`);
  await b.eval(`window.__fire('dictation.partial', { text: 'two ki' })`);
  const guess = await b.eval(`${shadow('voice', '.panel .guess')}.textContent`);
  await b.eval(`window.__fire('dictation.partial', { text: '' }); window.__fire('dictation.final', { text: 'two kicks' })`);
  const prompt = await b.eval(`${shadow('voice', '.panel textarea')}.value`);
  check(guess === 'two ki' && prompt === 'two kicks', `onPartial shows the live guess (${guess}), onFinal('two kicks') puts the text in the compose prompt (${prompt})`);
  await clickIn(b, 'voice', 'button[part="dictate"]');
  await b.waitFor(`window.__voice.includes('dictation.stop')`, 5000).catch(() => false);
  const micOff = await b.eval(`${shadow('voice', 'button[part="dictate"]')}.getAttribute('aria-pressed')`);
  check((await b.eval('window.__voice')).includes('dictation.stop') && micOff === 'false', `a second click on the mic calls stop, and idle releases it (aria-pressed ${micOff})`);
  await b.eval(`${el('voice')}.load('tempo 120\\ndeck A: kick nonsense(\\n')`);
  const np = (await report(b, 'voice')).problems.length;
  await clickIn(b, 'voice', 'button[part="speak"]');
  await b.waitFor('window.__spoken.length >= 1', 5000).catch(() => false);
  const spoken = await b.eval('window.__spoken');
  const said1 = spoken[0] || '';
  check(np > 0 && spoken.length === 1 && said1.startsWith(`${np} problem${np === 1 ? '' : 's'}. Line `) && said1.length <= 2000, `a click on the speaker speaks the problems, naming their count (${np}: ${said1.slice(0, 80)})`);
  await clickIn(b, 'voice', 'button[part="speak"]');
  await b.waitFor(`window.__voice.includes('tts.stop')`, 5000).catch(() => false);
  const spk = await b.eval(`${shadow('voice', 'button[part="speak"]')}.getAttribute('aria-pressed')`);
  check((await b.eval('window.__voice')).includes('tts.stop') && spk === 'false', `a second click on the speaker calls stop (aria-pressed ${spk})`);
  const e5 = b.errors().map((e) => e.text).filter((t) => !/blob:/.test(t));
  check(e5.length === 0, `dictation and tts: no console error (${e5.join('; ')})`);
} finally {
  await b.close();
  site.s.close();
  other.s.close();
  fs.rmSync(dist, { recursive: true, force: true });
  fs.rmSync(dlDir, { recursive: true, force: true });
}
console.log(failures.length ? `# ${failures.length} failed` : '# all passed');
process.exit(failures.length ? 1 : 0);
