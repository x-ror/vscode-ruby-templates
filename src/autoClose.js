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
// A start tag that just got its `>`: name, then attributes, quoted values or whole ERB tags. Each alternative
// starts with a different character and ERB can't run past its own `%>`, so matching stays linear.
const START_TAG =
  /<([a-zA-Z][\w:-]*)(?:\s(?:[^<>"'%]|%(?!>)|"[^"]*"|'[^']*'|<%[^%]*(?:%(?!>)[^%]*)*%>)*)?>$/;
const RAW_TEXT = ["script", "style", "textarea"];

/** Whether the last `open` in `text` comes after its last `close`. */
const stillOpen = (text, open, close) =>
  text.lastIndexOf(open) > text.lastIndexOf(close);

/**
 * The element to close after the `>` that ends `textBefore` (the document up to the cursor), or null: not after a
 * void or self-closing tag, inside an ERB tag (`<% if a > b`), an HTML comment, or a `<script>`/`<style>` body.
 */
function tagToClose(textBefore) {
  if (textBefore.endsWith("/>")) return null;
  const lineStart = textBefore.lastIndexOf("\n") + 1;
  const match = START_TAG.exec(textBefore.slice(lineStart));
  if (!match || VOID.has(match[1].toLowerCase())) return null;
  // The context is the one the start tag opens in, so a just-typed `<script>` doesn't count as its own body.
  const prefix = textBefore.slice(0, lineStart + match.index);
  const lower = prefix.toLowerCase();
  if (stillOpen(prefix, "<%", "%>") || stillOpen(prefix, "<!--", "-->"))
    return null;
  if (RAW_TEXT.some((t) => stillOpen(lower, `<${t}`, `</${t}`))) return null;
  return match[1];
}

/** Views whose output is HTML, by the name the server's `html_host` uses: `show.html.erb`, `x.html+phone.erb`, `x.rhtml`. */
const isHtmlView = (fileName) =>
  /\.(html|turbo_stream)(\+[\w-]+)?\.(erb|erubi|erubis)$|\.rhtml$/i.test(
    fileName,
  );

export { tagToClose, isHtmlView };
