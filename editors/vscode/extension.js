// The BeatScript extension: the grammar colours the text; everything else is `beatscript lsp` (crates/beatscriptls), the language server
// the browser editor runs too (as wasm), so VS Code and the page say the same things.
//
// References (a `gen@2` call, a `seed(...)`, a stored program's `<cid>.drop.bass`, a template): the server puts two code lenses
// over each, `show expansion` and `eject`, and answers `beatscript/expand`. Which expansions are shown is this extension's state, not
// the server's: a shown expansion is a decoration after the reference (its first line and how many more, the whole of it in the
// decoration's hover), kept per document by the reference's call and place, so it survives edits that move it. `eject` is the
// server's command: it proves the edit (the program must play the same) and sends it back as `workspace/applyEdit`.
'use strict';

const vscode = require('vscode');
const { LanguageClient } = require('vscode-languageclient/node');

let client;

// ---- references: show or hide an expansion, copy it, open it beside, eject it ---------------------------------------------

const TOGGLE = 'beatscript.expand.toggle'; // the server's commands (executeCommandProvider): the lenses and the code actions name them
const COPY = 'beatscript.expand.copy';
const EJECT = 'beatscript.eject';

const shown = new Map(); // document uri -> Set of reference keys whose expansion is shown
let decoration;
const opened = new Map(); // beatscript-expansion: uri -> text
const changed = new vscode.EventEmitter();

const keyOf = (r) => `${r.kind}|${r.call}|${r.place}|${r.name || ''}`;
const shortCid = (c) => (c && c.length > 12 ? `${c[0]}…${c.slice(-8)}` : c || '');

/** The reference a command is about: a lens's or an action's argument, else the caret of the active BeatScript editor. */
function here(args) {
  if (args && args[0] && args[0].uri) return args[0];
  const ed = vscode.window.activeTextEditor;
  if (!ed || ed.document.languageId !== 'beatscript' || !client) return null;
  return { uri: ed.document.uri.toString(), position: client.code2ProtocolConverter.asPosition(ed.selection.active) };
}

/** `beatscript/expand`: every reference of a document, or the one at a position. */
async function expand(uri, position) {
  const params = { textDocument: { uri } };
  if (position) params.position = position;
  const res = await client.sendRequest('beatscript/expand', params);
  if (res && res.errors && res.errors.length) vscode.window.setStatusBarMessage(`BeatScript: ${res.errors[0].message.split('\n')[0]}`, 4000);
  return (res && res.references) || [];
}

async function referenceAt(args) {
  const at = here(args);
  if (!at) return null;
  const [r] = await expand(at.uri, at.position);
  if (!r) vscode.window.setStatusBarMessage('BeatScript: no reference here', 3000);
  return r ? { uri: at.uri, ref: r } : null;
}

/** Paint the shown expansions of one document in its visible editors. */
async function paint(uri) {
  const editors = vscode.window.visibleTextEditors.filter((e) => e.document.uri.toString() === uri);
  if (!editors.length || !client) return;
  const keys = shown.get(uri);
  const refs = (keys && keys.size ? await expand(uri) : []).filter((r) => keys.has(keyOf(r)));
  for (const e of editors) {
    const decos = refs.map((r) => {
      // at the end of the reference's line (a part line's own params may follow the call)
      const line = e.document.lineAt(Math.min(client.protocol2CodeConverter.asRange(r.range).end.line, e.document.lineCount - 1));
      const lines = r.text.split('\n');
      const first = lines[0].length > 72 ? `${lines[0].slice(0, 71)}…` : lines[0];
      const more = lines.length > 1 ? `  … ${lines.length - 1} more line${lines.length === 2 ? '' : 's'}` : '';
      const md = new vscode.MarkdownString(`\`${r.call}\` → \`${r.resolved || ''}\` (${r.kind}, ${r.half})\n\n`);
      md.appendCodeblock(`${r.from}\n${r.text}`, 'beatscript');
      return { range: new vscode.Range(line.range.end, line.range.end), hoverMessage: md, renderOptions: { after: { contentText: `  ⇒ ${first}${more}  [${shortCid(r.resolved)}]` } } };
    });
    e.setDecorations(decoration, decos);
  }
}

/** The lenses' titles follow what is shown: ask the server for them again. */
function refreshLenses(uri) {
  const feature = client && client.getFeature && client.getFeature('textDocument/codeLens');
  const doc = vscode.workspace.textDocuments.find((d) => d.uri.toString() === uri);
  const p = feature && doc && feature.getProvider ? feature.getProvider(doc) : undefined;
  if (p && p.onDidChangeCodeLensEmitter) p.onDidChangeCodeLensEmitter.fire();
}

async function toggle(args) {
  const found = await referenceAt(args);
  if (!found) return null;
  const { uri, ref } = found;
  if (!shown.has(uri)) shown.set(uri, new Set());
  const keys = shown.get(uri);
  const k = keyOf(ref);
  if (keys.has(k)) keys.delete(k);
  else keys.add(k);
  await paint(uri);
  refreshLenses(uri);
  return ref;
}

/** Every reference of the active document: show them all, or hide them all when any is shown. */
async function toggleAll() {
  const ed = vscode.window.activeTextEditor;
  if (!ed || ed.document.languageId !== 'beatscript' || !client) return;
  const uri = ed.document.uri.toString();
  const keys = shown.get(uri) || new Set();
  if (keys.size) keys.clear();
  else for (const r of await expand(uri)) keys.add(keyOf(r));
  shown.set(uri, keys);
  await paint(uri);
  refreshLenses(uri);
}

async function copy(args) {
  const found = await referenceAt(args);
  if (!found) return null;
  const { ref } = found;
  await vscode.env.clipboard.writeText(`${ref.from}\n${ref.text}\n`);
  vscode.window.setStatusBarMessage(`BeatScript: copied the expansion of ${ref.call}`, 3000);
  return ref;
}

/** The expansion of the reference at the caret as a read-only BeatScript document beside the program. */
async function openBeside(args) {
  const found = await referenceAt(args);
  if (!found) return;
  const { ref } = found;
  const name = ref.call.replace(/[^A-Za-z0-9@=._-]+/g, '-').slice(0, 60);
  const uri = vscode.Uri.parse(`beatscript-expansion:/${name}.beatscript`);
  opened.set(uri.toString(), `${ref.from}\n${ref.text}\n`);
  changed.fire(uri);
  const doc = await vscode.workspace.openTextDocument(uri);
  await vscode.languages.setTextDocumentLanguage(doc, 'beatscript');
  await vscode.window.showTextDocument(doc, { viewColumn: vscode.ViewColumn.Beside, preview: true, preserveFocus: true });
}

const middleware = {
  // the server's commands, run here first: show/hide and copy are this client's (it asks `beatscript/expand`); eject goes to the
  // server, which proves the edit and sends it back as workspace/applyEdit
  executeCommand: async (command, args, next) => {
    try {
      if (command === TOGGLE) return await toggle(args);
      if (command === COPY) return await copy(args);
      if (command === EJECT) {
        const at = here(args);
        return at ? await next(command, [at]) : undefined;
      }
      return await next(command, args);
    } catch (e) {
      vscode.window.showWarningMessage(`BeatScript: ${e.message || e}`);
      return undefined;
    }
  },
  // a lens over a shown expansion says `hide expansion`
  provideCodeLenses: async (document, token, next) => {
    const lenses = await next(document, token);
    const keys = shown.get(document.uri.toString());
    if (!lenses || !keys || !keys.size) return lenses;
    const at = new Map((await expand(document.uri.toString())).map((r) => [`${r.range.start.line}:${r.range.start.character}`, keyOf(r)]));
    for (const l of lenses) {
      if (l.command && l.command.command === TOGGLE && keys.has(at.get(`${l.range.start.line}:${l.range.start.character}`))) {
        l.command.title = 'hide expansion';
      }
    }
    return lenses;
  },
};

// ---- the client -------------------------------------------------------------------------------------------------------

function start(context) {
  const cfg = vscode.workspace.getConfiguration('beatscript');
  const command = cfg.get('server.path', 'beatscript');
  const args = cfg.get('server.args', ['lsp']);
  const serverOptions = { run: { command, args }, debug: { command, args } };
  const clientOptions = {
    // files and unsaved buffers; not the beatscript-expansion: documents opened beside (they are the server's answers)
    documentSelector: [{ scheme: 'file', language: 'beatscript' }, { scheme: 'untitled', language: 'beatscript' }],
    outputChannelName: 'BeatScript',
    middleware,
  };
  client = new LanguageClient('beatscript', 'BeatScript', serverOptions, clientOptions);
  client.start().catch((e) => {
    vscode.window.showErrorMessage(`BeatScript: could not start \`${command} ${args.join(' ')}\` (${e.message}). Set beatscript.server.path to the beatscript binary.`);
  });
  context.subscriptions.push({ dispose: () => client && client.stop() });
}

function activate(context) {
  decoration = vscode.window.createTextEditorDecorationType({
    after: { color: new vscode.ThemeColor('editorCodeLens.foreground'), fontStyle: 'italic' },
    rangeBehavior: vscode.DecorationRangeBehavior.ClosedClosed,
  });
  require('./preview').registerPreview(context, vscode); // beatscript.preview: the document in the published player, beside it
  start(context);
  let repaint;
  context.subscriptions.push(
    decoration,
    changed,
    vscode.workspace.registerTextDocumentContentProvider('beatscript-expansion', { onDidChange: changed.event, provideTextDocumentContent: (uri) => opened.get(uri.toString()) || '' }),
    vscode.commands.registerCommand('beatscript.restartServer', async () => {
      if (client) await client.stop();
      start(context);
    }),
    vscode.commands.registerCommand('beatscript.expand.all', toggleAll),
    vscode.commands.registerCommand('beatscript.expand.open', openBeside),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('beatscript.server')) vscode.commands.executeCommand('beatscript.restartServer');
    }),
    // an edit moves the references: paint the shown ones again once the text rests (the server has the change by then)
    vscode.workspace.onDidChangeTextDocument((e) => {
      const uri = e.document.uri.toString();
      if (!shown.has(uri) || !shown.get(uri).size) return;
      clearTimeout(repaint);
      repaint = setTimeout(() => paint(uri).catch(() => {}), 300);
    }),
    vscode.window.onDidChangeVisibleTextEditors((eds) => {
      for (const e of eds) if (shown.has(e.document.uri.toString())) paint(e.document.uri.toString()).catch(() => {});
    }),
    vscode.workspace.onDidCloseTextDocument((d) => shown.delete(d.uri.toString())),
  );
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
