import assert from "assert";
import fs from "fs";
const { onEnterRules, indentationRules } = JSON.parse(
  fs.readFileSync(
    new URL("../language-configuration/erb.json", import.meta.url),
    "utf8",
  ),
);

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

// Each erb-* host comments in its own syntax; only markup hosts keep the HTML pairs, indent rules and `<!--` folding.
const readJson = (file) =>
  JSON.parse(fs.readFileSync(new URL(file, import.meta.url), "utf8"));
const { contributes } = readJson("../package.json");
const erbConfig = readJson("../language-configuration/erb.json");
const comments = {
  "erb-js": { lineComment: "//", blockComment: ["/*", "*/"] },
  "erb-scss": { lineComment: "//", blockComment: ["/*", "*/"] },
  "erb-css": { blockComment: ["/*", "*/"] },
  "erb-ruby": { lineComment: "#" },
  "erb-yaml": { lineComment: "#" },
  "erb-xml": { blockComment: ["<!--", "-->"] },
  "erb-markdown": { blockComment: ["<!--", "-->"] },
  "erb-json": erbConfig.comments,
  "erb-text": erbConfig.comments,
  "erb-percent": erbConfig.comments,
};
const regionLine = {
  "erb-js": "// #region",
  "erb-scss": "// #region",
  "erb-css": "/* #region */",
  "erb-ruby": "# region",
  "erb-yaml": "#region",
};
const hasAngle = (config) =>
  config.brackets.some(([open]) => open === "<") ||
  config.surroundingPairs.some(([open]) => open === "<") ||
  config.autoClosingPairs.some(({ open }) => open === "<!--");
for (const { id, configuration } of contributes.languages) {
  if (!id.startsWith("erb-")) continue;
  const config = readJson(`../${configuration}`);
  assert.deepStrictEqual(config.comments, comments[id], id);
  assert(
    config.autoClosingPairs.some(({ open }) => open === "<%"),
    `${id} closes <%`,
  );
  assert(
    config.brackets.some(([open]) => open === "<%"),
    id,
  );
  assert.deepStrictEqual(config.onEnterRules, erbConfig.onEnterRules, id);
  assert.strictEqual(config.wordPattern, erbConfig.wordPattern, id);
  const start = new RegExp(config.folding.markers.start);
  assert(start.test("<%# region"), `${id} folds <%# region`);
  if (regionLine[id]) {
    assert(start.test(regionLine[id]), `${id} folds ${regionLine[id]}`);
    assert(!hasAngle(config), `${id} has HTML pairs`);
    assert(
      !new RegExp(config.indentationRules.increaseIndentPattern).test("<div>"),
      `${id} indents <div>`,
    );
  } else {
    assert.deepStrictEqual(config.folding, erbConfig.folding, id);
    assert.deepStrictEqual(
      config.indentationRules,
      erbConfig.indentationRules,
      id,
    );
    assert(hasAngle(config), `${id} keeps HTML pairs`);
  }
}

console.log(
  `langconfig: ${cases.length + inside.length + 9 + Object.keys(comments).length * 5} ok`,
);
