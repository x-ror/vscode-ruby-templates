# Decisions

Ideas and questions waiting for an accept or reject, plus the decisions already made and the known open bugs.

To decide an item, change its `Status:` to `accepted` or `rejected` and add a short reason. Accepted items then move to "Decided", with the commit that implements them.

## Pending

### D6. Migrate from tower-lsp to tower-lsp-server

tower-lsp 0.20 is unmaintained. Its maintained fork, tower-lsp-server, has a slightly different API.

- **A:** migrate now.
- **B:** migrate when a bug or a missing LSP 3.18 feature forces it.

**Recommendation:** B. Nothing is broken today, and `cargo audit` in CI will flag any advisory.
Status: pending

### D10. Mutation testing

`cargo-mutants` found real test gaps once: 31 surviving mutants, now only equivalent ones are left. A full run on all files takes hours.

- **A:** run it by hand on the touched files before a release.
- **B:** run it in CI on changed files.
- **C:** drop it.

**Recommendation:** A.
Status: pending

## Decided

- **Locale-only ERB views are HTML** (D1): `<name>.<locale>.erb` under `app/views` gets the HTML checks (`html_host`).
- **turbo_stream views keep the HTML partial fallback** (D2): turbo-rails registers no lookup fallback itself, but Turbo sends `Accept: text/vnd.turbo-stream.html, text/html`, so ActionView looks up both formats. A missed warning is the worse case of being wrong.
- **Erubi, Jbuilder and Builder are supported; Liquid is deferred** (D5) until someone asks, since Shopify ships its own extension. Plans stay in `docs/*_TODO.md`.
- **Formats an app registers itself are read into the project context** (D11): `KNOWN_FORMATS` (`partials.rs`) is a fresh app's `Mime::SET`; `Mime::Type.register` / `register_alias "…", :name` in `config/initializers/**/*.rb` (the template's app and the lockfile's app, as `Mime::SET` is process-wide) are added by `Project::is_format`, used wherever a template name is split, so `_card.mobile.erb` is the partial `card`. Not read: gem registrations (a setting only when one comes up), multi-line or indirect calls; not `bin/rails runner` (boots the app).
- **Symlinked folders under `app/views` are followed** (D7), by completion, partial rename and the template index; walkdir detects loops.
- **Form-builder completion and helper hover only inside Ruby code** (D8), not in template text.
- **Coding rules live in `CLAUDE.md`, and the agents point to it.** Bugs go through the cycle feature-dev → refactorer → test-writer. feature-dev and test-writer run on Sonnet.
- **Conventional Commits, one logical change per commit, formatting in separate `style:` commits.** Tests are written only where they catch a plausible regression, and each is seen failing first.
- **rustfmt** with `max_width = 140` and `use_small_heuristics = "Max"`; **`bun fmt`** with default rules. A plain git **pre-commit hook** formats staged files, with no husky.
- **The JS is ES modules, and the extension requires VS Code ≥ 1.100.** vscode-languageclient is on 11 with a log output channel.
- **Standard library before crates.** `once_cell` was replaced by `LazyLock`, `serde` was dropped, the minimum Rust version is 1.82, and `cargo audit` runs in CI.
- **Partial names follow ActionView**: `name[.locale][.format][+variant].handler`. A partial must be in the view's format or have none; JS views also find HTML partials.
- **Jbuilder and Builder keep the `ruby` language id**, because Ruby LSP owns those files. **Liquid** gets no Shopify-specific features.
- **CI checks formatting and lints** (D4, option A plus clippy): a `lint` job runs `cargo fmt --check`, `cargo clippy -D warnings` and `bun run format:check`, and packaging waits for it. `bun test/e2e.js` runs in the linux-x64, darwin-arm64 and win32-x64 server jobs; `bun run test:format` (needs Ruby and the slim gem) is not in CI yet.
- **Bun is the JS toolchain**: `bun install` (`bun.lock`, version in `packageManager`), scripts and tests through `bun`, and `bun build` bundles `src/` into `dist/extension.js`, the only JS in the `.vsix`. The extension itself still runs in VS Code's Node. Formatting is `bun fmt`, built into Bun, so there is no formatter dependency and no `node_modules/.bin` shim for Windows Smart App Control to block.
- **Client behaviour is tested inside a real VS Code** (D3, beyond one test): `test/vscode/` with `@vscode/test-electron` checks commands, providers, formatters, linters and the server; CI runs it under xvfb, and checks whose tool isn't installed are skipped.
- **On Windows, Ruby tools run through `cmd.exe`.** Arguments containing `"` are refused. A `%VAR%` in an argument would still expand. This path (slimrb, haml, the ERB script, `bin/rails runner`, `bundle exec`) is checked by code review only, an accepted risk while the extension has no users (D9); test it once by hand on Windows before the first public release.
- **A built-in whitespace-only formatter** (D12), from the language server, the `ruby-templates` CLI and the npm package; superseded by D13 except its whitespace-only scope. Original plan: `docs/FORMATTER_PLAN.md`.
- **Our own checks and formatter, official sources only, no external tools** (D13): Slim, Haml and ERB are checked and formatted by our Rust code following only the languages' official projects (Slim 5.2.2, Haml 7.5.1, ruby/erb, Erubi and Rails' ERB handler, Prism 2.0.0 for Ruby syntax) and, for the HTML of ERB templates and the HTML Slim and Haml compile to, the WHATWG HTML standard: their parsers' and compilers' errors with their messages, their changelogs, their test suites; formatting is whitespace only, in the 2-space unit of their examples, and must leave the official compiled output unchanged (ERB copies text verbatim, so ERB formatting changes rendered whitespace, as `docs/LINT_PLAN.md` 3b describes). Third-party tools (slim-lint, haml-lint, erb_lint, Herb, htmlbeautifier, erb-format) are neither sources nor run, and their config files are not read; the settings that run them are removed. Plan and status: `docs/LINT_PLAN.md`.

## Open bugs

- None known. The fixed bugs are in `git log --grep "^fix"` and `CHANGELOG.md`.
