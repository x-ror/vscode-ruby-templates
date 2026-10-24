// The npm packages end to end: assemble them from the local release build, pack, install into a temp project
// offline, and run the `ruby-templates` bin through the wrapper.
import assert from "assert";
import { execFileSync, spawnSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";
import { assemble } from "../npm/assemble.js";

const exe = process.platform === "win32" ? ".exe" : "";
const built =
  process.env.RUBY_TEMPLATES ??
  path.join(
    import.meta.dirname,
    "..",
    "server",
    "target",
    "release",
    `ruby-templates${exe}`,
  );
const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "rt-npm-")));
// npm is a .cmd on Windows, which spawn only runs through a shell.
const npm = (args, cwd) =>
  execFileSync("npm", args, {
    cwd,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    shell: process.platform === "win32",
  });

try {
  const binaries = path.join(
    root,
    "binaries",
    `cli-${process.platform}-${process.arch}`,
  );
  fs.mkdirSync(binaries, { recursive: true });
  fs.copyFileSync(built, path.join(binaries, `ruby-templates${exe}`));
  const folders = assemble(
    path.dirname(binaries),
    path.join(root, "packages"),
    "0.0.1-test",
  );
  assert.strictEqual(
    folders.length,
    2,
    "this platform's package and the main one",
  );
  const main = JSON.parse(
    fs.readFileSync(path.join(folders[1], "package.json"), "utf8"),
  );
  assert.deepStrictEqual(Object.keys(main.optionalDependencies), [
    `${main.name}-${process.platform}-${process.arch}`,
  ]);

  const tarballs = folders.map((dir) =>
    path.join(
      root,
      npm(["pack", "--pack-destination", root], dir).trim().split("\n").at(-1),
    ),
  );
  const project = path.join(root, "project");
  fs.mkdirSync(project);
  fs.writeFileSync(
    path.join(project, "package.json"),
    '{ "name": "rt-npm-test", "private": true }\n',
  );
  npm(
    ["install", "--offline", "--no-audit", "--no-fund", ...tarballs],
    project,
  );

  const bin = path.join(
    project,
    "node_modules",
    ".bin",
    `ruby-templates${process.platform === "win32" ? ".cmd" : ""}`,
  );
  const run = (args, input) =>
    spawnSync(bin, args, {
      cwd: project,
      input,
      encoding: "utf8",
      shell: process.platform === "win32",
    });
  let r = run(
    ["format", "--stdin-filepath", "app/views/users/show.html.erb"],
    "<div>\n<p>x</p>  \n</div>",
  );
  assert.strictEqual(r.status, 0, r.stderr);
  assert.strictEqual(r.stdout, "<div>\n  <p>x</p>\n</div>\n");
  r = run(
    ["format", "--check", "--stdin-filepath", "a.html.erb"],
    "<div>\n<p>x</p>\n</div>\n",
  );
  assert.strictEqual(r.status, 1, "exit codes come through the wrapper");
  r = run(["--wat"]);
  assert.strictEqual(r.status, 2);
  console.log("npm package ok");
} finally {
  fs.rmSync(root, { recursive: true, force: true });
}
