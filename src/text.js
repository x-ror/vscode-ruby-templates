// Pure text helpers shared by the editing commands (no vscode dependency).

const leadingWhitespace = (line) => /^[ \t]*/.exec(line)[0];

/** Last line a selection covers; one ending at column 0 of a later line doesn't take that line along. */
const lastSelectedLine = ({ start, end }) =>
  end.character === 0 && end.line > start.line ? end.line - 1 : end.line;

/** The leading whitespace the non-blank lines have in common (as a string, so tabs and spaces don't mix). */
function commonIndent(lines) {
  const indents = lines.filter((l) => l.trim()).map(leadingWhitespace);
  return indents.reduce((common, ws) => {
    let i = 0;
    while (i < common.length && common[i] === ws[i]) i++;
    return common.slice(0, i);
  }, indents[0] ?? "");
}

/** `lines` without their `commonIndent`, which is returned as `indent`. */
function dedent(lines) {
  const indent = commonIndent(lines);
  return {
    indent,
    lines: lines.map((l) =>
      l.startsWith(indent) ? l.slice(indent.length) : l.trimStart(),
    ),
  };
}

/** Each group of line numbers without the lines an earlier group has; groups left empty are dropped. */
function disjointLineGroups(groups) {
  const claimed = new Set();
  return groups
    .map((lines) => {
      const own = lines.filter((line) => !claimed.has(line));
      for (const line of own) claimed.add(line);
      return own;
    })
    .filter((group) => group.length);
}

export {
  leadingWhitespace,
  lastSelectedLine,
  commonIndent,
  dedent,
  disjointLineGroups,
};
