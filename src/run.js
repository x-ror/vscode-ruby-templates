// Running the Ruby CLIs (renderers, converters, bin/rails) for the commands.
import cp from "child_process";
import path from "path";
import fs from "fs";

import { WINDOWS_COMMAND_NOT_FOUND, windowsShellCommand } from "./shell.js";

// Rails boots slowly on big apps, but a hung runner shouldn't leave the preview waiting forever.
const RAILS_RUNNER_TIMEOUT_MS = 60_000;
// How far up from a template to look for a Gemfile or bin/rails.
const MAX_PARENT_DIRS = 6;
const MAX_OUTPUT_BYTES = 16 * 1024 * 1024;
const RAILS_BIN = path.join("bin", "rails");

/** The nearest of `dir` and its parents that contains `rel`, or undefined. */
function findUp(dir, rel) {
  let d = dir;
  for (let i = 0; i < MAX_PARENT_DIRS; i++) {
    if (fs.existsSync(path.join(d, rel))) return d;
    const parent = path.dirname(d);
    if (parent === d) break;
    d = parent;
  }
  return undefined;
}

function run(cmd, args, { useBundler, ...options }, input) {
  return new Promise((resolve) => {
    const [bin, binArgs] =
      useBundler && findUp(options.cwd, "Gemfile") !== undefined
        ? ["bundle", ["exec", cmd, ...args]]
        : [cmd, args];
    const windows = process.platform === "win32";
    try {
      const [file, fileArgs, spawnOptions] = windows
        ? windowsShellCommand(bin, binArgs)
        : [bin, binArgs, {}];
      const child = cp.execFile(
        file,
        fileArgs,
        { ...options, ...spawnOptions, maxBuffer: MAX_OUTPUT_BYTES },
        (err, stdout, stderr) => {
          // Callers detect a missing tool by ENOENT, which a shell can't report.
          if (windows && err?.code === WINDOWS_COMMAND_NOT_FOUND) {
            err.code = "ENOENT";
            err.path = bin;
          }
          resolve({
            code: err ? (err.code ?? 1) : 0,
            stdout: String(stdout || ""),
            stderr: String(stderr || ""),
            err,
          });
        },
      );
      if (input !== undefined && child.stdin) {
        child.stdin.on("error", () => {});
        child.stdin.end(input);
      }
    } catch (err) {
      resolve({ code: err.code ?? 1, stdout: "", stderr: err.message, err });
    }
  });
}

/** `haml <subcommand> -` (Haml 6+), falling back to the Haml < 6 CLI, which has no subcommands and reads stdin. */
async function runHaml(bin, subcommand, legacyArgs, options, src) {
  const r = await run(bin, [subcommand, "-"], options, src);
  // Haml < 6 opens the subcommand name as the input file.
  if (
    r.code !== 0 &&
    r.stderr.includes("ENOENT") &&
    r.stderr.includes(` - ${subcommand}`)
  )
    return run(bin, legacyArgs, options, src);
  // Thor reports usage errors on stderr but exits 0.
  if (r.code === 0 && !r.stdout && r.stderr) return { ...r, code: 1 };
  return r;
}

/** `bin/rails runner script` in the app at `root`, a run that times out reported as a failure. */
async function railsRunner(root, script, { ruby, useBundler, input }) {
  const r = await run(
    ruby,
    [RAILS_BIN, "runner", script],
    { cwd: root, timeout: RAILS_RUNNER_TIMEOUT_MS, useBundler },
    input,
  );
  if (!r.err?.killed) return r;
  return {
    ...r,
    code: 1,
    stderr: `bin/rails runner timed out after ${RAILS_RUNNER_TIMEOUT_MS / 1000}s.\n${r.stderr}`,
  };
}

const firstLine = (r) => (r.stderr || r.stdout).split("\n")[0];

export { RAILS_BIN, findUp, run, runHaml, railsRunner, firstLine };
