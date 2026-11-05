import * as vscode from "vscode";

import { registerServer, stopClient } from "./client.js";
import {
  toggleErbTag,
  toggleErbLineComment,
  wrapInErb,
} from "./erbCommands.js";
import { toggleActiveHamlLines, reindentActiveHaml } from "./hamlCommands.js";
import { toggleSlimTag, wrapInSlim } from "./slimCommands.js";
import { extractPartial } from "./extractPartial.js";
import { switchRelated } from "./relatedFile.js";
import { registerPreview } from "./preview.js";
import { convert } from "./convertTemplate.js";
import { registerRoutes } from "./railsRoutes.js";
import { registerErbAutoClose } from "./erbAutoClose.js";

const COMMANDS = {
  "rubyTemplates.toggleErbTag": toggleErbTag,
  "rubyTemplates.toggleErbLineComment": toggleErbLineComment,
  "rubyTemplates.wrapInErb": wrapInErb,
  "rubyTemplates.convertToHaml": () => convert("haml"),
  "rubyTemplates.convertToSlim": () => convert("slim"),
  "rubyTemplates.convertToErb": () => convert("erb"),
  "rubyTemplates.extractPartial": extractPartial,
  "rubyTemplates.switchRelated": switchRelated,
  "rubyTemplates.toggleHamlPrefix": toggleActiveHamlLines,
  "rubyTemplates.reindentHaml": reindentActiveHaml,
  "rubyTemplates.toggleSlimTag": toggleSlimTag,
  "rubyTemplates.wrapInSlim": wrapInSlim,
  "rubyTemplates.extractSlimPartial": extractPartial,
};

function activate(context) {
  for (const [id, handler] of Object.entries(COMMANDS))
    context.subscriptions.push(vscode.commands.registerCommand(id, handler));
  registerServer(context);
  registerRoutes(context);
  registerErbAutoClose(context);
  registerPreview(context);
}

function deactivate() {
  return stopClient();
}

export { activate, deactivate };
