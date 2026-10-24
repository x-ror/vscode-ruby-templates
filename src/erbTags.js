'use strict';

// Erubi's tag syntax: openers <%, <%=, <%==, <%-, <%#, <%% (literal); closers %>, -%>, =%>.
const ERB_RE = /<%(==|=|#|-|%)?([\s\S]*?)([-=]?)%>/g;

// Toggle cycle: code (<% / <%-) -> output (<%= / <%==) -> comment (<%#) -> code (<%).
// The opener's `-` trim can't combine with `=` or `#` in Erubi, so it is dropped;
// the closer (%>, -%>, =%>) is kept verbatim.
const NEXT_KIND = { '': '=', '-': '=', '=': '#', '==': '#', '#': '' };

/** Edit replacing the opener of the ERB tag containing `offset` with its next kind, or null.
 *  Only the opener changes, so the body's layout and the cursor are left alone. */
function toggleErbTagAt(text, offset) {
  for (const m of text.matchAll(ERB_RE)) {
    if (m.index > offset) return null;
    if (offset > m.index + m[0].length) continue;
    const [, kind = '', body] = m;
    if (kind === '%') return null;
    const space = /^\s/.test(body) ? '' : ' ';
    return { start: m.index, end: m.index + 2 + kind.length, replacement: `<%${NEXT_KIND[kind]}${space}` };
  }
  return null;
}

module.exports = { toggleErbTagAt };
