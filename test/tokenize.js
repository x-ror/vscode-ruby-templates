// Usage: node test/tokenize.js                  check every sample against test/__snapshots__ (UPDATE=1 rewrites them)
//        node test/tokenize.js <scope> <file>   print tokens for one file
import fs from "fs";
import path from "path";
import vsctm from "vscode-textmate";
import onig from "vscode-oniguruma";
import { grammars as vendored } from "tm-grammars";
const root = path.join(import.meta.dirname, "..");
const wasm = fs.readFileSync(
  path.join(root, "node_modules/vscode-oniguruma/release/onig.wasm"),
).buffer;
const grammars = Object.fromEntries(
  JSON.parse(
    fs.readFileSync(path.join(root, "package.json"), "utf8"),
  ).contributes.grammars.map((g) => [g.scopeName, g.path]),
);
const registry = new vsctm.Registry({
  onigLib: onig.loadWASM(wasm).then(() => ({
    createOnigScanner: (s) => new onig.OnigScanner(s),
    createOnigString: (s) => new onig.OnigString(s),
  })),
  loadGrammar: async (scope) => {
    if (grammars[scope])
      return vsctm.parseRawGrammar(
        fs.readFileSync(path.join(root, grammars[scope]), "utf8"),
        grammars[scope],
      );
    // VS Code's own grammars (html, ruby, js, css…), pinned through the tm-grammars dev dependency.
    const builtin = vendored.find((g) => g.scopeName === scope);
    if (builtin) {
      const file = path.join(
        root,
        "node_modules/tm-grammars/grammars",
        `${builtin.name}.json`,
      );
      return vsctm.parseRawGrammar(fs.readFileSync(file, "utf8"), file);
    }
    // Stub for anything else: mark words so we can see the embedding worked.
    return {
      scopeName: scope,
      patterns: [
        {
          name: "stub." + scope,
          match: "[^\\s{}()\\[\\]\"']+|\"[^\"]*\"|'[^']*'",
        },
      ],
    };
  },
});
const samples = {
  "sample.html.erb": "text.html.erb",
  "sample.html.haml": "text.haml",
  "sample.html.slim": "text.slim",
  "sample.skim": "text.skim",
  "sample.js.erb": "source.js.erb",
  "sample.json.erb": "source.json.erb",
  "sample.text.erb": "text.plain.erb",
};
const snapshots = path.join(import.meta.dirname, "__snapshots__");
const CONTEXT_LINES = 3;

async function dump(scope, file) {
  const g = await registry.loadGrammar(scope);
  const out = [];
  let state = vsctm.INITIAL;
  for (const line of fs.readFileSync(file, "utf8").split(/\r?\n/)) {
    const r = g.tokenizeLine(line, state);
    out.push(JSON.stringify(line));
    for (const t of r.tokens) {
      const txt = line.slice(t.startIndex, t.endIndex);
      if (!txt.trim()) continue;
      out.push(
        "   " +
          JSON.stringify(txt).padEnd(28) +
          t.scopes
            .slice(1)
            .map((s) => s.replace(/\.(erb|haml|slim)$/, ""))
            .join(" > "),
      );
    }
    state = r.ruleStack;
  }
  return out.join("\n") + "\n";
}

// Shows the first divergence with a little context; enough to spot which token's scope changed.
function firstDiff(expected, actual) {
  const [e, a] = [expected.split("\n"), actual.split("\n")];
  const i = e.findIndex((line, n) => line !== a[n]);
  const at = i === -1 ? e.length : i;
  const from = Math.max(0, at - CONTEXT_LINES);
  return (
    [
      ...e.slice(from, at).map((l) => "  " + l),
      ...e.slice(at, at + CONTEXT_LINES).map((l) => "- " + l),
      ...a.slice(at, at + CONTEXT_LINES).map((l) => "+ " + l),
    ].join("\n") + `\n  (first difference at line ${at + 1})`
  );
}

// Skim is Slim syntax with CoffeeScript instead of Ruby; engine blocks keep their own languages.
function skimGrammar() {
  const toCoffee = (value) =>
    typeof value === "string"
      ? value.replace(/^source\.ruby$/, "source.coffee")
      : Array.isArray(value)
        ? value.map(toCoffee)
        : value && typeof value === "object"
          ? Object.fromEntries(
              Object.entries(value).map(([k, v]) => [k, toCoffee(v)]),
            )
          : value;
  const {
    repository: { embedded, ...rules },
    ...slim
  } = JSON.parse(
    fs.readFileSync(path.join(root, grammars["text.slim"]), "utf8"),
  );
  return JSON.stringify(
    {
      ...toCoffee(slim),
      name: "Skim",
      scopeName: "text.skim",
      fileTypes: ["skim"],
      repository: { embedded, ...toCoffee(rules) },
    },
    null,
    2,
  );
}

function checkSkimGrammar() {
  const file = path.join(root, grammars["text.skim"]);
  const expected = skimGrammar();
  if (process.env.UPDATE) {
    fs.writeFileSync(file, expected);
    console.log(`wrote ${grammars["text.skim"]}`);
    return 0;
  }
  // Compared as data: `bun fmt` reflows the file, so its text never matches the generator's.
  if (
    fs.existsSync(file) &&
    JSON.stringify(JSON.parse(fs.readFileSync(file, "utf8"))) ===
      JSON.stringify(JSON.parse(expected))
  )
    return 0;
  console.error(
    `FAIL ${grammars["text.skim"]} is out of sync with the Slim grammar`,
  );
  return 1;
}

async function check() {
  if (process.env.UPDATE) fs.mkdirSync(snapshots, { recursive: true });
  let failed = checkSkimGrammar();
  for (const f of fs
    .readdirSync(import.meta.dirname)
    .filter((f) => f.startsWith("sample.") && !samples[f])) {
    failed++;
    console.error(`FAIL ${f}: add it to the samples map in test/tokenize.js`);
  }
  for (const [name, scope] of Object.entries(samples)) {
    const actual = await dump(scope, path.join(import.meta.dirname, name));
    const snap = path.join(snapshots, name + ".tokens.txt");
    const expected = fs.existsSync(snap) ? fs.readFileSync(snap, "utf8") : null;
    if (process.env.UPDATE) {
      fs.writeFileSync(snap, actual);
      console.log(`wrote ${path.relative(root, snap)}`);
    } else if (expected === null) {
      failed++;
      console.error(
        `FAIL ${name}: no baseline at ${path.relative(root, snap)}`,
      );
    } else if (expected !== actual) {
      failed++;
      console.error(
        `FAIL ${name} differs from ${path.relative(root, snap)}:\n${firstDiff(expected, actual)}\n`,
      );
    } else {
      console.log(`ok   ${name}`);
    }
  }
  if (failed) {
    console.error(
      `${failed} snapshot(s) failed; run \`UPDATE=1 npm run test:tokens\` if intended.`,
    );
    process.exit(1);
  }
}

const [scope, file] = process.argv.slice(2);
(scope
  ? dump(scope, file).then((s) => process.stdout.write(s))
  : check()
).catch((e) => {
  console.error(e);
  process.exit(1);
});
