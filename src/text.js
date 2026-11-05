// Pure text helpers shared by the editing commands (no vscode dependency).

/** Last line a selection covers; one ending at column 0 of a later line doesn't take that line along. */
const lastSelectedLine = ({ start, end }) =>
  end.character === 0 && end.line > start.line ? end.line - 1 : end.line;

export { lastSelectedLine };
