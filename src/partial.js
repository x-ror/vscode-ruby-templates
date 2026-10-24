"use strict";
// Pure part of "Extract Selection to Partial" (no vscode dependency; tested in test/haml.js).

const leadingWs = (line) => /^[ \t]*/.exec(line)[0];

/**
 * The block to move into a partial, given the document lines and a selection
 * `{ start: { line, character }, end: { line, character } }`. `wholeLines` (Haml, Slim) widens it to
 * full lines; otherwise it starts at column 0 when only indentation precedes it. A selection ending at
 * column 0 stops at the end of the previous line, keeping that newline in the template.
 * Returns the range to replace, the dedented text and the indentation it had, or null if it is blank.
 */
function extractBlock(lines, { start, end }, wholeLines) {
  const endsAtLineStart = end.character === 0 && end.line > start.line;
  const endLine = endsAtLineStart ? end.line - 1 : end.line;
  const onlyIndentBefore = !lines[start.line].slice(0, start.character).trim();
  const range = {
    start: {
      line: start.line,
      character: wholeLines || onlyIndentBefore ? 0 : start.character,
    },
    end: {
      line: endLine,
      character:
        wholeLines || endsAtLineStart ? lines[endLine].length : end.character,
    },
  };
  const picked = lines.slice(range.start.line, endLine + 1);
  picked[picked.length - 1] = picked.at(-1).slice(0, range.end.character);
  picked[0] = picked[0].slice(range.start.character);
  const indents = picked.filter((l) => l.trim()).map(leadingWs);
  if (!indents.length) return null;
  const width = Math.min(...indents.map((ws) => ws.length));
  const indent = indents.find((ws) => ws.length === width);
  const text = picked
    .map((l) => l.slice(Math.min(width, leadingWs(l).length)))
    .join("\n");
  return { range, indent, text: `${text}\n` };
}

/** `_name` plus every extension of the template: `show.html+phone.haml` → `_name.html+phone.haml`. */
function partialFileName(templateName, name) {
  const dot = templateName.indexOf(".");
  return `_${name}${dot < 0 ? "" : templateName.slice(dot)}`;
}

module.exports = { extractBlock, partialFileName };
