// "Switch to Related File": other formats, the controller action, a component's class, template and preview.
import * as vscode from "vscode";
import path from "path";

import {
  relatedFiles,
  relatedDirs,
  actionSource,
  actionLine,
  previewPartners,
} from "./related.js";
import { exists } from "./workspace.js";

const actionName = (view) => actionSource(view, path.sep)?.action;

/** The controller or mailer that renders `view`, when it exists. */
async function controllerAction(view) {
  const source = actionSource(view, path.sep);
  return source && (await exists(vscode.Uri.file(source.file)))
    ? source.file
    : undefined;
}

/**
 * Open the view's other formats (`.html.erb` ↔ `.text.erb`), its controller action, or a component's class ↔
 * template ↔ preview.
 */
async function switchRelated() {
  const doc = vscode.window.activeTextEditor?.document;
  if (doc?.uri.scheme !== "file") return;
  const dir = path.dirname(doc.uri.fsPath);
  const name = path.basename(doc.uri.fsPath);
  const candidates = [];
  for (const folder of [dir, ...relatedDirs(dir, name, path.sep)]) {
    let entries;
    try {
      entries = await vscode.workspace.fs.readDirectory(
        vscode.Uri.file(folder),
      );
    } catch {
      continue; // most components have no sidecar folder
    }
    const files = entries
      .filter(([, type]) => type === vscode.FileType.File)
      .map(([n]) => n);
    candidates.push(
      ...relatedFiles(name, files).map((n) => path.join(folder, n)),
    );
  }
  const action = await controllerAction(doc.uri.fsPath);
  if (action) candidates.push(action);
  for (const partner of previewPartners(doc.uri.fsPath, path.sep))
    if (await exists(vscode.Uri.file(partner))) candidates.push(partner);
  if (!candidates.length) {
    vscode.window.showInformationMessage(`No related file for ${name}.`);
    return;
  }
  const picked =
    candidates.length === 1
      ? candidates[0]
      : (
          await vscode.window.showQuickPick(
            candidates.map((file) => ({
              label:
                file === action
                  ? `${path.basename(file)} #${actionName(doc.uri.fsPath)}`
                  : path.basename(file),
              description: vscode.workspace.asRelativePath(path.dirname(file)),
              file,
            })),
            { placeHolder: "Related file" },
          )
        )?.file;
  if (!picked) return;
  const editor = await vscode.window.showTextDocument(vscode.Uri.file(picked));
  if (picked !== action) return;
  const line = actionLine(
    editor.document.getText(),
    actionName(doc.uri.fsPath),
  );
  if (line < 0) return;
  const at = new vscode.Position(line, 0);
  editor.selection = new vscode.Selection(at, at);
  editor.revealRange(
    new vscode.Range(at, at),
    vscode.TextEditorRevealType.InCenter,
  );
}

export { switchRelated };
