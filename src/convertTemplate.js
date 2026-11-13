// The "Convert to Haml/Slim/ERB" commands, through the converter gems' CLIs.
import * as vscode from "vscode";
import path from "path";

import { CONVERTERS, convertedFileName } from "./convert.js";
import { engineOf } from "./languages.js";
import {
  commandPath,
  useBundler,
  confirmOverwrite,
  workspaceFolderFor,
} from "./workspace.js";
import { run, firstLine } from "./run.js";

/** The converted `src` of `doc`, or undefined after telling the user why the converter failed. */
async function runConverter({ command, args, gem }, doc, src) {
  const bin = commandPath(command);
  const r = await run(
    bin,
    args,
    { cwd: workspaceFolderFor(doc), useBundler: useBundler() },
    src,
  );
  const missingGem = r.stderr.includes(`cannot load such file -- ${gem}`);
  if (r.err?.code === "ENOENT" || missingGem) {
    const what = missingGem ? `The ${gem} gem was` : `"${bin}" was`;
    const pick = await vscode.window.showErrorMessage(
      `${what} not found. Install with: gem install ${gem}`,
      "Copy command",
    );
    if (pick) await vscode.env.clipboard.writeText(`gem install ${gem}`);
    return undefined;
  }
  if (r.code !== 0) {
    vscode.window.showErrorMessage(`Conversion failed: ${firstLine(r)}`);
    return undefined;
  }
  return r.stdout;
}

/** Asks where the result goes: `"save"` (only for a file on disk), `"untitled"` or `"replace"`. */
async function pickDestination(doc, from, target, newName) {
  const canSave = doc.uri.scheme === "file";
  const choice = await vscode.window.showQuickPick(
    [
      ...(canSave
        ? [
            {
              label: `Save as ${path.basename(newName)} next to the original`,
              id: "save",
            },
          ]
        : []),
      { label: "Open as untitled document", id: "untitled" },
      { label: "Replace contents of the current file", id: "replace" },
    ],
    {
      placeHolder: `Converted ${from} → ${target}. What to do with the result?`,
    },
  );
  return choice?.id;
}

async function openUntitled(language, content) {
  const doc = await vscode.workspace.openTextDocument({ language, content });
  await vscode.window.showTextDocument(doc, {
    viewColumn: vscode.ViewColumn.Beside,
  });
}

async function saveAs(fileName, content) {
  const uri = vscode.Uri.file(fileName);
  if (!(await confirmOverwrite(uri))) return;
  await vscode.workspace.fs.writeFile(uri, new TextEncoder().encode(content));
  await vscode.window.showTextDocument(uri, {
    viewColumn: vscode.ViewColumn.Beside,
  });
}

/** Replaces the current contents if unchanged since conversion started, and switches the document to `language`. */
async function replaceContents(ed, version, content, language) {
  if (ed.document.version !== version) {
    vscode.window.showWarningMessage(
      "Template changed while converting; conversion cancelled.",
    );
    return;
  }
  const text = ed.document.getText();
  const full = new vscode.Range(
    ed.document.positionAt(0),
    ed.document.positionAt(text.length),
  );
  await ed.edit((eb) => eb.replace(full, content));
  await vscode.languages.setTextDocumentLanguage(ed.document, language);
}

async function convert(target) {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const doc = ed.document;
  const from = engineOf(doc.languageId);
  const converter = CONVERTERS[`${from}>${target}`];
  if (!converter) {
    vscode.window.showInformationMessage(
      `No converter for ${from} → ${target}.`,
    );
    return;
  }
  const version = doc.version;
  const src = doc.getText();
  const converted = await runConverter(converter, doc, src);
  if (converted === undefined) return;

  const newName = convertedFileName(doc.fileName, from, target);
  const destination = await pickDestination(doc, from, target, newName);
  if (destination === "untitled") await openUntitled(target, converted);
  else if (destination === "save") await saveAs(newName, converted);
  else if (destination === "replace")
    await replaceContents(ed, version, converted, target);
}

export { convert };
