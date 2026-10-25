// Pure Haml text helpers (no vscode dependency, so test/haml.js can run them under plain node).

const RUBY_PREFIX_RE = /^([ \t]*)(-#|[!&]?~|[!&]=(?!=)|=(?!=)|-)[ \t]?(.*)$/;
// Same cycle as the ERB toggle: code → output → comment → code.
const NEXT_PREFIX = { "-": "=", "=": "-#", "-#": "-" };
// Lines nested under these aren't Haml structure: filter bodies and `-#` comment bodies.
const VERBATIM_HEAD_RE = /^(:\w+\s*$|-#)/;
const leadingWs = (line) => /^[ \t]*/.exec(line)[0];

/** The line with its `-` / `=` / `-#` prefix moved one step along the cycle, or null if it has none. */
function toggleHamlPrefix(line) {
  const m = RUBY_PREFIX_RE.exec(line);
  if (!m) return null;
  const [, indent, prefix, body] = m;
  const next = NEXT_PREFIX[prefix] ?? "-#";
  return body ? `${indent}${next} ${body}` : `${indent}${next}`;
}

/**
 * Toggled text for each of the `targets` line numbers, as [line, text] pairs. Lines inside filter or
 * comment bodies are left alone, and so is a `-#` with nested lines, which would turn its body into code.
 */
function toggleHamlLines(lines, targets) {
  const nextBodyLine = (i) => lines.slice(i + 1).find((l) => l.trim());
  const edits = [];
  let bodyOf; // width of the verbatim head whose body we're in
  lines.forEach((line, i) => {
    if (!line.trim()) return;
    const width = leadingWs(line).length;
    if (bodyOf !== undefined && width > bodyOf) return;
    const body = line.slice(width);
    bodyOf = VERBATIM_HEAD_RE.test(body) ? width : undefined;
    if (!targets.has(i)) return;
    if (
      body.startsWith("-#") &&
      leadingWs(nextBodyLine(i) ?? "").length > width
    )
      return;
    const next = toggleHamlPrefix(line);
    if (next !== null) edits.push([i, next]);
  });
  return edits;
}

/**
 * Offenses from `haml-lint --reporter json` output, with 0-based lines. Syntax errors are reported on
 * line 0 and prefixed with the file path, which is dropped.
 */
function hamlLintOffenses(json) {
  const { files = [] } = JSON.parse(json);
  return files.flatMap(({ path: file, offenses = [] }) =>
    offenses.map(({ severity, message, linter_name: linter, location }) => ({
      line: Math.max((location?.line ?? 1) - 1, 0),
      severity,
      linter,
      message: message.startsWith(`${file} - `)
        ? message.slice(file.length + 3)
        : message,
    })),
  );
}

export { toggleHamlPrefix, toggleHamlLines, hamlLintOffenses };
