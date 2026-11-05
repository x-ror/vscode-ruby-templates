// Erubi's tag syntax: openers <%, <%=, <%==, <%-, <%#, <%% (literal); closers %>, -%>, =%>.
const ERB_RE = /<%(==|=|#|-|%)?([\s\S]*?)([-=]?)%>/g;

// Toggle cycle: code (<% / <%-) -> output (<%= / <%==) -> comment (<%#) -> code (<%).
// The opener's `-` trim can't combine with `=` or `#` in Erubi, so it is dropped;
// the closer (%>, -%>, =%>) is kept verbatim.
const NEXT_KIND = { "": "=", "-": "=", "=": "#", "==": "#", "#": "" };

/** Edit replacing the opener of the ERB tag containing `offset` with its next kind, or null.
 *  Only the opener changes, so the body's layout and the cursor are left alone. */
function toggleErbTagAt(text, offset) {
  for (const m of text.matchAll(ERB_RE)) {
    if (m.index > offset) return null;
    if (offset > m.index + m[0].length) continue;
    const [, kind = "", body] = m;
    if (kind === "%") return null;
    const space = /^\s/.test(body) ? "" : " ";
    return {
      start: m.index,
      end: m.index + 2 + kind.length,
      replacement: `<%${NEXT_KIND[kind]}${space}`,
    };
  }
  return null;
}

function countNewlines(text, from, to) {
  let count = 0;
  for (
    let i = text.indexOf("\n", from);
    i !== -1 && i < to;
    i = text.indexOf("\n", i + 1)
  )
    count++;
  return count;
}

/** Line numbers up to `lastLine` strictly between a code or output tag's `<%` line and its `%>` line. */
function erbTagBodyLines(text, lastLine) {
  const lines = new Set();
  let offset = 0;
  let line = 0;
  for (const m of text.matchAll(ERB_RE)) {
    const opener = line + countNewlines(text, offset, m.index);
    if (opener > lastLine) break;
    const closerStart = m.index + m[0].length - 2 - m[3].length;
    const closer = opener + countNewlines(text, m.index, closerStart);
    offset = closerStart;
    line = closer;
    if (m[1] === "#" || m[1] === "%") continue;
    for (let body = opener + 1; body < closer; body++) lines.add(body);
  }
  return lines;
}

const COMMENT_RE = /^(\s*)#(?!\{)( ?)/;

function commonIndent(lines) {
  return lines
    .map((l) => l.match(/^\s*/)[0])
    .reduce((shared, indent) => {
      let n = 0;
      while (n < shared.length && shared[n] === indent[n]) n++;
      return shared.slice(0, n);
    });
}

/** Per group of lines: the edits (`{ index, start, end, text }` per non-blank line) that add Ruby `# `
 *  at the group's common indent, or remove it when every non-blank line has it. */
function toggleRubyComment(groups) {
  return groups.map((lines) => {
    const filled = lines.filter((l) => l.trim());
    const uncomment = filled.every((l) => COMMENT_RE.test(l));
    const indent = uncomment ? "" : commonIndent(filled);
    return lines.flatMap((l, index) => {
      if (!l.trim()) return [];
      if (!uncomment)
        return [
          { index, start: indent.length, end: indent.length, text: "# " },
        ];
      const [match, lead] = l.match(COMMENT_RE);
      return [{ index, start: lead.length, end: match.length, text: "" }];
    });
  });
}

export { toggleErbTagAt, erbTagBodyLines, toggleRubyComment };
