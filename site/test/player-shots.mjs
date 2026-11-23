#!/usr/bin/env node
// player-shots.mjs: screenshots of the radio player (/radio) while it plays, with a queue of five rows in mixed render
// states (ready, rendering, failed, queued, none) above the station's own tracks, at a desktop and a phone width.
//
//   node site/test/player-shots.mjs [--out DIR]          writes DIR/player-desktop.png, player-mobile.png, player-peek.png
//
// Runs against mock-server.mjs (the real wasm when site/web/wasm is built: the station track plays on the real engine).
// The render states are set through the mock's /__render_state/<cid> (what /api/render/<cid>/status then answers) and,
// for "queued" (this page asked, the server has not started it), through the page's own tracker. Exit 1 on a console error.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { launch, sleep } from './cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i < 0 ? d : process.argv[i + 1]; };
const OUT = arg('out', path.join(here, 'shots', 'player'));
fs.mkdirSync(OUT, { recursive: true });

const port = await new Promise((res) => { const s = net.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => res(p)); }); });
const server = spawn('node', [path.join(here, 'mock-server.mjs'), '--port', String(port)], { stdio: ['ignore', 'ignore', 'inherit'] });
process.on('exit', () => { try { server.kill('SIGKILL'); } catch { /* gone */ } });
const base = `http://127.0.0.1:${port}`;
for (let i = 0; i < 50; i++) { try { if ((await fetch(`${base}/healthz`)).ok) break; } catch { /* starting */ } await sleep(100); }

const QUEUE = ['first-beat', 'trance-138', 'acid-line', 'matrix-130', 'such-stuff-as-dreams-122'];
const STATES = [{ state: 'ready' }, { state: 'rendering', pct: 45 }, { state: 'failed' }, null, null];

const page = await launch({ width: 1280, height: 1000, args: ['--disable-audio-output', '--autoplay-policy=no-user-gesture-required'] });
let code = 0;
try {
  const ex = await (await fetch(`${base}/api/examples`)).json();
  const cids = QUEUE.map((n) => (ex.find((e) => e.name === n) || {}).cid);
  for (let i = 0; i < cids.length; i++) if (STATES[i]) await fetch(`${base}/__render_state/${cids[i]}`, { method: 'POST', body: JSON.stringify(STATES[i]) });
  await page.goto(`${base}/radio`);
  await page.waitFor('window.__radio && window.__radio.state.cid && window.__radio.state.now && !window.__radio.waiting && window.__radio.session && window.__radio.session.id', 30000);
  // the station's track 2 is rendered already (another listener played it)
  const s2 = await page.eval('window.__radio.ahead.find((t) => t.n === 2).cid');
  await fetch(`${base}/__render_state/${s2}`, { method: 'POST', body: JSON.stringify({ state: 'ready' }) });
  await page.click('#p-play');
  await page.waitFor('window.__radio.playing && window.__radio.item && window.__radio.item.kind === "track"', 30000);
  // five rows added from the playlist (the page's own POST …/queue), one at a time as a person would
  for (const name of QUEUE) {
    await page.eval(`document.getElementById('pl-add-in').value = ${JSON.stringify(name)}`);
    await page.click('#pl-add button');
    await page.waitFor(`window.__radio.queue && window.__radio.queue.up_next.some((x) => x.example === ${JSON.stringify(name)})`, 10000);
  }
  await page.eval(`window.__radio.tracker.mark(${JSON.stringify(cids[3])}, { state: 'queued' })`);
  await page.waitFor(`window.__radio.rows && window.__radio.rows.filter((r) => r.kind === 'queue').length === 5 && window.__radio.tracker.get(${JSON.stringify(cids[1])}).state === 'rendering' && window.__radio.tracker.get(${JSON.stringify(cids[2])}).state === 'failed'`, 15000);
  await page.eval(`window.__radio.selected = window.__radio.rows[2].key; window.__radio.renderList()`);
  await page.waitFor('window.__radio.pos && window.__radio.pos.songBar >= 2', 30000).catch(() => {});
  await sleep(800);
  const states = await page.eval('JSON.stringify(window.__radio.rows.map((r) => [r.kind, r.title, r.render.state, r.source]))');
  console.log('rows:', states);
  await page.shot(path.join(OUT, 'player-desktop.png'));
  // the peek drawer, open over the playing track
  await page.click('#p-peek');
  await sleep(400);
  await page.shot(path.join(OUT, 'player-peek.png'));
  await page.click('#peek-close');
  // a phone
  await page.cdp('Emulation.setDeviceMetricsOverride', { width: 412, height: 915, deviceScaleFactor: 2, mobile: true });
  await sleep(700);
  await page.shot(path.join(OUT, 'player-mobile.png'));
  const m = await page.eval('JSON.stringify({ overflowX: document.documentElement.scrollWidth > document.documentElement.clientWidth, playing: window.__radio.playing, source: window.__radio.playingFrom })');
  console.log(m);
  if (JSON.parse(m).overflowX) { console.log('the phone layout scrolls sideways'); code = 1; }
  const bad = page.errors().filter((e) => !/favicon/i.test(e.text));
  if (bad.length) { console.log('console errors:', bad.map((e) => e.text).join(' | ').slice(0, 600)); code = 1; }
} catch (e) {
  console.log('player-shots:', e.message);
  code = 1;
} finally {
  await page.close();
  try { server.kill('SIGKILL'); } catch { /* gone */ }
  console.log(`shots in ${OUT}`);
  process.exit(code);
}
