// Pure part of "Extract Selection to Partial" (no vscode dependency; tested in test/haml.js).
import path from "path";

import { dedent, lastSelectedLine } from "./text.js";
import { viewsRoot, viewDirOf } from "./views.js";

/**
 * The block to move into a partial, given the document lines and a selection
 * `{ start: { line, character }, end: { line, character } }`. `wholeLines` (Haml, Slim) widens it to
 * full lines; otherwise it starts at column 0 when only indentation precedes it. A selection ending at
 * column 0 stops at the end of the previous line, keeping that newline in the template.
 * Returns the range to replace, the dedented lines and the indentation they had, or null if they are blank.
 */
function extractBlock(lines, selection, wholeLines) {
  const { start, end } = selection;
  const endLine = lastSelectedLine(selection);
  const endsAtLineStart = endLine !== end.line;
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
  if (!picked.some((l) => l.trim())) return null;
  return { range, ...dedent(picked) };
}

/** `_name` plus the template's extensions without its `+variant`, so every variant renders it: `show.html+phone.haml` → `_name.html.haml`. */
function partialFileName(templateName, name) {
  const dot = templateName.indexOf(".");
  const extensions = dot < 0 ? "" : templateName.slice(dot);
  return `_${name}${extensions.replace(/\+[^.]*/, "")}`;
}

/**
 * Where `render "name"` from `file` looks for the partial: next to `file` for a bare name,
 * under the `app/views` root for `dir/name`. Null when `dir/name` is used outside app/views.
 */
function partialPath(file, name) {
  const slash = name.lastIndexOf("/");
  const partial = partialFileName(path.basename(file), name.slice(slash + 1));
  if (slash < 0) return path.join(path.dirname(file), partial);
  const root = viewsRoot(file);
  return root ? path.join(root, name.slice(0, slash), partial) : null;
}

/** The path to render: a bare name under the template's views folder, so any controller finds it; `dir/name` as given. */
function renderPath(file, name) {
  const dir = !name.includes("/") && viewDirOf(file);
  return dir ? `${dir}/${name}` : name;
}

export { extractBlock, partialFileName, partialPath, renderPath };
