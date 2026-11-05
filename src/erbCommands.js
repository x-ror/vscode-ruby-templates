// ERB editing commands: toggle tag kind, comment lines inside a tag, wrap in output.
import * as vscode from "vscode";

import {
  erbTagBodyLines,
  toggleErbTagAt,
  toggleRubyComment,
} from "./erbTags.js";
import { selectedLines } from "./editor.js";

async function toggleErbTag() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const doc = ed.document;
  const text = doc.getText();
  // Several cursors in one tag must yield one edit; overlapping edits make ed.edit throw.
  const done = new Set();
  await ed.edit((eb) => {
    for (const sel of ed.selections) {
      const tag = toggleErbTagAt(text, doc.offsetAt(sel.active));
      if (!tag || done.has(tag.start)) continue;
      done.add(tag.start);
      eb.replace(
        new vscode.Range(doc.positionAt(tag.start), doc.positionAt(tag.end)),
        tag.replacement,
      );
    }
  });
}

// The built-in toggle reads the language at column 1, which is the ERB host on an indented line.
async function toggleErbLineComment() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const doc = ed.document;
  const lastLine = Math.max(...ed.selections.map((sel) => sel.end.line));
  const body = erbTagBodyLines(doc.getText(), lastLine);
  // One group per selection, like the built-in: each decides add or remove on its own.
  const claimed = new Set();
  const groups = ed.selections
    .map((sel) =>
      selectedLines(sel).filter(
        (line) => !claimed.has(line) && claimed.add(line),
      ),
    )
    .filter((group) => group.length);
  if (!groups.length || !groups.flat().every((line) => body.has(line))) {
    return vscode.commands.executeCommand("editor.action.commentLine");
  }
  const edits = toggleRubyComment(
    groups.map((group) => group.map((line) => doc.lineAt(line).text)),
  );
  await ed.edit((eb) => {
    groups.forEach((group, g) => {
      for (const { index, start, end, text } of edits[g]) {
        const line = group[index];
        eb.replace(new vscode.Range(line, start, line, end), text);
      }
    });
  });
}

async function wrapInErb() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  await ed.edit((eb) => {
    for (const sel of ed.selections) {
      const text = ed.document.getText(sel);
      eb.replace(sel, text ? `<%= ${text} %>` : "<%=  %>");
    }
  });
  if (ed.selections.length === 1 && ed.selection.isEmpty) {
    const p = ed.selection.active.translate(0, -3);
    ed.selection = new vscode.Selection(p, p);
  }
}

export { toggleErbTag, toggleErbLineComment, wrapInErb };
