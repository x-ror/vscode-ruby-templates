// Runs the release build's `--check` on test/sample.*, without relying on the shell to expand the glob.
import { spawnSync } from "child_process";
import fs from "fs";
import path from "path";

const name =
  process.platform === "win32" ? "ruby-templates-ls.exe" : "ruby-templates-ls";
const bin = path.join(import.meta.dirname, "target", "release", name);
const testDir = path.join(import.meta.dirname, "..", "test");
const samples = fs
  .readdirSync(testDir)
  .filter((f) => f.startsWith("sample."))
  .map((f) => path.join(testDir, f));

const { status, error } = spawnSync(bin, ["--check", ...samples], {
  stdio: "inherit",
});
if (error) throw error;
process.exit(status ?? 1);
