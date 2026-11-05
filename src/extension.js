import * as vscode from "vscode";

import { registerServer, stopClient } from "./client.js";
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
import {
  openPreview,
  refreshPreviewOnSave,
  closePreview,
  showCompiledSource,
} from "./preview.js";
import { convert } from "./convertTemplate.js";
import { registerRoutes } from "./railsRoutes.js";
import { registerErbAutoClose } from "./erbAutoClose.js";

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

  const output = registerServer(context);
  registerErbAutoClose(context);
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

function deactivate() {
  closePreview();
  return stopClient();
}

export { activate, deactivate };
