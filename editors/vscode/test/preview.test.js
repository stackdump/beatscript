// The preview's page (its CSP, the player's pin and SRI, the text in its attribute) and the command it registers, on a fake
// vscode: no VS Code is needed. `node test/preview.test.js`
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { PLAYER, previewHtml, registerPreview } = require('../preview');

const TEXT = '#! name: two bars\ntempo 120\ndeck A: kick x ~ x ~ | snare ~ x ~ x\n';
const BASE = `https://cdn.jsdelivr.net/npm/@stackdump/beatscript-player@${PLAYER.version}/`;
const page = (o = {}) => previewHtml({ text: TEXT, nonce: 'N0nce+/=', ...o });
const tags = (html, name) => [...html.matchAll(new RegExp(`<${name}\\b[^>]*>`, 'g'))].map((m) => m[0]);
// an attribute's value as a browser reads it: the five entities artifact.rs's esc writes, &amp; last
const unescape = (s) => s.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, '&');
const textOf = (html) => unescape(html.match(/<beatscript-track theme="[^"]*" text="([^"]*)"><\/beatscript-track>/)[1]);

// ---- the CSP: scripts from the nonce and the pinned package's directory, WebAssembly, blob: media, nothing else ------

{
  const [meta] = tags(page(), 'meta').filter((t) => t.includes('Content-Security-Policy'));
  const csp = unescape(meta.match(/content="([^"]*)"/)[1]);
  const dirs = Object.fromEntries(csp.split(';').map((d) => d.trim()).filter(Boolean).map((d) => [d.split(' ')[0], d.split(' ').slice(1).join(' ')]));
  assert.deepEqual(dirs, {
    'default-src': "'none'",
    'script-src': `'nonce-N0nce+/=' ${BASE} 'wasm-unsafe-eval'`,
    'media-src': 'blob:',
    'connect-src': "'none'",
  });
  assert.ok(!csp.includes('unsafe-inline') && !csp.includes("'unsafe-eval'") && !csp.includes('*'), 'nothing broader');
  // the policy comes before any script, so it governs all of them
  const html = page();
  assert.ok(html.indexOf('Content-Security-Policy') < html.indexOf('<script'));
  // every script is the nonce's: the import map and the entry
  for (const t of tags(html, 'script')) assert.ok(t.includes('nonce="N0nce+/="'), t);
}

// ---- the player: the pinned version, its SRI as the lock gives it, the import map as artifact.rs writes it -------------

{
  const lockPath = path.join(__dirname, '..', '..', '..', 'crates', 'btwserve', 'src', 'mcp', 'player-cdn.lock.json');
  const lock = JSON.parse(fs.readFileSync(lockPath, 'utf8'));
  // the pin is the server's: package, version and entry as the lock has them, and every hash exactly its own
  assert.deepEqual([PLAYER.package, PLAYER.version, PLAYER.entry], [lock.package, lock.version, lock.entry]);
  assert.ok(!lock.placeholder, 'the player lock has real hashes');
  assert.deepEqual(PLAYER.integrity, Object.fromEntries(Object.entries(lock.files).map(([f, x]) => [f, x.integrity])));

  const html = page();
  const [map, entry] = tags(html, 'script');
  assert.equal(map, '<script type="importmap" nonce="N0nce+/=">');
  assert.equal(entry, `<script type="module" nonce="N0nce+/=" src="${BASE}beatscript-track.js" integrity="${lock.files['beatscript-track.js'].integrity}" crossorigin="anonymous">`);
  assert.ok(html.indexOf(map) < html.indexOf(entry), 'the import map comes before the module');
  // artifact.rs PlayerCdn::tags: every .js of the lock but the entry and the UMD wrapper, in the lock's order, keyed
  // by its jsDelivr URL, serialized compactly with < as \u003c
  const want = {};
  for (const [f, x] of Object.entries(lock.files)) if (f.endsWith('.js') && f !== lock.entry && !f.endsWith('.umd.js')) want[`${BASE}${f}`] = x.integrity;
  const body = html.slice(html.indexOf(map) + map.length, html.indexOf('</script>'));
  assert.equal(body, JSON.stringify({ integrity: want }).replace(/</g, '\\u003c'));
  assert.ok(Object.keys(want).length >= 10, 'the lock lists the engine modules');

  // no hashes: no integrity at all, rather than wrong ones
  const bare = page({ player: { ...PLAYER, integrity: {} } });
  assert.deepEqual(tags(bare, 'script'), [`<script type="module" nonce="N0nce+/=" src="${BASE}beatscript-track.js" crossorigin="anonymous">`]);
  assert.ok(!bare.includes('integrity') && !bare.includes('importmap'));
}

// ---- the text: the element's text attribute, which gives the player the text byte for byte ---------------------------

{
  const html = page();
  assert.ok(html.includes(`<body>\n<beatscript-track theme="dark" text="`));
  assert.equal(textOf(html), TEXT);
  assert.ok(page({ theme: 'light' }).includes('<beatscript-track theme="light" '));

  // nothing in the text can end the attribute or the element, and it comes back unchanged
  const evil = 'tempo 120\n# </script><img src=x onerror=alert(1)> </SCRIPT >\n# <!-- <script> "double" \'single\' & &amp; &#39;\n';
  const bad = page({ text: evil });
  assert.equal(textOf(bad), evil);
  const value = bad.match(/ text="([^"]*)"/)[1];
  assert.ok(!/[<>"']/.test(value), 'only entities in the value');
  assert.ok(value.includes('&lt;/script&gt;') && value.includes('&lt;!--') && value.includes('&quot;double&quot;') && value.includes('&#39;single&#39;') && value.includes('&amp;amp;'));
  assert.equal(tags(bad, 'script').length, 2, 'the import map and the entry, nothing injected');
  assert.ok(!/<img/i.test(bad.replace(value, '')));
  // attributes are escaped as artifact.rs's esc escapes them
  assert.ok(page({ nonce: `"><x'&` }).includes('nonce="&quot;&gt;&lt;x&#39;&amp;"'));
}

// ---- registerPreview: btw.preview opens one panel beside, writes the page again on a change (once it rests) and a save ---

(async () => {
  const handlers = {};
  const commands = {};
  const panels = [];
  const messages = [];
  const on = (name) => (fn) => {
    handlers[name] = fn;
    return { dispose() {} };
  };
  const doc = { languageId: 'btw', uri: { path: '/w/two-bars.btw' }, text: TEXT, getText() { return this.text; } };
  const vscode = {
    ViewColumn: { Beside: -2 },
    ColorThemeKind: { Light: 1, Dark: 2, HighContrast: 3, HighContrastLight: 4 },
    commands: { registerCommand: (name, fn) => { commands[name] = fn; return { dispose() {} }; } },
    workspace: { onDidChangeTextDocument: on('change'), onDidSaveTextDocument: on('save') },
    window: {
      activeTextEditor: null,
      activeColorTheme: { kind: 1 },
      showInformationMessage: (m) => messages.push(m),
      onDidChangeActiveColorTheme: on('theme'),
      createWebviewPanel(viewType, title, show, options) {
        const p = { viewType, title, show, options, webview: { html: '' }, reveals: 0, reveal() { this.reveals++; }, onDidDispose: (fn) => { p.dispose = fn; } };
        panels.push(p);
        return p;
      },
    },
  };
  const context = { subscriptions: [] };
  registerPreview(context, vscode);
  assert.equal(typeof commands['btw.preview'], 'function', 'btw.preview is registered');
  assert.ok(context.subscriptions.length >= 4, 'its listeners are disposed with the extension');

  // no BeatScript editor: a message, no panel
  commands['btw.preview']();
  assert.equal(panels.length, 0);
  assert.equal(messages.length, 1);

  vscode.window.activeTextEditor = { document: doc };
  commands['btw.preview']();
  assert.equal(panels.length, 1);
  const [p] = panels;
  assert.equal(p.viewType, 'btw.preview');
  assert.deepEqual(p.show, { viewColumn: -2, preserveFocus: true });
  assert.deepEqual(p.options, { enableScripts: true, localResourceRoots: [], retainContextWhenHidden: false });
  assert.equal(textOf(p.webview.html), TEXT);
  assert.ok(p.webview.html.includes('theme="light"'));
  const nonce = unescape(p.webview.html).match(/'nonce-([^']+)'/)[1];
  assert.ok(nonce.length >= 16 && p.webview.html.includes(`<script type="module" nonce="${nonce}"`));

  // a second run reveals the same panel
  commands['btw.preview']();
  assert.equal(panels.length, 1);
  assert.equal(p.reveals, 1);

  // an edit: written again only once the typing rests (400 ms), with a fresh nonce
  doc.text = 'tempo 90\n';
  handlers.change({ document: doc });
  handlers.change({ document: { ...doc } }); // another document: ignored
  assert.ok(!p.webview.html.includes('tempo 90'));
  await new Promise((r) => setTimeout(r, 450));
  assert.ok(p.webview.html.includes('tempo 90'));
  assert.notEqual(unescape(p.webview.html).match(/'nonce-([^']+)'/)[1], nonce);

  // a save: written again at once
  doc.text = 'tempo 100\n';
  handlers.save(doc);
  assert.ok(p.webview.html.includes('tempo 100'));

  // a save or a theme event that changes nothing leaves the page (and the playing track) alone; a new theme writes it
  const before = p.webview.html;
  handlers.save(doc);
  handlers.theme();
  assert.equal(p.webview.html, before);
  vscode.window.activeColorTheme = { kind: 2 };
  handlers.theme();
  assert.ok(p.webview.html.includes('theme="dark"') && textOf(p.webview.html) === 'tempo 100\n');

  // closed with an edit pending: its timer is cleared, and nothing more is written
  doc.text = 'tempo 105\n';
  handlers.change({ document: doc });
  p.dispose();
  await new Promise((r) => setTimeout(r, 450));
  assert.ok(!p.webview.html.includes('tempo 105'));
  doc.text = 'tempo 110\n';
  handlers.save(doc);
  handlers.change({ document: doc });
  await new Promise((r) => setTimeout(r, 450));
  assert.ok(!p.webview.html.includes('tempo 110'));

  // opened again: a new panel, written at once even though the text is what the old one last showed
  commands['btw.preview']();
  assert.equal(panels.length, 2);
  assert.equal(textOf(panels[1].webview.html), 'tempo 110\n');
  for (const s of context.subscriptions) s.dispose();
  console.log('preview ok');
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
