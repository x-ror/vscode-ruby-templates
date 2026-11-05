// "Extract Selection to Partial" for ERB and Haml, and its Slim counterpart.
import * as vscode from "vscode";
import path from "path";

import { extractBlock, partialFileName } from "./partial.js";
import { partialPath } from "./slim.js";
import { engineOf } from "./languages.js";
import { exists, confirmOverwrite } from "./workspace.js";
import { documentLines } from "./editor.js";
import { lastSelectedLine, dedent } from "./text.js";

const PARTIAL_NAME_RE = /^\w+$/;
const RENDER_CALL = {
  erb: (p) => `<%= render "${p}" %>`,
  haml: (p) => `= render "${p}"`,
};

/** Move the selection into `_name.<ext>` next to the template and render it in its place. */
async function extractPartial() {
  const ed = vscode.window.activeTextEditor;
  if (!ed || ed.selection.isEmpty) return;
  const doc = ed.document;
  const lang = engineOf(doc.languageId);
  // Slim has its own extract command.
  if (!RENDER_CALL[lang]) return;
  if (doc.isUntitled) {
    vscode.window.showInformationMessage(
      "Save the template before extracting a partial.",
    );
    return;
  }
  // Haml nests by indentation, so only whole lines can move.
  const block = extractBlock(documentLines(doc), ed.selection, lang === "haml");
  if (!block) return;
  const name = await vscode.window.showInputBox({
    prompt: "Partial name (without the leading _ and the extension)",
    validateInput: (v) =>
      PARTIAL_NAME_RE.test(v) ? null : "Use letters, digits and _ only.",
  });
  if (!name) return;

  const { start, end } = block.range;
  const range = new vscode.Range(
    start.line,
    start.character,
    end.line,
    end.character,
  );
  const partial = vscode.Uri.joinPath(
    doc.uri,
    "..",
    partialFileName(path.basename(doc.uri.path), name),
  );
  const viewDir = /\/app\/views\/(.+)\/[^/]+$/.exec(doc.uri.path)?.[1];
  const renderPath = viewDir ? `${viewDir}/${name}` : name;

  if (!(await confirmOverwrite(partial))) return;
  await vscode.workspace.fs.writeFile(
    partial,
    new TextEncoder().encode(block.text),
  );
  await ed.edit((eb) =>
    eb.replace(range, `${block.indent}${RENDER_CALL[lang](renderPath)}`),
  );
  await vscode.window.showTextDocument(partial, {
    viewColumn: vscode.ViewColumn.Beside,
    preview: true,
  });
}

async function extractSlimPartial() {
  const ed = vscode.window.activeTextEditor;
  if (!ed || ed.selection.isEmpty || ed.document.uri.scheme !== "file") return;
  const doc = ed.document;
  const last = lastSelectedLine(ed.selection);
  const range = new vscode.Range(
    ed.selection.start.line,
    0,
    last,
    doc.lineAt(last).text.length,
  );

  const name = await vscode.window.showInputBox({
    prompt:
      'Partial name, rendered as `= render "name"` (`card`, `shared/card`)',
    validateInput: (v) =>
      /^([\w-]+\/)*[a-z_]\w*$/.test(v)
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
  const targetUri = vscode.Uri.file(target);
  if (await exists(targetUri)) {
    vscode.window.showErrorMessage(`${path.basename(target)} already exists.`);
    return;
  }
  const eol = doc.eol === vscode.EndOfLine.CRLF ? "\r\n" : "\n";
  const { indent, lines } = dedent(doc.getText(range).split(/\r?\n/));
  // One edit, so a single undo restores the selection and a failed create leaves the template alone.
  const edit = new vscode.WorkspaceEdit();
  edit.createFile(targetUri, { overwrite: false, ignoreIfExists: false });
  // A new document takes files.eol (CRLF on Windows) and would convert the inserted text to it.
  edit.set(targetUri, [
    vscode.TextEdit.setEndOfLine(doc.eol),
    vscode.TextEdit.insert(new vscode.Position(0, 0), lines.join(eol) + eol),
  ]);
  edit.replace(doc.uri, range, `${indent}= render "${name}"`);
  if (!(await vscode.workspace.applyEdit(edit))) {
    vscode.window.showErrorMessage(`Couldn't create ${path.basename(target)}.`);
    return;
  }
  // Text edits leave the new partial open and unsaved.
  await (await vscode.workspace.openTextDocument(targetUri)).save();
}

export { extractPartial, extractSlimPartial };
