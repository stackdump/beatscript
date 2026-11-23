// The extension's settings: `beatscript.*`, read first, and the `btw.*` they were called before the rename, read when the new
// one is not set anywhere (so a settings.json written before keeps working; the old keys are declared deprecated in
// package.json). Pure: it reads what VS Code's `WorkspaceConfiguration.inspect()` answers, so it is tested without VS Code.
// (rename-to-beatscript: keep. This file names the old settings on purpose.)
'use strict';

const SECTION = 'beatscript';
const LEGACY_SECTION = 'btw';

/** The keys, under either section. */
const KEYS = ['server.path', 'server.args', 'trace.server'];

/** The value a person set, from the most specific scope that has one; undefined when it is only the default. */
function setValue(info) {
  if (!info) return undefined;
  for (const k of ['workspaceFolderLanguageValue', 'workspaceLanguageValue', 'globalLanguageValue', 'workspaceFolderValue', 'workspaceValue', 'globalValue']) {
    if (info[k] !== undefined) return info[k];
  }
  return undefined;
}

/**
 * `key` (one of KEYS): the `beatscript.` setting when it is set, else the `btw.` one when that is set, else `dflt`.
 * `inspect(section, key)` is `vscode.workspace.getConfiguration(section).inspect(key)`.
 */
function read(inspect, key, dflt) {
  const now = setValue(inspect(SECTION, key));
  if (now !== undefined) return { value: now, from: `${SECTION}.${key}` };
  const old = setValue(inspect(LEGACY_SECTION, key));
  if (old !== undefined) return { value: old, from: `${LEGACY_SECTION}.${key}` };
  return { value: dflt, from: null };
}

/** Does a configuration change touch the server's settings, under either name? */
function affectsServer(e) {
  return e.affectsConfiguration(`${SECTION}.server`) || e.affectsConfiguration(`${LEGACY_SECTION}.server`);
}

module.exports = { SECTION, LEGACY_SECTION, KEYS, read, setValue, affectsServer };
