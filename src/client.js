'use strict';
const vscode = require('vscode');
const path = require('path');
const fs = require('fs');
const { LanguageClient, TransportKind } = require('vscode-languageclient/node');

/** @type {LanguageClient | undefined} */
let client;

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
  const config = vscode.workspace.getConfiguration('rubyTemplates');
  if (!config.get('server.enabled', true)) return;
  const command = findServer(context);
  output.appendLine(`[ruby-templates] starting language server: ${command}`);

  client = new LanguageClient(
    'rubyTemplates',
    'Ruby Templates Language Server',
    { command, transport: TransportKind.stdio },
    {
      documentSelector: context.extension.packageJSON.contributes.languages.map(({ id }) => ({ language: id })),
      outputChannel: output,
      initializationOptions: {
        slim: {
          extraEngines: config.get('slim.extraEngines', []),
          disabledDiagnostics: config.get('slim.disabledDiagnostics', [])
        }
      },
      synchronize: { fileEvents: vscode.workspace.createFileSystemWatcher('**/app/views/**') }
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
