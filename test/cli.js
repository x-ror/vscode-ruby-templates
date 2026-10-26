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
const notes = file("notes.txt", "x");

try {
  let r = run(["check", clean]);
  assert.strictEqual(r.status, 0, r.stdout + r.stderr);
  r = run(["check", views]);
  assert.strictEqual(r.status, 1, "a broken file is a finding");
  assert.match(r.stdout, /broken\.html\.slim:\d+:\d+: error: /);
  assert.doesNotMatch(
    r.stdout + r.stderr,
    /notes\.txt/,
    "folders yield templates only",
  );
  r = run(["check", notes]);
  assert.strictEqual(r.status, 2);
  assert.match(r.stderr, /notes\.txt: unknown template type/);

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
  r = run(["format", "--stdin-filepath", "x.txt"], "keep  \n");
  assert.strictEqual(r.stdout, "keep  \n", "unknown files pass through");

  r = run(["format", "--write", views]);
  assert.strictEqual(r.status, 0, r.stderr);
  assert.deepStrictEqual(r.stdout.trim().split("\n"), [messy]);
  assert.strictEqual(fs.readFileSync(messy, "utf8"), "div\n  p\n");
  assert.strictEqual(fs.readFileSync(broken, "utf8"), "p\n    a\n  b\n");
  assert.strictEqual(run(["format", "--check", views]).status, 0);

  // Config: .editorconfig beats the linter config; --verbose lists what it read and ignored.
  fs.writeFileSync(
    path.join(root, ".editorconfig"),
    "root = true\n[*.slim]\nindent_size = 4\nmax_line_length = 100\n",
  );
  fs.writeFileSync(
    path.join(root, ".slim-lint.yml"),
    "linters:\n  LineLength:\n    max: 100\n",
  );
  r = run(["format", "--verbose", messy]);
  assert.strictEqual(r.stdout, "div\n    p\n");
  assert.match(r.stderr, /\.editorconfig: ignored max_line_length = 100/);
  assert.match(r.stderr, /\.slim-lint\.yml: ignored linters\.LineLength/);

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
  fs.rmSync(lock);

  console.log("cli ok");
} finally {
  fs.rmSync(root, { recursive: true, force: true });
}
