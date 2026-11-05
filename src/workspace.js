// Small vscode helpers shared by the commands.
import * as vscode from "vscode";
import path from "path";

const SECTION = "rubyTemplates";

const settings = () => vscode.workspace.getConfiguration(SECTION);

/** The executable set in `rubyTemplates.commands.<name>`, else `name` itself. */
const commandPath = (name) => settings().get(`commands.${name}`, name);

const useBundler = () => settings().get("preview.useBundler", false);

async function exists(uri) {
  try {
    await vscode.workspace.fs.stat(uri);
    return true;
  } catch (e) {
    if (e instanceof vscode.FileSystemError && e.code === "FileNotFound")
      return false;
    throw e;
  }
}

/** Whether `uri` may be written: it doesn't exist yet, or the user agreed to overwrite it. */
async function confirmOverwrite(uri) {
  if (!(await exists(uri))) return true;
  const ok = await vscode.window.showWarningMessage(
    `${path.basename(uri.path)} exists. Overwrite?`,
    { modal: true },
    "Overwrite",
  );
  return ok === "Overwrite";
}

function workspaceFolderFor(doc) {
  return (
    vscode.workspace.getWorkspaceFolder(doc.uri)?.uri.fsPath ||
    path.dirname(doc.uri.fsPath)
  );
}

export {
  SECTION,
  settings,
  commandPath,
  useBundler,
  exists,
  confirmOverwrite,
  workspaceFolderFor,
};
