'use strict';
const vscode = require('vscode');
const path = require('path');
const fs = require('fs');
const { LanguageClient, TransportKind } = require('vscode-languageclient/node');

/** @type {LanguageClient | undefined} */
let client;
/** @type {vscode.FileSystemWatcher[] | undefined} */
let watchers;

function serverBinaryName() {
  return process.platform === 'win32' ? 'ruby-templates-ls.exe' : 'ruby-templates-ls';
}

/** Locate the language server: setting → bundled per-platform binary → dev build → PATH. */
function findServer(context) {
  const configured = vscode.workspace.getConfiguration('rubyTemplates').get('server.path', '');
  if (configured && fs.existsSync(configured)) return configured;
  const target = `${process.platform}-${process.arch}`;
  const candidates = [
    context.asAbsolutePath(path.join('bin', target, serverBinaryName())),
    context.asAbsolutePath(path.join('server', 'target', 'release', serverBinaryName())),
    context.asAbsolutePath(path.join('server', 'target', 'debug', serverBinaryName()))
  ];
  for (const c of candidates) if (fs.existsSync(c)) return c;
  return serverBinaryName(); // hope it is on PATH
}

async function startClient(context, output) {
  const enabled = vscode.workspace.getConfiguration('rubyTemplates').get('server.enabled', true);
  if (!enabled) return;
  const command = findServer(context);
  output.appendLine(`[ruby-templates] starting language server: ${command}`);

  // The server caches app/views listings and parsed locales; these events invalidate them. The folder globs
  // catch deleting/renaming a whole app/ or config/ tree, which VS Code reports as one event for the folder.
  // Created once: restarts reuse them instead of piling up live watchers.
  if (!watchers) {
    watchers = ['**/app/views/**', '**/config/locales/**', '**/{app,config,app/views,config/locales}']
      .map(glob => vscode.workspace.createFileSystemWatcher(glob));
    context.subscriptions.push(...watchers);
  }
  client = new LanguageClient(
    'rubyTemplates',
    'Ruby Templates Language Server',
    { command, transport: TransportKind.stdio },
    {
      documentSelector: context.extension.packageJSON.contributes.languages.map(({ id }) => ({ language: id })),
      outputChannel: output,
      synchronize: { fileEvents: watchers }
    }
  );
  try {
    await client.start();
    context.subscriptions.push(client);
  } catch (e) {
    client = undefined;
    const msg = `Ruby Templates language server failed to start (${command}). ` +
      'Build it with `cargo build --release` in the extension\'s server/ folder, or set rubyTemplates.server.path.';
    output.appendLine(`[ruby-templates] ${msg}\n${e && e.message ? e.message : e}`);
    const pick = await vscode.window.showWarningMessage(msg, 'Open settings', 'Show log');
    if (pick === 'Open settings') vscode.commands.executeCommand('workbench.action.openSettings', 'rubyTemplates.server');
    if (pick === 'Show log') output.show();
  }
}

async function stopClient() {
  if (client) { await client.stop(); client = undefined; }
}

async function restartClient(context, output) {
  await stopClient();
  await startClient(context, output);
}

module.exports = { startClient, stopClient, restartClient };
