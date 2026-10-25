// The Slim and Haml formatters must never change what the template compiles to: compile each fixture with the slim
// or haml gem before and after formatting and compare. Skipped when the gem is missing.
import assert from "assert";
import { execFileSync, spawnSync } from "child_process";
import fs from "fs";
import path from "path";

const bin = path.join(
  import.meta.dirname,
  "..",
  "server",
  "target",
  "release",
  "ruby-templates-ls",
);
const compile = (source) =>
  execFileSync(
    "ruby",
    ["-rslim", "-e", "print Slim::Engine.new.call(STDIN.read)"],
    { input: source, encoding: "utf8" },
  );
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
