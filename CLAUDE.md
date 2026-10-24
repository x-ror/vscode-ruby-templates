# Coding standards

Applies to all code in this repo: Rust (`server/`), JS (`src/`, `test/`), Ruby in templates/snippets.

## Dependencies: language first

1. Standard library / language features (Rust `std`, Node built-ins, Ruby/Rails built-ins).
2. Something that already exists in this repo (`server/src/util.rs`, `helpers.rs`, ...). Search before writing.
3. An already-installed dependency.
4. Only then custom code. A new dependency needs a reason a few lines of code can't cover; state it in the commit.

## Comments

- Only when the code can't say it: why a decision was made, a non-obvious constraint, a workaround with its cause.
- One short line where possible. No comments that restate the code, no commented-out code, no changelog-style notes.
- Public Rust items get a one-line `///` doc only if the name doesn't already explain them.

## Design (SOLID / clean code, pragmatically)

- Single responsibility: a function does one thing, its name says what. Split when pieces have real names, not to hit a line count.
- Small interfaces, explicit dependencies; pass data in rather than reaching for globals.
- No abstraction without a second use: no trait/class/interface with one implementation, no factory for one product, no config for a value that never changes. YAGNI beats speculative "open/closed".
- Early returns over nesting. Descriptive names over abbreviations. No magic numbers in logic; name them.
- Handle errors explicitly: Rust `Result`/`?`, no `unwrap` on user input; JS no swallowed promise rejections.
- Delete dead code instead of keeping it "just in case".

## Formatting

- Whitespace basics for every file: `.editorconfig` (UTF-8, LF, final newline, 2 spaces; 4 for Rust).
- Rust: `rustfmt` with `server/rustfmt.toml` (`max_width = 140`, `use_small_heuristics = "Max"`). Run `cargo fmt` before committing.
- JS: `bun fmt` with its default rules (no config file): `bun run format` before committing, `bun run format:check` to verify.
- JSON (grammars, snippets, `package.json`): 2 spaces, keep key order.
- `.githooks/pre-commit` (enabled by `bun install` via `prepare`) formats staged JS and Rust files and re-stages them; it refuses files that also have unstaged changes.
- Formatting-only changes go in their own `style:` commit, never mixed with logic.

## Language idioms

- Rust: `cargo clippy` clean; borrow over `clone`; iterators where clearer; LSP positions are UTF-16.
- JS:
  - Bun is the toolchain (install, scripts, tests, bundling), but `src/` runs in VS Code's Node: no `Bun.*` or `bun:*` APIs there.
  - ES modules (`"type": "module"`, VS Code ≥ 1.100): `import`/`export`, never `require`/`module.exports`. Relative imports end in `.js`; paths from `import.meta.dirname`/`import.meta.url`; JSON via `JSON.parse(fs.readFileSync(…))`.
  - Node built-ins and the language before packages: `fs/promises`, `path`, `URL`, `child_process`, `Array`/`Object`/`Map`/`Set` methods, `structuredClone`, template literals. No lodash/underscore-style helpers.
  - `const` by default, `let` when reassigned, never `var`; `===`; optional chaining `?.` and `??` instead of manual null checks.
  - async/await over callbacks and `.then` chains; every promise awaited or its rejection handled.
  - Small pure functions; destructuring for options; no mutation of arguments.
  - VS Code API: register every disposable in `context.subscriptions`; use built-in APIs (`workspace.fs`, `languages.*`) before custom code; syntax-check with `bun build --no-bundle <file>`.
- Ruby/templates: Rails helpers and conventions; no business logic in views.

## Checks

Run the ones that cover what you changed; all of them before a release.

- Rust: `cargo fmt --manifest-path server/Cargo.toml --check`, `cargo clippy --manifest-path server/Cargo.toml`, `cargo test --manifest-path server/Cargo.toml` (also asserts `test/broken/bad.html.{haml,slim}`)
- Server end to end: `bun run build:server && bun run test:server && bun test/e2e.js` (e2e asserts and exits non-zero on a mismatch)
- Grammars: `bun run test:tokens` (snapshots in `test/__snapshots__`, `UPDATE=1` rewrites them)
- JS: `bun run format:check`, `bun run test:unit`, `bun run test:slim`, `bun run test:snippets`, `bun run build` (the bundle VS Code loads); any JS file: `bun build --no-bundle <file>`
- Slim formatter: `bun run test:format` (needs `ruby -rslim`)
- Client in a real VS Code: `bun run test:vscode`, under `xvfb-run -a` on Linux without a display (`test/vscode/suite.js`; skips formatter/linter checks whose gem isn't installed)

## Where tests go

- Rust logic: `#[cfg(test)] mod tests` in the same file; file-system cases on a temp dir (see `partials.rs` `model_render_partials`).
- LSP behaviour through the real server: a case in `test/e2e.js`.
- Grammar scopes: a line in the matching `test/sample.*` (new samples need an entry in `test/tokenize.js` `samples`).
- JS helpers: the matching `test/*.js` run by `test:unit`/`test:slim`/`test:snippets`, with `assert`.
- Client behaviour that needs VS Code (commands, providers, settings): a check in `test/vscode/suite.js`.
- Broken-input diagnostics: `test/broken/*`, asserted from Rust via `include_str!` (`broken_fixture`); a fixture nothing reads isn't a test.

## Commits

[Conventional Commits](https://www.conventionalcommits.org/): `<type>(<scope>): <summary>`.

- Types: `feat`, `fix`, `refactor`, `perf`, `style`, `test`, `docs`, `build`, `ci`, `chore`.
- Scopes (optional): `server`, `client`, `grammar`, `snippets`, `docs`, `ci`.
- Summary: imperative, lowercase, no trailing period, ≤ 72 chars. Body explains why, not what.
- Breaking change: `!` after the type/scope and a `BREAKING CHANGE:` footer.
- One logical change per commit; a new dependency is justified in the body (see Dependencies).
