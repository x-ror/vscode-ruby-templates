import * as vscode from "vscode";
import path from "path";
import fs from "fs";
import { LanguageClient, TransportKind } from "vscode-languageclient/node";

/** @type {LanguageClient | undefined} */
let client;
/** @type {vscode.FileSystemWatcher[] | undefined} */
let watchers;

function serverBinaryName() {
  return process.platform === "win32"
    ? "ruby-templates-ls.exe"
    : "ruby-templates-ls";
}

/** Locate the language server: setting → bundled per-platform binary → dev build → PATH. */
function findServer(context) {
  const configured = vscode.workspace
    .getConfiguration("rubyTemplates")
    .get("server.path", "");
  if (configured && fs.existsSync(configured)) return configured;
  const target = `${process.platform}-${process.arch}`;
  const candidates = [
    context.asAbsolutePath(path.join("bin", target, serverBinaryName())),
    context.asAbsolutePath(
      path.join("server", "target", "release", serverBinaryName()),
    ),
    context.asAbsolutePath(
      path.join("server", "target", "debug", serverBinaryName()),
    ),
  ];
  for (const c of candidates) if (fs.existsSync(c)) return c;
  return serverBinaryName(); // hope it is on PATH
}

async function doStart(context, output) {
  const config = vscode.workspace.getConfiguration("rubyTemplates");
  if (!config.get("server.enabled", true)) return;
  const command = findServer(context);
  output.appendLine(`[ruby-templates] starting language server: ${command}`);

  // The server caches app/views listings and parsed locales; these events invalidate them. The folder globs
  // catch deleting/renaming a whole app/ or config/ tree, which VS Code reports as one event for the folder.
  // Created once: restarts reuse them instead of piling up live watchers.
  if (!watchers) {
    watchers = [
      "**/app/views/**",
      "**/config/locales/**",
      "**/{app,config,app/views,config/locales}",
    ].map((glob) => vscode.workspace.createFileSystemWatcher(glob));
    context.subscriptions.push(...watchers);
  }
  const started = new LanguageClient(
    "rubyTemplates",
    "Ruby Templates Language Server",
    { command, transport: TransportKind.stdio },
    {
      documentSelector: context.extension.packageJSON.contributes.languages.map(
        ({ id }) => ({ language: id }),
      ),
      outputChannel: output,
      // Our catch below reports the failure; returning false stops the client from also showing its own.
      initializationFailedHandler: () => false,
      initializationOptions: {
        slim: {
          extraEngines: config.get("slim.extraEngines", []),
          disabledDiagnostics: config.get("slim.disabledDiagnostics", []),
        },
      },
      synchronize: { fileEvents: watchers },
    },
  );
  client = started;
  try {
    await started.start();
    context.subscriptions.push(started);
  } catch (e) {
    client = undefined;
    await disposeQuietly(started, output);
    const msg =
      `Ruby Templates language server failed to start (${command}). ` +
      "Build it with `cargo build --release` in the extension's server/ folder, or set rubyTemplates.server.path.";
    output.appendLine(
      `[ruby-templates] ${msg}\n${e && e.message ? e.message : e}`,
    );
    void offerHelp(msg, output);
  }
}

// Not awaited by callers: the restart queue must not wait for the notification to be dismissed.
async function offerHelp(msg, output) {
  const pick = await vscode.window.showWarningMessage(
    msg,
    "Open settings",
    "Show log",
  );
  if (pick === "Open settings")
    vscode.commands.executeCommand(
      "workbench.action.openSettings",
      "rubyTemplates.server",
    );
  if (pick === "Show log") output.show();
}

async function disposeQuietly(c, output) {
  try {
    await c.dispose();
  } catch (e) {
    output.appendLine(`[ruby-templates] stopping language server: ${e}`);
  }
}

async function doStop(output) {
  const old = client;
  client = undefined;
  if (old) await disposeQuietly(old, output);
}

// stop() hands concurrent callers one shared promise, so overlapping restarts would each start a server.
let queue = Promise.resolve();
const serialize = (task, output) =>
  (queue = queue
    .then(task)
    .catch((e) => output.appendLine(`[ruby-templates] ${e?.message ?? e}`)));

const startClient = (context, output) =>
  serialize(() => doStart(context, output), output);
const stopClient = (output) => serialize(() => doStop(output), output);
const restartClient = (context, output) =>
  serialize(async () => {
    await doStop(output);
    await doStart(context, output);
  }, output);

export { startClient, stopClient, restartClient };
