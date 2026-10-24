const assert = require("assert");
const { toggleSlimMarker, wrapSlim, dedent, partialPath } = require("../src/slim");

for (const [line, next] of [
  ["- if x", "= if x"],
  ["  = link_to 'a', b", "  == link_to 'a', b"],
  ["==> raw x", "/ raw x"],
  ["=< x", "==< x"],
  ["/ note", "- note"],
  ["-x", "= x"],
  ["-", "="],
  ["='x'", "== 'x'"],
  ["/! html", null],
  ["/[if IE]", null],
  ["p = x", null],
  ["| text", null],
]) {
  assert.strictEqual(toggleSlimMarker(line), next, line);
}

assert.strictEqual(wrapSlim("user.name", true), "= user.name");
assert.strictEqual(wrapSlim("user.name", false), "#{user.name}");

assert.deepStrictEqual(dedent(["    .card", "", "      p = x", "\t"]), { indent: "    ", lines: [".card", "", "  p = x", ""] });

assert.strictEqual(partialPath("/app/views/users/show.html.slim", "card"), "/app/views/users/_card.html.slim");
assert.strictEqual(partialPath("/r/app/views/users/show.slim", "shared/ui/card"), "/r/app/views/shared/ui/_card.html.slim");
assert.strictEqual(partialPath("/tmp/show.slim", "shared/card"), null);

const { onEnterRules } = require("../language-configuration/slim.json");
const enterAction = (line) => onEnterRules.find((r) => new RegExp(r.beforeText).test(line))?.action.indent;
for (const [line, action] of [
  ["  | text", "indent"],
  ["' text,", "indent"],
  ["= link_to 'a',", "none"],
  ["- if a && \\", "none"],
  ["p = t 'x',", "none"],
  ["p a,", undefined],
  ["= link_to 'a', ", undefined],
]) {
  assert.strictEqual(enterAction(line), action, line);
}

console.log("slim editing helpers ok");
