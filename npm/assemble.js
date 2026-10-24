// Builds the npm packages: one per platform holding its `ruby-templates` binary, and the main package
// (`npm/ruby-templates`, whose `name` is the one place the package name is set) depending on them.
//
//   bun npm/assemble.js <binaries> <out> [version]
//
// <binaries> holds `cli-<platform>/ruby-templates[.exe]`, as CI downloads its artifacts; platforms without a folder
// are left out. The version defaults to the extension's.
import fs from "fs";
import path from "path";

const PLATFORMS = {
  "linux-x64": { os: "linux", cpu: "x64" },
  "linux-arm64": { os: "linux", cpu: "arm64" },
  "darwin-x64": { os: "darwin", cpu: "x64" },
  "darwin-arm64": { os: "darwin", cpu: "arm64" },
  "win32-x64": { os: "win32", cpu: "x64" },
};

const readJson = (file) => JSON.parse(fs.readFileSync(file, "utf8"));
const writeJson = (file, data) =>
  fs.writeFileSync(file, `${JSON.stringify(data, null, 2)}\n`);

/** Writes the packages under `out` and returns their folders, platform packages first. */
export function assemble(binaries, out, version) {
  const main = path.join(import.meta.dirname, "ruby-templates");
  const manifest = readJson(path.join(main, "package.json"));
  version ??= readJson(
    path.join(import.meta.dirname, "..", "package.json"),
  ).version;
  const license = path.join(import.meta.dirname, "..", "LICENSE");
  fs.rmSync(out, { recursive: true, force: true });
  const folders = [];
  const optional = {};
  for (const [platform, { os, cpu }] of Object.entries(PLATFORMS)) {
    const exe = os === "win32" ? "ruby-templates.exe" : "ruby-templates";
    const built = path.join(binaries, `cli-${platform}`, exe);
    if (!fs.existsSync(built)) continue;
    const name = `${manifest.name}-${platform}`;
    const dir = path.join(out, platform);
    fs.mkdirSync(path.join(dir, "bin"), { recursive: true });
    fs.copyFileSync(built, path.join(dir, "bin", exe));
    // Artifacts lose the executable bit.
    fs.chmodSync(path.join(dir, "bin", exe), 0o755);
    fs.copyFileSync(license, path.join(dir, "LICENSE"));
    writeJson(path.join(dir, "package.json"), {
      name,
      version,
      description: `The ruby-templates binary for ${platform}`,
      license: manifest.license,
      repository: manifest.repository,
      os: [os],
      cpu: [cpu],
      files: ["bin"],
    });
    optional[name] = version;
    folders.push(dir);
  }
  if (folders.length === 0)
    throw new Error(`no cli-<platform> binaries in ${binaries}`);
  const dir = path.join(out, "main");
  fs.cpSync(main, dir, { recursive: true });
  writeJson(path.join(dir, "package.json"), {
    ...manifest,
    version,
    optionalDependencies: optional,
  });
  fs.copyFileSync(
    path.join(import.meta.dirname, "..", "LICENSE"),
    path.join(dir, "LICENSE"),
  );
  folders.push(dir);
  return folders;
}

if (process.argv[1] === import.meta.filename) {
  const [binaries, out, version] = process.argv.slice(2);
  if (!binaries || !out) {
    console.error("usage: bun npm/assemble.js <binaries> <out> [version]");
    process.exit(2);
  }
  for (const dir of assemble(binaries, out, version)) console.log(dir);
}
