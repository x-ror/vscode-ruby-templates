// The `ruby-templates` command line on a temp folder: check, format to stdout, --check, --write, stdin, exit codes.
import assert from "assert";
import { spawnSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";
import { pathToFileURL } from "url";

const exe = process.platform === "win32" ? ".exe" : "";
// CI builds per target and passes the binary in, relative to the repo; cases run from other folders.
const bin = process.env.RUBY_TEMPLATES
  ? path.resolve(process.env.RUBY_TEMPLATES)
  : path.join(
      import.meta.dirname,
      "..",
      "server",
      "target",
      "release",
      `ruby-templates${exe}`,
    );
const run = (args, input, cwd) => {
  const r = spawnSync(bin, args, { encoding: "utf8", input, cwd });
  if (r.error) throw r.error;
  return r;
};

const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "rt-cli-")));
const views = path.join(root, "app", "views", "users");
fs.mkdirSync(views, { recursive: true });
const file = (name, text) => {
  const p = path.join(views, name);
  fs.writeFileSync(p, text);
  return p;
};
const messy = file("show.html.slim", "div\n    p  \n");
const clean = file("index.html.slim", "div\n  p\n");
const broken = file("broken.html.slim", "p\n    a\n  b\n");
const compileError = file("ending.html.slim", "div\n    p\n        - end\n");
const notes = file("notes.txt", "x");

try {
  let r = run(["check", clean]);
  assert.strictEqual(r.status, 0, r.stdout + r.stderr);
  r = run(["check", views]);
  assert.strictEqual(r.status, 1, "a broken file is a finding");
  assert.match(r.stdout, /broken\.html\.slim:\d+:\d+: error: /);
  const reported = [...r.stdout.matchAll(/(\w+)\.html\.slim:/g)].map(
    (m) => m[1],
  );
  assert.deepStrictEqual(
    reported,
    [...reported].sort(),
    "files checked in parallel print in walk order",
  );
  assert.doesNotMatch(
    r.stdout + r.stderr,
    /notes\.txt/,
    "folders yield templates only",
  );
  const nested = file(
    "nested.html.erb",
    `<% ${"[".repeat(5000)}${"]".repeat(5000)} %>\n`,
  );
  r = run(["check", nested]);
  assert.strictEqual(r.status, 0, "deep nesting fits a worker's stack");
  fs.rmSync(nested);
  r = run(["check", notes]);
  assert.strictEqual(r.status, 2);
  assert.match(r.stderr, /notes\.txt: unknown template type/);

  const liquid = path.join(root, "card.liquid");
  fs.writeFileSync(
    liquid,
    '{% render "card" %}\n{% if a %}\n   x\n{% endif %}\n',
  );
  r = run(["check", liquid]);
  assert.strictEqual(r.status, 0, r.stdout + r.stderr);
  assert.strictEqual(r.stdout + r.stderr, "", "Liquid has no checks");
  r = run(["format", liquid]);
  assert.strictEqual(r.status, 0, r.stderr);
  assert.strictEqual(r.stdout, fs.readFileSync(liquid, "utf8"), "kept as is");
  assert.match(r.stderr, /card\.liquid: left unchanged: /);
  fs.unlinkSync(liquid);

  // Ruby 3.3 rejects a line that starts with `&&`; the latest grammar, used without a project Ruby, takes it.
  const ruby = file("ruby.html.erb", "<% x = a\n  && b %>\n");
  r = run(["check", ruby]);
  assert.strictEqual(
    r.status,
    0,
    "no .ruby-version: the latest grammar\n" + r.stdout + r.stderr,
  );
  fs.writeFileSync(path.join(root, ".ruby-version"), "3.3.0\n");
  r = run(["check", ruby]);
  assert.strictEqual(r.status, 1, r.stdout + r.stderr);
  assert.match(
    r.stdout,
    /ruby\.html\.erb:2:\d+: error: unexpected '&&', ignoring it/,
  );
  fs.writeFileSync(path.join(root, ".ruby-version"), "4.1.0\n");
  assert.strictEqual(run(["check", ruby]).status, 0, "4.1 takes it");
  fs.unlinkSync(path.join(root, ".ruby-version"));
  fs.unlinkSync(ruby);

  r = run(["format", messy]);
  assert.strictEqual(r.status, 0, r.stderr);
  assert.strictEqual(r.stdout, "div\n  p\n");
  assert.strictEqual(
    fs.readFileSync(messy, "utf8"),
    "div\n    p  \n",
    "stdout mode writes nothing",
  );

  r = run(["format", "--check", views]);
  assert.strictEqual(r.status, 1);
  assert.deepStrictEqual(r.stdout.trim().split("\n"), [messy]);
  assert.match(r.stderr, /broken\.html\.slim: left unchanged: /);
  assert.match(
    r.stderr,
    /ending\.html\.slim: left unchanged: Slim would reject the file/,
    "an error Slim raises while compiling stops the formatter too",
  );
  assert.doesNotMatch(r.stderr, /notes|index/, "only doubts are reported");
  assert.strictEqual(run(["format", views]).status, 2, "stdout takes one file");

  r = run(["format", "--stdin-filepath", "x.html.slim", "-"], "ul\n    li\n");
  assert.strictEqual(r.status, 0, r.stderr);
  assert.strictEqual(r.stdout, "ul\n  li\n");
  r = run(
    ["format", "--check", "--stdin-filepath", "x.html.slim"],
    "ul\n  li\n",
  );
  assert.strictEqual(r.status, 0, "stdin that needs no change");
  const comment = "<div>\n<!-- a\n  b -->\n</div>\n";
  r = run(["format", "--stdin-filepath", "x.html.erb"], comment);
  assert.strictEqual(
    r.stdout,
    "<div>\n  <!-- a\n  b -->\n</div>\n",
    "comments kept",
  );
  r = run(
    ["format", "--align-comments", "--stdin-filepath", "x.html.erb"],
    comment,
  );
  assert.strictEqual(r.stdout, "<div>\n  <!-- a\n    b -->\n</div>\n");
  r = run(["format", "--stdin-filepath", "x.txt"], "keep  \n");
  assert.strictEqual(r.stdout, "keep  \n", "unknown files pass through");

  r = run(["format", "--write", views]);
  assert.strictEqual(r.status, 0, r.stderr);
  assert.deepStrictEqual(r.stdout.trim().split("\n"), [messy]);
  assert.strictEqual(fs.readFileSync(messy, "utf8"), "div\n  p\n");
  assert.strictEqual(fs.readFileSync(broken, "utf8"), "p\n    a\n  b\n");
  assert.strictEqual(run(["format", "--check", views]).status, 0);

  // Config: only the width of .editorconfig counts; --verbose lists what it read and ignored.
  fs.writeFileSync(
    path.join(root, ".editorconfig"),
    "root = true\n[*.slim]\nindent_size = 4\nmax_line_length = 100\n",
  );
  r = run(["format", "--verbose", messy]);
  assert.strictEqual(r.stdout, "div\n    p\n");
  assert.match(r.stderr, /\.editorconfig: ignored max_line_length = 100/);

  for (const args of [
    [],
    ["lint"],
    ["format"],
    ["format", "--check", "--write", views],
    ["format", "-"],
    ["check", "--wat"],
  ]) {
    assert.strictEqual(run(args).status, 2, `usage error: ${args.join(" ")}`);
  }
  assert.strictEqual(
    run(["format", path.join(root, "missing.html.slim")]).status,
    2,
  );
  const dangling = file("dangling.html.erb", '<%= render "users/missing" %>\n');
  const unresolved = /Missing partial users\/_missing/;
  assert.doesNotMatch(
    run(["check", dangling]).stdout,
    unresolved,
    "no lockfile",
  );
  const lock = path.join(root, "Gemfile.lock");
  fs.writeFileSync(lock, "GEM\n  specs:\n    actionview (7.1.3)\n");
  assert.match(run(["check", dangling]).stdout, unresolved, "Rails lockfile");
  fs.writeFileSync(lock, "GEM\n  specs:\n    sinatra (4.0.0)\n");
  assert.doesNotMatch(run(["check", dangling]).stdout, unresolved, "Sinatra");
  fs.writeFileSync(lock, "GEM\n  specs:\n    actionview (7.1.3)\n");
  const template = file(
    "template.html.erb",
    '<%= render template: "users/gone" %>\n',
  );
  assert.match(run(["check", template]).stdout, /Missing template users\/gone/);
  const controllers = path.join(root, "app", "controllers");
  fs.mkdirSync(controllers);
  fs.writeFileSync(
    path.join(controllers, "application_controller.rb"),
    "class ApplicationController < ActionController::Base\nend\n",
  );
  const bare = file("bare.html.erb", '<%= render "crad" %>\n');
  assert.doesNotMatch(
    run(["check", bare]).stdout,
    /Missing partial/,
    "no controller",
  );
  fs.writeFileSync(
    path.join(controllers, "users_controller.rb"),
    "class UsersController < ApplicationController\nend\n",
  );
  assert.match(
    run(["check", bare]).stdout,
    /Missing partial users\/_crad, application\/_crad/,
  );
  fs.rmSync(lock);

  // The HTML and accessibility rules run in Haml and Slim without a Rails project, and a same-line duplicate id is safe.
  const html = path.join(root, "html");
  fs.mkdirSync(html);
  const linted = path.join(html, "show.html.slim");
  fs.writeFileSync(linted, 'iframe src="x"\ndiv#a: span#a x\na.btn Menu\n');
  r = run(["check", linted]);
  assert.strictEqual(r.status, 1, r.stdout + r.stderr);
  for (const message of [
    "`<iframe>` without `title`",
    "Duplicate id `a`",
    "`<a>` without `href`",
  ]) {
    assert.ok(r.stdout.includes(message), `${message}: ${r.stdout}`);
  }

  // .ruby-templates.toml: rules, switches, nesting, exclusions and its own problems.
  const project = path.join(root, "toml");
  fs.mkdirSync(path.join(project, "vendor"), { recursive: true });
  fs.mkdirSync(path.join(project, "sub"));
  const toml = path.join(project, ".ruby-templates.toml");
  const write = (name, text) => {
    const p = path.join(project, name);
    fs.writeFileSync(p, text);
    return p;
  };
  const noAlt = write("img.html.erb", '<img src="/a.png">\n');
  const vendored = write("vendor/img.html.erb", '<img src="/a.png">\n');
  const inner = write("sub/img.html.erb", '<img src="/a.png">\n');
  const messyErb = write("messy.html.erb", "<div>\n<p>a</p>\n</div>\n");
  assert.match(run(["check", noAlt]).stdout, /without `alt`/, "no file");
  fs.writeFileSync(toml, "[Html.ImgAlt]\nenabled = false\n");
  r = run(["check", noAlt]);
  assert.strictEqual(r.status, 0, r.stdout + r.stderr);
  assert.strictEqual(r.stdout, "", "a rule turned off in the file");
  write("sub/.ruby-templates.toml", "[Html.ImgAlt]\nenabled = true\n");
  assert.match(run(["check", inner]).stdout, /without `alt`/, "nested file");
  assert.strictEqual(run(["check", noAlt]).stdout, "");
  fs.writeFileSync(toml, 'exclude = ["vendor/**"]\n');
  assert.strictEqual(run(["check", vendored]).stdout, "", "global exclude");
  assert.match(run(["check", noAlt]).stdout, /without `alt`/);
  r = run(["format", vendored]);
  assert.match(
    r.stderr,
    /left unchanged: excluded by .*\.ruby-templates\.toml/,
  );
  fs.writeFileSync(toml, "[erb]\nformat.align_comments = true\n");
  const comments = write(
    "comment.html.erb",
    "<div>\n<!-- a\n  b -->\n</div>\n",
  );
  assert.match(
    run(["format", comments]).stdout,
    /<!-- a\n {4}b -->/,
    "align_comments",
  );
  fs.writeFileSync(toml, "[erb]\nformat.enabled = false\n");
  r = run(["format", messyErb]);
  assert.strictEqual(r.stdout, fs.readFileSync(messyErb, "utf8"));
  assert.match(r.stderr, /left unchanged: formatting is turned off/);

  fs.writeFileSync(toml, "[Html.ImgAltt]\nenabled = false\n");
  r = run(["check", noAlt, inner]);
  assert.strictEqual(r.status, 1);
  const problem =
    /\.ruby-templates\.toml:1:7: warning: unknown rule `Html\/ImgAltt`/g;
  assert.strictEqual(r.stdout.match(problem)?.length, 1, "printed once");
  fs.writeFileSync(toml, "[Html.ImgAlt\n");
  r = run(["check", clean, path.join(project, "messy.html.erb")]);
  assert.strictEqual(r.status, 1, "invalid TOML is a finding");
  assert.match(
    r.stdout,
    /\.ruby-templates\.toml:\d+:\d+: warning: invalid TOML/,
  );
  fs.writeFileSync(toml, "[Html.ImgAltt]\nenabled = false\n");
  r = run(["format", "--verbose", messyErb]);
  assert.match(r.stderr, /read .*\.ruby-templates\.toml/);
  assert.match(r.stderr, /\.ruby-templates\.toml:1:7: warning: unknown rule/);

  // Layout/EmptyLines and Layout/TrailingEmptyLines: switched in the file, for all three languages.
  const blank = path.join(root, "blank");
  fs.mkdirSync(blank);
  const blankToml = path.join(blank, ".ruby-templates.toml");
  const format = (name, text) =>
    run(["format", "--stdin-filepath", path.join(blank, name)], text).stdout;
  const gappy = {
    "a.html.erb": "<h1>x</h1>\n\n\n\n<pre>\n\n\n</pre>\n<p>y</p>\n\n\n",
    "a.html.haml": "%h1 x\n\n\n\n:plain\n  a\n\n\n  b\n%p y\n\n\n",
    "a.html.slim": "h1 x\n\n\n\n| a\n\n\n  b\np y\n\n\n",
  };
  for (const [name, text] of Object.entries(gappy)) {
    assert.strictEqual(
      format(name, text),
      name.endsWith(".erb") ? text : text.replace(/\n+$/, "\n"),
      `${name}: only Haml and Slim lose the blank lines at the end`,
    );
  }
  assert.strictEqual(
    format("a.html.erb", "<p>x</p>"),
    "<p>x</p>",
    "ERB is rendered byte for byte: no final newline added",
  );
  fs.writeFileSync(blankToml, "[Layout.TrailingEmptyLines]\nenabled = true\n");
  assert.strictEqual(format("a.html.erb", "<p>x</p>\n\n\n"), "<p>x</p>\n");
  assert.strictEqual(format("a.html.erb", "<p>x</p>"), "<p>x</p>\n");
  fs.writeFileSync(
    blankToml,
    "[Layout.EmptyLines]\nenabled = true\nmax = 1\n[Layout.TrailingEmptyLines]\nenabled = true\n",
  );
  assert.strictEqual(
    format("a.html.erb", gappy["a.html.erb"]),
    "<h1>x</h1>\n\n<pre>\n\n\n</pre>\n<p>y</p>\n",
    "erb: <pre> keeps its lines",
  );
  assert.strictEqual(
    format("a.html.haml", gappy["a.html.haml"]),
    "%h1 x\n\n:plain\n  a\n\n\n  b\n%p y\n",
    "haml: filters keep their lines",
  );
  assert.strictEqual(
    format("a.html.slim", gappy["a.html.slim"]),
    "h1 x\n\n| a\n\n\n  b\np y\n",
    "slim: text blocks keep their lines",
  );
  fs.writeFileSync(
    blankToml,
    "[Layout.EmptyLines]\nenabled = true\nmax = 1\n[Layout.TrailingEmptyLines]\nenabled = false\n",
  );

  assert.strictEqual(
    format("a.html.slim", "h1 x\n\n\np y\n\n\n"),
    "h1 x\n\np y\n\n",
  );
  assert.strictEqual(
    format("a.html.erb", "<p>x</p>\n\n\n<p>y</p>"),
    "<p>x</p>\n\n<p>y</p>",
    "no final line break added",
  );
  assert.strictEqual(format("a.html.haml", "%p x\n\n\n"), "%p x\n\n");
  fs.writeFileSync(blankToml, "[Layout.EmptyLines]\nmax = -1\n");
  r = run(
    [
      "format",
      "--verbose",
      "--stdin-filepath",
      path.join(blank, "a.html.slim"),
    ],
    "p\n",
  );
  assert.match(
    r.stderr,
    /:2:7: warning: `max` must be a whole number, 0 or more/,
  );

  // The rules around block and element bodies and branches: one table for all three, each style, all three languages.
  const around = path.join(root, "around");
  fs.mkdirSync(around);
  fs.writeFileSync(
    path.join(around, ".editorconfig"),
    "root = true\n[*]\nindent_size = 2\n",
  );
  const aroundFormat = (style, name, text) => {
    fs.writeFileSync(
      path.join(around, ".ruby-templates.toml"),
      [
        "Layout.EmptyLinesAroundBlockBody",
        "Layout.EmptyLinesAroundElementBody",
        "Layout.EmptyLinesAroundBranches",
      ]
        .map((rule) => `[${rule}]\nenabled = true\nstyle = "${style}"\n`)
        .join(""),
    );
    return run(["format", "--stdin-filepath", path.join(around, name)], text)
      .stdout;
  };
  const bodies = {
    "a.html.erb": [
      "<div>\n\n  <% if a %>\n\n    <p>x</p>\n\n  <% else %>\n\n    <pre>\n\n    </pre>\n  <% end %>\n\n</div>\n",
      "<div>\n  <% if a %>\n    <p>x</p>\n  <% else %>\n    <pre>\n\n    </pre>\n  <% end %>\n</div>\n",
      "<div>\n\n  <% if a %>\n\n    <p>x</p>\n\n  <% else %>\n\n    <pre>\n\n    </pre>\n\n  <% end %>\n\n</div>\n",
    ],
    "a.html.haml": [
      "%div\n\n  - if a\n\n    %p x\n\n  - else\n\n    :plain\n      a\n\n      b\n\n%footer\n",
      // The blank line before `%footer` separates siblings; Haml has no closing line.
      "%div\n  - if a\n    %p x\n  - else\n    :plain\n      a\n\n      b\n\n%footer\n",
      "%div\n\n  - if a\n\n    %p x\n\n  - else\n\n    :plain\n      a\n\n      b\n\n%footer\n",
    ],
    "a.html.slim": [
      "div\n\n  - if a\n\n    p x\n\n  - else\n\n    | a\n\n      b\n\nfooter\n",
      "div\n  - if a\n    p x\n  - else\n    | a\n\n      b\n\nfooter\n",
      "div\n\n  - if a\n\n    p x\n\n  - else\n\n    | a\n\n      b\n\nfooter\n",
    ],
  };
  for (const [name, [messy, none, one]] of Object.entries(bodies)) {
    assert.strictEqual(
      aroundFormat("no_empty_lines", name, messy),
      none,
      `${name}: no_empty_lines`,
    );
    assert.strictEqual(
      aroundFormat("empty_lines", name, none),
      one,
      `${name}: empty_lines adds them`,
    );
  }
  fs.writeFileSync(
    path.join(around, ".ruby-templates.toml"),
    '[Layout.EmptyLinesAroundBranches]\nstyle = "keep"\n',
  );
  r = run(
    [
      "format",
      "--verbose",
      "--stdin-filepath",
      path.join(around, "a.html.erb"),
    ],
    "<p>x</p>\n",
  );
  assert.match(
    r.stderr,
    /:2:9: warning: `style` must be one of no_empty_lines, empty_lines, got "keep"/,
  );

  // The in-line rules: spaces in ERB tags, `<% #` comments and the space after Haml and Slim script markers.
  const inline = path.join(root, "inline");
  fs.mkdirSync(inline);
  const inlineToml = path.join(inline, ".ruby-templates.toml");
  fs.writeFileSync(
    path.join(inline, ".editorconfig"),
    "root = true\n[*]\nindent_size = 2\n",
  );
  const inlineFormat = (name, text) =>
    run(["format", "--stdin-filepath", path.join(inline, name)], text).stdout;
  // Each: the text, the formatter's output with the rules on, and with them off (re-indented only).
  const inlineTexts = {
    "a.html.erb": [
      '<div>\n<% # note %>\n<p  id = "a" ><%=user.name%></p>\n</div>\n',
      '<div>\n  <%# note %>\n  <p id="a"><%= user.name %></p>\n</div>\n',
      '<div>\n  <% # note %>\n  <p  id = "a" ><%=user.name%></p>\n</div>\n',
    ],
    "a.html.haml": [
      "%div\n  -if a\n    %p=x\n    =render y\n",
      "%div\n  - if a\n    %p= x\n    = render y\n",
      "%div\n  -if a\n    %p=x\n    =render y\n",
    ],
    "a.html.slim": [
      "div\n  -if a\n    p=x\n    =render y\n",
      "div\n  - if a\n    p= x\n    = render y\n",
      "div\n  -if a\n    p=x\n    =render y\n",
    ],
  };
  fs.writeFileSync(inlineToml, "");
  for (const [name, [messy, , off]] of Object.entries(inlineTexts)) {
    assert.strictEqual(
      inlineFormat(name, messy),
      off,
      `${name}: off by default`,
    );
  }
  fs.writeFileSync(
    inlineToml,
    [
      "Layout.SpaceInsideErbTag",
      "Layout.SpaceInHtmlTag",
      "Style.ErbCommentSyntax",
      "Layout.SpaceAfterScriptMarker",
    ]
      .map((rule) => `[${rule}]\nenabled = true\n`)
      .join(""),
  );
  for (const [name, [messy, tidy]] of Object.entries(inlineTexts)) {
    assert.strictEqual(inlineFormat(name, messy), tidy, name);
    assert.strictEqual(inlineFormat(name, tidy), tidy, `${name}: stable`);
  }
  // A rule applies to its languages only.
  assert.strictEqual(
    inlineFormat("b.html.slim", "p\n  <%=x%>\n"),
    "p\n  <%=x%>\n",
  );

  // The style rules rewrite tags, and `style` picks quotes and the end of void tags; Slim gets the tag name case only.
  const tagStyle = path.join(root, "tag-style");
  fs.mkdirSync(tagStyle);
  const tagStyleFormat = (name, text) =>
    run(["format", "--stdin-filepath", path.join(tagStyle, name)], text).stdout;
  const loud = "<DIV class='a' id=b><INPUT checked=\"checked\" /><BR/></DIV>\n";
  fs.writeFileSync(path.join(tagStyle, ".ruby-templates.toml"), "");
  assert.strictEqual(
    tagStyleFormat("a.html.erb", loud),
    loud,
    "off by default",
  );
  fs.writeFileSync(
    path.join(tagStyle, ".ruby-templates.toml"),
    [
      '[Style.AttributeQuotes]\nenabled = true\nstyle = "single_quotes"',
      "[Style.BooleanAttributes]\nenabled = true",
      '[Style.VoidTagClosing]\nenabled = true\nstyle = "slash"',
      "[Style.TagNameCase]\nenabled = true",
    ].join("\n") + "\n",
  );
  const quiet = "<div class='a' id='b'><input checked /><br /></div>\n";
  assert.strictEqual(tagStyleFormat("a.html.erb", loud), quiet);
  assert.strictEqual(tagStyleFormat("a.html.erb", quiet), quiet, "stable");
  assert.strictEqual(
    tagStyleFormat("a.html.slim", "DIV.a\n  SPAN x\n"),
    "div.a\n    span x\n",
  );
  assert.strictEqual(
    tagStyleFormat("a.xml.slim", "DIV.a\n"),
    "DIV.a\n",
    "XML output",
  );

  // The Haml and Slim tag rules, with `style` picking the attribute syntax.
  fs.writeFileSync(
    path.join(tagStyle, ".ruby-templates.toml"),
    '[Style.ImplicitDiv]\nenabled = true\n[Style.ClassesBeforeIds]\nenabled = true\n[Style.HashAttributes]\nenabled = true\nstyle = "html"\n',
  );
  assert.strictEqual(
    tagStyleFormat("a.html.haml", '%div#a.b{href: "/"} x\n'),
    '.b#a(href="/") x\n',
  );
  assert.strictEqual(tagStyleFormat("a.html.slim", "div#a.b x\n"), "#a.b x\n");

  // Layout/ElementContentOnOwnLine moves the content of the listed tags; Layout/LineLength only reports.
  fs.writeFileSync(
    path.join(tagStyle, ".ruby-templates.toml"),
    '[Layout.ElementContentOnOwnLine]\nenabled = true\ntags = ["Div", "li"]\n[Layout.LineLength]\nenabled = true\nmax = 20\n',
  );
  const content = "<div><li>y</li></div>\n";
  const moved = "<div>\n  <li>\n    y\n  </li>\n</div>\n";
  assert.strictEqual(tagStyleFormat("a.html.erb", content), moved);
  assert.strictEqual(tagStyleFormat("a.html.erb", moved), moved, "stable");
  assert.strictEqual(
    tagStyleFormat("a.html.haml", "%div\n  %li y\n  %p x\n"),
    "%div\n  %li\n    y\n  %p x\n",
  );
  assert.strictEqual(
    tagStyleFormat("a.html.slim", "div\n  li y\n  p x\n"),
    "div\n    li\n        | y\n    p x\n",
  );
  const longLine = path.join(tagStyle, "long.html.erb");
  fs.writeFileSync(longLine, "<p>short</p>\n<p>this line is longer</p>\n");
  r = run(["check", longLine]);
  assert.strictEqual(r.status, 1);
  assert.match(
    r.stdout,
    /long\.html\.erb:2:21: warning: line is 26 characters long; the limit is 20/,
  );
  fs.writeFileSync(
    path.join(tagStyle, ".ruby-templates.toml"),
    '[Layout.ElementContentOnOwnLine]\ntags = ["<p>"]\n',
  );
  r = run(["check", longLine]);
  assert.match(
    r.stdout,
    /\.ruby-templates\.toml:2:9: warning: `<p>` is not an element name/,
  );

  // Layout/AttributesPerLine splits the start tags over max_line_length and leaves the others.
  fs.writeFileSync(
    path.join(tagStyle, ".ruby-templates.toml"),
    "[Layout.AttributesPerLine]\nenabled = true\nmax_line_length = 40\n",
  );
  const perLine =
    '<ul>\n<li class="a" id="b" data-x="yyyyyyyyyyyyyyy">z</li>\n<li class="a" id="b">z</li>\n</ul>\n';
  const perLineSplit =
    '<ul>\n  <li\n    class="a"\n    id="b"\n    data-x="yyyyyyyyyyyyyyy">z</li>\n  <li class="a" id="b">z</li>\n</ul>\n';
  assert.strictEqual(tagStyleFormat("a.html.erb", perLine), perLineSplit);
  assert.strictEqual(
    tagStyleFormat("a.html.erb", perLineSplit),
    perLineSplit,
    "stable",
  );

  // Layout/IndentationStyle (ERB tabs, from the file or .editorconfig) and Layout/TrailingWhitespace.
  const layout = path.join(root, "layout");
  fs.mkdirSync(layout);
  const layoutToml = path.join(layout, ".ruby-templates.toml");
  fs.writeFileSync(
    path.join(layout, ".editorconfig"),
    "root = true\n[*]\nindent_size = 2\n",
  );
  const layoutFormat = (name, text) =>
    run(["format", "--stdin-filepath", path.join(layout, name)], text).stdout;
  const spaced = "<div>\n  <pre>\n  a  \n  </pre>\n  <p>x</p>\n</div>\n";
  const tabbed = "<div>\n\t<pre>\n  a  \n  </pre>\n\t<p>x</p>\n</div>\n";
  fs.writeFileSync(layoutToml, "");
  assert.strictEqual(
    layoutFormat("a.html.erb", tabbed),
    spaced,
    "spaces by default",
  );
  fs.writeFileSync(layoutToml, '[Layout.IndentationStyle]\nstyle = "tabs"\n');
  assert.strictEqual(
    layoutFormat("a.html.erb", spaced),
    tabbed,
    "tabs, <pre> kept",
  );
  assert.strictEqual(layoutFormat("a.html.erb", tabbed), tabbed, "stable");
  assert.strictEqual(
    layoutFormat("a.html.slim", "div\n    p x\n"),
    "div\n  p x\n",
    "Slim stays spaces",
  );
  fs.writeFileSync(layoutToml, "");
  fs.writeFileSync(
    path.join(layout, ".editorconfig"),
    "root = true\n[*]\nindent_style = tab\n",
  );
  assert.strictEqual(
    layoutFormat("a.html.erb", spaced),
    tabbed,
    ".editorconfig indent_style = tab",
  );
  fs.writeFileSync(layoutToml, '[Layout.IndentationStyle]\nstyle = "spaces"\n');
  assert.strictEqual(
    layoutFormat("a.html.erb", tabbed),
    spaced,
    "the toml beats .editorconfig",
  );
  fs.writeFileSync(
    layoutToml,
    "[Layout.TrailingWhitespace]\nenabled = false\n",
  );
  const trailing = {
    "a.html.erb": ["<p>x \t\n  \n</p>  \n", "<p>x\n\n</p>\n"],
    "a.html.haml": ["%p  \n   \n  %b x \n", "%p\n\n  %b x\n"],
    "a.html.slim": ["p  \n   \n  b \n", "p\n\n  b\n"],
  };
  for (const [name, [untidy, tidy]] of Object.entries(trailing)) {
    assert.strictEqual(
      layoutFormat(name, untidy),
      untidy,
      `${name}: kept when off`,
    );
    fs.writeFileSync(layoutToml, "");
    assert.strictEqual(
      layoutFormat(name, untidy),
      tidy,
      `${name}: stripped by default`,
    );
    fs.writeFileSync(
      layoutToml,
      "[Layout.TrailingWhitespace]\nenabled = false\n",
    );
  }

  // --format: relative paths, characters instead of UTF-16 units, escaping, and a config problem.
  const ci = path.join(root, "ci");
  fs.mkdirSync(path.join(ci, "app", "views"), { recursive: true });
  fs.writeFileSync(
    path.join(ci, "app", "views", "a b.html.erb"),
    '<p>😀д</p><img src="a">\n',
  );
  fs.writeFileSync(path.join(ci, ".ruby-templates.toml"), "[Html.ImgAlt\n");
  const checkCi = (format) =>
    run(["check", "app", "--format", format], undefined, ci);
  const imgAlt =
    "`<img>` without `alt`: the alt attribute must be specified (HTML 4.8.4.4.1)";
  const tomlProblem = "invalid TOML: unclosed table, expected `]`";
  r = checkCi("text");
  assert.strictEqual(r.status, 1);
  assert.strictEqual(
    r.stdout,
    `.ruby-templates.toml:1:13: warning: ${tomlProblem}\n` +
      `app/views/a b.html.erb:1:11: warning: ${imgAlt}\n`,
    "text keeps UTF-16 columns",
  );
  r = checkCi("github");
  assert.strictEqual(r.status, 1);
  assert.strictEqual(
    r.stdout,
    `::warning file=.ruby-templates.toml,line=1,col=13,endLine=1,endColumn=13,title=Config/Invalid::${tomlProblem}\n` +
      `::warning file=app/views/a b.html.erb,line=1,col=10,endLine=1,endColumn=14,title=Html/ImgAlt::${imgAlt}\n`,
  );
  r = checkCi("json");
  assert.strictEqual(r.status, 1);
  assert.deepStrictEqual(r.stdout.trim().split("\n").map(JSON.parse), [
    {
      file: ".ruby-templates.toml",
      line: 1,
      column: 13,
      endLine: 1,
      endColumn: 13,
      severity: "warning",
      rule: "Config/Invalid",
      message: tomlProblem,
    },
    {
      file: "app/views/a b.html.erb",
      line: 1,
      column: 10,
      endLine: 1,
      endColumn: 14,
      severity: "warning",
      rule: "Html/ImgAlt",
      message: imgAlt,
    },
  ]);
  r = checkCi("sarif");
  assert.strictEqual(r.status, 1);
  const sarif = JSON.parse(r.stdout);
  const [sarifRun] = sarif.runs;
  assert.strictEqual(sarif.version, "2.1.0");
  assert.strictEqual(sarifRun.tool.driver.name, "ruby-templates");
  assert.strictEqual(sarifRun.columnKind, "unicodeCodePoints");
  const site = "https://x-ror.github.io/vscode-ruby-templates/reference/";
  assert.deepStrictEqual(sarifRun.tool.driver.rules, [
    {
      id: "Config/Invalid",
      shortDescription: { text: "A problem in a .ruby-templates.toml." },
      helpUri: `${site}rules/#configuration`,
    },
    {
      id: "Html/ImgAlt",
      shortDescription: { text: "An image without alt text." },
      fullDescription: { text: "An image without alt text." },
      helpUri: `${site}diagnostics/#html-imgalt`,
    },
  ]);
  assert.deepStrictEqual(sarifRun.results[1], {
    ruleId: "Html/ImgAlt",
    level: "warning",
    message: { text: imgAlt },
    locations: [
      {
        physicalLocation: {
          artifactLocation: {
            uri: pathToFileURL(path.join(ci, "app", "views", "a b.html.erb"))
              .href,
          },
          region: { startLine: 1, startColumn: 10, endLine: 1, endColumn: 14 },
        },
      },
    ],
  });
  assert.strictEqual(sarifRun.results[0].ruleId, "Config/Invalid");
  assert.strictEqual(sarifRun.results[0].level, "warning");
  // Where a config problem prints doesn't depend on which worker gets there first.
  for (let i = 0; i < 12; i++) {
    fs.writeFileSync(path.join(ci, "app", `b${i}.html.erb`), "<p>ok</p>\n");
  }
  for (let i = 0; i < 5; i++) {
    const lines = checkCi("github").stdout.trim().split("\n");
    assert.strictEqual(lines.length, 2, lines.join("\n"));
    assert.match(lines[0], /^::warning file=\.ruby-templates\.toml,/);
  }
  fs.writeFileSync(
    path.join(ci, "app", "bom.html.erb"),
    '\uFEFF<img src="a">\n',
  );
  assert.match(
    run(["check", "app/bom.html.erb", "--format", "github"], undefined, ci)
      .stdout,
    /file=app\/bom\.html\.erb,line=1,col=1,endLine=1,endColumn=5,/,
    "a byte order mark takes no column",
  );
  if (process.platform !== "win32") {
    const link = path.join(root, "ci-link");
    fs.symlinkSync(ci, link);
    r = run(
      ["check", path.join(link, "app", "bom.html.erb"), "--format", "json"],
      undefined,
      link,
    );
    assert.strictEqual(
      JSON.parse(r.stdout.split("\n").at(-2)).file,
      "app/bom.html.erb",
    );
  }
  fs.rmSync(path.join(ci, "app", "bom.html.erb"));
  r = run(["check", views, "--format", "json"]);
  assert.ok(
    r.stdout
      .trim()
      .split("\n")
      .map(JSON.parse)
      .some((f) => f.severity === "error"),
    "syntax errors are errors",
  );
  r = run(["check", clean, "--format", "sarif"]);
  assert.strictEqual(r.status, 0, "no findings, exit 0");
  assert.deepStrictEqual(JSON.parse(r.stdout).runs[0].results, []);
  r = run(["check", "--format", "xml", clean]);
  assert.strictEqual(r.status, 2);
  assert.match(r.stderr, /unknown format xml/);

  fs.mkdirSync(path.join(ci, "fmt"));
  fs.writeFileSync(path.join(ci, "fmt", "m,n.html.slim"), "div\n p  \n");
  fs.writeFileSync(path.join(ci, ".ruby-templates.toml"), "");
  r = run(["format", "--check", "--format", "github", "fmt"], undefined, ci);
  assert.strictEqual(r.status, 1);
  assert.strictEqual(
    r.stdout,
    "::warning file=fmt/m%2Cn.html.slim,title=Format::needs formatting; run `ruby-templates format --write`\n",
  );
  assert.strictEqual(
    run(["format", "--format", "github", "fmt"], undefined, ci).status,
    2,
    "--format github needs --check",
  );

  console.log("cli ok");
} finally {
  fs.rmSync(root, { recursive: true, force: true });
}
