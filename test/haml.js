import assert from "assert";
import { toggleHamlPrefix, toggleHamlLines } from "../src/haml.js";
import path from "path";
import {
  extractBlock,
  partialFileName,
  partialPath,
  renderPath,
} from "../src/partial.js";

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
    lines: [".card", "  %h2= x"],
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
    lines: ["<p><%= a %></p>"],
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
    lines: ["<ul>", "  <li>a</li>", "</ul>"],
  },
);
assert.deepStrictEqual(
  extractBlock(["<p>Hi <%= a %> there</p>"], sel(0, 6, 0, 14), false),
  {
    range: { start: pos(0, 6), end: pos(0, 14) },
    indent: "",
    lines: ["<%= a %>"],
  },
);
assert.strictEqual(
  extractBlock(["%div", "   ", "%p"], sel(1, 0, 1, 3), true),
  null,
);
// Tabs and spaces don't mix: the common indent is compared as a string.
assert.deepStrictEqual(
  extractBlock(["\t\t%p", "    %q"], sel(0, 0, 1, 6), true).lines,
  ["\t\t%p", "    %q"],
);
assert.strictEqual(
  partialFileName("show.html+phone.haml", "card"),
  "_card.html.haml",
);
assert.strictEqual(partialFileName("Gemfile", "card"), "_card");
assert.strictEqual(
  partialFileName("show.html.erb.tt", "card"),
  "_card.html.erb.tt",
);

// partialPath joins with the platform separator, so the expected paths do too.
assert.strictEqual(
  partialPath("/app/views/users/show.html.slim", "card"),
  path.normalize("/app/views/users/_card.html.slim"),
);
assert.strictEqual(
  partialPath("/r/app/views/users/show.html.erb", "shared/ui/card"),
  path.normalize("/r/app/views/shared/ui/_card.html.erb"),
);
assert.strictEqual(partialPath("/tmp/show.slim", "shared/card"), null);
assert.strictEqual(
  partialPath("/app/views/posts/feed.xml.haml", "item"),
  path.normalize("/app/views/posts/_item.xml.haml"),
);
assert.strictEqual(
  partialPath("/app/views/posts/show.html+phone.slim", "item"),
  path.normalize("/app/views/posts/_item.html.slim"),
);

assert.strictEqual(
  renderPath("/r/app/views/admin/users/show.html.erb", "card"),
  "admin/users/card",
);
assert.strictEqual(
  renderPath("/r/app/views/users/show.html.erb", "shared/card"),
  "shared/card",
);
assert.strictEqual(renderPath("/r/app/views/show.html.erb", "card"), "card");
assert.strictEqual(renderPath("/tmp/show.html.erb", "card"), "card");

console.log("haml helpers ok");
