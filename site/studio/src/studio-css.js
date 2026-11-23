// The studio's styles: one constructed stylesheet for its shadow root (adoptedStyleSheets), so a page whose style-src is
// 'self' (a Claude artifact) shows them. No <style> element and no style attribute anywhere.
//
// The editor's part is site/web/css/beatscript.css's editor layers (.ed and what editor.js builds inside it), unchanged but for
// two things: the variables live on :host, not :root, and the squiggles are gradients, not data: SVGs (an artifact's
// default-src 'self' refuses a data: image, and every error would be a CSP violation).

export const CSS = `
:host { display: block; max-width: 100%; font: 14px/1.35 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; color-scheme: dark;
  --bg: #1a1a2e; --panel: #16213e; --panel-2: #12192f; --line: #0f3460; --line-2: #243055; --ink: #eceef6; --dim: #8d95b0;
  --faint: #5d6585; --accent: #e94560; --link: #88aaff; --btn: #0f3460; --btn-hover: #1a4a7a; --stage: #0b0d12; --gutter: #0e1118;
  --code: #d8dce6; --amber: #ffc850; --amber-2: #f0a830; --green: #4cd06a; --teal: #56b6c2; --red: #e05050; --warn: #e5c07b;
  --mono: ui-monospace, "SF Mono", SFMono-Regular, Menlo, Monaco, Consolas, "DejaVu Sans Mono", "Liberation Mono", monospace;
  --fs: 14px; --lh: 21px; --padx: 14px; --pady: 10px; --gut-w: 52px; --ed-h: 320px; }
.studio[data-theme="light"] { color-scheme: light; --bg: #f6f7fb; --panel: #eef0f7; --panel-2: #e6e9f3; --line: #c9d0e6; --line-2: #d6d9e6;
  --ink: #1a1a2e; --dim: #5d6585; --faint: #8a8fa8; --accent: #c2304a; --link: #2b4fb3; --btn: #dfe6f7; --btn-hover: #c9d5f2;
  --stage: #ffffff; --gutter: #f1f3f9; --code: #2a2d3a; --amber-2: #b87a10; --red: #ca1243; --warn: #986801; }
@media (prefers-color-scheme: light) {
  .studio[data-theme="auto"] { color-scheme: light; --bg: #f6f7fb; --panel: #eef0f7; --panel-2: #e6e9f3; --line: #c9d0e6; --line-2: #d6d9e6;
    --ink: #1a1a2e; --dim: #5d6585; --faint: #8a8fa8; --accent: #c2304a; --link: #2b4fb3; --btn: #dfe6f7; --btn-hover: #c9d5f2;
    --stage: #ffffff; --gutter: #f1f3f9; --code: #2a2d3a; --amber-2: #b87a10; --red: #ca1243; --warn: #986801; }
}
* { box-sizing: border-box; }
[hidden] { display: none !important; }
button, select, input, textarea { font: inherit; color: inherit; }
.studio { background: var(--bg); color: var(--ink); border: 1px solid var(--line-2); border-radius: 4px; overflow: hidden; }
.studio.compact { --ed-h: 200px; --fs: 13px; --lh: 20px; }

/* ---- the toolbar ---- */
.bar { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; padding: 6px 8px; background: var(--panel); border-bottom: 1px solid var(--line); }
.tb { display: inline-flex; align-items: center; gap: 5px; background: var(--btn); border: 1px solid transparent; border-radius: 4px; padding: 5px 9px; line-height: 1; white-space: nowrap; cursor: pointer; font-size: 13px; }
.tb:hover:not(:disabled) { background: var(--btn-hover); }
.tb:disabled { opacity: .45; cursor: default; }
.tb[aria-pressed="true"], .tb[aria-expanded="true"] { background: var(--btn-hover); border-color: var(--link); }
.tb:focus-visible { outline: 2px solid var(--link); outline-offset: 1px; }
.tb.play { background: var(--accent); color: #fff; min-width: 64px; justify-content: center; }
.tb.play.on { filter: brightness(1.15); }
.bar select.set { background: var(--btn); border: 1px solid var(--line); border-radius: 4px; padding: 4px 6px; font-size: 13px; max-width: 220px; }
.name { color: var(--dim); font-size: 13px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; flex: 1 1 80px; }

/* ---- the panel under the toolbar: share, open, generate, compose, library, help ---- */
.panel { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; padding: 8px; background: var(--panel-2); border-bottom: 1px solid var(--line-2); font-size: 13px; }
.panel input[type=text], .panel select, .panel textarea { background: var(--stage); border: 1px solid var(--line); border-radius: 4px; padding: 5px 8px; color: var(--ink); min-width: 0; }
.panel input.grow, .panel textarea.grow { flex: 1 1 200px; }
.panel textarea { min-height: 56px; resize: vertical; font-family: inherit; }
.panel pre { margin: 0; width: 100%; max-height: 320px; overflow: auto; background: var(--stage); border: 1px solid var(--line-2); border-radius: 4px; padding: 8px 10px; font: 12px/1.5 var(--mono); white-space: pre-wrap; color: var(--code); }
.panel ul { list-style: none; margin: 0; padding: 0; width: 100%; max-height: 200px; overflow: auto; }
.panel li button { background: none; border: 0; padding: 3px 0; color: var(--link); cursor: pointer; text-align: left; }
.panel .msg { color: var(--dim); width: 100%; }
.panel .msg.err { color: var(--red); }
.panel .guess { width: 100%; color: var(--dim); font-style: italic; }
.panel .guess:empty { display: none; }

/* ---- the lines under the editor ---- */
.problems { list-style: none; margin: 0; padding: 0; max-height: 132px; overflow: auto; background: var(--panel-2); border-top: 1px solid var(--line-2); }
.problems li button { display: block; width: 100%; text-align: left; background: none; border: 0; border-bottom: 1px solid var(--line-2); padding: 5px 12px; font: 12px/1.4 var(--mono); cursor: pointer; }
.problems li button:hover, .problems li button:focus-visible { background: rgba(127,127,127,.12); outline: none; }
.problems .pos { color: var(--link); margin-right: 8px; }
.problems .sev-error { color: var(--red); } .problems .sev-warning, .problems .sev-note { color: var(--warn); }
.status { display: flex; gap: 12px; align-items: center; padding: 5px 12px; font-size: 12.5px; color: var(--dim); background: var(--panel); border-top: 1px solid var(--line); font-variant-numeric: tabular-nums; }
.status .ok { color: var(--green); } .status .bad { color: var(--red); }
.status .spacer { flex: 1; }
.status a { color: var(--link); }
.status .tb { padding: 2px 7px; font-size: 12px; }
.notes { margin: 0; padding: 4px 12px 6px; font-size: 12px; color: var(--faint); background: var(--panel); }
.notes:empty { display: none; }
audio { display: block; width: 100%; }
a.dl { color: var(--link); font-size: 12.5px; }

/* ---- the editor (site/web/css/beatscript.css's editor layers) ---- */
.ed { position: relative; height: var(--ed-h); min-height: 80px; background: var(--stage); overflow: hidden; }
.ed-scroll { position: absolute; inset: 0; overflow-x: hidden; overflow-y: auto; scrollbar-gutter: stable; overscroll-behavior: contain; }
.ed-body { display: flex; width: 100%; min-height: 100%; position: relative; }
.ed-gutter { position: sticky; left: 0; z-index: 4; flex: none; width: var(--gut-w); padding-top: var(--pady); background: var(--gutter); border-right: 1px solid var(--line-2); user-select: none; -webkit-user-select: none; }
.gl { height: var(--lh); display: flex; align-items: flex-start; font: 12px/var(--lh) var(--mono); color: var(--faint); position: relative; }
.gl b { flex: 1; text-align: right; font-weight: 400; padding-right: 8px; }
.gl.sec-hdr b { color: var(--amber-2); }
.gi { width: 14px; height: 14px; margin-left: 4px; margin-top: calc((var(--lh) - 14px) / 2); flex: none; }
.gi[data-sev="error"] { background: radial-gradient(circle, var(--red) 0 4px, transparent 4.5px); }
.gi[data-sev="warn"] { background: radial-gradient(circle, var(--warn) 0 3.5px, transparent 4px); }
.gd { width: 6px; flex: none; height: 12px; margin-right: 2px; }
.ed-code { position: relative; flex: 1 1 auto; min-width: 0; font: var(--fs) / var(--lh) var(--mono); --hang: 0ch; }
.ly { position: absolute; inset: 0; margin: 0; pointer-events: none; }
.ly-hl, .ed-ta { font: var(--fs) / var(--lh) var(--mono); padding: var(--pady) var(--padx); white-space: pre-wrap; tab-size: 2; -moz-tab-size: 2; letter-spacing: 0; word-spacing: 0; font-variant-ligatures: none; font-kerning: none; text-indent: 0; border: 0; margin: 0; overflow-wrap: anywhere; word-break: normal; line-break: auto; hyphens: none; box-sizing: border-box; }
.ly-hl { color: var(--code); overflow: hidden; }
.ed-ta { position: absolute; inset: 0; width: 100%; height: 100%; resize: none; overflow: hidden; background: transparent; color: transparent; -webkit-text-fill-color: transparent; caret-color: var(--ink); outline: none; z-index: 6; }
.ed-ta::selection { background: rgba(97,175,239,.38); -webkit-text-fill-color: transparent; }
.ed-ta[readonly] { caret-color: transparent; }
.ed-measure { position: absolute; left: 0; top: 0; visibility: hidden; white-space: pre; pointer-events: none; font: var(--fs) / var(--lh) var(--mono); font-variant-ligatures: none; font-kerning: none; letter-spacing: 0; }
.ly-under { z-index: 1; } .ly-hl { z-index: 2; } .ly-over { z-index: 3; } .ly-bars { z-index: 4; } .ly-diag { z-index: 5; }
.sq { position: absolute; height: 3px; background-repeat: repeat-x; background-size: 6px 3px; }
.sq-error { background-image: linear-gradient(135deg, transparent 40%, var(--red) 40% 60%, transparent 60%), linear-gradient(45deg, transparent 40%, var(--red) 40% 60%, transparent 60%); background-size: 3px 3px, 3px 3px; background-position: 0 0, 3px 0; }
.sq-warn { background-image: linear-gradient(135deg, transparent 40%, var(--warn) 40% 60%, transparent 60%), linear-gradient(45deg, transparent 40%, var(--warn) 40% 60%, transparent 60%); background-size: 3px 3px, 3px 3px; background-position: 0 0, 3px 0; }
.ed-tip { position: absolute; z-index: 20; max-width: min(520px, 90%); background: var(--panel); border: 1px solid var(--line-2); border-radius: 4px; padding: 7px 10px; font-size: 12.5px; box-shadow: 0 6px 22px rgba(0,0,0,.45); pointer-events: none; }
.ed-tip strong { color: var(--red); font-weight: 600; margin-right: 4px; }
.ed-tip em { display: block; font-style: normal; color: var(--amber-2); margin-top: 3px; font-family: var(--mono); font-size: 12px; }
.ed-edges { position: absolute; inset: 0; pointer-events: none; z-index: 10; }

/* the editor's colours (beatscript.css), and their light-theme counterparts (the player's) */
.c-comment { color: #6c786c; } .c-keyword { color: #c678dd; } .c-role { color: #61afef; } .c-action { color: #e5c07b; }
.c-deck { color: #e06c75; } .c-number { color: #d19a66; } .c-step, .c-note { color: #98c379; } .c-rest { color: #7a7a86; }
.c-generator, .c-def { color: #56b6c2; } .c-param { color: #d19a66; } .c-op, .c-punct { color: #abb2bf; } .c-string { color: #98c379; }
.c-error { color: #e05050; } .c-ident, .c-section { color: #e5c07b; } .c-word { color: var(--code); }
.studio[data-theme="light"] .c-comment { color: #8a8f98; } .studio[data-theme="light"] .c-keyword { color: #a626a4; } .studio[data-theme="light"] .c-role { color: #4078f2; }
.studio[data-theme="light"] .c-action, .studio[data-theme="light"] .c-ident, .studio[data-theme="light"] .c-section { color: #986801; }
.studio[data-theme="light"] .c-deck { color: #e45649; } .studio[data-theme="light"] .c-number, .studio[data-theme="light"] .c-param { color: #b76b01; }
.studio[data-theme="light"] .c-step, .studio[data-theme="light"] .c-note, .studio[data-theme="light"] .c-string { color: #50a14f; } .studio[data-theme="light"] .c-rest { color: #9a9aa6; }
.studio[data-theme="light"] .c-generator, .studio[data-theme="light"] .c-def { color: #0184bc; } .studio[data-theme="light"] .c-op, .studio[data-theme="light"] .c-punct { color: #696c77; }
`;

let sheet = null;

/** The stylesheet, made once per page; null where constructed stylesheets are not supported. */
export function studioSheet() {
  if (!sheet && typeof CSSStyleSheet === 'function') {
    try { sheet = new CSSStyleSheet(); sheet.replaceSync(CSS); } catch { sheet = null; }
  }
  return sheet;
}
