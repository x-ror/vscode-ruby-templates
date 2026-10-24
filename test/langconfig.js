const assert = require("assert");
const {
  onEnterRules,
  indentationRules,
} = require("../language-configuration/erb.json");

// Mirrors VS Code: the first rule whose beforeText (and afterText, if set) matches wins.
const rules = onEnterRules.map(({ beforeText, afterText, action }) => ({
  before: new RegExp(beforeText),
  after: afterText && new RegExp(afterText),
  indent: action.indent,
}));
const onEnter = (before, after = "") =>
  rules.find((r) => r.before.test(before) && (!r.after || r.after.test(after)))
    ?.indent ?? null;

const cases = [
  ["<% if user %>", "indent"],
  ["  <%- unless x -%>", "indent"],
  ["<% else %>", "indent"],
  ["<% elsif x %>", "indent"],
  ["<% when :a %>", "indent"],
  ["<% rescue => e %>", "indent"],
  ["<% items.each do %>", "indent"],
  ["<% items.each do |x| %>", "indent"],
  ["<%= form_with model: @post do |f| %>", "indent"],
  ["<% hash.each do |a, b| -%>", "indent"],
  ["<% list.each do |x| =%>", "indent"],
  ["<% x = 10 % 3; items.each do %>", "indent"],
  ["<% if a %>b<% end %>", null],
  ["<% items.each do |x| %><%= x %><% end %>", null],
  ["<%# items.each do |x| %>", null],
  ["<%# if user %>", null],
  ["<% x %> <%# y do %>", null],
  ["<%= user.name %>", null],
  ["<% do_something %>", null],
];
for (const [line, expected] of cases)
  assert.strictEqual(onEnter(line), expected, line);

const inside = [
  ["<%", " %>", "indentOutdent"],
  ["  <%= ", "%>", "indentOutdent"],
  ["<%- ", " -%>", "indentOutdent"],
  ["<%#", " %>", "indentOutdent"],
  ["<% x ", "%>", null],
  ["<% ", "x %>", null],
];
for (const [before, after, expected] of inside)
  assert.strictEqual(onEnter(before, after), expected, `${before}|${after}`);

const increase = new RegExp(indentationRules.increaseIndentPattern);
const decrease = new RegExp(indentationRules.decreaseIndentPattern);
assert(increase.test("<% list.each do |x| =%>"));
assert(!increase.test("<%# list.each do |x| %>"));
assert(!increase.test("<% if a %>b<% end =%>"));
assert(increase.test('<div class="a">'));
assert(!increase.test("<div>x</div>"), "element closed on the same line");
for (const line of [
  "<% else %>",
  "  <% elsif x %>",
  "<% when 1 %>",
  "<%- end -%>",
])
  assert(decrease.test(line), line);

console.log(`langconfig: ${cases.length + inside.length + 9} ok`);
