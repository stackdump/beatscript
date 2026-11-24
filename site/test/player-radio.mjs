// <beatscript-radio> (site/player/src/beatscript-radio.js) in headless Chrome: a station artifact page, as the MCP
// `artifact` tool makes it, under a Claude-artifact-like CSP, its player loaded from "cdn.jsdelivr.net" with the page's
// Subresource Integrity, and the package's own test/radio.html beside its files.
//
//   BEATSCRIPT_STATION_PAGE=<the artifact page> BEATSCRIPT_PLAYER_DIST=<the unpacked package> node site/test/player-radio.mjs
//
// BEATSCRIPT_STATION_PAGE: the page the artifact tool made for a station (cargo test -p beatscript-serve --bin beatscript-serve
// a_station_makes_a_radio_page with BEATSCRIPT_STATION_PAGE_OUT=<file> writes it: the short techno station, 40-bar tracks
// at 140, no wait of minutes for a track to end). BEATSCRIPT_PLAYER_DIST: the package the page's lock was made from
// (the npm tarball unpacked: its package/), else one is built from site/web/wasm (or BEATSCRIPT_WASM) into a temp dir,
// and the artifact case is skipped (a fresh build's hashes are not the lock's). Without a wasm it skips.
//
// The page's requests to https://cdn.jsdelivr.net/npm/<package>@<version>/... are answered from the package (CDP Fetch),
// with CORS, so the page is byte for byte the tool's: its CSP, its import map and every integrity hash are checked by
// the browser against the package's bytes. Then:
//   loads      the element is defined and the station answered (computed in the page: no request but the CDN's), and
//              nothing plays before a click: no AudioContext, no engine
//   plays      one click on Play: it plays (the section and bar move) and there is sound
//   no gap     a tap on the element's output (a ScriptProcessor, 1024-frame buffers) records every run of silence
//              (|x| < 1e-4 on both channels) at sample resolution. Across every track change (a natural end, Next with
//              the next track ready, Next while it is still resolving, which bridges) no silence is longer than one
//              buffer (1024 frames) from a second before the change to four after
//   tidy       at most three engines at once (playing, fading, primed); Stop is silent and lets every engine go
//   sandbox    no securitypolicyviolation, nothing fetched but the page's origin and the CDN, no cookie written, no
//              localStorage or sessionStorage touched, no console error

import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { launch, sleep } from './cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const wasm = process.env.BEATSCRIPT_WASM || path.join(here, '..', 'web', 'wasm');
let dist = process.env.BEATSCRIPT_PLAYER_DIST;
const pagePath = process.env.BEATSCRIPT_STATION_PAGE;
if (!dist) {
  if (!fs.existsSync(path.join(wasm, 'beatscript_bg.wasm'))) {
    console.log(`# skip: no ${wasm}/beatscript_bg.wasm (scripts/build-web-wasm.sh ${wasm}) and no BEATSCRIPT_PLAYER_DIST`);
    process.exit(0);
  }
  dist = fs.mkdtempSync(path.join(os.tmpdir(), 'beatscript-player-'));
  execFileSync(process.execPath, [path.join(here, '..', 'player', 'build.mjs'), '--wasm', wasm, '--out', dist], { stdio: 'inherit' });
}
dist = path.resolve(dist);
const pkg = JSON.parse(fs.readFileSync(path.join(dist, 'package.json'), 'utf8'));
const CDN = `https://cdn.jsdelivr.net/npm/${pkg.name}@${pkg.version}/`;

// an artifact's policy, with 'wasm-unsafe-eval' (the artifact probe found it allowed; without it no wasm runs)
const CSP = "default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval' cdn.jsdelivr.net; connect-src 'self'";
const TYPES = { '.js': 'text/javascript', '.html': 'text/html; charset=utf-8', '.json': 'application/json', '.ts': 'text/plain' };
// what the test page sees before any of its own scripts: blocked requests, and any storage or cookie use
const WATCH = `window.__violations = []; document.addEventListener('securitypolicyviolation', (e) => window.__violations.push({ directive: e.violatedDirective, blocked: e.blockedURI }));
window.__storage = [];
for (const k of ['getItem', 'setItem', 'removeItem', 'clear', 'key']) { const f = Storage.prototype[k]; Storage.prototype[k] = function (...a) { window.__storage.push(k); return f.apply(this, a); }; }
{ const d = Object.getOwnPropertyDescriptor(Document.prototype, 'cookie'); Object.defineProperty(document, 'cookie', { get() { return d.get.call(document); }, set(v) { window.__storage.push('cookie'); d.set.call(document, v); } }); }`;
// the tap: every run of silence on the element's output, with when it ended (on the context's clock), and every track change
const TAP = `window.__gaps = (el) => {
  if (el.__tap) return true;
  if (!el.output) return false;
  const ctx = el.output.context;
  const sp = ctx.createScriptProcessor(1024, 2, 2);
  const z = ctx.createGain(); z.gain.value = 0;
  el.output.connect(sp); sp.connect(z); z.connect(ctx.destination);
  const t = el.__tap = { sp, runs: [], run: 0, frames: 0, loud: 0, changes: [], rate: ctx.sampleRate, since: ctx.currentTime };
  sp.onaudioprocess = (e) => {
    const a = e.inputBuffer.getChannelData(0), b = e.inputBuffer.getChannelData(1);
    for (let i = 0; i < a.length; i++) {
      if (Math.abs(a[i]) < 1e-4 && Math.abs(b[i]) < 1e-4) t.run++;
      else { if (t.run > 256) t.runs.push({ end: e.playbackTime + i / t.rate, frames: t.run }); t.run = 0; t.loud++; }
    }
    t.frames += a.length;
  };
  // (a swap: the element had an engine playing, now fading under the new one; the first Play comes out of silence)
  el.addEventListener('trackchange', (e) => t.changes.push({ at: ctx.currentTime, key: e.detail.kind + ':' + e.detail.n, swap: el.outgoing.length > 0 }));
  return true;
};`;

function serve(handler) {
  return new Promise((resolve) => {
    const s = http.createServer(handler);
    s.listen(0, '127.0.0.1', () => resolve({ s, origin: `http://127.0.0.1:${s.address().port}` }));
  });
}
function file(res, p, headers = {}) {
  if (!p.startsWith(dist) || !fs.existsSync(p) || fs.statSync(p).isDirectory()) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { 'Content-Type': TYPES[path.extname(p)] || 'application/octet-stream', 'Cache-Control': 'no-store', ...headers });
  res.end(fs.readFileSync(p));
}

const artifact = pagePath ? fs.readFileSync(pagePath, 'utf8') : null;
const site = await serve((req, res) => {
  const u = new URL(req.url, 'http://x');
  if (u.pathname === '/favicon.ico') { res.writeHead(204); res.end(); return; }
  if (u.pathname === '/artifact.html' && artifact) { res.writeHead(200, { 'Content-Type': TYPES['.html'], 'Content-Security-Policy': CSP }); res.end(artifact); return; }
  file(res, path.join(dist, decodeURIComponent(u.pathname)), u.pathname.endsWith('.html') ? { 'Content-Security-Policy': CSP } : {});
});

const failures = [];
const check = (ok, what) => { console.log(`${ok ? 'ok' : 'FAIL'} - ${what}`); if (!ok) failures.push(what); };

/** The CDN: requests to cdn.jsdelivr.net answered from the package, as jsDelivr serves the published tarball. */
async function cdn(b) {
  const served = [];
  b.listen(async (d) => {
    if (d.method !== 'Fetch.requestPaused') return;
    const { requestId, request } = d.params;
    const ok = request.url.startsWith(CDN);
    const p = ok ? path.join(dist, decodeURIComponent(request.url.slice(CDN.length).split('?')[0])) : '';
    if (!ok || !p.startsWith(dist) || !fs.existsSync(p)) {
      served.push({ url: request.url, status: 404 });
      await b.cdp('Fetch.fulfillRequest', { requestId, responseCode: 404, responseHeaders: [{ name: 'Access-Control-Allow-Origin', value: '*' }], body: '' }).catch(() => {});
      return;
    }
    served.push({ url: request.url, status: 200 });
    await b.cdp('Fetch.fulfillRequest', {
      requestId, responseCode: 200,
      responseHeaders: [{ name: 'Content-Type', value: TYPES[path.extname(p)] || 'application/octet-stream' }, { name: 'Access-Control-Allow-Origin', value: '*' }],
      body: fs.readFileSync(p).toString('base64'),
    }).catch(() => {});
  });
  await b.cdp('Fetch.enable', { patterns: [{ urlPattern: 'https://cdn.jsdelivr.net/*' }] });
  return served;
}

const R = `document.getElementById('radio') || document.querySelector('beatscript-radio')`;
const report = (b) => b.eval(`(${R}).report`);
async function clickIn(b, sel) {
  const r = await b.eval(`(()=>{const e=(${R}).shadowRoot.querySelector(${JSON.stringify(sel)}); if(!e) return null; const r=e.getBoundingClientRect(); return {x:r.x+r.width/2,y:r.y+r.height/2}})()`);
  if (!r) throw new Error(`no ${sel} in the radio`);
  for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) await b.cdp('Input.dispatchMouseEvent', { type, x: r.x, y: r.y, button: 'left', clickCount: 1 });
}
const tap = (b) => b.eval(`window.__gaps(${R})`);
const tapState = (b) => b.eval(`(() => { const t = (${R}).__tap; return t ? { runs: t.runs.slice(), run: t.run, frames: t.frames, loud: t.loud, changes: t.changes.slice(), rate: t.rate, since: t.since, now: (${R}).output.context.currentTime } : null; })()`);
async function until(b, fn, ms, step = 200) {
  const t0 = Date.now();
  for (;;) {
    const r = await report(b);
    if (fn(r) || Date.now() - t0 > ms) return r;
    await sleep(step);
  }
}

/** The longest silence around a change at `at` (context seconds): the runs that end in [at - 1, at + 4] or are still going. */
function gapAround(tp, at) {
  let worst = 0;
  for (const g of tp.runs) {
    const start = g.end - g.frames / tp.rate;
    if (g.end >= at - 1 && start <= at + 4) worst = Math.max(worst, g.frames);
  }
  if (tp.now - tp.run / tp.rate <= at + 4 && tp.now >= at - 1) worst = Math.max(worst, tp.run);
  return worst;
}

async function radioCase(url, label, { natural = false } = {}) {
  const b = await launch({ width: 900, height: 900 });
  try {
    await b.cdp('Page.addScriptToEvaluateOnNewDocument', { source: WATCH });
    const served = await cdn(b);
    await b.goto(url);
    await b.waitFor(`customElements.get('beatscript-radio') && (${R}) && (${R}).report.station`, 60000);
    await b.eval(TAP);
    const r0 = await report(b);
    check(!!r0.station && /^z/.test(r0.station.cid), `${label}: the station is answered in the page (${r0.station && r0.station.cid}, ${r0.rows.length} rows: ${r0.rows.map((x) => x.title).slice(0, 3).join(' | ')})`);
    const ctx0 = await b.eval(`String((${R}).ctx)`);
    check(!r0.playing && r0.engines === 0 && ctx0 === 'null', `${label}: nothing plays before a click (no AudioContext: ${ctx0}, ${r0.engines} engines, ${r0.state})`);
    // one click
    // the tap goes on the output as soon as there is one (the click makes it)
    await b.eval(`window.__tapTimer = setInterval(() => { if (window.__gaps(${R})) clearInterval(window.__tapTimer); }, 5)`);
    await clickIn(b, '[part=play]');
    const r1 = await until(b, (r) => r.playing || /does not|refused|cannot|error/i.test(r.state_line), 90000);
    check(r1.playing, `${label}: a click on Play plays (${r1.state}; ${r1.state_line})`);
    if (!r1.playing) return;
    await b.waitFor(`window.__gaps(${R})`, 5000);
    await sleep(2500);
    const seen = new Set();
    for (let i = 0; i < 10; i++) { seen.add((await report(b)).position); await sleep(300); }
    const tp0 = await tapState(b);
    check(tp0.loud > 0.5 * tp0.frames, `${label}: there is sound (${tp0.loud} of ${tp0.frames} frames above -80 dBFS)`);
    check(seen.size >= 2, `${label}: the section and bar move (${[...seen].join(' / ')})`);
    // Next, with the next track ready (primed on its own engine)
    const ready = await until(b, (r) => r.next && r.next.state === 'ready', 60000);
    check(ready.next && ready.next.state === 'ready', `${label}: the next item is ready while this one plays (${JSON.stringify(ready.next)}, ${ready.engines} engines)`);
    let maxEngines = ready.engines;
    const before = ready.item.key;
    await clickIn(b, '[part=next]');
    const r2 = await until(b, (r) => r.item && r.item.key !== before, 10000, 50);
    check(r2.item && r2.item.key !== before, `${label}: Next changes the track (${before} -> ${r2.item && r2.item.key})`);
    // Next again at once (the button's own handler, in the same turn): the next item is still resolving (it is compiled
    // only once the swap's fade is over), so the bridge covers it, hot-reloaded into what plays
    const r3a = await b.eval(`(() => { const r = (${R}); const before = r.report; r.skip(); return { ...before, after: r.report.state }; })()`);
    const r3 = await until(b, (r) => r.state === 'bridging' || (r.item && r.item.key !== r3a.item.key), 5000, 50);
    console.log(`# ${label}: second Next while ${JSON.stringify(r3a.next)}: ${r3a.after}, then ${r3.state} ${r3.item && r3.item.key}`);
    check(r3a.after === 'bridging', `${label}: Next with nothing ready bridges (${r3a.after}; next ${JSON.stringify(r3a.next)})`);
    const r4 = await until(b, (r) => r.state === 'playing' && r.item && r.item.key !== r3a.item.key, 90000, 200);
    check(r4.state === 'playing' && r4.item.key !== r3a.item.key, `${label}: after the bridge, the next track plays (${r4.item && r4.item.key})`);
    for (let i = 0; i < 10; i++) { maxEngines = Math.max(maxEngines, (await report(b)).engines); await sleep(400); }
    if (natural) {
      // a track that ends by itself: the station's tracks are 40 bars at 140 (68.6 s)
      const k = (await report(b)).item.key;
      const r5 = await until(b, (r) => r.item && r.item.key !== k, 120000, 500);
      check(r5.item && r5.item.key !== k, `${label}: a track that ends hands over by itself (${k} -> ${r5.item && r5.item.key}; swap ${JSON.stringify(r5.swaps[r5.swaps.length - 1])})`);
      for (let i = 0; i < 12; i++) { maxEngines = Math.max(maxEngines, (await report(b)).engines); await sleep(400); }
    }
    const tp = await tapState(b);
    const fin = await report(b);
    console.log(`# ${label}: changes ${JSON.stringify(tp.changes)}; silences over 256 frames ${JSON.stringify(tp.runs.map((g) => [+g.end.toFixed(2), g.frames]))}; swaps ${JSON.stringify(fin.swaps)}`);
    // the first change is the first Play (out of silence): the ones after it are the swaps
    const swaps = tp.changes.filter((c) => c.swap);
    check(swaps.length >= (natural ? 3 : 2), `${label}: ${swaps.length} track changes heard by the tap`);
    for (const c of swaps) {
      const g = gapAround(tp, c.at);
      check(g <= 1024, `${label}: no gap over one buffer across the change to ${c.key} at ${c.at.toFixed(2)} s (longest silence ${g} frames, ${(1000 * g / tp.rate).toFixed(1)} ms)`);
    }
    check(maxEngines <= 3, `${label}: at most three engines at once (${maxEngines})`);
    // Stop: silent, every engine let go
    await clickIn(b, '[part=play]');
    await sleep(400);
    const rs = await report(b);
    const ts = await tapState(b);
    check(!rs.playing && rs.engines === 0, `${label}: Stop stops and lets every engine go (${rs.engines})`);
    check(ts.run > 4096 || (await b.eval(`(${R}).output.context.state`)) !== 'running', `${label}: Stop is silent (${ts.run} silent frames)`);
    const v = await b.eval('window.__violations');
    const st = await b.eval('window.__storage');
    const origin = new URL(url).origin;
    const fetched = await b.eval(`performance.getEntriesByType('resource').map((e) => e.name)`);
    const foreign = fetched.filter((u) => !u.startsWith(origin) && !u.startsWith('https://cdn.jsdelivr.net/'));
    check(v.length === 0, `${label}: no request blocked by the CSP (${JSON.stringify(v)})`);
    check(foreign.length === 0, `${label}: nothing fetched but the page's origin and the CDN (${foreign.join(', ')})`);
    check(st.length === 0, `${label}: no cookie and no storage touched (${st.join(', ')})`);
    check(served.every((s) => s.status === 200), `${label}: every CDN request was a file of the package (${served.filter((s) => s.status !== 200).map((s) => s.url).join(', ')})`);
    const errors = b.errors().map((e) => e.text).filter((t) => !/favicon/.test(t));
    check(errors.length === 0, `${label}: no console error (${errors.join(' | ')})`);
    if (process.env.BEATSCRIPT_SHOT) await b.shot(process.env.BEATSCRIPT_SHOT.replace(/\.png$/, `-${label}.png`));
  } finally {
    await b.close();
  }
}

if (artifact) {
  check(artifact.includes(`src="${CDN}beatscript-radio.js"`), `artifact: the page loads ${CDN}beatscript-radio.js (the package's version)`);
  await radioCase(`${site.origin}/artifact.html`, 'artifact', { natural: true });
} else {
  console.log('# skip artifact: no BEATSCRIPT_STATION_PAGE');
}
await radioCase(`${site.origin}/test/radio.html`, 'files');

site.s.close();
console.log(failures.length ? `\n${failures.length} FAILED` : '\nall ok');
process.exit(failures.length ? 1 : 0);
