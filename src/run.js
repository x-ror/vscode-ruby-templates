// Running the Ruby CLIs (renderers, converters, bin/rails) for the commands.
import cp from "child_process";
import path from "path";
import fs from "fs";

import { cfg } from "./workspace.js";
import { WINDOWS_COMMAND_NOT_FOUND, windowsShellCommand } from "./shell.js";

// Rails boots slowly on big apps, but a hung runner shouldn't leave the preview waiting forever.
const RAILS_RENDER_TIMEOUT_MS = 60_000;
// How far up from a template to look for a Gemfile or bin/rails.
const MAX_PARENT_DIRS = 6;

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

function run(
  cmd,
  args,
  { useBundler = cfg().get("preview.useBundler", false), ...options },
  input,
) {
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
        { ...options, ...spawnOptions, maxBuffer: 16 * 1024 * 1024 },
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
async function runHaml(bin, subcommand, legacyArgs, cwd, src) {
  const r = await run(bin, [subcommand, "-"], { cwd }, src);
  // Haml < 6 opens the subcommand name as the input file.
  if (
    r.code !== 0 &&
    r.stderr.includes("ENOENT") &&
    r.stderr.includes(` - ${subcommand}`)
  )
    return run(bin, legacyArgs, { cwd }, src);
  // Thor reports usage errors on stderr but exits 0.
  if (r.code === 0 && !r.stdout && r.stderr) return { ...r, code: 1 };
  return r;
}

const firstLine = (r) => (r.stderr || r.stdout).split("\n")[0];

export { RAILS_RENDER_TIMEOUT_MS, findUp, run, runHaml, firstLine };
