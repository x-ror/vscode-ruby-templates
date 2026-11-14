// Runs the release build's `check` on test/sample.*, without relying on the shell to expand the glob.
// The samples exercise every construct, so warnings are expected; only an error or a failed run fails.
import { spawnSync } from "child_process";
import fs from "fs";
import path from "path";

const name =
  process.platform === "win32" ? "ruby-templates.exe" : "ruby-templates";
const bin =
  process.env.RUBY_TEMPLATES ??
  path.join(import.meta.dirname, "target", "release", name);
const testDir = path.join(import.meta.dirname, "..", "test");
const samples = fs
  .readdirSync(testDir)
  .filter((f) => f.startsWith("sample."))
  .map((f) => path.join(testDir, f));

const { status, error, stdout, stderr } = spawnSync(
  bin,
  ["check", ...samples],
  {
    encoding: "utf8",
  },
);
if (error) throw error;
process.stdout.write(stdout);
process.stderr.write(stderr);
const FINDINGS = 1;
if (status !== 0 && status !== FINDINGS) process.exit(status ?? 2);
process.exit(/: error: /.test(stdout) ? FINDINGS : 0);
