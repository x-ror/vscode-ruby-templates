// Slim editing commands: cycle the line marker, wrap in output.
import * as vscode from "vscode";

import {
  INTERPOLATION_CLOSE,
  toggleSlimMarker,
  inVerbatimBlock,
  wrapSlim,
} from "./slim.js";
import { selectedLines } from "./editor.js";

async function toggleSlimTag() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const text = ed.document.getText().split(/\r?\n/);
  const lines = new Set(ed.selections.flatMap(selectedLines));
  await ed.edit((eb) => {
    for (const n of lines) {
      if (inVerbatimBlock(text, n)) continue;
      const line = ed.document.lineAt(n);
      const next = toggleSlimMarker(line.text);
      if (next !== null) eb.replace(line.range, next);
    }
  });
}

async function wrapInSlim() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const doc = ed.document;
  if (ed.selections.some((s) => !s.isSingleLine)) {
    vscode.window.showInformationMessage(
      "Wrap in Slim Output skips selections that span several lines.",
    );
  }
  await ed.edit((eb) => {
    for (const sel of ed.selections.filter((s) => s.isSingleLine)) {
      const line = doc.lineAt(sel.start.line);
      const content = line.text.trim();
      if (content && doc.getText(sel).trim() === content) {
        const start = line.firstNonWhitespaceCharacterIndex;
        eb.replace(
          new vscode.Range(
            line.lineNumber,
            start,
            line.lineNumber,
            start + content.length,
          ),
          wrapSlim(content, true),
        );
      } else {
        eb.replace(sel, wrapSlim(doc.getText(sel), false));
      }
    }
  });
  if (ed.selections.length === 1 && ed.selection.isEmpty) {
    const p = ed.selection.active.translate(0, -INTERPOLATION_CLOSE.length);
    ed.selection = new vscode.Selection(p, p);
  }
}

export { toggleSlimTag, wrapInSlim };
