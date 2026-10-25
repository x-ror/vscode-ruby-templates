#!/usr/bin/env node
// Runs the `ruby-templates` binary from the package built for this platform (`<name>-<platform>-<arch>`), which npm
// installs through optionalDependencies.
import { spawnSync } from "child_process";
import fs from "fs";
import path from "path";
import { createRequire } from "module";

const require = createRequire(import.meta.url);
const { name, optionalDependencies } = JSON.parse(
  fs.readFileSync(path.join(import.meta.dirname, "..", "package.json"), "utf8"),
);
const exe =
  process.platform === "win32" ? "ruby-templates.exe" : "ruby-templates";

function binary() {
  if (process.env.RUBY_TEMPLATES_BINARY)
    return process.env.RUBY_TEMPLATES_BINARY;
  const platformPackage = `${name}-${process.platform}-${process.arch}`;
  try {
    const manifest = require.resolve(`${platformPackage}/package.json`);
    return path.join(path.dirname(manifest), "bin", exe);
  } catch {
    const why =
      platformPackage in optionalDependencies
        ? `${platformPackage} is not installed; reinstall without --omit=optional`
        : `no prebuilt binary is published for ${process.platform}-${process.arch}`;
    console.error(`ruby-templates: ${why}, or set RUBY_TEMPLATES_BINARY.`);
    process.exit(2);
  }
}

const { status, signal, error } = spawnSync(binary(), process.argv.slice(2), {
  stdio: "inherit",
});
if (error) {
  console.error(`ruby-templates: ${error.message}`);
  process.exit(2);
}
if (signal) process.kill(process.pid, signal);
process.exit(status ?? 2);
