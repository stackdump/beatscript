// The editor's "render" source in headless Chrome, under the site's own CSP (mock-server.mjs --csp strict, which is
// beatscript-serve's policy: media-src 'self' blob:).
//
//   BEATSCRIPT_WASM=<dir> node site/test/render-source.mjs     (needs beatscript.js + beatscript_bg.wasm: site/web/wasm by default; skips without)
//
// The source switch: render beside engine and stream, remembered per browser (localStorage), back to auto. Play renders
// first (a Worker), showing progress as a percentage and bars, then plays the WAV on an <audio> element (blob:) with a
// seek bar, pause and loop; the follow view and header follow the audio's currentTime (a forward seek counts on, a seek
// back restarts the run at bar 0 and catches up); Stop is silent at once (the <audio> is paused there and then), while it
// plays and while it renders; a second Play of the same CID plays the cached render, a changed text renders again; the
// stutter hint offers render. No CSP violation and no console error, all the way through.

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { launch, sleep } from './cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const wasm = process.env.BEATSCRIPT_WASM || path.join(here, '..', 'web', 'wasm');
if (!fs.existsSync(path.join(wasm, 'beatscript_bg.wasm'))) {
  console.log(`# skip: no ${wasm}/beatscript_bg.wasm (scripts/build-web-wasm.sh ${wasm})`);
  process.exit(0);
}

const failures = [];
const check = (ok, what) => { console.log(`${ok ? 'ok' : 'FAIL'} - ${what}`); if (!ok) failures.push(what); };

// the site's files with the module beside them, served by the mock under the strict (beatscript-serve's) CSP
const web = fs.mkdtempSync(path.join(os.tmpdir(), 'btw-render-source-'));
fs.cpSync(path.join(here, '..', 'web'), web, { recursive: true });
fs.mkdirSync(path.join(web, 'wasm'), { recursive: true });
for (const f of fs.readdirSync(wasm)) if (/^beatscript(_bg)?\.(js|wasm)$|^hashes\.json$/.test(f)) fs.copyFileSync(path.join(wasm, f), path.join(web, 'wasm', f));
const port = await new Promise((res) => { const s = net.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => res(p)); }); });
const server = spawn(process.execPath, [path.join(here, 'mock-server.mjs'), '--port', String(port), '--web', web, '--csp', 'strict'], { stdio: ['ignore', 'ignore', 'inherit'] });
const base = `http://127.0.0.1:${port}`;
for (let i = 0; i < 100; i++) { try { if ((await fetch(`${base}/healthz`)).ok) break; } catch { /* starting */ } await sleep(100); }
const cleanup = () => { try { server.kill(); } catch { /* gone */ } try { fs.rmSync(web, { recursive: true, force: true }); } catch { /* busy */ } };
process.on('exit', cleanup);

const b = await launch({ width: 1280, height: 800 });
await b.cdp('Page.addScriptToEvaluateOnNewDocument', { source: `window.__violations = []; document.addEventListener('securitypolicyviolation', (e) => window.__violations.push({ directive: e.violatedDirective, blocked: e.blockedURI }));` });
const ready = () => b.eval('(async()=>{ for (let i=0;i<300 && !(window.__btw);i++) await new Promise(r=>setTimeout(r,50)); await window.__btw.ready; return true })()', true);
const pressed = (id) => b.eval(`document.getElementById(${JSON.stringify(id)}).getAttribute('aria-pressed')`);
const status = () => b.eval(`document.getElementById('st-long').textContent`);
const pos = () => b.eval(`(() => { const a = __app, p = a.player, l = a.driver && a.driver.last; return { t: p ? p.mediaTime() : null, bar: l && l.status ? l.status.bar : null, sec: document.getElementById('h-sec-txt').textContent, epoch: p ? p.epoch : null, kind: a.driver ? a.driver.kind : null, state: p ? p.state : null }; })()`);
const setProgram = (text) => b.eval(`__app.setProgram(${JSON.stringify(text)}, { name: 'render test', quiet: true, keepHash: true }), true`);

// 16 bars at 120 (a bar is 2 s): verse 4, chorus 4, twice
const DRUMS = 'let four = x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\nlet backs = ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~\nlet offs = ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~\n';
const song = (tempo) => `#! name: render source\ntempo ${tempo}\nkey am\n${DRUMS}section verse 4 bars {\n  deck A: kick four | hihat offs\n  deck B: melody a4 ~ c5 ~ e5 ~ c5 ~ a4 ~ g4 ~ a4 ~ ~ ~\n}\nsection chorus 4 bars {\n  deck A: kick four | snare backs | hihat offs\n}\narrange verse chorus verse chorus\n`;

try {
  await b.goto(`${base}/`);
  await ready();
  check((await b.eval('__app.wasmOk')) === true, 'the page loads under the strict CSP with the real module');
  check((await pressed('src-render')) === 'false' && (await b.eval(`document.getElementById('src-render').disabled`)) === false, 'a render button beside engine and stream, enabled, not chosen');

  // ---- the switch, remembered
  await b.click('#src-render');
  check((await pressed('src-render')) === 'true' && (await b.eval(`localStorage.getItem('btw-source')`)) === 'render', 'render chosen, and remembered in localStorage');

  // ---- render, with progress, then play
  await setProgram(song(120));
  await b.click('#btn-play');
  const seen = new Set();
  const t0 = Date.now();
  while (Date.now() - t0 < 60000) {
    const s = await status();
    if (/rendering \d+%/.test(s)) seen.add(s.replace(/^render · /, ''));
    if (await b.eval(`!!(__app.player && __app.player.kind === 'render' && __app.player.state === 'playing')`)) break;
    await sleep(25);
  }
  const r = await b.eval(`__app.player && __app.player.rendered`);
  check(!!r, `it renders and plays (${JSON.stringify(r)})`);
  check([...seen].some((s) => /rendering \d+% · bar \d+\/16/.test(s)), `progress as a percentage and bars (${[...seen].slice(0, 4).join(' | ')})`);
  check(r && r.via === 'worker', `rendered in a Worker (${r && r.via})`);
  check(r && r.playback === 'audio', `played on an <audio> element from a blob: WAV, which the site's CSP allows (${r && r.playback})`);
  check(r && Math.abs(r.seconds - 32) < 0.01, `one pass: 16 bars at 120 is 32 s (${r && r.seconds})`);
  check(/render · rendered audio/.test(await status()), `the status says so (${await status()})`);
  check((await b.eval(`document.getElementById('rd-ctl').hidden`)) === false, 'the seek bar, pause and loop are shown');
  check((await b.eval(`__app.player.transport.el.paused`)) === false, 'the <audio> plays');

  // ---- the code lights up with the audio: the runtime's bar is the audio's
  await sleep(2600);
  let p = await pos();
  check(p.kind === 'render' && p.bar != null && Math.abs(p.bar - Math.floor(p.t / 2)) <= 1, `the follow view follows currentTime: bar ${p.bar} at ${p.t && p.t.toFixed(2)} s`);
  check(/verse/.test(p.sec), `the header names the section (${p.sec})`);

  // forward seek: into the chorus (bar 6 is 12-14 s)
  await b.eval(`(() => { const s = document.getElementById('rd-seek'); s.value = '12.5'; s.dispatchEvent(new Event('input')); s.dispatchEvent(new Event('change')); })()`);
  await sleep(600);
  p = await pos();
  check(p.epoch === 0 && p.t >= 12.5 && p.bar >= 6 && p.bar <= 7 && /chorus/.test(p.sec), `a seek forward: the runtime counts on to the audio (bar ${p.bar}, ${p.t.toFixed(2)} s, ${p.sec}, epoch ${p.epoch})`);
  // back to the start: a fresh run, caught up at once
  await b.eval(`(() => { const s = document.getElementById('rd-seek'); s.value = '0.5'; s.dispatchEvent(new Event('input')); s.dispatchEvent(new Event('change')); })()`);
  await sleep(700);
  p = await pos();
  check(p.epoch === 1 && p.bar != null && p.bar <= 1 && /verse/.test(p.sec), `a seek back: a fresh run from bar 0 (bar ${p.bar}, ${p.t.toFixed(2)} s, ${p.sec}, epoch ${p.epoch})`);
  check((await b.eval('__app.driver.kind')) === 'render' && (await b.eval('__app.isPlaying()')), 'and it plays on, on the render');

  // pause holds the place; resume goes on from it
  await b.click('#rd-pause');
  const t1 = (await pos()).t;
  await sleep(500);
  const held = await pos();
  check(held.state === 'paused' && Math.abs(held.t - t1) < 0.01 && (await b.eval(`__app.player.transport.el.paused`)), `pause holds the place (${t1.toFixed(2)} -> ${held.t.toFixed(2)} s)`);
  check(/paused/.test(await status()), `the status says paused (${await status()})`);
  await b.click('#rd-pause');
  await sleep(500);
  const on = await pos();
  check(on.state === 'playing' && on.t > held.t + 0.2, `resume goes on from there (${held.t.toFixed(2)} -> ${on.t.toFixed(2)} s)`);

  // loop
  await b.click('#rd-loop');
  check((await b.eval(`__app.player.transport.el.loop`)) === true, 'loop on loops the <audio>');
  await b.click('#rd-loop');
  check((await b.eval(`__app.player.transport.el.loop`)) === false, 'and off again');

  // ---- Stop is silent at once
  const stopped = await b.eval(`(() => { const el = __app.player.transport.el; document.getElementById('btn-play').click(); return { paused: el.paused, src: el.getAttribute('src'), playing: __app.isPlaying(), player: !!__app.player }; })()`);
  check(stopped.paused && !stopped.src && !stopped.playing && !stopped.player, `Stop: the <audio> is paused and let go in the same task (${JSON.stringify(stopped)})`);
  check((await b.eval(`document.getElementById('rd-ctl').hidden`)) === true, 'the render controls go with it');

  // ---- the same CID plays its cached render; a changed text renders again
  await b.click('#btn-play');
  await b.waitFor(`__app.player && __app.player.kind === 'render' && __app.player.state === 'playing'`, 15000);
  check((await b.eval('__app.player.rendered.cached')) === true, 'Play again, the same text: the cached render plays (no re-render)');
  // an edit evaluated while it plays (Cmd+Enter): the old render plays on while the new one renders, then stops before the
  // new one sounds (never both)
  await b.eval(`window.__old = __app.player.transport.el`);
  await setProgram(song(121));
  await b.eval(`__app.play('eval'), true`);
  await b.waitFor(`__app.pendingPlayer && __app.pendingPlayer.state === 'rendering'`, 10000);
  check((await b.eval(`window.__old.paused`)) === false, 'while the new text renders, the old render plays on');
  await b.waitFor(`__app.player && __app.player.kind === 'render' && __app.player.state === 'playing' && __app.player.transport.el !== window.__old`, 60000);
  check((await b.eval('__app.player.rendered.cached')) === false, 'a changed text (a new CID) renders again');
  const swap = await b.eval(`({ old: window.__old.paused && !window.__old.getAttribute('src'), now: !__app.player.transport.el.paused })`);
  check(swap.old && swap.now, `then the old one is let go and only the new one plays (${JSON.stringify(swap)})`);
  await b.click('#btn-play');

  // ---- Stop while it renders: the render is dropped and nothing plays
  await setProgram(song(122));
  await b.click('#btn-play');
  await b.waitFor(`__app.pendingPlayer && __app.pendingPlayer.state === 'rendering'`, 10000);
  await b.click('#btn-play');
  const mid = await b.eval(`({ pending: !!__app.pendingPlayer, status: document.getElementById('st-long').textContent })`);
  check(!mid.pending && !/rendering/.test(mid.status), `Stop while rendering: the render is dropped at once (${JSON.stringify(mid)})`);
  await sleep(3000);
  check((await b.eval('__app.isPlaying()')) === false && (await b.eval('__app.player')) === null, 'and nothing starts playing after');

  // ---- the stutter hint
  await b.eval(`__app.stutterHint()`);
  const banner = await b.eval(`document.getElementById('banner').textContent`);
  check(/Audio stuttering\? Try render\./.test(banner), `the stutter hint (${banner.trim().slice(0, 60)})`);

  // ---- remembered across a reload; a second click goes back to auto
  await b.goto('about:blank');
  await b.goto(`${base}/`);
  await ready();
  check((await pressed('src-render')) === 'true', 'after a reload the page remembers render');
  await b.click('#src-render');
  check((await b.eval(`localStorage.getItem('btw-source')`)) === 'auto' && (await pressed('src-render')) === 'false', 'clicking it again goes back to auto, remembered');

  const v = await b.eval('window.__violations');
  check(Array.isArray(v) && v.length === 0, `no CSP violation (${JSON.stringify(v)})`);
  const errors = b.errors().map((e) => e.text).filter((t) => !/favicon/.test(t));
  check(errors.length === 0, `no console error (${JSON.stringify(errors.slice(0, 5))})`);
} catch (e) {
  check(false, `the run threw: ${e && e.stack ? e.stack : e}`);
} finally {
  await b.close();
  cleanup();
}
console.log(failures.length ? `# ${failures.length} failed` : '# all passed');
process.exit(failures.length ? 1 : 0);
