const assert = require("assert");
const { toggleErbTagAt } = require("../src/erbTags");

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
for (const [input, expected] of cases) assert.strictEqual(toggle(input), expected, input);

// Offset picks the enclosing tag; text outside any tag is a no-op.
const text = "<a><% a %> <%= b -%></a>";
assert.deepStrictEqual(toggleErbTagAt(text, text.indexOf("b")), { start: 11, end: 14, replacement: "<%#" });
assert.strictEqual(toggleErbTagAt(text, 1), null);
assert.strictEqual(toggleErbTagAt(text, text.length - 1), null);

console.log(`erbTags: ${cases.length + 3} ok`);
