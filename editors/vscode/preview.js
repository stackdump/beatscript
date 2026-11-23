// The preview: the document played by the published player, <beatscript-track>, in a webview beside the editor.
//
// The page is previewHtml's, a pure function, so test/preview.test.js checks it without VS Code. Its CSP allows scripts
// from this render's nonce (the import map and nothing else inline) and from the pinned package's directory on
// cdn.jsdelivr.net only, plus 'wasm-unsafe-eval' (without it no WebAssembly compiles) and blob: media (the render
// mode's <audio>); no connections, no styles (the player's are a constructed stylesheet, which no policy governs), no
// images. The engine is in the package (the module inlined as base64), so nothing else is needed. The player is loaded
// at one exact version, with each module's Subresource Integrity hash in an import map, exactly as the web embed gives
// them (crates/beatscript-serve/src/mcp/artifact.rs PlayerCdn::tags), and the document's text is the element's text attribute,
// escaped as artifact.rs's esc does: the attribute gives the player the text byte for byte, where a child script's
// escapes would change it (and so its CID). Sound starts only from the player's Play button. The page is written
// again when the document's text or the theme has changed (once the typing rests, or at a save), so what plays is
// what the editor holds, and a save that changes nothing does not stop the playing track.
'use strict';

const crypto = require('crypto');

// The package and its exact version. integrity maps a file of the package to its SRI hash, exactly as
// crates/beatscript-serve/src/mcp/player-cdn.lock.json gives them (test/preview.test.js holds it to that file). Empty, the page
// carries no integrity at all rather than wrong hashes, which would stop the player.
const PLAYER = {
  package: '@stackdump/beatscript-player',
  version: '0.3.0',
  entry: 'beatscript-track.js',
  integrity: {
    'beatscript-render.js': 'sha384-DFT911SEgTa1XDEgYZy2U5KMFwbQNSLfCKxCeutt+QSroocDeqaRLf15v1NlJ0vs',
    'beatscript-track.js': 'sha384-gMZzBWcnEYg2/4yLGawBpdH36j2IUnfbPCTBr46YbM6zOnYN6s7RQCxp7zi8iQjJ',
    'beatscript-track.umd.js': 'sha384-BFmeZkF5eDDEJzIfDCl8jbuvI8dnnNeijcPQFsD800deWrcgwJbiLoI5GZrlzB96',
    'embed-config.js': 'sha384-MSWPNqsPS7ku5tU/s5121wdaF7Zz+nsU4e9Cs04v9g9r/Ou8PpQtUMw/Ugk8O/gq',
    'js/embed-model.js': 'sha384-2bMFrpO2YPKzlfzdHUR4OD3DKxm49b12NifeekjL4t2fCa5so0si3re5fb+ptBIg',
    'js/engine-calls.js': 'sha384-aW3JQJU9hlOuHARYchyAN2KHmi4uGdTyCzhPt3LTL+5ohQ+muWGtKl+83zqvvvjM',
    'js/engine-composer.js': 'sha384-baeA8bwSFJjWBZmOsmk63naQ4GfX34zbGyQedLuQ6evoKyVPGZ5B4ZwbD8HPTpdi',
    'js/engine-polyfill.js': 'sha384-ZDXMDcLeHV5uWZI/7YPKLBt8f86R1Hjb5ksiScaDaf/bmETYAhRC29QVX64EcOm2',
    'js/engine-worklet.js': 'sha384-3O6g7AHJbnYYiSe/OCz+ev29Wk83lHKXVU+6Ns0Lste8ttsZsXAhd1/MG6RAaZ7U',
    'js/engine.js': 'sha384-YvQpqtS1OpdWDPjraaMIf5rQ1AqmOd+0HXZuSgNzep9AwiA+gi90eQWvZ2p+8Wds',
    'js/highlight.js': 'sha384-DntO3RkbEUrpuy4ps1tV1uRm3Jixxj7A5SJHTVOHfbDGr4XKU5NGPYN5HvkH1sSG',
    'js/mixer-model.js': 'sha384-3VluHJuwCDuhaotOqq616HSWXpyfHrtrl20e/aeatY4YXKX70sRonCntsCbhvQpk',
    'js/render-client.js': 'sha384-3aD1M2azaXHpzjTg+cgpLon3QyYLPPECT1fQGLr/u6Q1Bx5/u8jaLOuVfvG6A8i9',
    'js/render-model.js': 'sha384-hNCprqGhaqYyPyzo6HybZ+Vbo0yEMgQcMmd4xRV3grMJIR/0+NAMtvMzibuPGlVk',
    'js/render-worker.js': 'sha384-Gi5ura78nXA0dLKcJE2hi1ng1VnoD0BSmEkP89uiCzTb56ai9gHk/Jeu0RsiKKaW',
    'js/render.js': 'sha384-m/1bRW9Rk172CuaVKcP1zx0vuIkF0wdbhXa2/0Ezw7WpnRBSgqkQnblcUjxxrYKw',
    'js/station-model.js': 'sha384-nwxAtNgZGVULGN4Ng6uOkXUbGF+g/KERZNhPz3a3BmYiMUx8GWJ2N37SQNKjG7gW',
    'js/util.js': 'sha384-l7f0LTebsCE9QaQWxS2x5oV6qYQNUc+vpW+guG20CaMNvSmx1y2uBsUd/nY57fRT',
    'js/version.js': 'sha384-0JtfQGiesnXXRNalVwtZvidCODVWfw5Dk2jXNqT96T3330guC/XwiJcju2oYoUhR',
    'js/wasm-mock.js': 'sha384-r4zf7hW8FYRAuIOkOgrDGVmwx2C3FZgdey44EqCE97T98bbewwMIFw8HCWa6SGKP',
    'js/wasm.js': 'sha384-jlsQ8Dj+qDMfm46HaSIXnh6LU86oRXPBQSOjvpvQc6BhhanJiTwfppwWLAYpZm+P',
    'render-model.js': 'sha384-39uzNQWa1ST290s+IWHBpwXM0PzPHzpynEm7O896zDanAc7awDzBAY2gnhqu6sNz',
    'wasm/beatscript.js': 'sha384-cp/1klCH/CCGzksh4mSZI54zGvbmzdRR6Cqm+BznwolsLtf5asQ9UCdWVxVrtJf0',
    'wasm/beatscript_bg.wasm.b64.js': 'sha384-mvZ3Qdx2iMnin4YNFqTGpjh6j97r1rybvMdVKtCdVA9BK4+QIxVsHnd3d/LRU6SZ',
  },
};
const CDN = 'https://cdn.jsdelivr.net/npm/';
const COMMAND = 'beatscript.preview';
const DEBOUNCE_MS = 400;

// HTML text and attribute escaping, the five characters artifact.rs's esc changes, as it changes them
const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
const attr = (s) => String(s).replace(/[&<>"']/g, (c) => ESC[c]);

/** The webview's page for a program `text`. `nonce` is this render's. */
function previewHtml({ text, nonce, player = PLAYER, theme = 'dark' }) {
  const dir = `${CDN}${player.package}@${player.version}/`;
  const url = (f) => dir + f;
  const hashes = player.integrity || {};
  // every module the entry can import: each .js but the entry itself and the UMD wrapper, as artifact.rs gives them
  const modules = Object.keys(hashes).sort().filter((f) => f.endsWith('.js') && f !== player.entry && !f.endsWith('.umd.js'));
  const csp = [
    "default-src 'none'",
    `script-src 'nonce-${nonce}' ${dir} 'wasm-unsafe-eval'`,
    'media-src blob:',
    "connect-src 'none'",
  ].join('; ');
  const map = JSON.stringify({ integrity: Object.fromEntries(modules.map((f) => [url(f), hashes[f]])) }).replace(/</g, '\\u003c');
  const sri = hashes[player.entry] ? ` integrity="${attr(hashes[player.entry])}"` : '';
  return [
    '<!doctype html>',
    '<html lang="en">',
    '<head>',
    '<meta charset="utf-8">',
    `<meta http-equiv="Content-Security-Policy" content="${attr(csp)}">`,
    modules.length ? `<script type="importmap" nonce="${attr(nonce)}">${map}</script>` : '',
    `<script type="module" nonce="${attr(nonce)}" src="${attr(url(player.entry))}"${sri} crossorigin="anonymous"></script>`,
    '</head>',
    '<body>',
    `<beatscript-track theme="${attr(theme)}" text="${attr(text)}"></beatscript-track>`,
    '</body>',
    '</html>',
  ].filter(Boolean).join('\n');
}

/** `beatscript.preview`: open the active BeatScript document's preview beside it, one panel that follows that document. */
function registerPreview(context, vscode) {
  let panel = null;
  let doc = null;
  let shown = null; // the text and theme the page was last written with
  let timer;
  const light = () => [vscode.ColorThemeKind.Light, vscode.ColorThemeKind.HighContrastLight].includes(vscode.window.activeColorTheme && vscode.window.activeColorTheme.kind);
  const render = () => {
    clearTimeout(timer);
    if (!panel || !doc) return;
    const name = doc.uri.path.split('/').pop() || 'BeatScript';
    panel.title = `Preview ${name}`;
    const text = doc.getText();
    const theme = light() ? 'light' : 'dark';
    // writing the page reloads it, which stops the track: only when what it plays or how it looks has changed
    if (shown && shown.text === text && shown.theme === theme) return;
    shown = { text, theme };
    panel.webview.html = previewHtml({ text, theme, nonce: crypto.randomBytes(16).toString('base64') });
  };
  const open = () => {
    const ed = vscode.window.activeTextEditor;
    if (!ed || ed.document.languageId !== 'beatscript') {
      vscode.window.showInformationMessage('BeatScript: open a .beatscript file to preview it.');
      return;
    }
    doc = ed.document;
    if (panel) panel.reveal(vscode.ViewColumn.Beside, true);
    else {
      // no local file is ever loaded: an empty localResourceRoots keeps the webview from reading the workspace
      panel = vscode.window.createWebviewPanel(COMMAND, 'BeatScript preview', { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true }, { enableScripts: true, localResourceRoots: [], retainContextWhenHidden: false });
      panel.onDidDispose(() => {
        clearTimeout(timer);
        panel = null;
        doc = null;
        shown = null;
      });
    }
    render();
  };
  context.subscriptions.push(
    vscode.commands.registerCommand(COMMAND, open),
    vscode.workspace.onDidChangeTextDocument((e) => {
      if (!doc || e.document !== doc) return;
      clearTimeout(timer);
      timer = setTimeout(render, DEBOUNCE_MS);
    }),
    vscode.workspace.onDidSaveTextDocument((d) => d === doc && render()),
    vscode.window.onDidChangeActiveColorTheme(() => render()),
    { dispose: () => clearTimeout(timer) },
  );
}

module.exports = { PLAYER, previewHtml, registerPreview };
