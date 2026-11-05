// Pure Slim editing helpers; kept free of `vscode` so `test/slim.js` can run them under plain Node.
import path from "path";

import { leadingWhitespace } from "./text.js";
import { viewsRoot } from "./views.js";

// `/!` (HTML comment) and `/[` (conditional comment) are not code markers, nor is a `-#` Ruby comment.
// Only output lines take `<`/`>` modifiers; a line-start `='x'` is Ruby `'x'` in Slim 5.
const MARKER_RE = /^(\s*)(?:(==?)([<>]*)|(-(?!#)|\/(?![![])))\s?(.*)$/;
// Lines whose deeper-indented children are not Slim: engines (`javascript:`), comments, text blocks.
const VERBATIM_PARENT_RE = /^\s*([a-z]\w*(\s.*)?:\s*$|\/|[|'])/;
// Line starts that make a whole-line `= ...` wrong: markers, text, comments, inline HTML.
const SLIM_LINE_START_RE = /^[-=|'\/<>]/;
const NEXT_MARKER = { "-": "=", "=": "==", "==": "/", "/": "-" };

/** Cycle a line's leading `-` → `=` → `==` → `/` → `-`; null when the line has no marker. */
function toggleSlimMarker(line) {
  const m = MARKER_RE.exec(line);
  if (!m) return null;
  const [, indent, output, modifiers, other, body] = m;
  const next = NEXT_MARKER[output ?? other];
  const keep = next.startsWith("=") ? (modifiers ?? "") : "";
  return `${indent}${next}${keep} ${body}`.trimEnd();
}

/** Whether line `n` sits inside an engine, comment or text block, where `-`/`=` are not Slim markers. */
function inVerbatimBlock(lines, n) {
  let indent = leadingWhitespace(lines[n]).length;
  for (let i = n - 1; i >= 0 && indent > 0; i--) {
    if (!lines[i].trim()) continue;
    const own = leadingWhitespace(lines[i]).length;
    if (own >= indent) continue;
    if (VERBATIM_PARENT_RE.test(lines[i])) return true;
    indent = own;
  }
  return false;
}

/** `= text` when the selection is a whole line's plain content, else inline `#{text}`. */
function wrapSlim(text, wholeLine) {
  return wholeLine && !SLIM_LINE_START_RE.test(text)
    ? `= ${text}`
    : `#{${text}}`;
}

/**
 * Where `render "name"` from `file` looks for the partial: next to `file` for a bare name,
 * under the `app/views` root for `dir/name`. Null when `dir/name` is used outside app/views.
 * Keeps the file's format (`feed.xml.slim` → `_x.xml.slim`) minus any `+variant`, else `html`.
 */
function partialPath(file, name) {
  const slash = name.lastIndexOf("/");
  const format =
    /\.([^.+]+)(?:\+[^.]*)?\.slim$/.exec(path.basename(file))?.[1] ?? "html";
  const partial = `_${name.slice(slash + 1)}.${format}.slim`;
  if (slash < 0) return path.join(path.dirname(file), partial);
  const root = viewsRoot(file);
  return root ? path.join(root, name.slice(0, slash), partial) : null;
}

export { toggleSlimMarker, inVerbatimBlock, wrapSlim, partialPath };
