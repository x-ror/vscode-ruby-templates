import assert from "node:assert";
import { convertedFileName } from "../src/convert.js";
import { windowsShellCommand } from "../src/shell.js";

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
