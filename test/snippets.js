import assert from "assert";
import fs from "fs";
import path from "path";

const dir = path.join(import.meta.dirname, "../snippets");
// An unescaped `$` that isn't a tabstop/variable silently breaks a snippet (e.g. Ruby's `$stdout`).
const strayDollar =
  /(?<!\\)\$(?![0-9{]|TM_|CLIPBOARD|CURRENT_|RANDOM|LINE_COMMENT|BLOCK_COMMENT|WORKSPACE_)/;
for (const file of fs.readdirSync(dir).filter((f) => f.endsWith(".json"))) {
  const seen = new Set();
  for (const [name, { prefix, body }] of Object.entries(
    JSON.parse(fs.readFileSync(path.join(dir, file), "utf8")),
  )) {
    assert(
      !seen.has(prefix),
      `${file}: duplicate prefix "${prefix}" (${name})`,
    );
    seen.add(prefix);
    const lines = typeof body === "string" ? [body] : body;
    assert(
      Array.isArray(lines) && lines.every((l) => typeof l === "string"),
      `${file}: bad body in ${name}`,
    );
    assert(
      !lines.some((l) => strayDollar.test(l)),
      `${file}: unescaped $ in ${name}`,
    );
  }
  console.log(`${file}: ${seen.size} snippets ok`);
}
