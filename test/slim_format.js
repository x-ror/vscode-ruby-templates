// The Slim formatter must never change what Slim compiles: compile each fixture with the slim gem
// before and after `ruby-templates-ls --format` and compare. Skipped when `ruby -rslim` is missing.
const assert = require("assert");
const { execFileSync, spawnSync } = require("child_process");
const fs = require("fs"),
  path = require("path");

const bin = path.join(
  __dirname,
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

if (spawnSync("ruby", ["-rslim", "-e", ""]).status !== 0) {
  console.log("slim formatting: skipped (`ruby -rslim` unavailable)");
  process.exit(0);
}

const dir = path.join(__dirname, "format");
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
