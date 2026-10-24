const assert = require("assert");
const { toggleHamlPrefix, toggleHamlLines, reindentHaml, hamlLintOffenses } = require("../src/haml");
const { extractBlock, partialFileName } = require("../src/partial");

// toggleHamlPrefix: - → = → -# → -, other output prefixes become comments, == text and plain lines are left alone.
assert.strictEqual(toggleHamlPrefix("  - x = 1"), "  = x = 1");
assert.strictEqual(toggleHamlPrefix("  = link_to 'a', b"), "  -# link_to 'a', b");
assert.strictEqual(toggleHamlPrefix("  -# note"), "  - note");
assert.strictEqual(toggleHamlPrefix("-#"), "-");
assert.strictEqual(toggleHamlPrefix("!= raw x"), "-# raw x");
assert.strictEqual(toggleHamlPrefix("&~ x"), "-# x");
assert.strictEqual(toggleHamlPrefix("== hi #{x}"), null);
assert.strictEqual(toggleHamlPrefix("%p= x"), null);
assert.strictEqual(toggleHamlPrefix("plain"), null);
// toggleHamlLines: filter/comment bodies and a `-#` with nested lines are skipped.
const toggleDoc = [":ruby", "  - x", "-#", "  - y", "-# z", "- w"];
assert.deepStrictEqual(toggleHamlLines(toggleDoc, new Set([0, 1, 2, 3, 4, 5])), [[4, "- z"], [5, "= w"]]);

// reindentHaml: levels follow the first indented line's unit; filter/comment bodies and continuations keep their offsets.
const reindent = src => reindentHaml(src.split("\n"), 2).join("\n");
assert.strictEqual(
  reindent(["%div", "    %p", "        %b x", "    %p y", "  %i odd", "", "%footer"].join("\n")),
  ["%div", "    %p", "        %b x", "    %p y", "    %i odd", "", "%footer"].join("\n"));
assert.strictEqual(
  reindent(["%ul", "  - each do |x|", "      %li= x", "        -# deep"].join("\n")),
  ["%ul", "  - each do |x|", "    %li= x", "      -# deep"].join("\n"));
assert.strictEqual(
  reindent(["%div", "    :javascript", "      if (a) {", "          b();", "      }", "    %p"].join("\n")),
  ["%div", "    :javascript", "      if (a) {", "          b();", "      }", "    %p"].join("\n"));
assert.strictEqual(
  reindent(["%div", "  %section", "      :css", "        a { b: c }", "      %p"].join("\n")),
  ["%div", "  %section", "    :css", "      a { b: c }", "    %p"].join("\n"));
assert.strictEqual(
  reindent(["%div", "  %div", "      %a{href: x,", "         title: y} link", "      = link_to 'x',", "            class: 'y'", "      %p"].join("\n")),
  ["%div", "  %div", "    %a{href: x,", "       title: y} link", "    = link_to 'x',", "          class: 'y'", "    %p"].join("\n"));
assert.strictEqual(
  reindent(["%div", "\t%p", "\t\t%b"].join("\n")),
  ["%div", "\t%p", "\t\t%b"].join("\n"));
// Plain text ending in `,` doesn't pull the next line along.
assert.strictEqual(
  reindent(["%div", "    %p", "      Dear Sir,", "  %p"].join("\n")),
  ["%div", "    %p", "        Dear Sir,", "    %p"].join("\n"));

// Filter bodies keep their whitespace past the filter's indent (tabs, alignment, whitespace-only lines).
const unchanged = [
  ["%div", "\t:preserve", "\t\tif (a) {", "\t\t    b();", "\t\t}"],
  ["%div", "  :preserve", "    a", "    \tb"],
  ["%div", "  :preserve", "    a", "        ", "    b"],
  ["%pre", "  :plain", "    a", "          ", "      b"]
];
for (const lines of unchanged) assert.deepStrictEqual(reindentHaml(lines, 2), lines);
assert.deepStrictEqual(
  reindentHaml(["%div", "  %p", "      :preserve", "        a", "        \tb", "            ", "  %i"], 2),
  ["%div", "  %p", "    :preserve", "      a", "      \tb", "            ", "  %i"]);
// A bracket inside a string doesn't open a continuation.
assert.strictEqual(
  reindent(["%div", "    %a{title: \"(\"} x", "    %p", "        %b y"].join("\n")),
  ["%div", "    %a{title: \"(\"} x", "    %p", "        %b y"].join("\n"));
// Many attribute groups stay linear (a backtracking regex took seconds at 28).
const manyGroups = ["%div", `  %p${"{}".repeat(2000)} x,`, "  %i"];
const startedAt = Date.now();
reindentHaml(manyGroups, 2);
assert.ok(Date.now() - startedAt < 100, "reindentHaml is slow on many attribute groups");

// extractBlock: whole lines for Haml; ERB keeps the selection but drops a trailing column-0 line and leading indentation.
const sel = (sl, sc, el, ec) => ({ start: { line: sl, character: sc }, end: { line: el, character: ec } });
const pos = (line, character) => ({ line, character });
assert.deepStrictEqual(
  extractBlock(["%div", "  .card", "    %h2= x", "  %p"], sel(1, 3, 2, 5), true),
  { range: { start: pos(1, 0), end: pos(2, 10) }, indent: "  ", text: ".card\n  %h2= x\n" });
assert.deepStrictEqual(
  extractBlock(["<div>", "  <p><%= a %></p>", "</div>"], sel(1, 0, 2, 0), false),
  { range: { start: pos(1, 0), end: pos(1, 17) }, indent: "  ", text: "<p><%= a %></p>\n" });
assert.deepStrictEqual(
  extractBlock(["<div>", "  <ul>", "    <li>a</li>", "  </ul>"], sel(1, 2, 3, 7), false),
  { range: { start: pos(1, 0), end: pos(3, 7) }, indent: "  ", text: "<ul>\n  <li>a</li>\n</ul>\n" });
assert.deepStrictEqual(
  extractBlock(["<p>Hi <%= a %> there</p>"], sel(0, 6, 0, 14), false),
  { range: { start: pos(0, 6), end: pos(0, 14) }, indent: "", text: "<%= a %>\n" });
assert.strictEqual(extractBlock(["%div", "   ", "%p"], sel(1, 0, 1, 3), true), null);
assert.strictEqual(partialFileName("show.html+phone.haml", "card"), "_card.html+phone.haml");
assert.strictEqual(partialFileName("Gemfile", "card"), "_card");

// hamlLintOffenses: shape of haml-lint 0.78 `--reporter json`.
const json = JSON.stringify({
  metadata: { haml_lint_version: "0.78.0" },
  files: [{
    path: "app/views/x.html.haml",
    offenses: [
      { severity: "warning", message: "`%div.foo` can be written as `.foo` since `%div` is implicit", corrected: false, correctable: true, location: { line: 3 }, linter_name: "ImplicitDiv" },
      { severity: "error", message: "app/views/x.html.haml - Unbalanced brackets.", corrected: false, correctable: false, location: { line: 0 }, linter_name: "Syntax" }
    ]
  }],
  summary: { offense_count: 2 }
});
assert.deepStrictEqual(hamlLintOffenses(json), [
  { line: 2, severity: "warning", linter: "ImplicitDiv", message: "`%div.foo` can be written as `.foo` since `%div` is implicit" },
  { line: 0, severity: "error", linter: "Syntax", message: "Unbalanced brackets." }
]);
assert.deepStrictEqual(hamlLintOffenses(JSON.stringify({ files: [] })), []);

console.log("haml helpers ok");
