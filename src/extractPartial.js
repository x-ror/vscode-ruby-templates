// "Extract Selection to Partial": the selection moves into a new partial and a render call takes its place.
import * as vscode from "vscode";
import path from "path";

import { extractBlock, partialPath, renderPath } from "./partial.js";
import { engineOf } from "./languages.js";
import { confirmOverwrite } from "./workspace.js";
import { documentLines } from "./editor.js";

const PARTIAL_NAME_RE = /^([\w-]+\/)*[a-z_]\w*$/;
const RENDER_CALL = {
  erb: (p) => `<%= render "${p}" %>`,
  haml: (p) => `= render "${p}"`,
  slim: (p) => `= render "${p}"`,
};

async function extractPartial() {
  const ed = vscode.window.activeTextEditor;
  if (!ed || ed.selection.isEmpty) return;
  const doc = ed.document;
  const engine = engineOf(doc.languageId);
  const renderCall = RENDER_CALL[engine];
  if (!renderCall) return;
  if (doc.uri.scheme !== "file") {
    vscode.window.showInformationMessage(
      "Save the template before extracting a partial.",
    );
    return;
  }
  // Haml and Slim nest by indentation, so only whole lines can move.
  const block = extractBlock(
    documentLines(doc),
    ed.selection,
    engine !== "erb",
  );
  if (!block) return;
  const name = await vscode.window.showInputBox({
    prompt:
      "Partial name, without the leading _ and the extension (`card`, `shared/card`)",
    validateInput: (v) =>
      PARTIAL_NAME_RE.test(v)
        ? null
        : "Use a lowercase Ruby identifier, optionally prefixed by directories.",
  });
  if (!name) return;
  const target = partialPath(doc.uri.fsPath, name);
  if (!target) {
    vscode.window.showErrorMessage(
      `"${name}" renders from app/views, but this file isn't under app/views.`,
    );
    return;
  }
  if (path.resolve(target) === path.resolve(doc.uri.fsPath)) {
    vscode.window.showErrorMessage(
      `"${name}" names this partial itself; pick another name.`,
    );
    return;
  }
  const targetUri = vscode.Uri.file(target);
  if (!(await confirmOverwrite(targetUri))) return;

  const eol = doc.eol === vscode.EndOfLine.CRLF ? "\r\n" : "\n";
  const { start, end } = block.range;
  // One edit, so a single undo restores the selection and a failed create leaves the template alone.
  const edit = new vscode.WorkspaceEdit();
  edit.createFile(targetUri, { overwrite: true });
  // A new document takes files.eol (CRLF on Windows) and would convert the inserted text to it.
  edit.set(targetUri, [
    vscode.TextEdit.setEndOfLine(doc.eol),
    vscode.TextEdit.insert(
      new vscode.Position(0, 0),
      block.lines.join(eol) + eol,
    ),
  ]);
  edit.replace(
    doc.uri,
    new vscode.Range(start.line, start.character, end.line, end.character),
    `${block.indent}${renderCall(renderPath(doc.uri.fsPath, name))}`,
  );
  if (!(await vscode.workspace.applyEdit(edit))) {
    vscode.window.showErrorMessage(`Couldn't create ${path.basename(target)}.`);
    return;
  }
  // Text edits leave the new partial open and unsaved.
  const partial = await vscode.workspace.openTextDocument(targetUri);
  await partial.save();
  await vscode.window.showTextDocument(partial, {
    viewColumn: vscode.ViewColumn.Beside,
    preview: true,
  });
}

export { extractPartial };
