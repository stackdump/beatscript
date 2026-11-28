// stage.js: the stage view (?view=stage), the DOM half of stage-model.js. Installed only in the stage view; record mode
// (?record=1) never loads anything from here, so the demo-video renderer's page is the page it always was.
//
// What it adds to record mode's page: a full-screen toggle (a button in the header and the F key; Esc leaves; a hint says
// so for a few seconds; where the element cannot go full screen, as on an iPhone, the stage fills the window instead), the
// set's tracklist line (`2/5  now X  ·  next Y`) when the program is a `set { }`, the stage's type scale (whole pixels from
// the window's width), the stage's lamp palette for a light theme, and a read-only editor. Nothing on it edits.

import { STAGE, stageType, lampPalette, setTracks, setPosition, tracklistLine, fullscreenOf, stageKey, exitHint } from './stage-model.js';

const ios = () => /iPad|iPhone|iPod/.test(navigator.userAgent || '') || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);

export function installStage(app) {
  const doc = document;
  const root = doc.documentElement;
  const fs = fullscreenOf(doc);
  root.classList.add('stage');

  // read-only: the text can be selected and copied, never changed
  const ta = app.editor && app.editor.ta;
  if (ta) {
    ta.readOnly = true;
    ta.setAttribute('aria-readonly', 'true');
    ta.setAttribute('aria-label', 'The program, read-only');
    ta.style.caretColor = 'transparent';
  }

  // the header: the tracklist line and the full-screen button
  const hdr = app.el.hdr;
  const tl = doc.createElement('span');
  tl.className = 'stage-tl';
  tl.id = 'stage-tl';
  tl.hidden = true;
  tl.setAttribute('aria-live', 'polite');
  const btn = doc.createElement('button');
  btn.type = 'button';
  btn.className = 'tb tb-sm stage-fs';
  btn.id = 'btn-stage-fs';
  btn.setAttribute('aria-pressed', 'false');
  btn.setAttribute('aria-keyshortcuts', 'f');
  btn.innerHTML = '<span class="gl-ic" aria-hidden="true">⛶</span><span class="lbl">full screen</span> <kbd>F</kbd>';
  btn.title = 'Full screen (F); Esc leaves';
  const spacer = hdr.querySelector('.spacer');
  hdr.insertBefore(tl, spacer || null);
  hdr.insertBefore(btn, app.el['btn-follow'] || null);
  const hint = doc.createElement('div');
  hint.className = 'stage-hint';
  hint.id = 'stage-hint';
  hint.hidden = true;
  hint.setAttribute('role', 'status');
  doc.body.append(hint);

  // the type scale: whole pixels from the width, so the editor's two layers wrap alike (editor.measureNow reads them)
  const applyType = () => {
    const t = stageType(window.innerWidth);
    root.style.setProperty('--fs', `${t.fs}px`);
    root.style.setProperty('--lh', `${t.lh}px`);
    root.style.setProperty('--gut-w', `${t.gut}px`);
    if (app.editor) app.editor.measureNow();
  };
  applyType();
  window.addEventListener('resize', applyType);

  // the lamps over the stage's own ground (a light theme lights amber out of white, not out of the editor's near-black)
  const light = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: light)') : null;
  const applyTheme = () => { if (app.painter) app.painter.palette = lampPalette(light && light.matches ? 'light' : 'dark'); };
  applyTheme();
  if (light && light.addEventListener) light.addEventListener('change', applyTheme);

  // full screen
  let hintTimer = 0;
  const showHint = (text) => {
    hint.textContent = text;
    hint.hidden = false;
    clearTimeout(hintTimer);
    hintTimer = setTimeout(() => { hint.hidden = true; }, STAGE.hintMs);
  };
  const isOn = () => (fs.native ? fs.active() : root.classList.contains('stage-max'));
  const reflect = () => {
    const on = isOn();
    btn.setAttribute('aria-pressed', String(on));
    btn.querySelector('.lbl').textContent = on ? 'leave full screen' : 'full screen';
    root.classList.toggle('stage-full', on);
    if (app.editor) app.editor.measureNow();
  };
  const enter = () => {
    if (fs.native) {
      return fs.request().then(() => showHint(exitHint(true, false))).catch(() => { root.classList.add('stage-max'); reflect(); showHint(exitHint(false, ios())); });
    }
    root.classList.add('stage-max');
    reflect();
    showHint(exitHint(false, ios()));
    return Promise.resolve();
  };
  const leave = () => {
    root.classList.remove('stage-max');
    reflect();
    hint.hidden = true;
    return fs.native && fs.active() ? fs.exit().catch(() => {}) : Promise.resolve();
  };
  const toggle = () => (isOn() ? leave() : enter());
  doc.addEventListener(fs.event, reflect);
  btn.addEventListener('click', () => { toggle(); });

  // keys, ahead of the page's own (window capture runs before document capture): F toggles, Esc leaves full screen and
  // never stops the music (it is the key everyone presses to leave full screen), G opens no editing panel
  window.addEventListener('keydown', (e) => {
    const k = stageKey(e);
    if (!k) return;
    if (k === 'exit' && !isOn()) { e.stopImmediatePropagation(); return; }
    e.preventDefault();
    e.stopImmediatePropagation();
    if (k === 'fullscreen') toggle();
    else if (k === 'exit') leave();
  }, { capture: true });

  // the tracklist line, from the running program's text and check and the frame's section
  let tracks = [];
  let tracksOf = null;
  let shown = '';
  const refresh = (f) => {
    const text = (app.driver && app.driver.evalText) || app.editor.text;
    const report = app.evalReport || app.report;
    if (tracksOf !== text) { tracks = setTracks(text, report); tracksOf = text; }
    const st = (f && f.status) || {};
    const pos = setPosition(tracks, { section: st.section, songBar: st.song_bar, sections: report && report.sections });
    const line = tracks.length ? (pos ? tracklistLine(pos) : `${tracks.length} tracks`) : '';
    if (line !== shown) {
      shown = line;
      tl.textContent = line;
      tl.hidden = !line;
    }
  };
  app.listeners.add((type, data) => {
    if (type === 'frame') refresh(data);
    else if (type === 'eval' || type === 'program' || type === 'stop') { tracksOf = null; refresh(null); }
  });

  return { toggle, enter, leave, isOn, refresh };
}
