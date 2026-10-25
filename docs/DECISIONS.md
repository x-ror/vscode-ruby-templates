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

### D9. Windows check

The cmd.exe path for Ruby tools was checked by code review only. The Windows CI job runs the Rust tests and the LSP end-to-end test, but no Ruby tool.

- **A:** test once by hand on a Windows machine with the haml and slim gems installed.
- **B:** add a Windows CI job with Ruby that runs the converter end to end.
- **C:** accept the risk.

**Recommendation:** A.
Status: pending

### D10. Mutation testing

`cargo-mutants` found real test gaps once: 31 surviving mutants, now only equivalent ones are left. A full run on all files takes hours.

- **A:** run it by hand on the touched files before a release.
- **B:** run it in CI on changed files.
- **C:** drop it.

**Recommendation:** A.
Status: pending

### D11. Formats an app registers itself

`KNOWN_FORMATS` (`server/src/partials.rs`) is the `Mime::SET` of a fresh Rails app. A format the app adds (`Mime::Type.register_alias "text/html", :mobile`) isn't one, so `_card.mobile.erb` isn't the partial `card`: `render "card"` warns that it's missing, and definition, completion and rename skip the file.

- **A:** a setting, `rubyTemplates.extraFormats`, like `slim.extraEngines`.
- **B:** read `Mime::Type.register` / `register_alias "…", :name` from the app's `config/initializers/*.rb`, where Rails generates `mime_types.rb`, as `db/schema.rb` and `Gemfile.lock` are read. Misses formats registered by gems or with computed names.
- **C:** `Mime::SET.symbols` through `bin/rails runner`, as for routes: exact, but boots the app and needs Ruby and a trusted folder.

**Recommendation:** B, and add A only when a gem-registered format comes up. The cost is passing the app's formats to the 19 calls of `template_format` / `template_stem` in 6 files instead of reading a constant.
Status: pending

## Decided

- **Locale-only ERB views are HTML** (D1): `<name>.<locale>.erb` under `app/views` gets the HTML checks (`html_host`).
- **turbo_stream views keep the HTML partial fallback** (D2): turbo-rails registers no lookup fallback itself, but Turbo sends `Accept: text/vnd.turbo-stream.html, text/html`, so ActionView looks up both formats. A missed warning is the worse case of being wrong.
- **Erubi, Jbuilder and Builder are supported; Liquid is deferred** (D5) until someone asks, since Shopify ships its own extension. Plans stay in `docs/*_TODO.md`.
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
- **On Windows, Ruby tools run through `cmd.exe`.** Arguments containing `"` are refused. A `%VAR%` in an argument would still expand.
- **A built-in formatter that changes only whitespace** (D12): indentation, trailing whitespace, the final newline and line endings, for HTML ERB (also Erubi, Erubis, `.rhtml`), Haml and Slim, from the language server, a `ruby-templates` CLI and an npm package that ships the same binary. It never reflows or reorders, never changes Ruby code, heredocs, filter bodies or `<pre>`/`<textarea>` bodies, moves `<script>`/`<style>` bodies and multi-line tags only as a block, and leaves a file unchanged when its structure is in doubt. Linting and generic formatting stay with the project's own tools (erb_lint, haml-lint, slim-lint, herb, htmlbeautifier, erb-format); their configs are read only for the few keys that mean the same as ours, and our own rules exist only where they don't duplicate a real linter. Plan and status: `docs/FORMATTER_PLAN.md`. Superseded by D13: only its whitespace-only scope for Slim and Haml formatting remains; reading third-party configs and delegating to htmlbeautifier, erb-format, haml-lint and the linters are dropped.
- **Our own checks and formatter, official sources only, no external tools** (D13): Slim, Haml and ERB are checked and formatted by our Rust code following only the languages' official projects (Slim 5.2.2, Haml 7.5.1, ruby/erb, Erubi and Rails' ERB handler, Prism 2.0.0 for Ruby syntax) and, for the HTML of ERB templates and the HTML Slim and Haml compile to, the WHATWG HTML standard: their parsers' and compilers' errors with their messages, their changelogs, their test suites; formatting is whitespace only, in the 2-space unit of their examples, and must leave the official compiled output unchanged (ERB copies text verbatim, so ERB formatting changes rendered whitespace, as `docs/LINT_PLAN.md` 3b describes). Third-party tools (slim-lint, haml-lint, erb_lint, Herb, htmlbeautifier, erb-format) are neither sources nor run, and their config files are not read; the settings that run them are removed. Plan and status: `docs/LINT_PLAN.md`.

## Open bugs

- herb-lint on Windows lints nothing below the folder it runs in ("excluded by configuration patterns", `filesChecked: 0`): its exclude check gets backslash relative paths. Upstream (herb-tools); `--force` would bypass the user's excludes.
- See also D11 (app-registered formats). The fixed bugs are in `git log --grep "^fix"` and `CHANGELOG.md`.
