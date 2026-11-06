// The examples on the site's rules page are tests: for every available autofix rule, format each `before` with the
// rule switched on in a .ruby-templates.toml and expect `after`. A partial rule's examples show what it will do.
import assert from "assert";
import { spawnSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";

const exe = process.platform === "win32" ? ".exe" : "";
// Resolved before the chdir: CI passes a path relative to the repository.
const bin = path.resolve(
  process.env.RUBY_TEMPLATES ??
    path.join(
      import.meta.dirname,
      "..",
      "server",
      "target",
      "release",
      `ruby-templates${exe}`,
    ),
);

// The site's data modules read the repository's files relative to the site folder, where its builds run.
process.chdir(path.join(import.meta.dirname, "..", "site"));
const { rules } = await import("../site/src/data/rules.ts");

const root = fs.realpathSync(
  fs.mkdtempSync(path.join(os.tmpdir(), "rt-rules-")),
);
let checked = 0;
try {
  for (const rule of rules.filter(
    (r) => r.type === "autofix" && r.status === "available",
  )) {
    const table = rule.name.replace("/", ".");
    const config = rule.config ?? `[${table}]\nenabled = true\n`;
    const dir = path.join(root, rule.name.replace("/", "-"));
    fs.mkdirSync(dir);
    fs.writeFileSync(
      path.join(dir, ".ruby-templates.toml"),
      `root = true\n${config}`,
    );
    for (const example of rule.examples) {
      const file = path.join(dir, example.file);
      const run = spawnSync(bin, ["format", "--stdin-filepath", file], {
        input: example.before,
        encoding: "utf8",
      });
      const label = `${rule.name} (${example.lang})`;
      assert.strictEqual(run.status, 0, `${label}: ${run.error ?? run.stderr}`);
      assert.strictEqual(run.stdout, example.after, label);
      checked += 1;
    }
  }
  assert.ok(checked >= 9, `only ${checked} examples were run`);
  console.log(`rules ok: ${checked} examples`);
} finally {
  fs.rmSync(root, { recursive: true, force: true });
}
