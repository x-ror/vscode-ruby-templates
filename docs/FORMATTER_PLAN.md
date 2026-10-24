# Built-in formatter: plan and status

Decision D12 in `docs/DECISIONS.md`. A whitespace-only formatter for HTML ERB (`.erb`, `.erubi`, `.erubis`, `.rhtml`), Haml and Slim, shared by the language server, the `ruby-templates` CLI and the npm package.

Invariants, each enforced by tests: (a) idempotent, (b) non-whitespace preserved, (c) no new diagnostics when the input had none of that kind, (d) on doubt the input comes back unchanged with a reason.

## Stages

### 0. Plan

- [x] `D12` in `docs/DECISIONS.md`, under "Decided"
- [x] This checklist, with `Blockers` and `Notes`

### 1. CLI

- [ ] Library target plus a second `[[bin]]` `ruby-templates`; `ruby-templates-ls --check/--format` keep working
- [ ] `check <files|dirs>`
- [ ] `format [--check] [--write] <files|dirs|->`, `--stdin-filepath`, exit codes 0/1/2
- [ ] Unit tests, a CLI test, CI packages the same `.vsix` contents

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
- Corpora (outside the repo, shallow sparse clones of `app/views` and `app/components`): Forem, Mastodon, rubygems.org, Whitehall, freeats. The network allowed it.
