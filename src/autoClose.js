// Pure part of ERB tag auto-closing (no vscode dependency; tested in test/clientPure.js).

const VOID = new Set([
  "area",
  "base",
  "br",
  "col",
  "embed",
  "hr",
  "img",
  "input",
  "link",
  "meta",
  "param",
  "source",
  "track",
  "wbr",
]);
// A start tag that just got its `>`: name, then attributes, quoted values or whole ERB tags.
const START_TAG =
  /<([a-zA-Z][\w:-]*)(?:\s(?:[^<>"']|"[^"]*"|'[^']*'|<%.*?%>)*)?>$/;

/**
 * The element to close after the `>` that ends `lineBefore`, or null: not after a void or self-closing tag,
 * nor inside an ERB tag (`<% if a > b`).
 */
function tagToClose(lineBefore) {
  const opens = lineBefore.split("<%").length - 1;
  const closes = lineBefore.split("%>").length - 1;
  if (opens > closes || lineBefore.endsWith("/>")) return null;
  const name = START_TAG.exec(lineBefore)?.[1];
  return name && !VOID.has(name.toLowerCase()) ? name : null;
}

export { tagToClose };
