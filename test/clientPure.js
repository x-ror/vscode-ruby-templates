import assert from "node:assert";
import { convertedFileName } from "../src/convert.js";
import { windowsShellCommand } from "../src/shell.js";
import { relatedFiles, relatedDirs } from "../src/related.js";
import { ERB_FORMATTERS, erbFormatterArgs } from "../src/erbFormat.js";
import { erbLintOffenses } from "../src/erbLint.js";
import { tagToClose } from "../src/autoClose.js";

// convertedFileName
const cases = [
  ["show.html.erb", "erb", "haml", "show.html.haml"],
  ["a.haml", "haml", "slim", "a.slim"],
  ["a.rhtml", "erb", "haml", "a.haml"],
  ["a.slim", "slim", "erb", "a.erb"],
  ["index.html.haml.tt", "haml", "haml", "index.html.haml.tt.haml"],
  ["show.html.haml.tt", "haml", "slim", "show.html.slim.tt"],
  ["index.html", "html", "haml", "index.html.haml"],
  ["a.html.erb", "html", "haml", "a.html.erb.haml"],
  ["notes.txt", "erb", "haml", "notes.txt.haml"],
  ["SHOW.HTML.ERB", "erb", "haml", "SHOW.HTML.haml"],
  ["show.html.erubi", "erb", "slim", "show.html.slim"],
  ["show.html.erubis", "erb", "haml", "show.html.haml"],
];
for (const [name, from, target, expected] of cases) {
  const actual = convertedFileName(name, from, target);
  assert.strictEqual(actual, expected, `${name} ${from}->${target}`);
  assert.notStrictEqual(actual, name, `${name} must not overwrite itself`);
}

// windowsShellCommand
assert.deepStrictEqual(
  windowsShellCommand("haml2erb", ["C:\\my dir\\a.haml"]),
  [
    "cmd.exe",
    ["/d", "/s", "/c", '""haml2erb" "C:\\my dir\\a.haml""'],
    { windowsVerbatimArguments: true },
  ],
);
assert.strictEqual(
  windowsShellCommand("x", ["C:\\a b\\"])[1][3],
  '""x" "C:\\a b\\\\""',
  "trailing backslash doubled",
);
assert.strictEqual(
  windowsShellCommand("x", ["a\\b"])[1][3],
  '""x" "a\\b""',
  "inner backslash untouched",
);
assert.throws(
  () => windowsShellCommand("x", ['a" & calc & "']),
  /double quote/,
);

// relatedFiles / relatedDirs
assert.deepStrictEqual(
  relatedFiles("welcome.html.erb", [
    "welcome.html.erb",
    "welcome.text.erb",
    "welcome_back.html.erb",
    "welcome",
  ]),
  ["welcome.text.erb"],
);
assert.deepStrictEqual(
  relatedFiles("show.html+phone.erb", [
    "show.json.jbuilder",
    "show.html.erb",
    "shows.html.erb",
  ]),
  ["show.html.erb", "show.json.jbuilder"],
);
assert.deepStrictEqual(
  relatedFiles("card_component.rb", [
    "card_component.html.slim",
    "card_component.rb",
    "card_component",
  ]),
  ["card_component.html.slim"],
);
assert.deepStrictEqual(relatedDirs("/a/components", "card_component.rb"), [
  "/a/components/card_component",
]);
assert.deepStrictEqual(
  relatedDirs("/a/components/card_component", "card_component.html.erb"),
  ["/a/components"],
);
assert.deepStrictEqual(relatedDirs("/a/views/users", "show.html.erb"), []);

// erbFormatterArgs
assert.deepStrictEqual(
  erbFormatterArgs("htmlbeautifier", { tabSize: 4, insertSpaces: true }),
  ["--tab-stops", "4"],
);
assert.deepStrictEqual(
  erbFormatterArgs("htmlbeautifier", { tabSize: 4, insertSpaces: false }),
  ["--tab"],
);
assert.deepStrictEqual(
  erbFormatterArgs("erb-format", { tabSize: 2, insertSpaces: true }),
  ["--stdin"],
);
assert.ok(Object.keys(ERB_FORMATTERS).every((k) => k !== "none"));

// erbLintOffenses, on erb_lint 0.9's report shape
const erbLintReport = JSON.stringify({
  files: [
    {
      path: "a.html.erb",
      offenses: [
        {
          linter: "SpaceAroundErbTag",
          message: "Use 1 space after `<%=` instead of 0 space.",
          location: {
            start_line: 2,
            start_column: 3,
            last_line: 2,
            last_column: 3,
            length: 0,
          },
        },
        {
          linter: "RequireInputAutocomplete",
          message: "Input tag is missing an autocomplete attribute.",
          location: {
            start_line: 4,
            start_column: 2,
            last_line: 5,
            last_column: 1,
            length: 9,
          },
        },
      ],
    },
  ],
});
assert.deepStrictEqual(erbLintOffenses(erbLintReport), [
  {
    line: 1,
    column: 3,
    endLine: 1,
    endColumn: 4,
    linter: "SpaceAroundErbTag",
    message: "Use 1 space after `<%=` instead of 0 space.",
  },
  {
    line: 3,
    column: 2,
    endLine: 4,
    endColumn: 1,
    linter: "RequireInputAutocomplete",
    message: "Input tag is missing an autocomplete attribute.",
  },
]);

// tagToClose
for (const [line, name] of [
  ["<div>", "div"],
  ['  <a href="<%= url %>" class="x">', "a"],
  ["<my-el>", "my-el"],
  ["<img src=x>", null],
  ["<br/>", null],
  ["<div/>", null],
  ["<% if a > b %><p>", "p"],
  ["<% if a >", null],
  ["<%= x >", null],
  ['<input value="a>b">', null],
  ['<span title="a>b">', "span"],
  ["a > b", null],
]) {
  assert.strictEqual(tagToClose(line), name, line);
}
