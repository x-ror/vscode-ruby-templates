// The Slim and Haml formatters must never change what the template compiles to: compile each fixture with the slim
// or haml gem before and after formatting and compare. Skipped when the gem is missing.
import assert from "assert";
import { execFileSync, spawnSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";

const bin = path.join(
  import.meta.dirname,
  "..",
  "server",
  "target",
  "release",
  "ruby-templates-ls",
);
// Slim keeps a `\n;` per source line; trailing blank lines dropped by the formatter only remove the last ones.
const compile = (source) =>
  execFileSync(
    "ruby",
    ["-rslim", "-e", "print Slim::Engine.new.call(STDIN.read)"],
    { input: source, encoding: "utf8" },
  ).replace(/(\n; )+_buf$/, "_buf");
const format = (file) =>
  spawnSync(bin, ["--format", file], { encoding: "utf8" });

const dir = path.join(import.meta.dirname, "format");
const hasGem = (gem) => spawnSync("ruby", [`-r${gem}`, "-e", ""]).status === 0;

// `ruby-templates-ls --format`, which the Slim half also keeps covered.
function checkSlim() {
  for (const name of fs.readdirSync(dir).filter((f) => f.endsWith(".slim"))) {
    const file = path.join(dir, name);
    const source = fs.readFileSync(file, "utf8");
    const run = format(file);
    assert.strictEqual(run.status, 0, `${name}: ${run.stderr}`);
    assert.notStrictEqual(
      run.stdout,
      source,
      `${name}: fixture should need formatting`,
    );
    assert.strictEqual(
      compile(run.stdout),
      compile(source),
      `${name}: compiled output changed`,
    );
    const again = path.join(dir, `.${name}.formatted`);
    fs.writeFileSync(again, run.stdout);
    try {
      assert.strictEqual(
        format(again).stdout,
        run.stdout,
        `${name}: not idempotent`,
      );
    } finally {
      fs.rmSync(again);
    }
  }
  console.log("slim formatting keeps compiled output ok");
}

const cli =
  process.env.RUBY_TEMPLATES ??
  path.join(
    import.meta.dirname,
    "..",
    "server",
    "target",
    "release",
    `ruby-templates${process.platform === "win32" ? ".exe" : ""}`,
  );
// Haml numbers its temporaries per process; the numbers aren't output.
const compileHaml = (source) =>
  execFileSync(
    "ruby",
    [
      "-rhaml",
      "-e",
      "print Haml::Engine.new.call(STDIN.read).gsub(/_(module|haml_compiler)\\d+/, '_x')",
    ],
    { input: source, encoding: "utf8" },
  );
const formatHaml = (name, source) =>
  spawnSync(cli, ["format", "--stdin-filepath", name], {
    input: source,
    encoding: "utf8",
  });
function checkHaml() {
  for (const name of fs.readdirSync(dir).filter((f) => f.endsWith(".haml"))) {
    const source = fs.readFileSync(path.join(dir, name), "utf8");
    const run = formatHaml(name, source);
    assert.strictEqual(run.status, 0, `${name}: ${run.stderr}`);
    assert.notStrictEqual(
      run.stdout,
      source,
      `${name}: fixture should need formatting`,
    );
    assert.strictEqual(
      compileHaml(run.stdout),
      compileHaml(source),
      `${name}: compiled output changed`,
    );
    assert.strictEqual(
      formatHaml(name, run.stdout).stdout,
      run.stdout,
      `${name}: not idempotent`,
    );
  }
  console.log("haml formatting keeps compiled output ok");
}

if (hasGem("slim")) checkSlim();
else console.log("slim formatting: skipped (`ruby -rslim` unavailable)");
if (hasGem("haml")) checkHaml();
else console.log("haml formatting: skipped (`ruby -rhaml` unavailable)");

// The Style/ImplicitDiv, ClassesBeforeIds and HashAttributes rules rewrite tags: the page must come out the same.
const styledDir = path.join(dir, "styled");
const renderHaml = (source) =>
  execFileSync(
    "ruby",
    [
      "-rhaml",
      "-e",
      'class Ctx; def initialize; @t = "T&<"; @n = 3; end; def url; "/u?a=1&b=2"; end; end; print Haml::Template.new { STDIN.read }.render(Ctx.new)',
    ],
    { input: source, encoding: "utf8" },
  );
const withRules = (rules, work) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "rt-style-"));
  try {
    fs.writeFileSync(
      path.join(root, ".ruby-templates.toml"),
      `root = true\n${rules}\n`,
    );
    work(root);
  } finally {
    fs.rmSync(root, { recursive: true });
  }
};
function checkStyleRules() {
  const run = (root, name, source) => {
    const done = spawnSync(
      cli,
      ["format", "--verbose", "--stdin-filepath", path.join(root, name)],
      { input: source, encoding: "utf8" },
    );
    assert.strictEqual(done.status, 0, `${name}: ${done.stderr}`);
    assert.ok(
      !/kept/.test(done.stderr),
      `${name}: a rule was refused: ${done.stderr}`,
    );
    return done.stdout;
  };
  const haml = fs.readFileSync(path.join(styledDir, "markup.haml"), "utf8");
  for (const style of ["ruby_hash", "html"]) {
    const rules = `[Style.ImplicitDiv]\nenabled = true\n[Style.ClassesBeforeIds]\nenabled = true\n[Style.HashAttributes]\nenabled = true\nstyle = "${style}"`;
    withRules(rules, (root) => {
      const formatted = run(root, "markup.haml", haml);
      assert.notStrictEqual(formatted, haml, `haml ${style}: should change`);
      assert.strictEqual(
        renderHaml(formatted),
        renderHaml(haml),
        `haml ${style}: rendered page changed`,
      );
      assert.strictEqual(
        run(root, "markup.haml", formatted),
        formatted,
        `haml ${style}: not idempotent`,
      );
    });
  }
  if (hasGem("slim")) {
    const slim = fs.readFileSync(path.join(styledDir, "markup.slim"), "utf8");
    withRules("[Style.ImplicitDiv]\nenabled = true", (root) => {
      const formatted = run(root, "markup.slim", slim);
      assert.notStrictEqual(formatted, slim, "slim: should change");
      assert.strictEqual(
        compile(formatted),
        compile(slim),
        "slim: compiled output changed",
      );
      assert.strictEqual(
        run(root, "markup.slim", formatted),
        formatted,
        "slim: not idempotent",
      );
    });
  }
  console.log("style rules keep the page ok");
}
if (hasGem("haml")) checkStyleRules();

// Layout/ElementContentOnOwnLine adds whitespace only: Slim renders the same page, Haml the same apart from the blanks
// around tags.
function checkContentRule() {
  const ctx =
    'class Ctx; def initialize; @t = "T&<"; end; end; print %s.new { STDIN.read }.render(Ctx.new)';
  const render = (gem, template, source) =>
    execFileSync("ruby", [`-r${gem}`, "-e", ctx.replace("%s", template)], {
      input: source,
      encoding: "utf8",
    });
  const aroundTags = (html) => html.replace(/\s*(<[^>]*>)\s*/g, "$1").trim();
  const run = (root, name, source) => {
    const done = spawnSync(
      cli,
      ["format", "--verbose", "--stdin-filepath", path.join(root, name)],
      { input: source, encoding: "utf8" },
    );
    assert.strictEqual(done.status, 0, `${name}: ${done.stderr}`);
    assert.ok(!/kept/.test(done.stderr), `${name}: refused: ${done.stderr}`);
    return done.stdout;
  };
  const rule = "[Layout.ElementContentOnOwnLine]\nenabled = true";
  for (const [lang, gem, template, same] of [
    ["haml", "haml", "Haml::Template", aroundTags],
    ["slim", "slim", "Slim::Template", (html) => html],
  ]) {
    if (!hasGem(gem)) continue;
    const name = `content.${lang}`;
    const source = fs.readFileSync(path.join(styledDir, name), "utf8");
    withRules(rule, (root) => {
      const formatted = run(root, name, source);
      assert.notStrictEqual(formatted, source, `${lang}: should change`);
      assert.strictEqual(
        same(render(gem, template, formatted)),
        same(render(gem, template, source)),
        `${lang}: rendered page changed`,
      );
      assert.strictEqual(
        run(root, name, formatted),
        formatted,
        `${lang}: not idempotent`,
      );
    });
  }
  console.log("element content rule keeps the page ok");
}
if (hasGem("haml") || hasGem("slim")) checkContentRule();
