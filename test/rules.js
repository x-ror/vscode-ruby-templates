// The examples on the site's rules page are tests: for every available autofix rule, format each `before` with the
// rule switched on in a .ruby-templates.toml and expect `after`. For every available warning, `check` prints more for
// `before` than with the rule switched off, and the same for `after`. A partial rule's examples show what it will do.
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

  // `check` doesn't print codes, so the rule shows as the difference a .ruby-templates.toml switching it off makes.
  const folder = (rule, enabled) => {
    const dir = path.join(root, `${rule.name.replace("/", "-")}-${enabled}`);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(
      path.join(dir, ".ruby-templates.toml"),
      `root = true\n[${rule.name.replace("/", ".")}]\nenabled = ${enabled}\n`,
    );
    // Rails-only rules look for the framework here.
    fs.writeFileSync(
      path.join(dir, "Gemfile.lock"),
      "GEM\n  specs:\n    actionview (8.0.2)\n",
    );
    return dir;
  };
  let reported = 0;
  for (const rule of rules.filter(
    (r) => r.type === "warning" && r.status === "available",
  )) {
    const folders = { true: folder(rule, true), false: folder(rule, false) };
    for (const example of rule.examples.filter(
      (e) => !e.afterFile && !e.note,
    )) {
      const label = `${rule.name} (${example.lang})`;
      const printed = (text, enabled) => {
        const file = path.join(folders[enabled], example.file);
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, text);
        const run = spawnSync(bin, ["check", file], { encoding: "utf8" });
        assert.ok(
          run.status === 0 || run.status === 1,
          `${label}: ${run.error ?? run.stderr}`,
        );
        return run.stdout.replaceAll(folders[enabled], "").split("\n");
      };
      assert.notDeepStrictEqual(
        printed(example.before, true),
        printed(example.before, false),
        `${label}: before is not reported`,
      );
      assert.deepStrictEqual(
        printed(example.after, true),
        printed(example.after, false),
        `${label}: after is reported`,
      );
      reported += 1;
    }
  }
  assert.ok(reported >= 9, `only ${reported} warning examples were run`);
  console.log(`rules ok: ${checked} formatted, ${reported} reported`);
} finally {
  fs.rmSync(root, { recursive: true, force: true });
}
