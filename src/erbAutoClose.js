// Typing `>` in ERB, on top of VS Code's own auto-closing.
import * as vscode from "vscode";

import { tagToClose, isHtmlView, doubledErbClose } from "./autoClose.js";
import { settings } from "./workspace.js";
import { log } from "./client.js";
import { isErb } from "./languages.js";

// Part of the user's typing, so undo takes it back together with the `>`.
const WITH_TYPING = { undoStopBefore: false, undoStopAfter: false };

/** The active editor and the position after the `>` the user just typed into its ERB document, or undefined. */
function typedGreaterThan(e) {
  if (!isErb(e.document.languageId) || e.reason !== undefined) return;
  if (e.contentChanges.length !== 1 || e.contentChanges[0].text !== ">") return;
  const editor = vscode.window.activeTextEditor;
  if (editor?.document !== e.document || editor.selections.length !== 1) return;
  return { editor, at: e.contentChanges[0].range.start.translate(0, 1) };
}

/** Adds the end tag of the start tag that `>` completed, in an HTML view, and keeps the cursor before it. */
async function closeHtmlTag(editor, at, lineAfter) {
  const doc = editor.document;
  if (!isHtmlView(doc.fileName, () => doc.getText())) return;
  if (!settings().get("erb.autoCloseTags", true)) return;
  const name = tagToClose(
    doc.getText(new vscode.Range(0, 0, at.line, at.character)),
  );
  if (!name || lineAfter.startsWith(`</${name}`)) return;
  const inserted = await editor.edit(
    (b) => b.insert(at, `</${name}>`),
    WITH_TYPING,
  );
  if (inserted) editor.selection = new vscode.Selection(at, at);
}

async function afterTyping(e) {
  const typed = typedGreaterThan(e);
  if (!typed) return;
  const { editor, at } = typed;
  const { text } = e.document.lineAt(at.line);
  const lineAfter = text.slice(at.character);
  const doubled = doubledErbClose(text.slice(0, at.character), lineAfter);
  if (doubled) {
    await editor.edit(
      (b) => b.delete(new vscode.Range(at, at.translate(0, doubled))),
      WITH_TYPING,
    );
    return;
  }
  await closeHtmlTag(editor, at, lineAfter);
}

/**
 * Typing `>` in ERB: drops the ` %>` auto-closing added when the user types the tag's own `%>`, and in HTML views
 * adds a start tag's end tag, as VS Code does for plain HTML.
 */
function registerErbAutoClose(context) {
  context.subscriptions.push(
    vscode.workspace.onDidChangeTextDocument((e) =>
      afterTyping(e).catch((err) => log(`auto-close: ${err?.message ?? err}`)),
    ),
  );
}

export { registerErbAutoClose };
