// Oracle harness: compares the official template engine with ruby-templates. `bun test/oracle.js slim`
// Stage 0 measures today's code, so mismatches don't fail the run; only a broken harness does.
import { spawn, spawnSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";

const SLIM_VERSION = "5.2.2";
// slim.rb's `column` is a 0-based UTF-16 offset in the line; `ruby-templates check` prints 1-based columns.
const SLIM_COLUMN_TO_CLI = 1;
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
const cacheDir =
  process.env.ORACLE_CACHE ?? path.join(os.tmpdir(), "ruby-templates-oracle");

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

const slimInstalled = () => {
  const r = spawnSync("ruby", ["-rslim", "-e", "print Slim::VERSION"], {
    encoding: "utf8",
  });
  return r.status === 0 && r.stdout === SLIM_VERSION;
};

const slimCheckout = () => {
  const dir = path.join(cacheDir, `slim-${SLIM_VERSION}`);
  if (fs.existsSync(dir)) return dir;
  fs.mkdirSync(cacheDir, { recursive: true });
  const partial = `${dir}.partial`;
  fs.rmSync(partial, { recursive: true, force: true });
  const clone = run("git", [
    "clone",
    "--depth",
    "1",
    "--branch",
    `v${SLIM_VERSION}`,
    "https://github.com/slim-template/slim.git",
    partial,
  ]);
  if (clone.status !== 0) throw new Error(`git clone failed: ${clone.stderr}`);
  fs.renameSync(partial, dir);
  return dir;
};

const collectCases = (checkout) => {
  const r = run("ruby", [path.join(oracleDir, "slim_cases.rb"), checkout], {
    maxBuffer: 1 << 28,
  });
  if (r.status !== 0) throw new Error(`slim_cases.rb failed: ${r.stderr}`);
  return JSON.parse(r.stdout);
};

const officialResult = async (source, ...flags) => {
  const r = await runWithInput(
    "ruby",
    [path.join(oracleDir, "slim.rb"), ...flags],
    source,
  );
  if (r.status !== 0) throw new Error(`slim.rb failed: ${r.stderr}`);
  return JSON.parse(r.stdout);
};

// A recorded expectation from the official test must agree with what slim.rb sees, or the case was mis-recorded.
const checkExpectation = ({ expected_error: expected, file, test }, result) => {
  if (!expected) return;
  const { message, line, slim_column: column } = result.error ?? {};
  const [, expectedLine, expectedColumn] =
    expected.match(/Line (\d+), Column (\d+)/) ?? [];
  const agrees =
    expected.split("\n")[0] === message &&
    Number(expectedLine) === line &&
    Number(expectedColumn) === column;
  if (!agrees) {
    throw new Error(
      `${file}#${test}: slim.rb gave ${JSON.stringify(result)}, the test expects ${JSON.stringify(expected)}`,
    );
  }
};

// Diagnostics of every file in `dir`, by case index: [{line, column, severity, message}].
const oursByCase = (dir) => {
  const r = run(bin, ["check", dir]);
  if (r.status !== 0 && r.status !== 1)
    throw new Error(`check failed: ${r.stderr}`);
  const byCase = new Map();
  let last;
  for (const text of r.stdout.split("\n")) {
    const m = text.match(/case-(\d+)\.html\.slim:(\d+):(\d+): (\w+): (.*)$/);
    if (m) {
      last = {
        line: Number(m[2]),
        column: Number(m[3]),
        severity: m[4],
        message: m[5],
      };
      const index = Number(m[1]);
      byCase.set(index, [...(byCase.get(index) ?? []), last]);
    } else if (last && text) {
      last.message += `\n${text}`;
    }
  }
  return byCase;
};

const sameError = (official, ours) =>
  ours.severity === "error" &&
  ours.message === official.message &&
  (official.line === null || ours.line === official.line) &&
  (official.column === null ||
    ours.column === official.column + SLIM_COLUMN_TO_CLI);

const describe = ({ message, line, column }) =>
  `${JSON.stringify(message)}${line === null ? "" : ` at ${line}:${column ?? "?"}`}`;

const compare = (cases, official, ours, skip) => {
  const counts = { compared: 0, officialErrors: 0, oursErrors: 0, same: 0 };
  const mismatches = [];
  const formattable = [];
  const notFormatted = [];
  cases.forEach((c, i) => {
    const result = official[i];
    if (result.dependency_missing) {
      skip(`dependency missing (${result.dependency_missing})`);
      return;
    }
    counts.compared++;
    checkExpectation(c, result);
    const errors = (ours.get(i) ?? []).filter((d) => d.severity === "error");
    const officialError = result.error;
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
      `${c.file}#${c.test} (case ${i}) ${JSON.stringify(c.source)}\n` +
        `    official: ${officialError ? describe(officialError) : "compiles"}\n` +
        `    ours:     ${errors.length ? errors.map(describe).join("; ") : "no error"}`,
    );
  });
  return { ...counts, mismatches, formattable, notFormatted };
};

// `ruby-templates format <file>` to stdout: the text, and whether the formatter left the file alone.
const formatFile = (file) => {
  const r = run(bin, ["format", file]);
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

// Formats each case, then checks the compiled Rails code is unchanged and a second format changes nothing.
const formatReport = async (cases, dir, indexes) => {
  const formatDir = fs.mkdtempSync(path.join(os.tmpdir(), "rt-oracle-fmt-"));
  try {
    const rows = await mapParallel(
      indexes,
      PARALLEL_RUBY_PROCESSES,
      async (i) => {
        const once = formatFile(path.join(dir, `case-${i}.html.slim`));
        const again = path.join(formatDir, `case-${i}.html.slim`);
        fs.writeFileSync(again, once.text);
        const twice = formatFile(again);
        const before = await officialResult(cases[i].source, "--rails");
        const after = await officialResult(once.text, "--rails");
        if (before.source === undefined) {
          throw new Error(
            `case ${i} (${cases[i].file}#${cases[i].test}) doesn't compile with --rails: ${JSON.stringify(before)}`,
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
      },
    );
    return rows;
  } finally {
    fs.rmSync(formatDir, { recursive: true, force: true });
  }
};

const slimReport = async () => {
  const all = collectCases(slimCheckout());
  const skipped = new Map();
  const skip = (reason) => skipped.set(reason, (skipped.get(reason) ?? 0) + 1);
  all.filter((c) => c.skipped).forEach((c) => skip(c.skipped.split(":")[0]));
  const cases = all.filter((c) => !c.skipped);

  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "rt-oracle-"));
  try {
    cases.forEach((c, i) =>
      fs.writeFileSync(path.join(dir, `case-${i}.html.slim`), c.source),
    );
    const official = await mapParallel(
      cases,
      PARALLEL_RUBY_PROCESSES,
      ({ source }) => officialResult(source),
    );
    const {
      compared,
      officialErrors,
      oursErrors,
      same,
      mismatches,
      formattable,
      notFormatted,
    } = compare(cases, official, oursByCase(dir), skip);
    const formatted = await formatReport(cases, dir, formattable);
    const count = (key) => formatted.filter((row) => row[key]).length;

    console.log(
      `slim ${SLIM_VERSION} vs ruby-templates  (${compared.toLocaleString("en-US")} files)`,
    );
    console.log(
      `errors:  official ${officialErrors}, ours ${oursErrors}, same ${same}`,
    );
    const skippedTotal = [...skipped.values()].reduce((a, b) => a + b, 0);
    const reasons = [...skipped].map(([r, n]) => `${r} ${n}`).join(", ");
    console.log(
      `format:  ${formatted.length.toLocaleString("en-US")} files, compiled output identical in ${count("identical")}, idempotent in ${count("idempotent")}` +
        `  (formatter declined ${count("declined")}; not formatted, our error: ${notFormatted.length})`,
    );
    console.log(`skipped: ${skippedTotal}${reasons ? ` (${reasons})` : ""}`);
    console.log(`mismatches: ${mismatches.length}`);
    if (process.env.VERBOSE) {
      mismatches.forEach((m) => console.log(m));
      formatted
        .filter((row) => row.failures.length)
        .forEach(({ i, failures }) =>
          console.log(
            `format: ${cases[i].file}#${cases[i].test} (case ${i}) ${JSON.stringify(cases[i].source)}\n` +
              failures.map((f) => `    ${f}`).join("\n"),
          ),
        );
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
};

const languages = { slim: slimReport };
const language = process.argv[2];
if (!languages[language]) {
  console.error(
    `usage: bun test/oracle.js <${Object.keys(languages).join("|")}>`,
  );
  process.exit(2);
}
if (!slimInstalled()) {
  console.log(
    `skipped: slim ${SLIM_VERSION} is not installed (ruby -rslim); see test/oracle/README.md`,
  );
  process.exit(0);
}
if (!fs.existsSync(bin)) {
  console.error(
    `missing ${bin}; run cargo build --release --manifest-path server/Cargo.toml`,
  );
  process.exit(2);
}
await languages[language]();
