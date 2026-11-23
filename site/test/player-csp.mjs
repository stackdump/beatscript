// The self-contained player (site/player, project-datum 6669) under a Claude-artifact-like CSP, in headless Chrome.
//
//   node site/test/player-csp.mjs        (needs site/web/wasm, or BEATSCRIPT_WASM=<dir>; skips without it)
//
// Builds the package, then plays player/test/csp.html's program (drums on an 808 kit, a melody, a sung line) three ways:
//   files   the package beside the page (one origin)
//   cdn     the package from a second origin with CORS, as from cdn.jsdelivr.net (the page allows it in script-src)
//   strict  the CSP exactly as an artifact states it, without 'wasm-unsafe-eval': WebAssembly cannot compile, and the
//           element must say so rather than fail silently
// and checks, for the first two: it plays (the section and bar advance), no request was blocked (no
// securitypolicyviolation), nothing was fetched from anywhere but the page's origin and the CDN, and no console error.
// Then, for 0.2.0:
//   stop     Stop is silent at once (an AnalyserNode on the element's output reads silence 100 ms after the click; it
//            played before), the context sleeps soon after, and the next Play starts from bar 1
//   links    the CID link is the self-sealing share link (it opens a program the site never stored), the same as
//            "Open on beatscript.app"; with `stored` it is /p/<CID>
//   render   mode="render" twice: beside the page with blob: media allowed (a Worker renders, an <audio> plays the
//            blob: WAV) and from the CDN under the artifact's CSP (the page's thread renders, Web Audio plays: the
//            CSP refuses blob: media). Each: progress is shown, the length is the program's 4 bars, the PCM is the
//            same as Node's render of the same text (sha256), it plays, seeks, pauses, resumes where it was, and stops
//            silent and back at 0
//   highlight  the source under the card, coloured by the engine's highlighter, and Copy

import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import crypto from 'node:crypto';
import { pathToFileURL } from 'node:url';
import { launch, sleep } from './cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const wasm = process.env.BEATSCRIPT_WASM || path.join(here, '..', 'web', 'wasm');
if (!fs.existsSync(path.join(wasm, 'beatscript_bg.wasm'))) {
  console.log(`# skip: no ${wasm}/beatscript_bg.wasm (scripts/build-web-wasm.sh ${wasm})`);
  process.exit(0);
}
const dist = fs.mkdtempSync(path.join(os.tmpdir(), 'btw-player-'));
execFileSync(process.execPath, [path.join(here, '..', 'player', 'build.mjs'), '--wasm', wasm, '--out', dist], { stdio: 'inherit' });

const ARTIFACT = "default-src 'self'; script-src 'self' 'unsafe-inline' cdn.jsdelivr.net unpkg.com; connect-src 'self'";
const TYPES = { '.js': 'text/javascript', '.html': 'text/html; charset=utf-8', '.json': 'application/json' };

function serve(handler) {
  return new Promise((resolve) => {
    const s = http.createServer(handler);
    s.listen(0, '127.0.0.1', () => resolve({ s, origin: `http://127.0.0.1:${s.address().port}` }));
  });
}
function file(res, p, headers) {
  if (!p.startsWith(dist) || !fs.existsSync(p) || fs.statSync(p).isDirectory()) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { 'Content-Type': TYPES[path.extname(p)] || 'application/octet-stream', 'Cache-Control': 'no-store', ...headers });
  res.end(fs.readFileSync(p));
}

const page = fs.readFileSync(path.join(dist, 'test', 'csp.html'), 'utf8');
const cdn = await serve((req, res) => file(res, path.join(dist, decodeURIComponent(new URL(req.url, 'http://x').pathname)), { 'Access-Control-Allow-Origin': '*' }));
let csp = '';
const site = await serve((req, res) => {
  const u = new URL(req.url, 'http://x');
  // the browser's own request, not the page's
  if (u.pathname === '/favicon.ico') { res.writeHead(204); res.end(); return; }
  if (u.pathname === '/render.html' || u.pathname === '/render-cdn.html') {
    // render mode, with the source shown, and a second card the site has `stored` (no meta CSP: the header's is the page's)
    let html = page.replace(/<meta http-equiv="Content-Security-Policy"[^>]*>/, '').replace('<beatscript-track id="track">', '<beatscript-track id="track" mode="render" highlight>');
    html = html.replace('<p id="violations">', '<beatscript-track id="stored" stored compact text="tempo 120&#10;deck A: kick x ~ x ~&#10;"></beatscript-track>\n<p id="violations">');
    if (u.pathname === '/render-cdn.html') html = html.replace('src="../beatscript-track.js"', `src="${cdn.origin}/beatscript-track.js"`);
    res.writeHead(200, { 'Content-Type': TYPES['.html'], 'Content-Security-Policy': csp });
    res.end(html);
    return;
  }
  if (u.pathname === '/cdn.html') {
    // the same page, the package from the other origin, which the CSP allows as it allows cdn.jsdelivr.net
    const html = page.replace('src="../beatscript-track.js"', `src="${cdn.origin}/beatscript-track.js"`).replace(/<meta http-equiv="Content-Security-Policy"[^>]*>/, '');
    res.writeHead(200, { 'Content-Type': TYPES['.html'], 'Content-Security-Policy': csp });
    res.end(html);
    return;
  }
  file(res, path.join(dist, decodeURIComponent(u.pathname)), { 'Content-Security-Policy': csp });
});

const failures = [];
const check = (ok, what) => { console.log(`${ok ? 'ok' : 'FAIL'} - ${what}`); if (!ok) failures.push(what); };

async function play(b, url, label) {
  await b.goto(url);
  await b.waitFor(`customElements.get('beatscript-track') && document.getElementById('track').shadowRoot`, 20000);
  // the play button is the card's first thing, at its left
  await b.click('#track', { dx: 48, dy: 60 });
  const t0 = Date.now();
  let r;
  for (;;) {
    r = await b.eval(`document.getElementById('track').report`);
    if (r.playing || (r.state && !/loading/.test(r.state)) || Date.now() - t0 > 60000) break;
    await sleep(200);
  }
  const seen = new Set();
  if (r.playing) for (let i = 0; i < 20; i++) { await sleep(200); const q = await b.eval(`document.getElementById('track').report`); seen.add(`${q.sec} ${q.bar}`); }
  // what the engine installed by the end (a deck's net lands on a bar line, after the first report)
  const installed = r.playing ? (await b.eval(`document.getElementById('track').report`)).installed : [];
  if (process.env.BEATSCRIPT_SHOT) await b.shot(process.env.BEATSCRIPT_SHOT.replace(/\.png$/, `-${label}.png`));
  const v = await b.eval('window.__violations');
  const fetched = await b.eval(`performance.getEntriesByType('resource').map((e) => e.name)`);
  const errors = b.errors().map((e) => e.text);
  await b.eval(`document.getElementById('track').stop()`);
  console.log(`# ${label}: ${JSON.stringify({ report: r, seen: [...seen], installed, violations: v, errors })}`);
  return { r, seen, installed, v, fetched, errors };
}

/** A click on a thing in the element's shadow root (its middle). */
async function clickIn(b, id, sel) {
  const r = await b.eval(`(()=>{const e=document.getElementById(${JSON.stringify(id)}).shadowRoot.querySelector(${JSON.stringify(sel)}); if(!e) return null; const r=e.getBoundingClientRect(); return {x:r.x+r.width/2,y:r.y+r.height/2}})()`);
  if (!r) throw new Error(`no ${sel} in #${id}`);
  for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) await b.cdp('Input.dispatchMouseEvent', { type, x: r.x, y: r.y, button: 'left', clickCount: 1 });
}
const report = (b, id = 'track') => b.eval(`document.getElementById(${JSON.stringify(id)}).report`);
/** An AnalyserNode on the element's output (every sound it makes goes through `out`), and its RMS now. */
// (the analyser reaches the destination through a zero gain, so the graph pulls it on every render quantum)
const TAP = `window.__tap = (el) => { if (el.__an && el.__an.context === el.ctx) return; const an = el.ctx.createAnalyser(); an.fftSize = 2048; el.out.connect(an); const z = el.ctx.createGain(); z.gain.value = 0; an.connect(z); z.connect(el.ctx.destination); el.__an = an; };
window.__rms = (el) => { const a = new Float32Array(el.__an.fftSize); el.__an.getFloatTimeDomainData(a); let s = 0; for (const x of a) s += x * x; return Math.sqrt(s / a.length); };`;
// a tap made now has nothing in it yet: it is given 200 ms (a few of its 2048-sample windows) before it is read
const rmsOf = async (b, id = 'track') => {
  const el = `document.getElementById(${JSON.stringify(id)})`;
  const fresh = await b.eval(`(() => { const t = ${el}; const had = !!(t.__an && t.__an.context === t.ctx); window.__tap(t); return !had; })()`);
  if (fresh) await sleep(200);
  return b.eval(`window.__rms(${el})`);
};
const LINK = /^https:\/\/beatscript\.app\/#z[1-9A-HJ-NP-Za-km-z]{20,120}\.[A-Za-z0-9_-]+/;

// the same text rendered in Node: what the browser's render must equal, byte for byte
const { renderProgram } = await import(pathToFileURL(path.join(dist, 'beatscript-render.js')).href);
const { pcm16 } = await import(pathToFileURL(path.join(dist, 'render-model.js')).href);
const nodeModule = await WebAssembly.compile(fs.readFileSync(path.join(wasm, 'beatscript_bg.wasm')));
const nodeSha = new Map();
async function nodeRender(text) {
  if (!nodeSha.has(text)) {
    const r = await renderProgram({ module: nodeModule, text, sampleRate: 44100 });
    const p = pcm16(r.left, r.right);
    nodeSha.set(text, crypto.createHash('sha256').update(Buffer.from(p.buffer, p.byteOffset, p.byteLength)).digest('hex'));
  }
  return nodeSha.get(text);
}

/** mode="render" on `url`: render, check, play, seek, pause, resume, stop. */
async function renderCase(b, url, label, want) {
  await b.goto(url);
  await b.waitFor(`customElements.get('beatscript-track') && document.getElementById('track').shadowRoot`, 20000);
  await b.eval(TAP);
  // before a click: the stored card's CID is plain text (no module yet to pack a link, and no cid attribute)
  const before = await report(b, 'stored');
  check(before.links.length === 0, `${label}: before Play, no link is made up (${JSON.stringify(before.links)})`);
  const text = await b.eval(`document.getElementById('track').text`);
  const t0 = Date.now();
  await clickIn(b, 'track', 'button.play');
  let r;
  const progress = new Set();
  for (;;) {
    r = await report(b);
    if (r.render && r.render.progress) progress.add(r.state);
    if (r.playing || (r.state && /line |refused|cannot|stopped|error/i.test(r.state)) || Date.now() - t0 > 120000) break;
    await sleep(50);
  }
  const renderedMs = Date.now() - t0;
  console.log(`# ${label}: ${JSON.stringify(r)} (${renderedMs} ms to play; progress seen ${[...progress].join(' ')})`);
  check(r.playing && r.render, `${label}: it renders and plays from a click (${r.state})`);
  if (!r.render) return;
  check(progress.size > 0 && [...progress].every((p) => /^\d+%$/.test(p)), `${label}: the render shows its progress (${[...progress].join(' ')})`);
  check(r.render.frames === 352800 && r.render.bars === 4 && Math.abs(r.render.seconds - 8) < 1e-9, `${label}: one pass, 4 bars at 120, ${r.render.frames} frames`);
  check(r.render.via === want.via, `${label}: rendered on ${r.render.via} (want ${want.via})`);
  check(r.render.playback === want.playback, `${label}: plays through ${r.render.playback} (want ${want.playback})`);
  check(r.render.loops === false && (await b.eval(`document.getElementById('track').loopBox.checked`)) === false, `${label}: a song that does not loop plays once`);
  const browserSha = await b.eval(`(async () => { const t = document.getElementById('track'); const buf = await t.wav.arrayBuffer(); const d = await crypto.subtle.digest('SHA-256', buf.slice(44)); return [...new Uint8Array(d)].map((x) => x.toString(16).padStart(2, '0')).join(''); })()`, true);
  const ns = await nodeRender(text);
  check(browserSha === ns, `${label}: the PCM is Node's render of the same text, byte for byte (${browserSha.slice(0, 16)} / ${ns.slice(0, 16)})`);
  // it plays: the time moves, and (Web Audio) the output has sound
  await sleep(700);
  const r1 = await report(b);
  check(r1.time > 0.3 && /^verse 1\/1$/.test(r1.sec) && /^bar [1-4]\/4$/.test(r1.bar), `${label}: the time and the bar move (${r1.time.toFixed(2)} s, ${r1.sec} ${r1.bar})`);
  if (want.playback === 'webaudio') check((await rmsOf(b)) > 1e-3, `${label}: there is sound on the element's output`);
  // seek to 5 s through the seek bar
  await b.eval(`(() => { const t = document.getElementById('track'); t.seek.value = '5'; t.seek.dispatchEvent(new Event('input')); })()`);
  await sleep(300);
  const r2 = await report(b);
  check(r2.time >= 5 && r2.time < 6 && r2.bar === 'bar 3/4', `${label}: the seek bar moves it to 5 s (${r2.time.toFixed(2)} s, ${r2.bar})`);
  // pause: the place holds and it is silent
  await clickIn(b, 'track', 'button.play');
  await sleep(150);
  const p1 = await report(b);
  await sleep(300);
  const p2 = await report(b);
  check(!p1.playing && p1.time === p2.time && p1.time >= 5, `${label}: Pause holds the place (${p1.time.toFixed(2)} s)`);
  if (want.playback === 'webaudio') check((await rmsOf(b)) < 1e-4, `${label}: paused, the output is silent`);
  else check(await b.eval(`document.getElementById('track').transport.el.paused`), `${label}: paused, the <audio> is paused`);
  // play again: from where it was
  await clickIn(b, 'track', 'button.play');
  await sleep(300);
  const p3 = await report(b);
  check(p3.playing && p3.time > p1.time && p3.time < p1.time + 1, `${label}: Play goes on from there (${p3.time.toFixed(2)} s), the render reused`);
  // stop(): silent at once, back at 0
  await b.eval(`document.getElementById('track').stop()`);
  await sleep(100);
  const s1 = await report(b);
  check(!s1.playing && s1.time === 0, `${label}: stop() goes back to 0 (${s1.time})`);
  if (want.playback === 'webaudio') { const q = await rmsOf(b); check(q < 1e-4, `${label}: stopped, the output is silent within 100 ms (RMS ${q.toExponential(2)})`); }
  else check(await b.eval(`(() => { const a = document.getElementById('track').transport.el; return a.paused && a.currentTime === 0; })()`), `${label}: stopped, the <audio> is paused at 0`);
  // the links: both the self-sealing share link; the stored card's CID is /p/<CID>
  const links = r.links;
  check(links.length === 2 && links[0].part === 'cid' && LINK.test(links[0].href) && links[0].href === links[1].href, `${label}: the CID link is the share link, which carries the program (${links[0] && links[0].href.slice(0, 60)}…)`);
  await b.eval(`document.getElementById('stored').prepare()`, true);
  const st = await report(b, 'stored');
  check(st.links[0] && /^https:\/\/beatscript\.app\/p\/z[1-9A-HJ-NP-Za-km-z]{20,120}$/.test(st.links[0].href) && LINK.test(st.links[1].href), `${label}: with \`stored\`, the CID links to /p/<CID> and Open to the share link (${JSON.stringify(st.links.map((l) => l.href.slice(0, 50)))})`);
  // highlight: the engine's colours, the text exactly, and Copy
  await b.waitFor(`document.getElementById('track').report.highlighted === 'engine'`, 20000);
  const hl = await b.eval(`(() => { const t = document.getElementById('track'); const c = t.shadowRoot.querySelector('.src code'); const cls = {}; for (const s of c.querySelectorAll('span')) cls[s.className] = (cls[s.className] || 0) + 1; return { same: c.textContent === t.text, cls, visible: !t.shadowRoot.querySelector('.src').hidden }; })()`);
  check(hl.visible && hl.same && hl.cls['c-keyword'] > 0 && hl.cls['c-deck'] > 0 && hl.cls['c-comment'] > 0, `${label}: highlight shows the program in the engine's colours (${JSON.stringify(hl.cls)})`);
  await clickIn(b, 'track', 'button.copy');
  await sleep(300);
  const cp = await b.eval(`[document.getElementById('track').report.copied, document.getElementById('track').copyBtn.textContent]`);
  check(['clipboard', 'command', 'selected'].includes(cp[0]) && /Copied|Selected/.test(cp[1]), `${label}: Copy copies, or selects the text when the clipboard is refused (${cp.join(': ')})`);
  // taken off the page and put back: silent, its context closed, and the render plays again (not rendered again)
  await clickIn(b, 'track', 'button.play');
  await sleep(200);
  const moved = await b.eval(`(async () => { const t = document.getElementById('track'); const ctx = t.ctx; const at = t.nextSibling; const p = t.parentNode; t.remove(); await new Promise((r) => setTimeout(r, 50)); const off = [ctx.state, t.report.playing]; p.insertBefore(t, at); return off; })()`, true);
  check(moved[0] === 'closed' && moved[1] === false, `${label}: removed from the page, it stops and closes its context (${moved.join(', ')})`);
  await clickIn(b, 'track', 'button.play');
  await sleep(500);
  const back = await report(b);
  check(back.playing && back.time > 0.2 && back.render && back.render.ms === r.render.ms, `${label}: put back, Play plays the same render (${back.time && back.time.toFixed(2)} s)`);
  await b.eval(`document.getElementById('track').stop()`);
  const v = await b.eval('window.__violations');
  const errors = b.errors().map((e) => e.text);
  return { v, errors, fetched: await b.eval(`performance.getEntriesByType('resource').map((e) => e.name)`) };
}

/** Live mode: Stop is silent at once, and the next Play starts clean from bar 1. */
async function stopCase(b, url, label) {
  await b.goto(url);
  await b.waitFor(`customElements.get('beatscript-track') && document.getElementById('track').shadowRoot`, 20000);
  await b.eval(TAP);
  await clickIn(b, 'track', 'button.play');
  await b.waitFor(`/bar [2-4]\\/4/.test(document.getElementById('track').report.bar)`, 30000);
  const loud = await rmsOf(b);
  check(loud > 1e-3, `${label}: playing, the element's output has sound (RMS ${loud.toFixed(4)})`);
  await clickIn(b, 'track', 'button.play');
  await sleep(100);
  const q = await rmsOf(b);
  const state = await b.eval(`document.getElementById('track').ctx.state`);
  check(q < 1e-4 && state === 'running', `${label}: 100 ms after Stop the output is silent (RMS ${q.toExponential(2)}, the context still ${state})`);
  await sleep(500);
  check((await b.eval(`document.getElementById('track').ctx.state`)) === 'suspended', `${label}: then the context sleeps`);
  // Play again: the first bar the card shows is bar 1, and both decks are installed again
  await clickIn(b, 'track', 'button.play');
  const seen = [];
  const t0 = Date.now();
  while (Date.now() - t0 < 6000) {
    const r = await report(b);
    if (r.playing && r.bar && seen[seen.length - 1] !== r.bar) seen.push(r.bar);
    if (seen.length >= 2) break;
    await sleep(50);
  }
  check(seen[0] === 'bar 1/4' && (await rmsOf(b)) > 1e-3, `${label}: the next Play starts from bar 1, with sound (${seen.join(' then ')})`);
  await b.waitFor(`document.getElementById('track').report.installed.length === 2`, 10000);
  await clickIn(b, 'track', 'button.play');
  await sleep(100);
  check((await rmsOf(b)) < 1e-4, `${label}: and stops silent again`);
  // taken off the page: silent, its context closed
  await clickIn(b, 'track', 'button.play');
  await b.waitFor(`document.getElementById('track').report.playing`, 10000);
  const closed = await b.eval(`(async () => { const t = document.getElementById('track'); const ctx = t.ctx; t.remove(); await new Promise((r) => setTimeout(r, 50)); return [ctx.state, t.report.playing]; })()`, true);
  check(closed[0] === 'closed' && closed[1] === false, `${label}: removed from the page, it stops and closes its context (${closed.join(', ')})`);
}

const b = await launch({ width: 640, height: 900 });
try {
  csp = ARTIFACT.replace("'unsafe-inline'", "'unsafe-inline' 'wasm-unsafe-eval'");
  const f = await play(b, `${site.origin}/test/csp.html`, 'files');
  check(f.r.playing, 'files: it plays from a click');
  check([...f.seen].some((s) => /^verse 1\/1 bar [2-4]\/4$/.test(s)), `files: the section and bar advance (${[...f.seen].join(' | ')})`);
  check(f.v.length === 0, `files: no blocked request (${JSON.stringify(f.v)})`);
  check(f.fetched.every((u) => u.startsWith(site.origin)), 'files: nothing fetched but the page\'s own files');
  check(f.errors.length === 0, `files: no console error (${f.errors.join('; ')})`);
  // deck B is the melody and the line sung on it: both decks' nets reached the engine
  check(JSON.stringify(f.installed) === '["A","B"]', `files: the drums (deck A) and the melody with its sung line (deck B) are installed (${JSON.stringify(f.installed)})`);

  csp = ARTIFACT.replace("'unsafe-inline'", `'unsafe-inline' 'wasm-unsafe-eval' ${cdn.origin}`);
  const c = await play(b, `${site.origin}/cdn.html`, 'cdn');
  check(c.r.playing, 'cdn: it plays from a click');
  check([...c.seen].some((s) => /^verse 1\/1 bar [2-4]\/4$/.test(s)), `cdn: the section and bar advance (${[...c.seen].join(' | ')})`);
  check(c.v.length === 0, `cdn: no blocked request (${JSON.stringify(c.v)})`);
  check(c.fetched.every((u) => u.startsWith(site.origin) || u.startsWith(cdn.origin)), `cdn: nothing fetched but the page and the CDN (${c.fetched.filter((u) => !u.startsWith(cdn.origin)).join(', ')})`);
  check(JSON.stringify(c.installed) === '["A","B"]', `cdn: both decks are installed (${JSON.stringify(c.installed)})`);
  check(c.r.composer === 'main', `cdn: the composer runs on the page thread (a cross-origin Worker is not allowed): ${c.r.composer}`);

  // 0.2.0: Stop, the links, render mode, highlight
  csp = ARTIFACT.replace("'unsafe-inline'", "'unsafe-inline' 'wasm-unsafe-eval'");
  await stopCase(b, `${site.origin}/test/csp.html`, 'stop');
  check(f.r.links.length === 2 && f.r.links[0].part === 'cid' && LINK.test(f.r.links[0].href) && f.r.links[0].href === f.r.links[1].href, `links: in live mode too, the CID link is the share link (${f.r.links.map((l) => l.href.slice(0, 50)).join(' | ')})`);

  // beside the page, blob: media allowed: a Worker renders, an <audio> plays the WAV
  csp = ARTIFACT.replace("'unsafe-inline'", "'unsafe-inline' 'wasm-unsafe-eval'") + "; media-src 'self' blob:";
  const rf = await renderCase(b, `${site.origin}/render.html`, 'render files', { via: 'worker', playback: 'audio' });
  check(rf.v.length === 0, `render files: no blocked request (${JSON.stringify(rf.v)})`);
  check(rf.errors.length === 0, `render files: no console error (${rf.errors.join('; ')})`);
  check(rf.fetched.every((u) => u.startsWith(site.origin) || u.startsWith('blob:')), 'render files: nothing fetched but the page\'s own files');

  // from the CDN under the artifact's CSP: the page's thread renders (no cross-origin Worker), Web Audio plays (no blob: media)
  csp = ARTIFACT.replace("'unsafe-inline'", `'unsafe-inline' 'wasm-unsafe-eval' ${cdn.origin}`);
  const rc = await renderCase(b, `${site.origin}/render-cdn.html`, 'render cdn', { via: 'page', playback: 'webaudio' });
  // the one thing the CSP refuses is the blob: WAV the element tries first (default-src 'self' covers media)
  check(rc.v.every((x) => /media-src|default-src/.test(x.directive) && /^blob/.test(x.blocked)), `render cdn: the only refusal is the blob: media probe (${JSON.stringify(rc.v.map((x) => `${x.directive} ${x.blocked}`))})`);
  check(rc.fetched.every((u) => u.startsWith(site.origin) || u.startsWith(cdn.origin) || u.startsWith('blob:')), 'render cdn: nothing fetched but the page and the CDN');

  csp = ARTIFACT;
  const s = await play(b, `${site.origin}/test/csp.html`, 'strict');
  check(!s.r.playing && /wasm-unsafe-eval/.test(s.r.state), `strict: without 'wasm-unsafe-eval' it does not play and says why (${s.r.state})`);
} finally {
  await b.close();
  site.s.close();
  cdn.s.close();
  fs.rmSync(dist, { recursive: true, force: true });
}
console.log(failures.length ? `# ${failures.length} failed` : '# all passed');
process.exit(failures.length ? 1 : 0);
