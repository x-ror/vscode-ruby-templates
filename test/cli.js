// The `ruby-templates` command line on a temp folder: check, format to stdout, --check, --write, stdin, exit codes.
import assert from "assert";
import { spawnSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";

const exe = process.platform === "win32" ? ".exe" : "";
// CI builds per target and passes the binary in.
const bin =
  process.env.RUBY_TEMPLATES ??
  path.join(
    import.meta.dirname,
    "..",
    "server",
    "target",
    "release",
    `ruby-templates${exe}`,
  );
const run = (args, input) => {
  const r = spawnSync(bin, args, { encoding: "utf8", input });
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
      "<div>\n<% # note %>\n<p><%=user.name%></p>\n</div>\n",
      "<div>\n  <%# note %>\n  <p><%= user.name %></p>\n</div>\n",
      "<div>\n  <% # note %>\n  <p><%=user.name%></p>\n</div>\n",
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

  console.log("cli ok");
} finally {
  fs.rmSync(root, { recursive: true, force: true });
}
