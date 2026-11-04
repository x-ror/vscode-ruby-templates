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

// The server's `html::TAGS`.
const HTML_TAGS = new Set(
  "a abbr article aside audio b blockquote body br button canvas caption code col colgroup dd details dialog div dl dt em fieldset figcaption figure footer form h1 h2 h3 h4 h5 h6 head header hr html i iframe img input label legend li link main meta nav noscript ol optgroup option p picture pre progress script section select small source span strong style sub summary sup svg table tbody td template textarea tfoot th thead time title tr ul video".split(
    " ",
  ),
);
const VIEWS_DIR = /\/app\/views\//;
const BARE_ERB_VIEW = /^[^./]+\.(erb|erubi|erubis)$/i;
// A locale (`about.de.erb`) but not the `js` format, as the server's `is_locale` and `is_known_format` decide.
const LOCALE_ERB_VIEW =
  /^[^./]+\.(?!js\.)[a-z]{2}(?:[-_][A-Z]{2})?\.(erb|erubi|erubis)$/i;
// An ERB tag never runs past its own `%>`, so a failed match can't backtrack through the tags.
const FIRST_MARKUP = /^\s*(?:<%(?:(?!%>)[\s\S])*%>\s*)*<(!|[a-z][a-z0-9-]*)/i;
// The markup test only needs the top of the file.
const MARKUP_SCAN_LENGTH = 4096;

/** Whether the first text outside ERB tags opens an HTML element, a custom element, a comment or a doctype, as the server decides it. */
function startsWithMarkup(text) {
  const name = FIRST_MARKUP.exec(
    text.slice(0, MARKUP_SCAN_LENGTH),
  )?.[1].toLowerCase();
  return (
    name !== undefined &&
    (name === "!" || name.includes("-") || HTML_TAGS.has(name))
  );
}

/**
 * Views whose output is HTML, by the name the server's `html_host` uses: `show.html.erb`, `x.html+phone.erb`, `x.rhtml`;
 * a bare `app/views/**\/x.erb` counts when `getText()` starts with markup.
 */
function isHtmlView(fileName, getText) {
  if (
    /\.(html|turbo_stream)(\+[\w-]+)?\.(erb|erubi|erubis)$|\.rhtml$/i.test(
      fileName,
    )
  )
    return true;
  const path = fileName.replaceAll("\\", "/");
  if (!VIEWS_DIR.test(path)) return false;
  const name = path.slice(path.lastIndexOf("/") + 1);
  if (LOCALE_ERB_VIEW.test(name)) return true;
  return BARE_ERB_VIEW.test(name) && startsWithMarkup(getText());
}

/**
 * After typing the `>` of a `%>`, the length of the ` %>` that auto-closing put after the cursor when `<%` was
 * typed, or 0. VS Code only overtypes one-character closers, so a tag typed in full otherwise ends in `%> %>`.
 */
function doubledErbClose(lineBefore, lineAfter) {
  if (!lineBefore.endsWith("%>") || lineBefore.endsWith("%%>")) return 0;
  const opened = lineBefore.lastIndexOf("<%");
  if (opened === -1 || opened > lineBefore.length - 4) return 0;
  return lineAfter.match(/^ ?%>/)?.[0].length ?? 0;
}

export { tagToClose, isHtmlView, doubledErbClose, HTML_TAGS };
