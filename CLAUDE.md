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
- JS: no formatter dependency. Match the existing style: 2 spaces, single quotes, semicolons, lines ≤ 120.
- JSON (grammars, snippets, `package.json`): 2 spaces, keep key order.
- Formatting-only changes go in their own `style:` commit, never mixed with logic.

## Language idioms

- Rust: `cargo clippy` clean; borrow over `clone`; iterators where clearer; LSP positions are UTF-16.
- JS:
  - Node built-ins and the language before packages: `fs/promises`, `path`, `URL`, `child_process`, `Array`/`Object`/`Map`/`Set` methods, `structuredClone`, template literals. No lodash/underscore-style helpers.
  - `const` by default, `let` when reassigned, never `var`; `===`; optional chaining `?.` and `??` instead of manual null checks.
  - async/await over callbacks and `.then` chains; every promise awaited or its rejection handled.
  - Small pure functions; destructuring for options; no mutation of arguments.
  - VS Code API: register every disposable in `context.subscriptions`; use built-in APIs (`workspace.fs`, `languages.*`) before custom code; syntax-check with `node --check <file>`.
- Ruby/templates: Rails helpers and conventions; no business logic in views.

## Checks

- `cargo fmt --manifest-path server/Cargo.toml --check`
- `cargo test --manifest-path server/Cargo.toml`
- `npm run build:server && npm run test:server && node test/e2e.js`
- `npm run test:tokens`

## Commits

[Conventional Commits](https://www.conventionalcommits.org/): `<type>(<scope>): <summary>`.

- Types: `feat`, `fix`, `refactor`, `perf`, `style`, `test`, `docs`, `build`, `ci`, `chore`.
- Scopes (optional): `server`, `client`, `grammar`, `snippets`, `docs`, `ci`.
- Summary: imperative, lowercase, no trailing period, ≤ 72 chars. Body explains why, not what.
- Breaking change: `!` after the type/scope and a `BREAKING CHANGE:` footer.
- One logical change per commit; a new dependency is justified in the body (see Dependencies).
