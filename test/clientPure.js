import assert from "node:assert";
import { convertedFileName } from "../src/convert.js";
import { windowsShellCommand } from "../src/shell.js";
import {
  relatedFiles,
  relatedDirs,
  actionSource,
  actionLine,
  previewPartners,
} from "../src/related.js";
import fs from "node:fs";
import path from "node:path";
import {
  tagToClose,
  isHtmlView,
  doubledErbClose,
  HTML_TAGS,
} from "../src/autoClose.js";
import { erbRubyArgs } from "../src/erbEngine.js";
import { routesFromOutput } from "../src/routes.js";
import { spawnSync } from "node:child_process";

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

// tagToClose
for (const [text, name] of [
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
  ["<!-- <div>", null],
  ["<!-- x -->\n<div>", "div"],
  ["<script>\nif (a <b>", null],
  ["<script></script>\n<p>", "p"],
  ["<script>", "script"],
  ["<p>100% <span>", "span"],
]) {
  assert.strictEqual(tagToClose(text), name, text);
}
// Many ERB tags on one line that never form a start tag must not backtrack exponentially.
const started = Date.now();
tagToClose(
  "<nav <%= attrs %>>" + "<%= link_to a, b %> | ".repeat(40) + "</nav>",
);
assert.ok(Date.now() - started < 100, "tagToClose is linear");

for (const [file, html] of [
  ["show.html.erb", true],
  ["show.html+phone.erb", true],
  ["create.turbo_stream.erb", true],
  ["old.rhtml", true],
  ["Foo.java.erb", false],
  ["nginx.conf.erb", false],
  ["index.js.erb", false],
]) {
  assert.strictEqual(
    isHtmlView(file, () => "<div>"),
    html,
    file,
  );
}

// a bare app/views/**/name.erb is HTML when its first text outside ERB is markup, as on the server
for (const [file, text, html] of [
  ["/r/app/views/pages/about.erb", "<div>", true],
  ["/r/app/views/about.erb", "\uFEFF\n<%# c %>\n<!DOCTYPE html>", true],
  ["C:\\r\\app\\views\\pages\\about.erb", "<% x = 1 %><DIV>", true],
  ["/r/app/views/pages/about.erb", "<turbo-frame id=x>", true],
  ["/r/app/views/pages/about.erb", "<rss><link>", false],
  ["/r/app/views/pages/about.erb", "Hello <b>", false],
  ["/r/app/views/pages/about.erb", "<% if a %>", false],
  ["/r/app/views/pages/about.erb", "", false],
  ["/r/app/views/pages/about.de.erb", "Hello", true],
  ["/r/app/views/pages/about.pt-BR.erb", "", true],
  ["/r/app/views/pages/about.js.erb", "Hello", false],
  ["/r/App/Views/pages/about.erb", "<div>", false],
  ["/r/config/about.erb", "<div>", false],
  ["/r/app/views/pages/nginx.conf.erb", "<div>", false],
]) {
  assert.strictEqual(
    isHtmlView(file, () => text),
    html,
    `${file} ${text}`,
  );
}

// a long run of ERB tags before the first text must not hang the markup check
assert.strictEqual(
  isHtmlView("/r/app/views/a.erb", () => "<% a %>\n".repeat(1000) + "Hello"),
  false,
);

// the client's tag list is the server's html::TAGS
const htmlRs = fs.readFileSync(
  path.join(import.meta.dirname, "../server/src/html.rs"),
  "utf8",
);
const serverTags = [
  ...htmlRs.match(/TAGS: &\[&str\] = &\[([^\]]*)\]/)[1].matchAll(/"([^"]+)"/g),
].map((m) => m[1]);
assert.deepStrictEqual([...HTML_TAGS].sort(), serverTags.sort());

// the text is only read for a bare .erb view: a named HTML view or an unrelated file never pays for it
const unread = () => assert.fail("getText must not be called");
for (const file of [
  "/r/app/views/pages/show.html.erb",
  "C:\\r\\app\\views\\show.html.erb",
  "/r/config/about.erb",
  "/r/app/views/pages/nginx.conf.erb",
]) {
  assert.doesNotThrow(() => isHtmlView(file, unread), file);
}

// doubledErbClose: what auto-closing left after a tag typed in full
for (const [before, after, length] of [
  ["<%= x %>", " %>", 3],
  ["<%=x%>", " %>", 3],
  ["<p><% if a %>", " %></p>", 3],
  ["<%= x %>", "%>", 2],
  ["<%= x %>", "</p>", 0],
  ["50%>", " %>", 0],
  ["<%%>", " %>", 0],
  ["<%= a %%>", " %>", 0],
]) {
  assert.strictEqual(
    doubledErbClose(before, after),
    length,
    `${before}|${after}`,
  );
}

// erbRubyArgs: Rails semantics through Erubi; skipped without `ruby -rerubi`.
if (spawnSync("ruby", ["-rerubi", "-e", ""]).status === 0) {
  const erb = (mode, input) =>
    spawnSync("ruby", erbRubyArgs(mode), { input, encoding: "utf8" });
  const rendered = erb(
    "render",
    '<p><%= "<b>" %></p>\n<%== "<i>" %>\n<%- if true -%>\nyes\n<%- end -%>\n',
  );
  assert.strictEqual(rendered.stdout, "<p>&lt;b&gt;</p>\n<i>\nyes\n");
  assert.match(erb("compile", "<%= x %>").stdout, /__erubi\.h\(\( x \)\)/);
  assert.notStrictEqual(erb("render", "<% if %>").status, 0);
  const ascii = spawnSync("ruby", erbRubyArgs("render"), {
    input: 'é <%= "ü" %><%= defined?(src).inspect %>',
    encoding: "utf8",
    env: { ...process.env, LANG: "C", LC_ALL: "C" },
  });
  assert.strictEqual(ascii.stdout, "é ünil");
}

// actionSource / actionLine
assert.deepStrictEqual(actionSource("/r/app/views/admin/users/show.html.erb"), {
  file: "/r/app/controllers/admin/users_controller.rb",
  action: "show",
});
assert.deepStrictEqual(
  actionSource("/r/app/views/user_mailer/welcome.text.erb"),
  {
    file: "/r/app/mailers/user_mailer.rb",
    action: "welcome",
  },
);
for (const view of [
  "/r/app/views/users/_form.html.erb",
  "/r/app/views/layouts/application.html.erb",
  "/r/app/views/show.html.erb",
  "/r/lib/views/users/show.html.erb",
]) {
  assert.strictEqual(actionSource(view), undefined, view);
}
const ordersController =
  "class X\n  def index; end\n  def show_all\n  def show(id = nil)\nend";
assert.strictEqual(actionLine(ordersController, "show"), 3);
assert.strictEqual(actionLine(ordersController, "edit"), -1);

// routesFromOutput
assert.deepStrictEqual(
  routesFromOutput(
    'Booting\r\n\r\n__RUBY_TEMPLATES_ROUTES__\r\n[{"name":"users"}]\r\n',
  ),
  [{ name: "users" }],
);
assert.strictEqual(routesFromOutput("Puma starting\n"), undefined);
assert.throws(() => routesFromOutput("__RUBY_TEMPLATES_ROUTES__\n{oops"));

// previewPartners
const previews = (dir) => [
  `/r/test/components/previews/${dir}card_component_preview.rb`,
  `/r/spec/components/previews/${dir}card_component_preview.rb`,
];
assert.deepStrictEqual(
  previewPartners("/r/app/components/admin/card_component.rb"),
  previews("admin/"),
);
assert.deepStrictEqual(
  previewPartners("/r/app/components/card_component/card_component.html.erb"),
  previews(""),
);
assert.deepStrictEqual(
  previewPartners(
    "/r/spec/components/previews/admin/card_component_preview.rb",
  ),
  ["/r/app/components/admin/card_component.rb"],
);
for (const file of [
  "/r/app/components/helpers.rb",
  "/r/app/views/users/show.html.erb",
  "/r/spec/components/previews/card_preview.rb",
]) {
  assert.deepStrictEqual(previewPartners(file), [], file);
}
