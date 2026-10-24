'use strict';
// Pure Haml text helpers (no vscode dependency, so test/haml.js can run them under plain node).

const RUBY_PREFIX_RE = /^([ \t]*)(-#|[!&]?~|[!&]=(?!=)|=(?!=)|-)[ \t]?(.*)$/;
// Same cycle as the ERB toggle: code → output → comment → code.
const NEXT_PREFIX = { '-': '=', '=': '-#', '-#': '-' };
// Lines nested under these aren't Haml structure: filter bodies and `-#` comment bodies.
const VERBATIM_HEAD_RE = /^(:\w+\s*$|-#)/;
const TAG_HEAD_RE = /^[%.#][\w:.#-]*/;
const RUBY_LINE_RE = /^([-=~]|[!&][=~])/;
const leadingWs = line => /^[ \t]*/.exec(line)[0];

/** The line with its `-` / `=` / `-#` prefix moved one step along the cycle, or null if it has none. */
function toggleHamlPrefix(line) {
  const m = RUBY_PREFIX_RE.exec(line);
  if (!m) return null;
  const [, indent, prefix, body] = m;
  const next = NEXT_PREFIX[prefix] ?? '-#';
  return body ? `${indent}${next} ${body}` : `${indent}${next}`;
}

/**
 * Toggled text for each of the `targets` line numbers, as [line, text] pairs. Lines inside filter or
 * comment bodies are left alone, and so is a `-#` with nested lines, which would turn its body into code.
 */
function toggleHamlLines(lines, targets) {
  const nextBodyLine = i => lines.slice(i + 1).find(l => l.trim());
  const edits = [];
  let bodyOf; // width of the verbatim head whose body we're in
  lines.forEach((line, i) => {
    if (!line.trim()) return;
    const width = leadingWs(line).length;
    if (bodyOf !== undefined && width > bodyOf) return;
    const body = line.slice(width);
    bodyOf = VERBATIM_HEAD_RE.test(body) ? width : undefined;
    if (!targets.has(i)) return;
    if (body.startsWith('-#') && leadingWs(nextBodyLine(i) ?? '').length > width) return;
    const next = toggleHamlPrefix(line);
    if (next !== null) edits.push([i, next]);
  });
  return edits;
}

/**
 * Bracket depth after scanning `text` from `from`, skipping quoted strings. With `untilClosed`, stops
 * right after the bracket that brings the depth back to 0 and returns its end index.
 */
function scanBrackets(text, { depth = 0, from = 0, untilClosed = false } = {}) {
  let quote;
  for (let i = from; i < text.length; i++) {
    const c = text[i];
    if (quote) {
      if (c === '\\') i++;
      else if (c === quote) quote = undefined;
    } else if (c === '"' || c === "'") quote = c;
    else if ('{(['.includes(c)) depth++;
    else if (')}]'.includes(c)) {
      depth--;
      if (untilClosed && depth === 0) return { depth, end: i + 1 };
    }
  }
  return { depth, end: text.length };
}

/** A tag line's `{}` / `()` / `[]` groups: the depth an unclosed one leaves open, and where they end. */
function attributeGroups(body) {
  const head = TAG_HEAD_RE.exec(body);
  if (!head) return { depth: 0, end: -1 };
  let end = head[0].length;
  while ('{(['.includes(body[end] ?? ' ')) {
    const group = scanBrackets(body, { from: end, untilClosed: true });
    if (group.depth > 0) return { depth: group.depth, end: -1 };
    end = group.end;
  }
  return { depth: 0, end };
}

/** Whether the next line belongs to this one: Haml `|` multiline, or Ruby continued after a trailing `,`. */
function continuesOnNextLine(body) {
  if (/\s\|\s*$/.test(body) && !/\bdo\s*\|[^|]*\|\s*$/.test(body)) return true;
  if (!/,\s*$/.test(body)) return false;
  if (RUBY_LINE_RE.test(body)) return true;
  const { end } = attributeGroups(body);
  return end >= 0 && /^[<>&!]*=/.test(body.slice(end));
}

/**
 * Re-indent Haml so each nesting level is one indent unit, the whitespace of the first indented line
 * (as Haml itself detects it). Filter and `-#` bodies and continuation lines keep their whitespace past
 * the head's indent, so their content is unchanged. Blank structural lines become empty.
 */
function reindentHaml(lines, tabSize) {
  const widthOf = ws => [...ws].reduce((w, c) => w + (c === '\t' ? tabSize : 1), 0);
  let unit;
  const levels = [0];
  let verbatim; // { ws, indent, width } of the head whose nested lines move with it
  let follow; // { ws, indent, depth } while the previous line continues onto this one
  const moved = (line, head) => line.startsWith(head.ws) ? head.indent + line.slice(head.ws.length) : line;
  return lines.map(line => {
    const ws = leadingWs(line);
    const body = line.slice(ws.length);
    if (follow) {
      if (!body) return line;
      const { depth } = scanBrackets(body, { depth: follow.depth });
      const continued = depth > 0 || (follow.depth === 0 && /(,|\s\|)\s*$/.test(body));
      const head = follow;
      follow = continued ? { ...head, depth: Math.max(depth, 0) } : undefined;
      return moved(line, head);
    }
    const width = widthOf(ws);
    if (verbatim && !body) return line;
    if (verbatim && width > verbatim.width) return moved(line, verbatim);
    verbatim = undefined;
    if (!body) return '';

    while (levels.at(-1) > width) levels.pop();
    if (levels.at(-1) < width) levels.push(width);
    if (width > 0) unit ??= ws;
    const indent = (unit ?? '').repeat(levels.length - 1);
    const head = { ws, indent, width };
    if (VERBATIM_HEAD_RE.test(body)) verbatim = head;
    const { depth } = attributeGroups(body);
    if (depth > 0 || continuesOnNextLine(body)) follow = { ...head, depth };
    return indent + body;
  });
}

/**
 * Offenses from `haml-lint --reporter json` output, with 0-based lines. Syntax errors are reported on
 * line 0 and prefixed with the file path, which is dropped.
 */
function hamlLintOffenses(json) {
  const { files = [] } = JSON.parse(json);
  return files.flatMap(({ path: file, offenses = [] }) => offenses.map(({ severity, message, linter_name: linter, location }) => ({
    line: Math.max((location?.line ?? 1) - 1, 0),
    severity,
    linter,
    message: message.startsWith(`${file} - `) ? message.slice(file.length + 3) : message
  })));
}

module.exports = { toggleHamlPrefix, toggleHamlLines, reindentHaml, hamlLintOffenses };
