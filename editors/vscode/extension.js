// The BeatScript extension: the grammar colours the text; everything else is `btwd lsp` (crates/btwls), the language server
// the browser editor runs too (as wasm), so VS Code and the page say the same things.
'use strict';

const vscode = require('vscode');
const { LanguageClient } = require('vscode-languageclient/node');

let client;

function start(context) {
  const cfg = vscode.workspace.getConfiguration('btw');
  const command = cfg.get('server.path', 'btwd');
  const args = cfg.get('server.args', ['lsp']);
  const serverOptions = { run: { command, args }, debug: { command, args } };
  const clientOptions = {
    documentSelector: [{ language: 'btw' }],
    outputChannelName: 'BeatScript',
  };
  client = new LanguageClient('btw', 'BeatScript', serverOptions, clientOptions);
  client.start().catch((e) => {
    vscode.window.showErrorMessage(`BeatScript: could not start \`${command} ${args.join(' ')}\` (${e.message}). Set btw.server.path to the btwd binary.`);
  });
  context.subscriptions.push({ dispose: () => client && client.stop() });
}

function activate(context) {
  start(context);
  context.subscriptions.push(
    vscode.commands.registerCommand('btw.restartServer', async () => {
      if (client) await client.stop();
      start(context);
    }),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('btw.server')) vscode.commands.executeCommand('btw.restartServer');
    }),
  );
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
