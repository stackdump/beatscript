// The manifest and the settings after the rename (btw -> beatscript): the language is `beatscript` for .beatscript and .btw
// files; the settings are beatscript.*, with the btw.* they were called before declared deprecated and read as a fallback;
// the client's id is the settings section (so `beatscript.trace.server` is the trace); the commands are the server's.
// (rename-to-beatscript: keep. This file names the old settings on purpose.)
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const pkg = require('../package.json');
const settings = require('../settings');

const c = pkg.contributes;
const lang = c.languages.find((l) => l.id === 'beatscript');
assert.ok(lang, 'the language id is beatscript');
assert.deepEqual(lang.extensions, ['.beatscript', '.btw']);
assert.equal(c.languages.length, 1);
assert.deepEqual(c.grammars.map((g) => [g.language, g.scopeName]), [['beatscript', 'source.beatscript']]);

const props = c.configuration.properties;
assert.equal(settings.SECTION, 'beatscript');
assert.equal(settings.LEGACY_SECTION, 'btw');
for (const k of settings.KEYS) {
  const now = props[`beatscript.${k}`];
  const old = props[`btw.${k}`];
  assert.ok(now && !now.deprecationMessage, `beatscript.${k} is declared`);
  assert.ok(old && old.deprecationMessage && old.deprecationMessage.includes(`beatscript.${k}`), `btw.${k} is declared deprecated, naming its new name`);
  assert.equal(old.default, undefined, `btw.${k} has no default (so inspect() tells set from unset)`);
  assert.equal(old.type, now.type);
}
assert.equal(props['beatscript.server.path'].default, 'beatscript');
assert.deepEqual(props['beatscript.server.args'].default, ['lsp']);
assert.equal(Object.keys(props).length, 2 * settings.KEYS.length);

// the client is made with the section as its id: vscode-languageclient reads `${id}.trace.server`
const ext = fs.readFileSync(path.join(__dirname, '..', 'extension.js'), 'utf8');
assert.match(ext, /new LanguageClient\(settings\.SECTION, /);
assert.ok(props[`${settings.SECTION}.trace.server`]);
// every command the extension contributes is beatscript.*, and the server's own (crates/beatscriptls/src/refs.rs) are among them
const ids = c.commands.map((x) => x.command);
assert.ok(ids.every((id) => id.startsWith('beatscript.')), ids.join(' '));
const refs = fs.readFileSync(path.join(__dirname, '..', '..', '..', 'crates', 'beatscriptls', 'src', 'refs.rs'), 'utf8');
const server = [...refs.matchAll(/pub const (?:TOGGLE|COPY|EJECT): &str = "([^"]+)";/g)].map((m) => m[1]);
assert.equal(server.length, 3);
// the extension's own names for them (its middleware runs them first) are the server's
const mine = [...ext.matchAll(/^const (?:TOGGLE|COPY|EJECT) = '([^']+)';/gm)].map((m) => m[1]);
assert.deepEqual(mine, server);
for (const id of server) assert.ok(ids.includes(id), `${id} is contributed`);
for (const m of c.menus.commandPalette) assert.equal(m.when, 'editorLangId == beatscript');

// the settings: the new key when set (in any scope), else the old one, else the default
const fake = (values) => (section, key) => values[`${section}.${key}`];
const r = (values, key, dflt) => settings.read(fake(values), key, dflt);
assert.deepEqual(r({}, 'server.path', 'beatscript'), { value: 'beatscript', from: null });
assert.deepEqual(r({ 'btw.server.path': { globalValue: '/opt/btwd' } }, 'server.path', 'beatscript'), { value: '/opt/btwd', from: 'btw.server.path' });
assert.deepEqual(r({ 'btw.server.path': { globalValue: '/opt/btwd' }, 'beatscript.server.path': { defaultValue: 'beatscript', workspaceValue: '/w/beatscript' } }, 'server.path', 'x'), { value: '/w/beatscript', from: 'beatscript.server.path' });
// the default alone is not a setting: the old key, set, wins over the new key's default
assert.deepEqual(r({ 'beatscript.trace.server': { defaultValue: 'off' }, 'btw.trace.server': { workspaceFolderValue: 'verbose' } }, 'trace.server', 'off'), { value: 'verbose', from: 'btw.trace.server' });
// the most specific scope wins
assert.equal(settings.setValue({ globalValue: 1, workspaceValue: 2, workspaceFolderValue: 3 }), 3);
assert.equal(settings.setValue({ globalValue: 1 }), 1);
assert.equal(settings.setValue(undefined), undefined);
const seen = [];
assert.equal(settings.affectsServer({ affectsConfiguration: (s) => { seen.push(s); return s === 'btw.server'; } }), true);
assert.deepEqual(seen, ['beatscript.server', 'btw.server']);
console.log('manifest ok');
