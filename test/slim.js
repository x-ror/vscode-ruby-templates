import assert from "assert";
import fs from "fs";
import path from "path";
import {
  toggleSlimMarker,
  inVerbatimBlock,
  wrapSlim,
  dedent,
  partialPath,
} from "../src/slim.js";

for (const [line, next] of [
  ["- if x", "= if x"],
  ["  = link_to 'a', b", "  == link_to 'a', b"],
  ["==> raw x", "/ raw x"],
  ["=< x", "==< x"],
  ["/ note", "- note"],
  ["-x", "= x"],
  ["-", "="],
  ["='x'", "== 'x'"],
  ["-# note", null],
  ["/! html", null],
  ["/[if IE]", null],
  ["p = x", null],
  ["| text", null],
]) {
  assert.strictEqual(toggleSlimMarker(line), next, line);
}

assert.strictEqual(wrapSlim("user.name", true), "= user.name");
assert.strictEqual(wrapSlim("user.name", false), "#{user.name}");
assert.strictEqual(wrapSlim("| Hi name", true), "#{| Hi name}");

const tree = [
  "javascript:",
  "  - x",
  "p",
  "  - y",
  "/ note",
  "  = z",
  "| text",
  "  - w",
  "- if a",
  "  = b",
];
assert.deepStrictEqual(
  tree.map((_, i) => inVerbatimBlock(tree, i)),
  [false, true, false, false, false, true, false, true, false, false],
);

assert.deepStrictEqual(dedent(["    .card", "", "      p = x", "\t"]), {
  indent: "    ",
  lines: [".card", "", "  p = x", ""],
});
assert.deepStrictEqual(dedent(["\t\tp", "    q"]), {
  indent: "",
  lines: ["\t\tp", "    q"],
});
assert.deepStrictEqual(dedent(["  \ta", "  b"]), {
  indent: "  ",
  lines: ["\ta", "b"],
});

// partialPath joins with the platform separator, so the expected paths do too.
assert.strictEqual(
  partialPath("/app/views/users/show.html.slim", "card"),
  path.normalize("/app/views/users/_card.html.slim"),
);
assert.strictEqual(
  partialPath("/r/app/views/users/show.slim", "shared/ui/card"),
  path.normalize("/r/app/views/shared/ui/_card.html.slim"),
);
assert.strictEqual(partialPath("/tmp/show.slim", "shared/card"), null);
assert.strictEqual(
  partialPath("/app/views/posts/feed.xml.slim", "item"),
  path.normalize("/app/views/posts/_item.xml.slim"),
);
assert.strictEqual(
  partialPath("/app/views/posts/show.html+phone.slim", "item"),
  path.normalize("/app/views/posts/_item.html.slim"),
);

const { onEnterRules } = JSON.parse(
  fs.readFileSync(
    new URL("../language-configuration/slim.json", import.meta.url),
    "utf8",
  ),
);
const enterAction = (line) =>
  onEnterRules.find((r) => new RegExp(r.beforeText).test(line))?.action.indent;
for (const [line, action] of [
  ["  | text", "indent"],
  ["' text,", "indent"],
  ["|< text", "indent"],
  ["  'b',", undefined],
  ["  || other", undefined],
  ["= link_to 'a',", "none"],
  ["- if a && \\", "none"],
  ["p = t 'x',", "none"],
  ["p a,", undefined],
  ["= link_to 'a', ", undefined],
]) {
  assert.strictEqual(enterAction(line), action, line);
}

console.log("slim editing helpers ok");
