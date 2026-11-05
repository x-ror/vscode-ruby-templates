// Reading lines from editors and documents for the editing commands.
import { lastSelectedLine } from "./text.js";

/** Line numbers a selection covers, up to `lastSelectedLine`. */
function selectedLines(sel) {
  return Array.from(
    { length: lastSelectedLine(sel) - sel.start.line + 1 },
    (_, i) => sel.start.line + i,
  );
}

const documentLines = (doc) =>
  Array.from({ length: doc.lineCount }, (_, i) => doc.lineAt(i).text);

export { selectedLines, documentLines };
