import * as vscode from "vscode";

import {
  startClient,
  stopClient,
  restartClient,
  serverFormatEdits,
} from "./client.js";
import {
  toggleErbTag,
  toggleErbLineComment,
  wrapInErb,
} from "./erbCommands.js";
import { toggleActiveHamlLines, reindentActiveHaml } from "./hamlCommands.js";
import { toggleSlimTag, wrapInSlim } from "./slimCommands.js";
import { extractPartial, extractSlimPartial } from "./extractPartial.js";
import { switchRelated } from "./relatedFile.js";
import { TEMPLATE_LANGS, engineOf } from "./render.js";
import { cfg } from "./workspace.js";
import {
  openPreview,
  refreshPreviewOnSave,
  closePreview,
  showCompiledSource,
} from "./preview.js";
import { convert } from "./convertTemplate.js";
import { registerRoutes } from "./railsRoutes.js";
import { registerErbAutoClose } from "./erbAutoClose.js";

// The server reads these at startup, so a change restarts it.
const SERVER_SETTINGS = [
  "rubyTemplates.server",
  "rubyTemplates.slim.extraEngines",
  "rubyTemplates.slim.disabledDiagnostics",
  "rubyTemplates.erb.disabledDiagnostics",
  "rubyTemplates.haml.disabledDiagnostics",
  "rubyTemplates.jbuilder.disabledDiagnostics",
  "rubyTemplates.erb.format.alignComments",
  ...["erb", "haml", "slim"].flatMap((lang) => [
    `rubyTemplates.${lang}.diagnostics.enabled`,
    `rubyTemplates.${lang}.format.enabled`,
  ]),
];

/** @type {vscode.LogOutputChannel | undefined} */
let output;

const COMMANDS = {
  "rubyTemplates.toggleErbTag": toggleErbTag,
  "rubyTemplates.toggleErbLineComment": toggleErbLineComment,
  "rubyTemplates.wrapInErb": wrapInErb,
  "rubyTemplates.showRenderedSource": showCompiledSource,
  "rubyTemplates.convertToHaml": () => convert("haml"),
  "rubyTemplates.convertToSlim": () => convert("slim"),
  "rubyTemplates.convertToErb": () => convert("erb"),
  "rubyTemplates.extractPartial": extractPartial,
  "rubyTemplates.switchRelated": switchRelated,
  "rubyTemplates.toggleHamlPrefix": toggleActiveHamlLines,
  "rubyTemplates.reindentHaml": reindentActiveHaml,
  "rubyTemplates.toggleSlimTag": toggleSlimTag,
  "rubyTemplates.wrapInSlim": wrapInSlim,
  "rubyTemplates.extractSlimPartial": extractSlimPartial,
};

function activate(context) {
  const sub = (cmd, fn) =>
    context.subscriptions.push(vscode.commands.registerCommand(cmd, fn));

  for (const [id, handler] of Object.entries(COMMANDS)) sub(id, handler);
  sub("rubyTemplates.preview", () => openPreview(context));

  // Language server (Rust): diagnostics, go-to-partial, completions, hover.
  output = vscode.window.createOutputChannel("Ruby Templates", {
    log: true,
  });
  context.subscriptions.push(output);
  registerServerFormatting(context);
  registerErbAutoClose(context);
  sub("rubyTemplates.restartServer", () => restartClient(context, output));
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (SERVER_SETTINGS.some((s) => e.affectsConfiguration(s)))
        void restartClient(context, output);
    }),
  );
  void startClient(context, output);
  registerRoutes(context, output);

  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument(refreshPreviewOnSave),
  );

  // Status bar: shows active template engine and offers preview.
  const status = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Right,
    90,
  );
  status.command = "rubyTemplates.preview";
  status.tooltip = "Ruby Templates: preview rendered HTML";
  const updateStatus = () => {
    const ed = vscode.window.activeTextEditor;
    if (ed && TEMPLATE_LANGS.has(engineOf(ed.document.languageId))) {
      status.text = `$(preview) ${ed.document.languageId.toUpperCase()}`;
      status.show();
    } else status.hide();
  };
  context.subscriptions.push(
    status,
    vscode.window.onDidChangeActiveTextEditor(updateStatus),
  );
  updateStatus();
}

// ---------------------------------------------------------------- formatting

const formatWithServer = async (doc, options, range) =>
  (await serverFormatEdits(doc, options, range)) ?? [];

/** Offers the server's formatting for ERB and Haml only while `rubyTemplates.<lang>.format.enabled` is on, so another extension's formatter isn't shadowed. */
function registerServerFormatting(context) {
  const registrations = new Map();
  const update = () => {
    for (const lang of ["erb", "haml"]) {
      const enabled = cfg().get(`${lang}.format.enabled`, true);
      if (enabled === registrations.has(lang)) continue;
      if (!enabled) {
        registrations.get(lang).dispose();
        registrations.delete(lang);
        continue;
      }
      registrations.set(
        lang,
        vscode.Disposable.from(
          vscode.languages.registerDocumentFormattingEditProvider(lang, {
            provideDocumentFormattingEdits: (doc, options) =>
              formatWithServer(doc, options),
          }),
          vscode.languages.registerDocumentRangeFormattingEditProvider(lang, {
            provideDocumentRangeFormattingEdits: (doc, range, options) =>
              formatWithServer(doc, options, range),
          }),
        ),
      );
    }
  };
  update();
  context.subscriptions.push(
    { dispose: () => registrations.forEach((r) => r.dispose()) },
    vscode.workspace.onDidChangeConfiguration(
      (e) =>
        (e.affectsConfiguration("rubyTemplates.erb.format.enabled") ||
          e.affectsConfiguration("rubyTemplates.haml.format.enabled")) &&
        update(),
    ),
  );
}

function deactivate() {
  closePreview();
  return stopClient(output);
}

export { activate, deactivate };
