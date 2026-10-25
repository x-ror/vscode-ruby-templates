// Oracle harness: compares the official template engine with ruby-templates. `bun test/oracle.js slim|haml`
// Stage 0 measures today's code, so mismatches don't fail the run; only a broken harness does.
import { spawn, spawnSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";

// The oracle scripts' `column` (Haml reports none) is a 0-based UTF-16 offset in the line; `ruby-templates check` prints 1-based columns.
const COLUMN_TO_CLI = 1;
const PARALLEL_RUBY_PROCESSES = 8;
const exe = process.platform === "win32" ? ".exe" : "";
const bin =
  process.env.RUBY_TEMPLATES ??
  path.join(
    import.meta.dirname,
    "..",
    "server",
    "target",
    "release",
    `ruby-templates${exe}`,
  );
const oracleDir = path.join(import.meta.dirname, "oracle");
// Absolute, so item paths match the paths `check` prints.
const cacheDir = path.resolve(
  process.env.ORACLE_CACHE ?? path.join(os.tmpdir(), "ruby-templates-oracle"),
);

const run = (command, args, options = {}) => {
  const result = spawnSync(command, args, { encoding: "utf8", ...options });
  if (result.error) throw result.error;
  return result;
};

const runWithInput = (command, args, input) =>
  new Promise((resolve, reject) => {
    const child = spawn(command, args);
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => (stdout += chunk));
    child.stderr.on("data", (chunk) => (stderr += chunk));
    child.on("error", reject);
    child.on("close", (status) => resolve({ status, stdout, stderr }));
    child.stdin.end(input);
  });

const mapParallel = async (items, limit, fn) => {
  const results = new Array(items.length);
  let next = 0;
  const worker = async () => {
    while (next < items.length) {
      const index = next++;
      results[index] = await fn(items[index]);
    }
  };
  await Promise.all(Array.from({ length: limit }, worker));
  return results;
};

// The test's own expectation (Slim: the message with "Line N, Column M"; Haml: {message, line}) against the oracle script's
// answer for the same template.
const slimExpectation = (expected, { error }) => {
  const [, line, column] = expected.match(/Line (\d+), Column (\d+)/) ?? [];
  return (
    error &&
    expected.split("\n")[0] === error.message &&
    Number(line) === error.line &&
    Number(column) === error.slim_column
  );
};
const hamlExpectation = (expected, { error }) =>
  error && expected.message === error.message && expected.line === error.line;

const languages = {
  slim: {
    gem: "slim",
    version: "5.2.2",
    versionExpression: "Slim::VERSION",
    repository: "https://github.com/slim-template/slim.git",
    extension: ".slim",
    casesScript: "slim_cases.rb",
    officialScript: "slim.rb",
    expectationAgrees: slimExpectation,
  },
  haml: {
    gem: "haml",
    version: "7.5.1",
    versionExpression: "Haml::VERSION",
    // Haml 7.5.1 mis-parses nested hashes with prism 2.x.
    pin: { gem: "prism", version: "1.9.0", expression: "Prism::VERSION" },
    repository: "https://github.com/haml/haml.git",
    extension: ".haml",
    casesScript: "haml_cases.rb",
    officialScript: "haml.rb",
    expectationAgrees: hamlExpectation,
  },
};

// Whether the gem, and the gem it is pinned to, load at the expected versions.
const installed = ({ gem, version, versionExpression, pin }) => {
  const activate = pin ? `gem "${pin.gem}", "${pin.version}"; ` : "";
  const pinned = pin ? `, " ", ${pin.expression}` : "";
  const r = spawnSync(
    "ruby",
    ["-e", `${activate}require "${gem}"; print ${versionExpression}${pinned}`],
    { encoding: "utf8" },
  );
  return (
    r.status === 0 && r.stdout === (pin ? `${version} ${pin.version}` : version)
  );
};

// The checkout of the official repository at the tag of the pinned version, shallow, cached.
const officialCheckout = ({ gem, version, repository }) => {
  const dir = path.join(cacheDir, `${gem}-${version}`);
  if (fs.existsSync(dir)) return dir;
  fs.mkdirSync(cacheDir, { recursive: true });
  const partial = `${dir}.partial`;
  fs.rmSync(partial, { recursive: true, force: true });
  const clone = run("git", [
    "clone",
    "--depth",
    "1",
    "--branch",
    `v${version}`,
    repository,
    partial,
  ]);
  if (clone.status !== 0) throw new Error(`git clone failed: ${clone.stderr}`);
  fs.renameSync(partial, dir);
  return dir;
};

const collectCases = ({ casesScript }, checkout) => {
  const r = run("ruby", [path.join(oracleDir, casesScript), checkout], {
    maxBuffer: 1 << 28,
  });
  if (r.status !== 0) throw new Error(`${casesScript} failed: ${r.stderr}`);
  return JSON.parse(r.stdout);
};

const officialResult = async ({ officialScript }, source, ...flags) => {
  const r = await runWithInput(
    "ruby",
    [path.join(oracleDir, officialScript), ...flags],
    source,
  );
  if (r.status !== 0) throw new Error(`${officialScript} failed: ${r.stderr}`);
  return JSON.parse(r.stdout);
};

// A recorded expectation from the official test must agree with what the oracle script sees, or the case was mis-recorded.
const checkExpectation = (
  lang,
  { expected_error: expected, label },
  result,
) => {
  if (!expected || lang.expectationAgrees(expected, result)) return;
  throw new Error(
    `${label}: ${lang.officialScript} gave ${JSON.stringify(result)}, the test expects ${JSON.stringify(expected)}`,
  );
};

// Diagnostics by absolute file path: [{line, column, severity, message}].
const oursByPath = ({ extension }, files) => {
  const known = new Set(files.map((file) => path.resolve(file)));
  const r = run(bin, ["check", ...files]);
  if (r.status !== 0 && r.status !== 1)
    throw new Error(`check failed: ${r.stderr}`);
  const diagnostic = new RegExp(
    `^(.*${extension.replace(".", "\\.")}):(\\d+):(\\d+): (\\w+): (.*)$`,
  );
  const byPath = new Map();
  let last;
  for (const text of r.stdout.split("\n")) {
    const m = text.match(diagnostic);
    if (m) {
      last = {
        line: Number(m[2]),
        column: Number(m[3]),
        severity: m[4],
        message: m[5],
      };
      const file = path.resolve(m[1]);
      if (!known.has(file))
        throw new Error(`check reported an unknown file: ${m[1]}`);
      byPath.set(file, [...(byPath.get(file) ?? []), last]);
    } else if (last && text) {
      last.message += `\n${text}`;
    }
  }
  return byPath;
};

const sameError = (official, ours) =>
  ours.severity === "error" &&
  ours.message === official.message &&
  (official.line === null || ours.line === official.line) &&
  (official.column === null || ours.column === official.column + COLUMN_TO_CLI);

// Haml can accept a template and generate Ruby that doesn't parse; that is the template's error, not Haml's.
const rubySyntaxError = ({ invalid_ruby: message, line }) =>
  message && {
    message: `Ruby syntax error: ${message}`,
    line: line ?? null,
    column: null,
  };

const describe = ({ message, line, column }) =>
  `${JSON.stringify(message)}${line === null ? "" : ` at ${line}:${column ?? "?"}`}`;

// items: [{label, source, path, expected_error?}]; official and ours are parallel to items / keyed by path.
const compare = (lang, items, official, ours, skip) => {
  const counts = { compared: 0, officialErrors: 0, oursErrors: 0, same: 0 };
  const mismatches = [];
  const formattable = [];
  const notFormatted = [];
  items.forEach((item, i) => {
    const result = official[i];
    if (result.dependency_missing) {
      skip(`dependency missing (${result.dependency_missing})`);
      return;
    }
    counts.compared++;
    checkExpectation(lang, item, result);
    const errors = (ours.get(path.resolve(item.path)) ?? []).filter(
      (d) => d.severity === "error",
    );
    const officialError = result.error ?? rubySyntaxError(result);
    if (officialError) counts.officialErrors++;
    if (errors.length) counts.oursErrors++;
    const agree = officialError
      ? errors.some((e) => sameError(officialError, e))
      : !errors.length;
    if (!officialError) (errors.length ? notFormatted : formattable).push(i);
    if (agree) {
      if (officialError) counts.same++;
      return;
    }
    mismatches.push(
      `${item.label} ${JSON.stringify(item.source)}\n` +
        `    official: ${officialError ? describe(officialError) : "compiles"}\n` +
        `    ours:     ${errors.length ? errors.map(describe).join("; ") : "no error"}`,
    );
  });
  return { ...counts, mismatches, formattable, notFormatted };
};

// `ruby-templates format` of `text` as if it were the file `file`, to stdout: the text, and whether the formatter left it alone.
const formatText = (file, text) => {
  const r = run(bin, ["format", "--stdin-filepath", file, "-"], {
    input: text,
  });
  if (r.status !== 0 && r.status !== 1) {
    return { text: r.stdout, failure: `exit ${r.status}: ${r.stderr.trim()}` };
  }
  return { text: r.stdout, declined: r.stderr.includes("left unchanged") };
};

const firstDifference = (before, after) => {
  let at = 0;
  while (at < before.length && before[at] === after[at]) at++;
  const around = (text) =>
    JSON.stringify(text.slice(Math.max(0, at - 30), at + 50));
  return `before ...${around(before)}\n      after  ...${around(after)}`;
};

// Formats each item, then checks the compiled Rails code is unchanged and a second format changes nothing.
const formatReport = (lang, items, indexes) =>
  mapParallel(indexes, PARALLEL_RUBY_PROCESSES, async (i) => {
    const { source, path: file, label } = items[i];
    const once = formatText(file, source);
    const twice = formatText(file, once.text);
    const before = await officialResult(lang, source, "--rails");
    const after = await officialResult(lang, once.text, "--rails");
    if (before.source === undefined) {
      throw new Error(
        `${label} doesn't compile with --rails: ${JSON.stringify(before)}`,
      );
    }
    const identical = !once.failure && before.source === after.source;
    const failures = [];
    if (once.failure) failures.push(`formatter failed, ${once.failure}`);
    else if (!identical) {
      failures.push(
        after.source === undefined
          ? `formatted text no longer compiles: ${JSON.stringify(after)}`
          : `compiled output differs\n      ${firstDifference(before.source, after.source)}`,
      );
    }
    const idempotent = !twice.failure && twice.text === once.text;
    if (!idempotent) {
      failures.push(
        twice.failure
          ? `second format failed, ${twice.failure}`
          : `second format changes it\n      ${firstDifference(once.text, twice.text)}`,
      );
    }
    return { i, identical, idempotent, declined: once.declined, failures };
  });

// Prints the report block for `items` under `title`.
const report = async (lang, title, items, skipped) => {
  const skip = (reason) => skipped.set(reason, (skipped.get(reason) ?? 0) + 1);
  const files = items.map((item) => item.path);
  const official = await mapParallel(
    items,
    PARALLEL_RUBY_PROCESSES,
    ({ source }) => officialResult(lang, source),
  );
  const {
    compared,
    officialErrors,
    oursErrors,
    same,
    mismatches,
    formattable,
    notFormatted,
  } = compare(lang, items, official, oursByPath(lang, files), skip);
  const formatted = await formatReport(lang, items, formattable);
  const count = (key) => formatted.filter((row) => row[key]).length;

  console.log(`${title}  (${compared.toLocaleString("en-US")} files)`);
  console.log(
    `errors:  official ${officialErrors}, ours ${oursErrors}, same ${same}`,
  );
  console.log(
    `format:  ${formatted.length.toLocaleString("en-US")} files, compiled output identical in ${count("identical")}, idempotent in ${count("idempotent")}` +
      `  (formatter declined ${count("declined")}; not formatted, our error: ${notFormatted.length})`,
  );
  const skippedTotal = [...skipped.values()].reduce((a, b) => a + b, 0);
  const reasons = [...skipped].map(([r, n]) => `${r} ${n}`).join(", ");
  console.log(`skipped: ${skippedTotal}${reasons ? ` (${reasons})` : ""}`);
  console.log(`mismatches: ${mismatches.length}`);
  if (process.env.VERBOSE) {
    mismatches.forEach((m) => console.log(m));
    formatted
      .filter((row) => row.failures.length)
      .forEach(({ i, failures }) =>
        console.log(
          `format: ${items[i].label} ${JSON.stringify(items[i].source)}\n` +
            failures.map((f) => `    ${f}`).join("\n"),
        ),
      );
  }
};

// Pinned public apps, fetched at one commit so the numbers are reproducible. `paths` are sparse-checked-out folders.
const corpora = {
  slim: [
    {
      name: "freeats",
      url: "https://github.com/freeats/freeats.git",
      commit: "aee7cf76d5206f4cd24fdaa4d18c024de07bfb3d",
      paths: ["app/views"],
    },
  ],
  haml: [
    {
      name: "mastodon",
      url: "https://github.com/mastodon/mastodon.git",
      commit: "f045f6b43fa813564ddeaeffd8d7736e0ef19835",
      paths: ["app/views"],
    },
  ],
};

const corpusCheckout = ({ name, url, commit, paths }) => {
  const dir = path.join(cacheDir, "corpora", `${name}-${commit.slice(0, 7)}`);
  if (fs.existsSync(dir)) return dir;
  const partial = `${dir}.partial`;
  fs.rmSync(partial, { recursive: true, force: true });
  fs.mkdirSync(partial, { recursive: true });
  const git = (...args) => {
    const r = run("git", args, { cwd: partial });
    if (r.status !== 0) throw new Error(`git ${args[0]} failed: ${r.stderr}`);
  };
  git("init", "--quiet");
  git("remote", "add", "origin", url);
  git("sparse-checkout", "set", ...paths);
  git(
    "fetch",
    "--quiet",
    "--depth",
    "1",
    "--filter=blob:none",
    "origin",
    commit,
  );
  git("checkout", "--quiet", "FETCH_HEAD");
  fs.renameSync(partial, dir);
  return dir;
};

const corpusItems = (corpus, extension) => {
  const root = corpusCheckout(corpus);
  return corpus.paths
    .flatMap((folder) =>
      fs
        .readdirSync(path.join(root, folder), { recursive: true })
        .map((name) => path.join(folder, name)),
    )
    .filter((relative) => relative.endsWith(extension))
    .sort()
    .map((relative) => {
      const file = path.join(root, relative);
      return {
        label: `${corpus.name}:${relative}`,
        source: fs.readFileSync(file, "utf8"),
        path: file,
      };
    });
};

const languageReport = async (lang) => {
  const all = collectCases(lang, officialCheckout(lang));
  const skipped = new Map();
  const skip = (reason) => skipped.set(reason, (skipped.get(reason) ?? 0) + 1);
  all.filter((c) => c.skipped).forEach((c) => skip(c.skipped.split(":")[0]));
  const name = `${lang.gem} ${lang.version} vs ruby-templates`;

  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "rt-oracle-"));
  try {
    const cases = all
      .filter((c) => !c.skipped)
      .map((c, i) => {
        const file = path.join(dir, `case-${i}.html${lang.extension}`);
        fs.writeFileSync(file, c.source);
        return { ...c, label: `${c.file}#${c.test} (case ${i})`, path: file };
      });
    await report(lang, name, cases, skipped);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }

  for (const corpus of corpora[lang.gem]) {
    console.log();
    await report(
      lang,
      `${name}, corpus ${corpus.name}@${corpus.commit.slice(0, 7)}`,
      corpusItems(corpus, lang.extension),
      new Map(),
    );
  }
};

const lang = languages[process.argv[2]];
if (!lang) {
  console.error(
    `usage: bun test/oracle.js <${Object.keys(languages).join("|")}>`,
  );
  process.exit(2);
}
if (!installed(lang)) {
  console.log(
    `skipped: ${lang.gem} ${lang.version}${lang.pin ? ` with ${lang.pin.gem} ${lang.pin.version}` : ""} is not installed (ruby -r${lang.gem}); see test/oracle/README.md`,
  );
  process.exit(0);
}
if (!fs.existsSync(bin)) {
  console.error(
    `missing ${bin}; run cargo build --release --manifest-path server/Cargo.toml`,
  );
  process.exit(2);
}
await languageReport(lang);
