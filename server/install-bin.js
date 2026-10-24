// Copies the release build into bin/<platform>-<arch>/, where the client looks for the bundled server.
import fs from "fs";
import path from "path";

const name =
  process.platform === "win32" ? "ruby-templates-ls.exe" : "ruby-templates-ls";
const built = path.join(import.meta.dirname, "target", "release", name);
const dir = path.join(
  import.meta.dirname,
  "..",
  "bin",
  `${process.platform}-${process.arch}`,
);

fs.mkdirSync(dir, { recursive: true });
fs.copyFileSync(built, path.join(dir, name));
console.log(`copied ${name} to ${path.relative(process.cwd(), dir)}`);
