// Small vscode helpers shared by the commands.
import * as vscode from "vscode";
import os from "os";
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

/** Folder commands run in: the document's workspace folder or directory, else the first folder, else home. */
function workspaceFolderFor(doc) {
  const folder = vscode.workspace.getWorkspaceFolder(doc.uri);
  if (folder) return folder.uri.fsPath;
  if (doc.uri.scheme === "file") return path.dirname(doc.uri.fsPath);
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? os.homedir();
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
