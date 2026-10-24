'use strict';
// Pure Slim editing helpers; kept free of `vscode` so `test/slim.js` can run them under plain Node.
const path = require('path');

// `/!` (HTML comment) and `/[` (conditional comment) are not code markers. Only output lines take
// `<`/`>` modifiers; a line-start `='x'` is Ruby `'x'` in Slim 5.
const MARKER_RE = /^(\s*)(?:(==?)([<>]*)|(-|\/(?![![])))\s?(.*)$/;
const NEXT_MARKER = { '-': '=', '=': '==', '==': '/', '/': '-' };

/** Cycle a line's leading `-` → `=` → `==` → `/` → `-`; null when the line has no marker. */
function toggleSlimMarker(line) {
  const m = MARKER_RE.exec(line);
  if (!m) return null;
  const [, indent, output, modifiers, other, body] = m;
  const next = NEXT_MARKER[output ?? other];
  const keep = next.startsWith('=') ? modifiers ?? '' : '';
  return `${indent}${next}${keep} ${body}`.trimEnd();
}

/** `= text` when the selection is a whole line's content, else inline `#{text}`. */
function wrapSlim(text, wholeLine) {
  return wholeLine ? `= ${text}` : `#{${text}}`;
}

/** Strip the common leading whitespace of the non-blank lines. */
function dedent(lines) {
  const indents = lines.filter(l => l.trim()).map(l => /^\s*/.exec(l)[0]);
  const indent = indents.reduce((a, b) => (b.length < a.length ? b : a), indents[0] ?? '');
  return { indent, lines: lines.map(l => l.slice(Math.min(indent.length, /^\s*/.exec(l)[0].length))) };
}

/**
 * Where `render "name"` from `file` looks for the partial: next to `file` for a bare name,
 * under the `app/views` root for `dir/name`. Null when `dir/name` is used outside app/views.
 */
function partialPath(file, name) {
  const slash = name.lastIndexOf('/');
  const partial = `_${name.slice(slash + 1)}.html.slim`;
  if (slash < 0) return path.join(path.dirname(file), partial);
  const views = /^(.*[/\\]app[/\\]views)[/\\]/.exec(file);
  return views ? path.join(views[1], name.slice(0, slash), partial) : null;
}

module.exports = { toggleSlimMarker, wrapSlim, dedent, partialPath };
