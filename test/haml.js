import assert from "assert";
import {
  toggleHamlPrefix,
  toggleHamlLines,
  hamlLintOffenses,
} from "../src/haml.js";
import { extractBlock, partialFileName } from "../src/partial.js";

// toggleHamlPrefix: - → = → -# → -, other output prefixes become comments, == text and plain lines are left alone.
assert.strictEqual(toggleHamlPrefix("  - x = 1"), "  = x = 1");
assert.strictEqual(
  toggleHamlPrefix("  = link_to 'a', b"),
  "  -# link_to 'a', b",
);
assert.strictEqual(toggleHamlPrefix("  -# note"), "  - note");
assert.strictEqual(toggleHamlPrefix("-#"), "-");
assert.strictEqual(toggleHamlPrefix("!= raw x"), "-# raw x");
assert.strictEqual(toggleHamlPrefix("&~ x"), "-# x");
assert.strictEqual(toggleHamlPrefix("== hi #{x}"), null);
assert.strictEqual(toggleHamlPrefix("%p= x"), null);
assert.strictEqual(toggleHamlPrefix("plain"), null);
// toggleHamlLines: filter/comment bodies and a `-#` with nested lines are skipped.
const toggleDoc = [":ruby", "  - x", "-#", "  - y", "-# z", "- w"];
assert.deepStrictEqual(
  toggleHamlLines(toggleDoc, new Set([0, 1, 2, 3, 4, 5])),
  [
    [4, "- z"],
    [5, "= w"],
  ],
);

// extractBlock: whole lines for Haml; ERB keeps the selection but drops a trailing column-0 line and leading indentation.
const sel = (sl, sc, el, ec) => ({
  start: { line: sl, character: sc },
  end: { line: el, character: ec },
});
const pos = (line, character) => ({ line, character });
assert.deepStrictEqual(
  extractBlock(
    ["%div", "  .card", "    %h2= x", "  %p"],
    sel(1, 3, 2, 5),
    true,
  ),
  {
    range: { start: pos(1, 0), end: pos(2, 10) },
    indent: "  ",
    text: ".card\n  %h2= x\n",
  },
);
assert.deepStrictEqual(
  extractBlock(
    ["<div>", "  <p><%= a %></p>", "</div>"],
    sel(1, 0, 2, 0),
    false,
  ),
  {
    range: { start: pos(1, 0), end: pos(1, 17) },
    indent: "  ",
    text: "<p><%= a %></p>\n",
  },
);
assert.deepStrictEqual(
  extractBlock(
    ["<div>", "  <ul>", "    <li>a</li>", "  </ul>"],
    sel(1, 2, 3, 7),
    false,
  ),
  {
    range: { start: pos(1, 0), end: pos(3, 7) },
    indent: "  ",
    text: "<ul>\n  <li>a</li>\n</ul>\n",
  },
);
assert.deepStrictEqual(
  extractBlock(["<p>Hi <%= a %> there</p>"], sel(0, 6, 0, 14), false),
  {
    range: { start: pos(0, 6), end: pos(0, 14) },
    indent: "",
    text: "<%= a %>\n",
  },
);
assert.strictEqual(
  extractBlock(["%div", "   ", "%p"], sel(1, 0, 1, 3), true),
  null,
);
assert.strictEqual(
  partialFileName("show.html+phone.haml", "card"),
  "_card.html+phone.haml",
);
assert.strictEqual(partialFileName("Gemfile", "card"), "_card");

// hamlLintOffenses: shape of haml-lint 0.78 `--reporter json`.
const json = JSON.stringify({
  metadata: { haml_lint_version: "0.78.0" },
  files: [
    {
      path: "app/views/x.html.haml",
      offenses: [
        {
          severity: "warning",
          message:
            "`%div.foo` can be written as `.foo` since `%div` is implicit",
          corrected: false,
          correctable: true,
          location: { line: 3 },
          linter_name: "ImplicitDiv",
        },
        {
          severity: "error",
          message: "app/views/x.html.haml - Unbalanced brackets.",
          corrected: false,
          correctable: false,
          location: { line: 0 },
          linter_name: "Syntax",
        },
      ],
    },
  ],
  summary: { offense_count: 2 },
});
assert.deepStrictEqual(hamlLintOffenses(json), [
  {
    line: 2,
    severity: "warning",
    linter: "ImplicitDiv",
    message: "`%div.foo` can be written as `.foo` since `%div` is implicit",
  },
  {
    line: 0,
    severity: "error",
    linter: "Syntax",
    message: "Unbalanced brackets.",
  },
]);
assert.deepStrictEqual(hamlLintOffenses(JSON.stringify({ files: [] })), []);

console.log("haml helpers ok");
