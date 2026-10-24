# Built-in formatter: plan and status

Decision D12 in `docs/DECISIONS.md`. A whitespace-only formatter for HTML ERB (`.erb`, `.erubi`, `.erubis`, `.rhtml`), Haml and Slim, shared by the language server, the `ruby-templates` CLI and the npm package.

Invariants, each enforced by tests: (a) idempotent, (b) non-whitespace preserved, (c) no new diagnostics when the input had none of that kind, (d) on doubt the input comes back unchanged with a reason.

## Stages

### 0. Plan

- [x] `D12` in `docs/DECISIONS.md`, under "Decided"
- [x] This checklist, with `Blockers` and `Notes`

### 1. CLI

- [x] Library target plus a second `[[bin]]` `ruby-templates`; `ruby-templates-ls --check/--format` keep working
- [x] `check <files|dirs>`
- [x] `format [--check] [--write] <files|dirs|->`, `--stdin-filepath`, exit codes 0/1/2
- [x] Unit tests, a CLI test, CI packages the same `.vsix` contents

### 2. ERB formatter

- [ ] Indentation from HTML nesting and ERB blocks (`if`/`else`/`elsif`/`end`, `do … end`, `case`/`when`), void elements, comments, multi-line tags, `<pre>`/`<textarea>` untouched, `<script>`/`<style>` moved as a unit
- [ ] Branches that leave different tags open: region left unchanged
- [ ] Non-HTML ERB (`.js.erb`, `.json.erb`, `.text.erb`, …) unchanged
- [ ] Unit tests per construct; invariant tests on `test/sample.*`
- [ ] Corpora: zero invariant violations, zero crashes
- [ ] Compared with `htmlbeautifier`; intentional differences listed below

### 3. Haml and Slim

- [ ] `format` covers Haml (re-indent) and Slim (`slim_format.rs`) with the same invariants
- [ ] `bun run test:format` passes

### 4. Config

- [ ] Editor options → `.editorconfig` → `.erb_lint.yml` / `.haml-lint.yml` / `.slim-lint.yml` → defaults
- [ ] Tests per source and for precedence
- [ ] Table of supported keys in `docs/`, one line in the README; `--verbose` lists ignored keys

### 5. Editor integration

- [ ] Language server formats ERB (`rubyTemplates.erb.formatter = builtin`), Haml and Slim with the editor's options
- [ ] `package.json` setting value, a check in `test/vscode/suite.js`, README, CHANGELOG

### 6. npm package

- [ ] `npm/`: main package, ES-module `bin` wrapper, platform package manifests generated from the CI binaries
- [ ] Assemble script and `bun test/npm.js`
- [ ] CI job that assembles the packages; publish job on `v*` tags with trusted publishing (not run)

### 7. Finish

- [ ] README, `docs/*_TODO.md`, CHANGELOG
- [ ] Full check list from `CLAUDE.md`
- [ ] Review of the whole branch against `main`; report below

## Blockers

None yet.

## Notes

- Branch `feat/formatter` from `origin/main` (the clone's local `main` was stale).
- Layout (stage 1): one package with a `[lib]` (`src/lib.rs`) and two `[[bin]]`s. The server moved from `main.rs` to `lsp.rs` unchanged, language detection to `lang.rs`; `main.rs` and `bin/ruby-templates.rs` only dispatch into `cli.rs` / `lsp::serve`. A Cargo workspace would have moved every path in CI, `install-bin.js` and the tests for no gain. CI uploads the CLI as `cli-<platform>` artifacts; the `server-*` pattern that feeds the `.vsix` doesn't match them.
- `git push` to `feat/formatter` works. `gh` is installed but its token is rejected (`gh auth status`), so the draft PR goes through the GitHub connector instead.
- Stage 1 review: fixed exit codes for invariant failures and unreadable files, folder-walk noise, multi-file stdout, symlinked folders, the non-whitespace guard (ASCII whitespace only). Deferred, not rejected: `trim_trailing_whitespace` and the blank-line steps had no reader yet; stage 2 (ERB) and stage 4 (config) use them.
- Found on the way: `index.js.erb` under `app/views` was taken for a localized HTML view (`js` looks like a locale), so tag checks ran on it; fixed in `lang.rs` with a test.
- Corpora (outside the repo, shallow sparse clones of `app/views` and `app/components`): Forem, Mastodon, rubygems.org, Whitehall, freeats. The network allowed it.
