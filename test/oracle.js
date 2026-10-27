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
const slimExpectation = (expected, { error, errors }) =>
  typeof expected === "object"
    ? // Ruby's syntax error at a template line: Prism's errors must include that line.
      errors?.some((e) => e.line === expected.ruby_line)
    : error &&
      expected.split("\n")[0] === error.message &&
      Number(expected.match(/Line (\d+), Column (\d+)/)?.[1]) === error.line &&
      Number(expected.match(/Line (\d+), Column (\d+)/)?.[2]) ===
        error.slim_column;
const hamlExpectation = (expected, { error }) =>
  error && expected.message === error.message && expected.line === error.line;

const nearestFile = (file, name) => {
  for (let dir = path.dirname(file); ; dir = path.dirname(dir)) {
    const candidate = path.join(dir, name);
    if (fs.statSync(candidate, { throwIfNoEntry: false })?.isFile())
      return candidate;
    if (path.dirname(dir) === dir) return undefined;
  }
};

// The path Rails names a template by (`short_identifier`): from the nearest Gemfile.lock's folder, else the file name.
const projectRelativePath = (file) => {
  const lockfile = nearestFile(file, "Gemfile.lock");
  return lockfile
    ? path.relative(path.dirname(lockfile), file)
    : path.basename(file);
};

const RELEASE = /(\d+\.\d+)(?:\.\d+)*/;

// The Ruby our server parses `file` with: the nearest .ruby-version, else the nearest Gemfile.lock's RUBY VERSION.
const projectRubyRelease = (file) => {
  const versionFile = nearestFile(file, ".ruby-version");
  const fromFile = versionFile
    ? fs
        .readFileSync(versionFile, "utf8")
        .split("\n")
        .map((line) => line.trim())
        .find((line) => line && !line.startsWith("#"))
        ?.match(new RegExp(`^(?:ruby-)?${RELEASE.source}`))?.[1]
    : undefined;
  if (fromFile) return fromFile;
  const lockfile = nearestFile(file, "Gemfile.lock");
  return lockfile
    ? fs
        .readFileSync(lockfile, "utf8")
        .match(
          new RegExp(`^RUBY VERSION\\r?\\n\\s+ruby ${RELEASE.source}`, "m"),
        )?.[1]
    : undefined;
};

// Prism 2.0.0's grammars: 3.3 (also for older Rubies), 3.4, 3.5 (also 4.0), 4.1; newer is the latest.
const prismVersion = (release) => {
  if (!release) return "latest";
  const [major, minor] = release.split(".").map(Number);
  const number = major * 100 + minor;
  if (number >= 402) return "latest";
  if (number >= 401) return "4.1";
  if (number >= 305) return "3.5";
  return number >= 304 ? "3.4" : "3.3";
};

// Both sides must parse with one grammar: ours reads the project, so the official script is told what ours finds.
const rubyVersionArguments = (file) => [
  "--ruby-version",
  prismVersion(projectRubyRelease(file)),
];

const languages = {
  slim: {
    name: "slim",
    gem: "slim",
    version: "5.2.2",
    versionExpression: "Slim::VERSION",
    repository: "https://github.com/slim-template/slim.git",
    extension: ".slim",
    casesScript: "slim_cases.rb",
    officialScript: "slim.rb",
    officialArguments: (item) => rubyVersionArguments(item.path),
    expectationAgrees: slimExpectation,
    perturb: async (source) => slimPerturbations(source),
  },
  haml: {
    name: "haml",
    gem: "haml",
    version: "7.5.1",
    versionExpression: "Haml::VERSION",
    // Haml 7.5.1 mis-parses nested hashes with prism 2.x.
    pins: [{ gem: "prism", version: "1.9.0", expression: "Prism::VERSION" }],
    repository: "https://github.com/haml/haml.git",
    extension: ".haml",
    casesScript: "haml_cases.rb",
    officialScript: "haml.rb",
    expectationAgrees: hamlExpectation,
    perturb: (source) => hamlPerturbations(source),
    reportsNormalForm: true,
  },
  // Corpora only: Erubi's own tests check output, not errors.
  erb: {
    name: "erb",
    gem: "erubi",
    version: "1.13.1",
    versionExpression: "Erubi::VERSION",
    pins: [
      {
        gem: "actionview",
        version: "8.1.4",
        expression: "ActionView::VERSION::STRING",
        require: "action_view",
      },
      { gem: "prism", version: "2.0.0", expression: "Prism::VERSION" },
    ],
    title: "erb (Erubi 1.13.1, Rails 8.1.4 handler, Prism 2.0.0)",
    extension: ".erb",
    officialScript: "erb.rb",
    // The name picks the format (.html.erb, .text.erb, ...) and appears in Rails' messages.
    officialArguments: (item) => [
      "--file",
      projectRelativePath(item.path),
      ...rubyVersionArguments(item.path),
    ],
    // 3b: compared with the leading whitespace of text lines removed; the formatter must also change something.
    formatFlags: ["--normalize"],
    reportsChanged: true,
    // 3a "Ruby": variants of compiling files that break the generated Ruby; every 10th file keeps a run short.
    breakVariants: (source) => erbBreakVariants(source),
    breakSample: 10,
  },
};

// Whether the gem and the gems it is pinned to load at the expected versions.
const installed = ({ gem, version, versionExpression, pins = [] }) => {
  const gems = [{ gem, version, expression: versionExpression }, ...pins];
  const script = [
    ...gems.map((g) => `gem "${g.gem}", "${g.version}"`),
    ...gems.map((g) => `require "${g.require ?? g.gem}"`),
    `print [${gems.map((g) => g.expression).join(", ")}].join(" ")`,
  ].join("; ");
  const r = spawnSync("ruby", ["-e", script], { encoding: "utf8" });
  return r.status === 0 && r.stdout === gems.map((g) => g.version).join(" ");
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

// Our errors with no Ruby oracle: HTML structure, which WHATWG defines.
const isHtmlMessage = (message) =>
  /end tag|never closed|<\/?[a-z][\w-]*>/i.test(message);

// items: [{label, source, path, expected_error?}]; official and ours are parallel to items / keyed by path.
// A file's official errors are `errors` (ERB lists every Prism error) or the one `error`.
const compare = (lang, items, official, ours, skip) => {
  const counts = {
    compared: 0,
    officialErrors: 0,
    oursErrors: 0,
    same: 0,
    sameLine: 0,
    oursOnlyHtml: 0,
  };
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
    const officialList =
      result.errors ??
      [result.error ?? rubySyntaxError(result)].filter(Boolean);
    if (officialList.length) counts.officialErrors++;
    if (errors.length) counts.oursErrors++;
    if (officialList.some((o) => errors.some((e) => e.line === o.line))) {
      counts.sameLine++;
    }
    const agree = officialList.length
      ? officialList.every((o) => errors.some((e) => sameError(o, e)))
      : !errors.length;
    if (!officialList.length) {
      (errors.length ? notFormatted : formattable).push(i);
    }
    if (agree) {
      if (officialList.length) counts.same++;
      return;
    }
    if (!officialList.length && errors.every((e) => isHtmlMessage(e.message))) {
      counts.oursOnlyHtml++;
    }
    mismatches.push(
      `${item.label} ${JSON.stringify(item.source)}\n` +
        `    official: ${officialList.length ? officialList.map(describe).join("; ") : "compiles"}\n` +
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
  return {
    text: r.stdout,
    declined: r.stderr.includes("left unchanged"),
    reason: r.stderr.match(/left unchanged: (.*)/)?.[1],
  };
};

// Other spellings of the same Slim template: its indentation is relative, a tab counts as tabsize 4, and a line
// ending or a space at the end of a bare tag line is nothing. Each is checked to compile to the same source before
// it is used, so a variant that changes the template (text lines, embedded code) is dropped, not formatted.
const slimPerturbations = (source) => {
  const lines = source.split("\n");
  const scaled = (factor) =>
    lines
      .map((line) =>
        line.replace(/^ +/, (spaces) => " ".repeat(spaces.length * factor)),
      )
      .join("\n");
  const gcd = (a, b) => (b ? gcd(b, a % b) : a);
  const unit = lines.map((line) => line.match(/^ */)[0].length).reduce(gcd, 0);
  const toTabs = (text) =>
    text.replace(/^( {4})+/gm, (spaces) => "\t".repeat(spaces.length / 4));
  const bareTagLine = /^\s*([a-z][\w-]*|[.#][\w-]+)+$|^$/;
  const variants = [
    { name: "indent x2", text: scaled(2) },
    { name: "indent x3", text: scaled(3) },
    unit && 4 % unit === 0
      ? { name: "tabs", text: toTabs(scaled(4 / unit)) }
      : undefined,
    { name: "CRLF", text: source.replace(/\r?\n/g, "\r\n") },
    {
      name: "trailing spaces",
      text: lines
        // (not the empty piece after the final newline, which is no line)
        .map((line, index) =>
          bareTagLine.test(line) && index < lines.length - 1
            ? `${line}  `
            : line,
        )
        .join("\n"),
    },
  ];
  return variants.filter((v) => v && v.text !== source);
};

// The same for Haml. Indentation must be uniform, so it is rescaled as a whole, except the whitespace past the
// structural prefix of filter and `-#` bodies, which is content (haml_perturb.rb finds those lines). Bare tag lines
// and the bodies are told apart by the same helper, so trailing spaces never land in a body.
const hamlPerturbations = async (source) => {
  const helper = await runWithInput(
    "ruby",
    [path.join(oracleDir, "haml_perturb.rb")],
    source,
  );
  if (helper.status !== 0) {
    throw new Error(`haml_perturb.rb failed: ${helper.stderr}`);
  }
  const { unit, flat } = JSON.parse(helper.stdout);
  const lines = source.split("\n");
  // Rewrites the structural part of a line's leading whitespace; a flat line keeps what follows its prefix.
  const restructured = (rewrite) => {
    const rewritten = lines.map((line, index) => {
      if (line.trim() === "") return line;
      const whitespace = line.match(/^[ \t]*/)[0];
      const prefix = flat[index] ?? whitespace.length;
      const structure = rewrite(whitespace.slice(0, prefix));
      return structure === undefined
        ? undefined
        : structure + line.slice(prefix);
    });
    return rewritten.includes(undefined) ? undefined : rewritten.join("\n");
  };
  const spaces = !unit.includes("\t") && unit.length > 0;
  const levels = (structure) =>
    /^ *$/.test(structure) && structure.length % unit.length === 0
      ? structure.length / unit.length
      : undefined;
  const withUnit = (width, render) =>
    spaces
      ? restructured((structure) => {
          const level = levels(structure);
          return level === undefined ? undefined : render(level * width);
        })
      : undefined;
  const bareTagLine = /^\s*((%[\w:-]+|[.#][\w-]+)([.#][\w-]+)*)?$/;
  const variants = [
    {
      name: "indent x2",
      text: spaces
        ? withUnit(unit.length * 2, (n) => " ".repeat(n))
        : undefined,
    },
    {
      name: "indent x3",
      text: spaces
        ? withUnit(unit.length * 3, (n) => " ".repeat(n))
        : undefined,
    },
    {
      name: "tabs",
      text:
        spaces && 4 % unit.length === 0
          ? withUnit(4, (n) => "\t".repeat(n / 4))
          : undefined,
    },
    {
      name: "unit 3",
      text:
        spaces && unit.length !== 3
          ? withUnit(3, (n) => " ".repeat(n))
          : undefined,
    },
    { name: "CRLF", text: source.replace(/\r?\n/g, "\r\n") },
    { name: "BOM", text: `\uFEFF${source}` },
    {
      name: "trailing spaces",
      text: lines
        .map((line, index) =>
          bareTagLine.test(line) && !(index in flat) && index < lines.length - 1
            ? `${line}  `
            : line,
        )
        .join("\n"),
    },
    { name: "extra blank lines", text: `${source.replace(/\n*$/, "\n")}\n\n` },
    {
      name: "whitespace-only last line",
      text: `${source.replace(/\n*$/, "\n")}  \n`,
    },
    { name: "no final newline", text: source.replace(/\n+$/, "") },
  ];
  return variants.filter((v) => v.text !== undefined && v.text !== source);
};

// Erubi's own regexp, so the tags are the ones Rails sees.
const erbTags = (source) => [
  ...source.matchAll(/<%(={1,2}|-|#|%)?(.*?)([-=])?%>([ \t]*\r?\n)?/gs),
];

const STRICT_LOCALS_HEAD = /#\s+locals:\s+\(/;

// Broken spellings of an ERB template, each a few characters from a compiling one (3a "Ruby"); a name is absent when
// the file has nothing to break that way.
const erbBreakVariants = (source) => {
  const tags = erbTags(source).filter(
    (tag) => tag[1] !== "#" && tag[1] !== "%",
  );
  const bare = (tag) => !tag[1] || tag[1] === "-";
  const codeStart = (tag) => tag.index + 2 + (tag[1] ?? "").length;
  const closeStart = (tag) =>
    tag.index + tag[0].length - (tag[4]?.length ?? 0) - 2;
  const splice = (from, to, replacement = "") =>
    source.slice(0, from) + replacement + source.slice(to);
  const end = tags.find((tag) => bare(tag) && tag[2].trim() === "end");
  const open = tags.find((tag) =>
    source.includes("<%", tag.index + tag[0].length),
  );
  const conditional = tags.find((tag) => bare(tag) && /^\s*if\s/.test(tag[2]));
  const parenthesized = tags.find((tag) => tag[2].includes(")"));
  return [
    {
      name: "delete end",
      text: end && splice(end.index, end.index + end[0].length),
    },
    {
      name: "drop %>",
      text: open && splice(closeStart(open), closeStart(open) + 2),
    },
    {
      name: "if to <%=",
      text:
        conditional &&
        splice(conditional.index + 2, codeStart(conditional), "="),
    },
    { name: "stray end", text: `<% end %>\n${source}` },
    {
      // Rails refuses positional strict locals (7.1+) once the Ruby is valid.
      name: "positional locals",
      text: STRICT_LOCALS_HEAD.test(source)
        ? undefined
        : `<%# locals: (a, b:) -%>\n${source}`,
    },
    {
      name: "delete )",
      text:
        parenthesized &&
        splice(
          codeStart(parenthesized),
          codeStart(parenthesized) + parenthesized[2].length,
          parenthesized[2].replace(/\)([^)]*)$/, "$1"),
        ),
    },
  ].filter((v) => v.text !== undefined && v.text !== source);
};

// Formats the variants of one file: each must compile to the original's source once formatted, and formatting
// the result again must change nothing. A refusal (the formatter leaves the text alone) is counted apart.
const perturbedRows = async (
  lang,
  item,
  flags,
  originalSource,
  originalFormatted,
) => {
  const rows = [];
  for (const variant of await lang.perturb(item.source)) {
    const compiled = await officialResult(lang, variant.text, ...flags);
    if (compiled.source !== originalSource) {
      rows.push({ name: variant.name, outcome: "not equivalent" });
      continue;
    }
    const once = formatText(item.path, variant.text);
    if (once.failure) {
      rows.push({
        name: variant.name,
        outcome: "failed",
        failures: [`formatter failed, ${once.failure}`],
      });
      continue;
    }
    if (once.declined) {
      rows.push({
        name: variant.name,
        outcome: "refused",
        reason: once.reason ?? "no reason given",
      });
      continue;
    }
    const after = await officialResult(lang, once.text, ...flags);
    const twice = formatText(item.path, once.text);
    const failures = [];
    if (after.source !== originalSource) {
      failures.push(
        after.source === undefined
          ? `formatted text no longer compiles: ${JSON.stringify(after)}`
          : `compiled output differs\n      ${firstDifference(originalSource, after.source)}`,
      );
    }
    const idempotent = !twice.failure && twice.text === once.text;
    if (!idempotent) {
      failures.push(
        `second format changes it\n      ${firstDifference(once.text, twice.text)}`,
      );
    }
    rows.push({
      name: variant.name,
      outcome: "formatted",
      identical: after.source === originalSource,
      changed: once.text !== variant.text,
      sameAsFormatted: once.text === originalFormatted,
      idempotent,
      failures,
      text: variant.text,
    });
  }
  return rows;
};

// What kind of change separates two compiled sources, to group the failures.
const differenceKind = (before, after) => {
  const without = (text, pattern) => text.replace(pattern, "");
  if (without(before, /[ \t]+(?=\n)/g) === without(after, /[ \t]+(?=\n)/g)) {
    return " (trailing whitespace only)";
  }
  if (without(before, /\s+/g) === without(after, /\s+/g)) {
    return " (whitespace only)";
  }
  return "";
};

const firstDifference = (before, after) => {
  let at = 0;
  while (at < before.length && before[at] === after[at]) at++;
  const around = (text) =>
    JSON.stringify(text.slice(Math.max(0, at - 30), at + 50));
  return `before ...${around(before)}\n      after  ...${around(after)}`;
};

// Formats each item, then checks the compiled Rails code is unchanged and a second format changes nothing.
// ORACLE_SAMPLE=n perturbs every n-th file only, to keep a run short.
const PERTURB_SAMPLE = Number(process.env.ORACLE_SAMPLE) || 1;

const formatReport = (lang, items, indexes, sample) =>
  mapParallel(indexes, PARALLEL_RUBY_PROCESSES, async (i) => {
    const { source, path: file, label } = items[i];
    const once = formatText(file, source);
    const twice = formatText(file, once.text);
    const flags = [
      ...(lang.formatFlags ?? ["--rails"]),
      ...(lang.officialArguments?.(items[i]) ?? []),
    ];
    const before = await officialResult(lang, source, ...flags);
    const after = await officialResult(lang, once.text, ...flags);
    const perturbed =
      lang.perturb && before.source !== undefined && i % sample === 0
        ? await perturbedRows(lang, items[i], flags, before.source, once.text)
        : [];
    const notCompared = Boolean(before.not_compared);
    if (before.source === undefined && !notCompared) {
      throw new Error(
        `${label} doesn't compile with ${flags.join(" ")}: ${JSON.stringify(before)}`,
      );
    }
    const identical =
      !notCompared && !once.failure && before.source === after.source;
    const failures = [];
    if (once.failure) failures.push(`formatter failed, ${once.failure}`);
    else if (!identical && !notCompared) {
      failures.push(
        after.source === undefined
          ? `formatted text no longer compiles: ${JSON.stringify(after)}`
          : `compiled output differs${differenceKind(before.source, after.source)}\n      ${firstDifference(before.source, after.source)}`,
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
    return {
      i,
      identical,
      notCompared,
      idempotent,
      declined: once.declined,
      changed: !once.failure && once.text !== source,
      perturbed,
      failures,
    };
  });

// 3a "Ruby": broken variants of compiling files; each Prism error of Rails must be in ours at the same line.
// A variant that still compiles, or that fails for a reason other than Ruby's syntax, is counted, not compared.
const breakReport = async (lang, items, indexes, sample) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "rt-oracle-break-"));
  try {
    // A Rails 8.1.4 project, so the variants get the Rails checks and are named by their path from `dir`.
    fs.writeFileSync(
      path.join(dir, "Gemfile.lock"),
      "GEM\n  specs:\n    actionview (8.1.4)\n",
    );
    const variants = indexes
      .filter((_, n) => n % sample === 0)
      .flatMap((i) =>
        lang
          .breakVariants(items[i].source)
          .map((variant) => ({ ...variant, item: items[i] })),
      )
      .map((variant, n) => {
        const file = path.join(
          dir,
          String(n),
          path.basename(variant.item.path),
        );
        fs.mkdirSync(path.dirname(file));
        fs.writeFileSync(file, variant.text);
        return { ...variant, path: file };
      });
    const official = await mapParallel(
      variants,
      PARALLEL_RUBY_PROCESSES,
      (variant) =>
        officialResult(lang, variant.text, ...lang.officialArguments(variant)),
    );
    const ours = oursByPath(
      lang,
      variants.map((variant) => variant.path),
    );
    const counts = { compiles: 0, other: 0, compared: 0, same: 0 };
    const mismatches = [];
    variants.forEach((variant, n) => {
      const list = official[n].errors;
      const errors = (ours.get(path.resolve(variant.path)) ?? []).filter(
        (d) => d.severity === "error",
      );
      let agree;
      if (!list) {
        counts.compiles++;
        agree = !errors.length;
      } else if (list.some((e) => e.line === null)) {
        counts.other++;
        return;
      } else {
        counts.compared++;
        agree = list.every((o) => errors.some((e) => sameError(o, e)));
        if (agree) counts.same++;
      }
      if (agree) return;
      mismatches.push(
        `break (${variant.name}): ${variant.item.label} ${JSON.stringify(variant.text)}\n` +
          `    official: ${list ? list.map(describe).join("; ") : "compiles"}\n` +
          `    ours:     ${errors.length ? errors.map(describe).join("; ") : "no error"}`,
      );
    });
    return {
      mismatches,
      summary:
        `broken:  ${variants.length} variants (every ${sample}th file), still compiles ${counts.compiles}, other error ${counts.other}, ` +
        `compared ${counts.compared}, same ${counts.same}, mismatches ${mismatches.length}`,
    };
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
};

// Prints the report block for `items` under `title`.
// `perturbSample` n: only every n-th file of a big corpus is perturbed.
const report = async (lang, title, items, skipped, perturbSample = 1) => {
  const skip = (reason) => skipped.set(reason, (skipped.get(reason) ?? 0) + 1);
  const files = items.map((item) => item.path);
  const official = await mapParallel(items, PARALLEL_RUBY_PROCESSES, (item) =>
    officialResult(
      lang,
      item.source,
      ...(lang.officialArguments?.(item) ?? []),
    ),
  );
  const {
    compared,
    officialErrors,
    oursErrors,
    same,
    sameLine,
    oursOnlyHtml,
    mismatches,
    formattable,
    notFormatted,
  } = compare(lang, items, official, oursByPath(lang, files), skip);
  const formatted = await formatReport(
    lang,
    items,
    formattable,
    perturbSample * PERTURB_SAMPLE,
  );
  const count = (key) => formatted.filter((row) => row[key]).length;
  const broken = lang.breakVariants
    ? await breakReport(
        lang,
        items,
        formattable,
        lang.breakSample * PERTURB_SAMPLE,
      )
    : undefined;

  console.log(`${title}  (${compared.toLocaleString("en-US")} files)`);
  console.log(
    `errors:  official ${officialErrors}, ours ${oursErrors}, same ${same}`,
  );
  if (lang.reportsChanged) {
    console.log(
      `lines:   official error line also in ours, in ${sameLine} files; ours only (HTML), in ${oursOnlyHtml} files`,
    );
  }
  console.log(
    `format:  ${formatted.length.toLocaleString("en-US")} files, compiled output identical in ${count("identical")}, idempotent in ${count("idempotent")}` +
      `  (formatter declined ${count("declined")}; ${lang.reportsChanged ? `changed ${count("changed")}; not compared, whitespace-sensitive element built by Ruby ${count("notCompared")}; ` : ""}not formatted, our error: ${notFormatted.length})`,
  );
  if (lang.perturb) {
    const variants = formatted.flatMap((row) => row.perturbed);
    const outcome = (name) => variants.filter((v) => v.outcome === name);
    const run = variants.filter((v) => v.outcome !== "not equivalent");
    const formattedVariants = outcome("formatted");
    const refusals = new Map();
    outcome("refused").forEach((v) =>
      refusals.set(v.reason, (refusals.get(v.reason) ?? 0) + 1),
    );
    const reasons = [...refusals].map(([r, n]) => `${r} ${n}`).join("; ");
    console.log(
      `format perturbed: ${run.length} variants, formatter changed ${formattedVariants.filter((v) => v.changed).length}, compiled identical in ${formattedVariants.filter((v) => v.identical).length}, idempotent in ${formattedVariants.filter((v) => v.idempotent).length}, refused ${outcome("refused").length}${reasons ? ` (${reasons})` : ""}, failed ${outcome("failed").length}; not equivalent to the original ${outcome("not equivalent").length}${lang.reportsNormalForm ? `; same text as the formatted original in ${formattedVariants.filter((v) => v.sameAsFormatted).length}` : ""}${perturbSample * PERTURB_SAMPLE > 1 ? ` (every ${perturbSample * PERTURB_SAMPLE}th file)` : ""}`,
    );
  }
  if (lang.perturb) {
    const names = [
      ...new Set(formatted.flatMap((row) => row.perturbed.map((v) => v.name))),
    ];
    const byName = names.map((name) => {
      const all = formatted
        .flatMap((row) => row.perturbed)
        .filter((v) => v.name === name);
      return `${name} ${all.filter((v) => v.outcome !== "not equivalent").length}/${all.length}`;
    });
    console.log(`  variants run/made: ${byName.join(", ")}`);
  }
  if (broken) {
    console.log(broken.summary);
    if (process.env.VERBOSE) broken.mismatches.forEach((m) => console.log(m));
  }
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
    formatted.forEach(({ i, perturbed }) =>
      perturbed
        .filter((v) => v.failures?.length)
        .forEach((v) =>
          console.log(
            `format perturbed (${v.name}): ${items[i].label} ${JSON.stringify(v.text ?? "")}\n` +
              v.failures.map((f) => `    ${f}`).join("\n"),
          ),
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
  erb: [
    {
      name: "forem",
      url: "https://github.com/forem/forem.git",
      commit: "64ee7392247587e6bf390ddebe31a3f884be931b",
      paths: ["app/views", "app/components"],
    },
    {
      name: "rubygems.org",
      url: "https://github.com/rubygems/rubygems.org.git",
      commit: "395a0e253a6a947f40e74c4ac254819b2ed9c580",
      paths: ["app/views", "app/components"],
    },
    {
      name: "whitehall",
      url: "https://github.com/alphagov/whitehall.git",
      commit: "133b88a7c77460d40663d03e1c40b90dfe531191",
      paths: ["app/views", "app/components"],
    },
  ],
  haml: [
    {
      name: "mastodon",
      url: "https://github.com/mastodon/mastodon.git",
      commit: "f045f6b43fa813564ddeaeffd8d7736e0ef19835",
      paths: ["app/views"],
    },
    {
      // gitlab.com answers `git ls-remote`; the GitHub mirror needs credentials here. 2,314 .haml files, so only
      // every 10th is perturbed.
      name: "gitlab",
      url: "https://gitlab.com/gitlab-org/gitlab.git",
      commit: "8cc12b9bfed2884db0f85805d6abe3659bb9ad59",
      paths: ["app/views", "ee/app/views"],
      perturbSample: 10,
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
    .filter((folder) => fs.existsSync(path.join(root, folder)))
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

// The official test suite's templates, written to a temp dir that is not a Rails project.
const suiteReport = async (lang, name) => {
  const all = collectCases(lang, officialCheckout(lang));
  const skipped = new Map();
  const skip = (reason) => skipped.set(reason, (skipped.get(reason) ?? 0) + 1);
  all.filter((c) => c.skipped).forEach((c) => skip(c.skipped.split(":")[0]));

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
};

const languageReport = async (lang) => {
  const name = `${lang.title ?? `${lang.gem} ${lang.version}`} vs ruby-templates`;
  if (lang.casesScript) await suiteReport(lang, name);
  for (const [index, corpus] of corpora[lang.name].entries()) {
    if (lang.casesScript || index > 0) console.log();
    await report(
      lang,
      `${name}, corpus ${corpus.name}@${corpus.commit.slice(0, 7)}`,
      corpusItems(corpus, lang.extension),
      new Map(),
      corpus.perturbSample,
    );
  }
};

// The rendering context: what Rails raises for `render` calls, per version of actionview, against ours per project.
const CONTEXT_ACTIONVIEW_VERSIONS = ["7.0.8.7", "7.1.5.1", "8.0.1", "8.0.2"];

const lockfile = (gem, version) =>
  `GEM\n  remote: https://rubygems.org/\n  specs:\n    ${gem} (${version})\n\nPLATFORMS\n  ruby\n\nDEPENDENCIES\n  ${gem}\n`;

// `lock` is null for the app without a Gemfile.lock; `rails` is the actionview version Rails renders with.
const contextFixtures = [
  ...CONTEXT_ACTIONVIEW_VERSIONS.map((version) => ({
    title: `actionview ${version}`,
    dir: `actionview-${version}`,
    lock: lockfile("actionview", version),
    rails: version,
  })),
  {
    title: "sinatra (no actionview)",
    dir: "sinatra",
    lock: lockfile("sinatra", "4.1.1"),
  },
  { title: "no Gemfile.lock", dir: "no-lockfile", lock: null },
];

const contextPartials = {
  "_card.html.erb": "<%# locals: (a:, b:) -%>\n<p><%= a %><%= b %></p>\n",
  "_none.html.erb": "<%# locals: () -%>\n<p>none</p>\n",
  "t.html.erb": "T",
  "../layouts/lt.html.erb": "LT[<%= yield %>]",
};

// Each render is its own template, so every error is attributable (Rails stops at the first per render).
const contextRenders = [
  'render "users/missing"',
  'render partial: "users/missing"',
  'render partial: "users/card", locals: {}',
  'render partial: "users/card", locals: { a: 1 }',
  'render partial: "users/card", locals: { a: 1, b: 2, c: 3 }',
  'render partial: "users/card", locals: { a: 1, b: 2, c: 3, d: 4 }',
  'render "users/card", a: 1',
  'render "users/card", a: 1, b: 2, c: 3',
  'render "users/card", a: 1, b: 2',
  'render partial: "users/none", locals: { x: 1 }',
  'render "users/none", x: 1',
  'render "users/none"',
  'render template: "users/t", layout: "layouts/lt"',
  'render template: "users/t", layout: "layouts/gone"',
  'render partial: "users/card", layout: "users/missing_layout", locals: { a: 1, b: 2 }',
  'render template: "users/gone"',
];

// The diagnostics of ours that answer a render: the unresolved partial and the strict locals ones.
const isRenderDiagnostic = (message) =>
  /^Missing (partial|template) |^(missing|unknown) (local|keywords?)\b|^no locals accepted/.test(
    message,
  );

// On 7.0 the `locals:` line is a comment, so a partial that uses its locals raises NameError from its own body; no
// render check covers that. Any other error is a Rails error our render checks are measured against.
const isBodyError = (fixture, error) =>
  fixture.rails.startsWith("7.0.") &&
  error.class === "NameError" &&
  error.message.startsWith("undefined local variable or method");

// MissingTemplate's message goes on with the formats and search paths, which depend on the app; only its start is compared.
const MISSING_TEMPLATE_DETAILS = " with ";

const contextOfficial = async (fixture, root, template) => {
  const r = await runWithInput("ruby", [
    path.join(oracleDir, "context.rb"),
    fixture.rails,
    root,
    `users/${template}`,
  ]);
  if (r.status !== 0) throw new Error(`context.rb failed: ${r.stderr}`);
  const { error, rendered } = JSON.parse(r.stdout);
  if (!error) return null;
  if (isBodyError(fixture, error)) return { bodyError: true };
  const cut = error.message.indexOf(MISSING_TEMPLATE_DETAILS);
  const missing = error.class === "ActionView::MissingTemplate" && cut >= 0;
  return {
    message: missing ? error.message.slice(0, cut) : error.message,
    line: error.line,
    partlyCompared: missing,
  };
};

const contextReport = async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "rt-oracle-context-"));
  try {
    for (const fixture of contextFixtures) {
      const root = path.join(dir, fixture.dir);
      const views = path.join(root, "app", "views", "users");
      fs.mkdirSync(views, { recursive: true });
      if (fixture.lock !== null) {
        fs.writeFileSync(path.join(root, "Gemfile.lock"), fixture.lock);
      }
      for (const [name, text] of Object.entries(contextPartials)) {
        fs.mkdirSync(path.dirname(path.join(views, name)), { recursive: true });
        fs.writeFileSync(path.join(views, name), text);
      }
      const renders = contextRenders.map((call, i) => {
        const template = `render_${i + 1}`;
        const file = path.join(views, `${template}.html.erb`);
        // The render sits on a different line in each template, so the line check tells lines apart.
        fs.writeFileSync(file, `${"<p>x</p>\n".repeat(i)}<%= ${call} %>\n`);
        return { call, template, file };
      });
      const official = await mapParallel(
        renders,
        PARALLEL_RUBY_PROCESSES,
        // Without actionview in the lockfile nothing renders through Rails, and no render check applies.
        (render) =>
          fixture.rails
            ? contextOfficial(fixture, root, render.template)
            : null,
      );
      const ours = oursByPath(
        { extension: ".erb" },
        renders.map((render) => render.file),
      );
      let officialErrors = 0;
      let oursErrors = 0;
      const same = { warning: 0, error: 0 };
      let partlyCompared = 0;
      let bodyErrors = 0;
      const mismatches = [];
      renders.forEach((render, i) => {
        const expected = official[i]?.bodyError ? null : official[i];
        if (official[i]?.bodyError) bodyErrors++;
        const answers = (ours.get(path.resolve(render.file)) ?? []).filter(
          (d) => isRenderDiagnostic(d.message),
        );
        if (expected) officialErrors++;
        if (expected?.partlyCompared) partlyCompared++;
        if (answers.length) oursErrors++;
        const agreeing = expected
          ? answers.find(
              (d) => d.message === expected.message && d.line === expected.line,
            )
          : undefined;
        if (agreeing || (!expected && !answers.length)) {
          if (agreeing) same[agreeing.severity]++;
          return;
        }
        const show = (d) => `${JSON.stringify(d.message)} at line ${d.line}`;
        mismatches.push(
          `${render.call}  (render_${i + 1}.html.erb)\n` +
            `    official: ${expected ? show(expected) : "no error"}${expected?.partlyCompared ? " (rest of the message not compared)" : ""}\n` +
            `    ours:     ${answers.length ? answers.map(show).join("; ") : "no diagnostic"}`,
        );
      });
      if (fixture !== contextFixtures[0]) console.log();
      console.log(`context ${fixture.title}  (${renders.length} renders)`);
      if (!fixture.rails) {
        console.log(
          `errors:  official n/a (no Rails render), ours ${oursErrors}`,
        );
      } else {
        // Ours are predictions of a render-time error, so they are warnings; say which severity agreed.
        const severities = Object.entries(same)
          .filter(([, count]) => count)
          .map(([severity, count]) => `${count} as ${severity}`);
        const agreed = same.warning + same.error;
        console.log(
          `errors:  official ${officialErrors}, ours ${oursErrors}, same ${agreed}${agreed ? ` (ours ${severities.join(", ")})` : ""}`,
        );
      }
      if (partlyCompared) {
        console.log(
          `not compared: ${partlyCompared} (MissingTemplate's formats and search paths)`,
        );
      }
      if (bodyErrors) {
        console.log(
          `not compared: ${bodyErrors} (errors from the partial's own body, which no render check covers)`,
        );
      }
      console.log(`mismatches: ${mismatches.length}`);
      if (process.env.VERBOSE) mismatches.forEach((m) => console.log(m));
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
};

// Every actionview version of the fixtures must load at exactly that version.
const contextInstalled = () =>
  CONTEXT_ACTIONVIEW_VERSIONS.every((version) => {
    const script = `gem "actionview", "${version}"; require "logger"; require "uri"; require "action_view"; print ActionView::VERSION::STRING`;
    const r = spawnSync("ruby", ["-e", script], { encoding: "utf8" });
    return r.status === 0 && r.stdout === version;
  });

const contextLanguage = {
  gem: "actionview",
  version: CONTEXT_ACTIONVIEW_VERSIONS.join(", "),
  installed: contextInstalled,
  report: contextReport,
};
const lang =
  process.argv[2] === "context" ? contextLanguage : languages[process.argv[2]];
if (!lang) {
  console.error(
    `usage: bun test/oracle.js <${[...Object.keys(languages), "context"].join("|")}>`,
  );
  process.exit(2);
}
if (!(lang.installed ?? installed)(lang)) {
  console.log(
    `skipped: ${lang.gem} ${lang.version}${lang.pins ? ` with ${lang.pins.map((p) => `${p.gem} ${p.version}`).join(", ")}` : ""} is not installed (ruby -r${lang.gem}); see test/oracle/README.md`,
  );
  process.exit(0);
}
if (!fs.existsSync(bin)) {
  console.error(
    `missing ${bin}; run cargo build --release --manifest-path server/Cargo.toml`,
  );
  process.exit(2);
}
await (lang.report ?? (() => languageReport(lang)))();
