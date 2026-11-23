#!/usr/bin/env node
// generate-browser.mjs: the ✦generate panel in headless Chrome, signed out.
//
//   node site/test/generate-browser.mjs [--server http://127.0.0.1:18097]
//
// A visitor who has not signed in reaches the seeded generator in one click (and by the G key), and Generate writes a
// program that checks clean with the pinned CID for (genre, seed); +1 steps to the next seed's program while it plays;
// "Insert as reference" puts the gen@2 call at the caret. Against the mock server (mock-server.mjs) with the real wasm.
// Skips (exit 0) when the wasm is not built or Chrome is not installed (CHROME=/usr/bin/google-chrome on Linux).
// Screenshots go to site/test/shots/gen-*.png. Exit code 1 if any check failed or the page logged a console error.

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { CHROME, launch, sleep } from './cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const web = path.join(here, '..', 'web');
const SHOTS = path.join(here, 'shots');
fs.mkdirSync(SHOTS, { recursive: true });
const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i < 0 ? d : process.argv[i + 1]; };

if (!fs.existsSync(path.join(web, 'wasm', 'beatscript_bg.wasm'))) { console.log('SKIP generate-browser: no built module in site/web/wasm'); process.exit(0); }
if (!fs.existsSync(CHROME)) { console.log(`SKIP generate-browser: no Chrome at ${CHROME}`); process.exit(0); }

// The pins: what seeded@2 prints for these inputs never changes (a generator that would print another program is a new
// version), so these CIDs are the generator's, and the node-side module must agree with them too.
const PINS = { 'techno 42': null, 'techno 43': null };
const m = await import(pathToFileURL(path.join(web, 'wasm', 'beatscript.js')).href);
m.initSync({ module: fs.readFileSync(path.join(web, 'wasm', 'beatscript_bg.wasm')) });
const gen = (genre, seed) => JSON.parse(m.beatscript_generate(JSON.stringify({ genre, seed, length: 'standard' })));
const EXPECT = {
  'techno 42': 'z4EBG9jBATRr9XWGL7beKXF7dm5Je1rPzaSymt6pvg12kd1nqHf',
  'techno 43': 'z4EBG9j8eoPMaSiPo6vfKXkWLj84e6GjWvZUDr5DHvsSXKkt2kh',
};
for (const k of Object.keys(PINS)) { const [g, s] = k.split(' '); PINS[k] = gen(g, s).cid; }

let failed = 0;
const check = (name, ok, detail = '') => { console.log(`${ok ? '  ok  ' : ' FAIL '} ${name}${!ok && detail ? `\n         ${detail}` : ''}`); if (!ok) failed++; };

check('the module prints the pinned CID for techno seed 42', PINS['techno 42'] === EXPECT['techno 42'], `${PINS['techno 42']} != ${EXPECT['techno 42']}`);
check('and for techno seed 43', PINS['techno 43'] === EXPECT['techno 43'], `${PINS['techno 43']} != ${EXPECT['techno 43']}`);

let serverProc = null;
let base = arg('server', null);
if (!base) {
  const port = await new Promise((res) => { const s = net.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => res(p)); }); });
  serverProc = spawn('node', [path.join(here, 'mock-server.mjs'), '--port', String(port)], { stdio: ['ignore', 'ignore', 'inherit'] });
  base = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 50; i++) { try { if ((await fetch(`${base}/healthz`)).ok) break; } catch { /* starting */ } await sleep(100); }
}
process.on('exit', () => { try { serverProc && serverProc.kill('SIGKILL'); } catch { /* gone */ } });

const page = await launch({ width: 1280, height: 800, args: ['--disable-audio-output', '--autoplay-policy=no-user-gesture-required'] });
const shot = (name) => page.shot(path.join(SHOTS, `gen-${name}.png`));
const q = (s) => page.eval(`(()=>{const e=document.querySelector(${JSON.stringify(s)}); return e ? e.textContent : null})()`);
const set = (sel, v) => page.eval(`(()=>{const e=document.querySelector(${JSON.stringify(sel)}); e.value=${JSON.stringify(v)}; e.dispatchEvent(new Event('change',{bubbles:true})); return e.value})()`);
const report = () => page.eval('(()=>{const r=__app.report; return r ? {ok: r.ok, errors: r.errors, cid: r.cid} : null})()');

try {
  await page.goto(`${base}/`);
  await page.eval('(async()=>{ for (let i=0;i<300 && !(window.__btw);i++) await new Promise(r=>setTimeout(r,50)); await window.__btw.ready; return true })()', true);
  await sleep(300);
  check('the page is up and nobody is signed in', (await page.eval('__app.wasmOk')) === true && (await page.eval('!__app.user')) === true);
  check('the toolbar has a generate button, beside compose', (await page.eval('!!document.getElementById("btn-generate") && document.getElementById("btn-generate").nextElementSibling.id === "btn-compose"')) === true);
  await shot('toolbar');

  // one click
  await page.click('#btn-generate');
  await sleep(200);
  check('one click opens the Generate panel', (await page.eval('!document.getElementById("drawer").hidden')) === true && (await q('#drawer-title')) === 'Generate');
  check('with genre, seed, -1/+1, random, length, structure, Generate and insert as reference, none of them disabled', (await page.eval('["gen-genre","gen-seed","gen-prev","gen-next","gen-random","gen-length","gen-structure","gen-go","gen-section","gen-part","gen-insert"].every(id => { const e = document.getElementById(id); return e && !e.disabled && !e.closest("fieldset:disabled"); })')) === true);
  check('it asks nobody to sign in', !/sign in/i.test(await q('#drawer-body')));
  check('the focus is in the panel (the genre)', (await page.eval('document.activeElement && document.activeElement.id')) === 'gen-genre');

  await set('#gen-genre', 'techno');
  await set('#gen-length', 'standard');
  await set('#gen-seed', '42');
  const structure = await q('#gen-structure');
  check('the structure is shown before generating', /^Structure: intro \d+ · .+ \(\d+ bars\)$/.test(structure || ''), String(structure));
  await shot('panel');

  await page.click('#gen-go');
  await page.waitFor('__app.report && __app.report.cid && /generator: seeded@2/.test(__app.editor.text)', 15000);
  await sleep(300);
  let r = await report();
  check('Generate writes the program into the editor and it checks clean', r && r.ok === true && r.errors === 0, JSON.stringify(r));
  check('with the pinned CID for techno seed 42', r && r.cid === EXPECT['techno 42'], `${r && r.cid}`);
  check('the panel says the CID', ((await page.eval('document.getElementById("gen-cid").title')) || '') === EXPECT['techno 42']);
  await page.waitFor('__app.isPlaying()', 15000).catch(() => {});
  check('and it plays', (await page.eval('__app.isPlaying()')) === true, String(await page.eval('__app.driver && __app.driver.kind')));
  await shot('generated');

  // the knob: +1 while it plays
  await page.click('#gen-next');
  await page.waitFor(`__app.report && __app.report.cid === ${JSON.stringify(EXPECT['techno 43'])}`, 15000).catch(() => {});
  r = await report();
  check('+1 while playing writes seed 43, which checks clean with its pinned CID', (await page.eval('document.getElementById("gen-seed").value')) === '43' && r && r.ok && r.cid === EXPECT['techno 43'], JSON.stringify(r));
  await sleep(500);
  check('and it is still playing', (await page.eval('__app.isPlaying()')) === true);
  await page.click('#gen-prev');
  await page.waitFor(`__app.report && __app.report.cid === ${JSON.stringify(EXPECT['techno 42'])}`, 15000).catch(() => {});
  check('-1 goes back to seed 42', ((await report()) || {}).cid === EXPECT['techno 42']);

  // insert as reference, at the caret
  await set('#gen-section', 'drop');
  const parts = await page.eval('[...document.getElementById("gen-part").options].map(o=>o.value)');
  check('the drop offers its parts', parts.includes('bass') && parts.includes('kick'), parts.join(','));
  await set('#gen-part', 'bass');
  // a let at the end of the program, the caret after its `=`
  await page.eval('(()=>{const ed=__app.editor; const n=ed.text.length; ed.applyEdit(n, n, "let bed = "); const e=ed.text.length; ed.ta.setSelectionRange(e, e);})()');
  await page.click('#gen-insert');
  await sleep(400);
  const tail = await page.eval('__app.editor.text.trimEnd().split("\\n").pop()');
  check('insert as reference puts the gen@2 call at the caret', tail === 'let bed = gen@2 techno seed=42 section=drop part=bass', tail);
  const ins = await page.eval('(()=>{const r=__app.beatscript.check(__app.editor.text); return {ok: r.ok, refs: (r.refs||[]).map(x=>x.call)}})()');
  check('and the program with it checks clean, the call one of its references', ins.ok === true && ins.refs.includes('gen@2 techno seed=42 section=drop part=bass'), JSON.stringify(ins));
  await shot('reference');

  // the keyboard: Esc closes, G opens
  await page.eval('document.activeElement && document.activeElement.blur && document.activeElement.blur()');
  await page.key('Escape');
  await sleep(150);
  check('Esc closes the panel', (await page.eval('document.getElementById("drawer").hidden')) === true);
  await page.eval('document.body.focus()');
  await page.key('g');
  await sleep(200);
  check('G opens it again', (await page.eval('!document.getElementById("drawer").hidden')) === true && (await q('#drawer-title')) === 'Generate');
  check('and it remembers the seed', (await page.eval('document.getElementById("gen-seed").value')) === '42');

  const errors = page.errors();
  check('no console errors', errors.length === 0, errors.map((e) => e.text).join(' | '));
} catch (e) {
  check('the run finished', false, e.stack || String(e));
} finally {
  await page.close();
}
console.log(failed ? `\n${failed} failed` : '\nall passed');
process.exit(failed ? 1 : 0);
