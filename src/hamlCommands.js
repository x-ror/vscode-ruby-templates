// Haml editing commands: cycle the Ruby prefix, re-indent through the server.
import * as vscode from "vscode";

import { serverFormatEdits } from "./client.js";
import { toggleHamlLines } from "./haml.js";
import { selectedLines, documentLines } from "./editor.js";

async function toggleActiveHamlLines() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const edits = toggleHamlLines(
    documentLines(ed.document),
    new Set(ed.selections.flatMap(selectedLines)),
  );
  await ed.edit((eb) =>
    edits.forEach(([n, text]) => eb.replace(ed.document.lineAt(n).range, text)),
  );
}

/** Re-indents the selected lines, or the whole document, through the server's formatter. */
async function reindentActiveHaml() {
  const ed = vscode.window.activeTextEditor;
  if (ed?.document.languageId !== "haml") return;
  const { selection, document: doc } = ed;
  const lines = selectedLines(selection);
  const range = selection.isEmpty
    ? undefined
    : new vscode.Range(lines[0], 0, lines.at(-1), 0);
  const { tabSize, insertSpaces } = ed.options;
  const options = {
    tabSize: Number(tabSize),
    insertSpaces: insertSpaces === true,
  };
  const edits = await serverFormatEdits(doc, options, range);
  if (!edits) {
    vscode.window.showInformationMessage(
      "Re-indent Haml: the language server is not running, or left this file unchanged (the reason is in the Ruby Templates output).",
    );
    return;
  }
  await ed.edit((eb) =>
    edits.forEach(({ range, newText }) => eb.replace(range, newText)),
  );
}

export { toggleActiveHamlLines, reindentActiveHaml };
