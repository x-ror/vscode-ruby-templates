import assert from "assert";
import {
  erbTagBodyLines,
  toggleErbTagAt,
  toggleRubyComment,
} from "../src/erbTags.js";

const toggle = (tag) => {
  const e = toggleErbTagAt(tag, 3);
  return e && tag.slice(0, e.start) + e.replacement + tag.slice(e.end);
};
const cases = [
  ["<% x %>", "<%= x %>"],
  ["<%= x %>", "<%# x %>"],
  ["<%# x %>", "<% x %>"],
  ["<%== x %>", "<%# x %>"],
  ["<%- x %>", "<%= x %>"],
  ["<% x -%>", "<%= x -%>"],
  ["<%= x -%>", "<%# x -%>"],
  ["<%# x -%>", "<% x -%>"],
  ["<%- x -%>", "<%= x -%>"],
  ["<%== x -%>", "<%# x -%>"],
  ["<% x =%>", "<%= x =%>"],
  ["<%= x =%>", "<%# x =%>"],
  ["<%-x-%>", "<%= x-%>"],
  ["<% \n  foo(\n    bar\n  )\n%>", "<%= \n  foo(\n    bar\n  )\n%>"],
  ["<%  %>", "<%=  %>"],
  ["<%=   a == b   %>", "<%#   a == b   %>"],
  ["<% if a\n  b %>", "<%= if a\n  b %>"],
  ["<%% x %>", null],
];
for (const [input, expected] of cases)
  assert.strictEqual(toggle(input), expected, input);

// Offset picks the enclosing tag; text outside any tag is a no-op.
const text = "<a><% a %> <%= b -%></a>";
assert.deepStrictEqual(toggleErbTagAt(text, text.indexOf("b")), {
  start: 11,
  end: 14,
  replacement: "<%#",
});
assert.strictEqual(toggleErbTagAt(text, 1), null);
assert.strictEqual(toggleErbTagAt(text, text.length - 1), null);

// Lines strictly between a tag's `<%` and `%>` lines; comments, `<%%` and one-liners have none.
const multi =
  "<div>\n<%\n  a\n  b\n%>\n<%# c\nd\n%>\n<%= e\n  f %>\n<% g -%>\n<%=\n  h\n=%>\n<%%\nx\n%>";
assert.deepStrictEqual([...erbTagBodyLines(multi, 99)], [2, 3, 12]);
assert.deepStrictEqual(
  [...erbTagBodyLines(multi.replaceAll("\n", "\r\n"), 99)],
  [2, 3, 12],
);
assert.deepStrictEqual([...erbTagBodyLines(multi, 2)], [2, 3]);
const many = "<%\n  a\n%>\n".repeat(5000);
assert.deepStrictEqual(
  [...erbTagBodyLines(many, 99999)].slice(-2),
  [14995, 14998],
);
assert.strictEqual(erbTagBodyLines(many, 99999).size, 5000);

// Each group decides on its own; edits are at the indent, not whole lines.
const edit = (index, start, end, text) => ({ index, start, end, text });
assert.deepStrictEqual(toggleRubyComment([["  a", "", "    b"]]), [
  [edit(0, 2, 2, "# "), edit(2, 2, 2, "# ")],
]);
assert.deepStrictEqual(toggleRubyComment([["  # a", "", "  #b"]]), [
  [edit(0, 2, 4, ""), edit(2, 2, 3, "")],
]);
assert.deepStrictEqual(toggleRubyComment([["# a"], ["b"]]), [
  [edit(0, 0, 2, "")],
  [edit(0, 0, 0, "# ")],
]);
assert.deepStrictEqual(toggleRubyComment([["# a", "b"]]), [
  [edit(0, 0, 0, "# "), edit(1, 0, 0, "# ")],
]);
// Shared whitespace prefix, not the shortest length: tab vs spaces.
assert.deepStrictEqual(toggleRubyComment([["\tfoo", "  bar"]]), [
  [edit(0, 0, 0, "# "), edit(1, 0, 0, "# ")],
]);
// `#{` is interpolation (heredoc), not a comment.
assert.deepStrictEqual(toggleRubyComment([["#{a}"]]), [[edit(0, 0, 0, "# ")]]);

console.log(`erbTags: ${cases.length + 17} ok`);
