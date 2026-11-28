#!/usr/bin/env node
// stage-browser.mjs: the stage view (?view=stage) in headless Chrome, and record mode (?record=1) unchanged by it.
//
//   node site/test/stage-browser.mjs [--base-ref <git ref>] [--example acid-line]
//
// Record mode is what the demo-video renderer drives, so it must stay byte for byte what it was: its page has no stage
// class, no stage element, an editable editor and the record API it had. With --base-ref (for instance the commit the
// stage view was branched from) the record frames are also rendered from that ref's site/web, on a second mock server,
// and the PNGs must be the same bytes. The stage view: the classes, a read-only editor, the play button and the
// full-screen button, F toggling full screen (natively, or by filling the window where the element cannot go full screen,
// as on an iPhone), Esc leaving full screen without stopping the music, G opening no panel, a set's tracklist line as the
// set plays (first track, into the mix, last track), the light theme and prefers-contrast.
// Against mock-server.mjs with the real wasm. Skips (exit 0) when the wasm is not built or Chrome is not installed
// (CHROME=/usr/bin/google-chrome on Linux). Screenshots go to site/test/shots/stage-*.png. Exit 1 if any check failed or
// the page logged a console error.

import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { CHROME, launch, sleep } from './cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const web = path.join(here, '..', 'web');
const SHOTS = path.join(here, 'shots');
fs.mkdirSync(SHOTS, { recursive: true });
const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i < 0 ? d : process.argv[i + 1]; };
const EXAMPLE = arg('example', 'acid-line');
const BASE_REF = arg('base-ref', null);

if (!fs.existsSync(path.join(web, 'wasm', 'beatscript_bg.wasm'))) { console.log('SKIP stage-browser: no built module in site/web/wasm'); process.exit(0); }
if (!fs.existsSync(CHROME)) { console.log(`SKIP stage-browser: no Chrome at ${CHROME}`); process.exit(0); }

let failed = 0;
const check = (name, ok, detail = '') => { console.log(`${ok ? '  ok  ' : ' FAIL '} ${name}${!ok && detail ? `\n         ${detail}` : ''}`); if (!ok) failed++; };
const sha = (f) => crypto.createHash('sha256').update(fs.readFileSync(f)).digest('hex');

const procs = [];
process.on('exit', () => { for (const p of procs) { try { p.kill('SIGKILL'); } catch { /* gone */ } } });
async function serve(webDir) {
  const port = await new Promise((res) => { const s = net.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => res(p)); }); });
  const p = spawn('node', [path.join(here, 'mock-server.mjs'), '--port', String(port), '--web', webDir], { stdio: ['ignore', 'ignore', 'inherit'] });
  procs.push(p);
  const base = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 80; i++) { try { if ((await fetch(`${base}/healthz`)).ok) break; } catch { /* starting */ } await sleep(100); }
  return base;
}

const base = await serve(web);
const page = await launch({ width: 1280, height: 800, args: ['--disable-audio-output', '--autoplay-policy=no-user-gesture-required'] });
const ready = () => page.eval('(async()=>{ for (let i=0;i<300 && !(window.__btw);i++) await new Promise(r=>setTimeout(r,50)); await window.__btw.ready; return true })()', true);
const fresh = async (u) => { await page.goto('about:blank'); await page.goto(u); };
// headless Chrome is light by default: every check names the scheme it means
const DARK = { name: 'prefers-color-scheme', value: 'dark' };
const media = (features) => page.cdp('Emulation.setEmulatedMedia', { features });

/** The record frames of a server at a few fixed times (manual clock): their PNG files. */
async function recordFrames(srv, tag) {
  await fresh(`${srv}/?record=1&clock=manual&example=${EXAMPLE}`);
  await ready();
  const out = [];
  for (const t of [0, 2.5, 5, 9.75]) {
    await page.eval(`window.__btw.frame(${t}), true`);
    out.push(await page.shot(path.join(SHOTS, `stage-record-${tag}-${String(t).replace('.', '_')}.png`)));
  }
  return out;
}

const SET = '#! beatscript 2\nset {\n  play gen@2 techno seed=42\n  mix 8 bars with crossfade\n  next seed=+1\n  mix 8 bars with echo_out\n  play gen@2 house seed=7\n}\nend\n';

try {
  await media([DARK]);
  // ---- record mode, unchanged -----------------------------------------------------------------------------------------
  const mine = await recordFrames(base, 'branch');
  const dom = await page.eval(`(()=>({
    cls: document.documentElement.className,
    stageEls: document.querySelectorAll('#btn-stage-fs, #stage-tl, #stage-hint, .stage-tl, .stage-fs, .stage-hint').length,
    ro: __app.editor.ta.readOnly, stage: __app.stage, record: __app.record,
    api: Object.keys(window.__beatscript).sort().join(','), same: window.__beatscript === window.__btw,
    play: getComputedStyle(document.getElementById('btn-play')).display,
    banner: getComputedStyle(document.getElementById('banner')).display,
    fs: document.documentElement.style.getPropertyValue('--fs'),
    palette: __app.painter.palette,
  }))()`);
  check('record mode: <html> is "record" and nothing else', dom.cls === 'record', dom.cls);
  check('record mode: no stage element on the page', dom.stageEls === 0, String(dom.stageEls));
  check('record mode: the editor is as it was (not made read-only), __app.stage false', dom.ro === false && dom.stage === false && dom.record === true);
  check('record mode: the record API is the one it had', dom.api === '_reject,_resolve,app,frame,info,ready,recorder,reset,setTime' && dom.same, dom.api);
  check('record mode: no play button, no banner, no stage type, the editor\'s lamps', dom.play === 'none' && dom.banner === 'none' && dom.fs === '' && dom.palette === null, JSON.stringify(dom));

  if (BASE_REF) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'beatscript-base-'));
    const tar = execFileSync('git', ['archive', BASE_REF, 'site/web', 'site/examples', 'site/test/mock-server.mjs'], { cwd: path.join(here, '..', '..'), maxBuffer: 1 << 28 });
    execFileSync('tar', ['-x', '-C', dir], { input: tar });
    fs.cpSync(path.join(web, 'wasm'), path.join(dir, 'site', 'web', 'wasm'), { recursive: true });
    const baseSrv = await serve(path.join(dir, 'site', 'web'));
    const theirs = await recordFrames(baseSrv, 'base');
    const same = mine.map((f, i) => sha(f) === sha(theirs[i]));
    check(`record mode: the frames are the same bytes as ${BASE_REF}'s (${same.filter(Boolean).length}/${same.length})`, same.every(Boolean), mine.map((f, i) => `${path.basename(f)} ${same[i] ? '=' : '!='} ${path.basename(theirs[i])}`).join('; '));
    fs.rmSync(dir, { recursive: true, force: true });
  } else console.log('  --  record frames not compared to a base (pass --base-ref <ref>)');

  // ---- the stage view -------------------------------------------------------------------------------------------------
  await fresh(`${base}/?view=stage&example=${EXAMPLE}`);
  await ready();
  await sleep(600);
  const st = await page.eval(`(()=>({
    cls: document.documentElement.className, ro: __app.editor.ta.readOnly, stage: __app.stage,
    btn: !!document.getElementById('btn-stage-fs') && getComputedStyle(document.getElementById('btn-stage-fs')).display !== 'none',
    play: getComputedStyle(document.getElementById('btn-play')).display !== 'none',
    tools: getComputedStyle(document.getElementById('top')).display, playing: __app.isPlaying(),
    fs: parseInt(document.documentElement.style.getPropertyValue('--fs'), 10),
    bg: getComputedStyle(document.body).backgroundColor,
  }))()`);
  check('stage: <html> is "record stage", __app.stage true', st.cls.split(' ').sort().join(' ') === 'record stage' && st.stage === true, st.cls);
  check('stage: the editor is read-only, the toolbar is gone', st.ro === true && st.tools === 'none', JSON.stringify(st));
  check('stage: the full-screen button and the play button show', st.btn && st.play, JSON.stringify(st));
  check('stage: it plays at once, like record mode', st.playing === true);
  check('stage: large type (1280 px wide: 21 px), on a black ground', st.fs === 21 && st.bg === 'rgb(0, 0, 0)', JSON.stringify(st));
  await page.shot(path.join(SHOTS, 'stage-view.png'));

  // nothing edits: typing in the editor changes nothing, G opens no panel
  const before = await page.eval('__app.editor.text');
  await page.click('#ed', { dx: 200, dy: 60 });
  await page.type('zzz');
  await page.key('g');
  await sleep(150);
  check('stage: typing into the program changes nothing', (await page.eval('__app.editor.text')) === before);
  check('stage: G opens no panel', (await page.eval('document.getElementById("drawer").hidden')) === true);

  // full screen: F, then Esc leaves it and the music plays on, then the button
  await page.eval('document.activeElement && document.activeElement.blur(), true');
  await page.key('f');
  await sleep(400);
  const on = await page.eval(`(()=>({ fs: !!document.fullscreenElement, max: document.documentElement.classList.contains('stage-max'), full: document.documentElement.classList.contains('stage-full'), pressed: document.getElementById('btn-stage-fs').getAttribute('aria-pressed'), hint: document.getElementById('stage-hint').hidden ? '' : document.getElementById('stage-hint').textContent }))()`);
  check('stage: F goes full screen and says how to leave', (on.fs || on.max) && on.full && on.pressed === 'true' && /F or Esc/.test(on.hint), JSON.stringify(on));
  await page.shot(path.join(SHOTS, 'stage-full.png'));
  await page.key('Escape');
  await sleep(400);
  const off = await page.eval(`(()=>({ fs: !!document.fullscreenElement, full: document.documentElement.classList.contains('stage-full'), playing: __app.isPlaying() }))()`);
  check('stage: Esc leaves full screen, and the music plays on', !off.fs && !off.full && off.playing, JSON.stringify(off));
  await page.click('#btn-stage-fs');
  await sleep(400);
  check('stage: the button goes full screen too', await page.eval('!!document.fullscreenElement || document.documentElement.classList.contains("stage-max")'));
  await page.click('#btn-stage-fs');
  await sleep(400);
  check('stage: and the button leaves it', await page.eval('!document.fullscreenElement && !document.documentElement.classList.contains("stage-full")'));

  // the tracklist line of a set, on the manual clock: the first track, into the mix, the last track
  await fresh(`${base}/?view=stage&clock=manual&example=${EXAMPLE}`);
  await ready();
  check('stage: a program that is not a set has no tracklist line', await page.eval('document.getElementById("stage-tl").hidden'));
  await page.eval(`(async()=>{ __app.setProgram(${JSON.stringify(SET)}, { name: 'set', quiet: true }); await window.__btw.reset(); return true })()`, true);
  const secs = await page.eval('__app.evalReport.sections.map(s=>[s.name,s.start,s.bars])');
  const bpm = await page.eval('window.__btw.info().bpm');
  const at = (bar) => (bar * 4 * 60) / bpm + 0.01;
  const tl = async (t) => { await page.eval(`window.__btw.frame(${t}), true`); return page.eval('document.getElementById("stage-tl").hidden ? "" : document.getElementById("stage-tl").textContent'); };
  const mix12 = secs.find((s) => /^mix 1→2/.test(s[0]));
  const s3 = secs.find((s) => /^3\. /.test(s[0]));
  const first = await tl(at(1));
  check('stage: a set shows its first track and the next', /^1\/3  now gen@2 techno seed=42  ·  next gen@2 techno seed=43$/.test(first), first);
  await page.shot(path.join(SHOTS, 'stage-set.png'));
  const mixing = mix12 ? await tl(at(mix12[1] + 1)) : '';
  check('stage: in a mix it says which two', /^mix 1→2  gen@2 techno seed=42 → gen@2 techno seed=43$/.test(mixing), `${mixing} (sections ${JSON.stringify(secs)})`);
  const last = s3 ? await tl(at(s3[1] + 1)) : '';
  check('stage: on the last track it says so', /^3\/3  now gen@2 house seed=7  ·  last$/.test(last), last);

  // the light theme and prefers-contrast
  await media([{ name: 'prefers-color-scheme', value: 'light' }]);
  await fresh(`${base}/?view=stage&clock=manual&example=${EXAMPLE}`);
  await ready();
  await page.eval('window.__btw.frame(2.5), true');
  const lt = await page.eval(`(()=>({ bg: getComputedStyle(document.body).backgroundColor, ink: getComputedStyle(document.documentElement).getPropertyValue('--code').trim(), palette: __app.painter.palette && __app.painter.palette.stage }))()`);
  check('stage: the light theme is a white ground with dark ink, and the lamps light out of white', lt.bg === 'rgb(255, 255, 255)' && lt.ink === '#101218' && JSON.stringify(lt.palette) === '[255,255,255]', JSON.stringify(lt));
  await page.shot(path.join(SHOTS, 'stage-light.png'));
  await media([DARK, { name: 'prefers-contrast', value: 'more' }]);
  await fresh(`${base}/?view=stage&clock=manual&example=${EXAMPLE}`);
  await ready();
  const hc = await page.eval(`getComputedStyle(document.documentElement).getPropertyValue('--code').trim()`);
  check('stage: prefers-contrast: more is white on black', hc === '#fff', hc);
  await media([DARK]);

  // an iPhone: no element full screen, so F fills the window and says how to lose the browser's bars
  await page.cdp('Page.addScriptToEvaluateOnNewDocument', { source: "delete Element.prototype.requestFullscreen; delete Element.prototype.webkitRequestFullscreen; Object.defineProperty(Document.prototype, 'fullscreenEnabled', { get: () => false }); Object.defineProperty(navigator, 'userAgent', { get: () => 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148' });" });
  await page.resize(390, 844);
  await fresh(`${base}/?view=stage&clock=manual&example=${EXAMPLE}`);
  await ready();
  await page.key('f');
  await sleep(200);
  const ph = await page.eval(`(()=>({ max: document.documentElement.classList.contains('stage-max'), fs: !!document.fullscreenElement, hint: document.getElementById('stage-hint').textContent, pos: getComputedStyle(document.getElementById('app')).position }))()`);
  check('stage on an iPhone: F fills the window (no element full screen) and says Add to Home Screen', ph.max && !ph.fs && ph.pos === 'fixed' && /Add to Home Screen/.test(ph.hint), JSON.stringify(ph));
  await page.shot(path.join(SHOTS, 'stage-iphone.png'));
  await page.key('f');
  await sleep(100);
  check('stage on an iPhone: F again gives the window back', await page.eval('!document.documentElement.classList.contains("stage-max")'));

  const errs = page.errors().filter((e) => !/favicon|Failed to load resource/.test(e.text));
  check('no console errors', errs.length === 0, errs.map((e) => e.text).join(' | '));
} catch (e) {
  check(`the run did not throw: ${e.message}`, false, e.stack);
} finally {
  await page.close();
}
console.log(failed ? `${failed} FAILED` : 'all ok');
process.exit(failed ? 1 : 0);
