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

- [x] Indentation from HTML nesting and ERB blocks (`if`/`else`/`elsif`/`end`, `do … end`, `case`/`when`), void elements, comments, multi-line tags, `<pre>`/`<textarea>` untouched, `<script>`/`<style>` moved as a unit
- [x] Branches that leave different tags open: region left unchanged
- [x] Non-HTML ERB (`.js.erb`, `.json.erb`, `.text.erb`, …) unchanged
- [x] Unit tests per construct; invariant tests on `test/sample.*`
- [x] Corpora: zero invariant violations, zero crashes
- [x] Compared with `htmlbeautifier`; intentional differences listed below

### 3. Haml and Slim

- [x] `format` covers Haml (re-indent) and Slim (`slim_format.rs`) with the same invariants
- [x] `bun run test:format` passes

### 4. Config

- [x] Editor options → `.editorconfig` → `.erb_lint.yml` / `.haml-lint.yml` / `.slim-lint.yml` → defaults
- [x] Tests per source and for precedence
- [x] Table of supported keys in `docs/`, one line in the README; `--verbose` lists ignored keys

### 5. Editor integration

- [x] Language server formats ERB (`rubyTemplates.erb.formatter = builtin`), Haml and Slim with the editor's options
- [x] `package.json` setting value, a check in `test/vscode/suite.js`, README, CHANGELOG

### 6. npm package

- [x] `npm/`: main package, ES-module `bin` wrapper, platform package manifests generated from the CI binaries
- [x] Assemble script and `bun test/npm.js`
- [x] CI job that assembles the packages; publish job on `v*` tags with trusted publishing (not run)

### 7. Finish

- [x] README, `docs/*_TODO.md`, CHANGELOG
- [x] Full check list from `CLAUDE.md`
- [x] Review of the whole branch against `main`; report below

## Blockers

None. Every tool the stages needed was available: network for the corpora and gems, Ruby 3.3 with htmlbeautifier, slim and haml, VS Code under `xvfb-run`, npm for packing.

## Notes

- Branch `feat/formatter` from `origin/main` (the clone's local `main` was stale).
- Layout (stage 1): one package with a `[lib]` (`src/lib.rs`) and two `[[bin]]`s. The server moved from `main.rs` to `lsp.rs` unchanged, language detection to `lang.rs`; `main.rs` and `bin/ruby-templates.rs` only dispatch into `cli.rs` / `lsp::serve`. A Cargo workspace would have moved every path in CI, `install-bin.js` and the tests for no gain. CI uploads the CLI as `cli-<platform>` artifacts; the `server-*` pattern that feeds the `.vsix` doesn't match them.
- `git push` to `feat/formatter` works. `gh` is installed but its token is rejected (`gh auth status`), so the draft PR goes through the GitHub connector instead.
- Stage 1 review: fixed exit codes for invariant failures and unreadable files, folder-walk noise, multi-file stdout, symlinked folders, the non-whitespace guard (ASCII whitespace only). Deferred, not rejected: `trim_trailing_whitespace` and the blank-line steps had no reader yet; stage 2 (ERB) and stage 4 (config) use them.
- Found on the way: `index.js.erb` under `app/views` was taken for a localized HTML view (`js` looks like a locale), so tag checks ran on it; fixed in `lang.rs` with a test.
- ERB (stage 2): `erb_format.rs` reads the `tags.rs` token stream (`tags::tokens`, refactored out of the tag checks so both agree on structure). A markup line gets one unit per open element and ERB block; `<% end %>`/`<% else %>` line up with their opening even under tags a branch left open; leading end tags dedent their line. Units (`<script>`, `<style>`, comments, multi-line tags, multi-line ERB tags) move with their first line; a unit whose lines can't follow (indented less than the first line, or Ruby with a heredoc or a string/`%`-literal/regexp/backtick across a line break, or a JS template literal in a script) keeps itself and every unit around it in place. `<pre>`, `<textarea>`, `<title>` bodies and quoted values spanning lines keep every byte. Optional-end elements (`<li>`, `<p>`, …) indent only when they have an end tag, paired as `tags.rs` pairs them.
- Multi-line HTML tags: "first line only" read as "only the first line gets a level; continuation lines move with it", except inside a quoted value (clarification 4 treats multi-line ERB tags the same way).
- Branches that leave different tags open: indentation is kept from the line where the nesting becomes uncertain to the end of the file (per-path stacks would be needed to resume); `--verbose` names the line.
- Corpus results (stage 2/3), `RUBY_TEMPLATES_CORPUS=… cargo test --release corpus -- --ignored`: 2,163 templates × 5 indentation variants (as is, dedented, doubled, tabs, CRLF) × 5 settings = 46,064 formatted variants, 0 invariant violations, 0 panics; 1,236 refused as in doubt, 6,775 not handled (non-HTML ERB, Jbuilder). The run found and fixed: two Slim indentation-check false positives that the 4-space variants turned into "would add a diagnostic" (continuation lines and engine bodies counted as structure, `25ce868`), and `final_newline = false` not being stable with trailing empty lines.
- Haml compiled output: all 331 Mastodon Haml views with doubled indentation and trailing spaces, formatted, compile to the same Ruby as before formatting (haml 7.5.1, temporaries renamed); the unchanged originals were already formatted. `test/format/nested.haml` keeps that in `bun run test:format`.
- htmlbeautifier 1.4.3 (`-b 1000`, so it keeps blank lines like we do) on the 1,425 `.html.erb` views: 1,167 identical; 103 where it adds or joins lines; 126 that differ in indentation only; 20 we leave unchanged as in doubt; 7 it fails on; 2 other. Differences we keep on purpose:
  - We never add or join lines (it moves `<tr>` and friends onto their own lines).
  - Continuation lines of multi-line ERB and HTML tags keep their offset to the first line; it re-indents each to its own level (most of the 126).
  - Optional-end elements without an end tag don't indent; it indents after every `<p>`, so one missing or extra `</p>` shifts the rest of its file (e.g. Forem `users/_response_templates`, `_auth_provider_settings`).
  - ERB between attributes of a tag and `>` at column 0 don't confuse the nesting (Whitehall `_child_of_banner`, rubygems `onboarding/gems/edit`).
  - Trailing whitespace is trimmed; it keeps it on text lines.
  - Files in doubt (mismatched tags, a `javascript_tag` block) are left alone; it formats them anyway.
- Haml (stage 3): ported to `haml_format.rs` (not `haml_indent.rs`, which only checks structure). The unit stays the file's own (first indented line), as Haml reads it and as the client's re-indenter did; indent settings don't convert Haml. Trailing whitespace is trimmed on structural lines only (Haml strips it there; filter bodies keep it). Haml's indentation errors are what it fixes, so only other errors leave a file alone. The extension's Format Document, Format Selection and _Re-indent Haml_ ask the server; the JS re-indenter and its tests are gone.
- Slim (stage 3): unchanged `slim_format.rs` behind the same entry point; tabs and files Slim rejects are left alone, and blank lines are content (Slim compiles every line).
- Stage 2 review: fixed the literal detection (`%()`, backticks, regexps, comments), the ERB-in-end-tag panic, unit pinning, a quadratic fixpoint and the Slim check hiding errors after an unclosed `[`; `.gitignore`'s `bin/` had kept `server/src/bin/ruby-templates.rs` out of the branch until `e6b6ec4` (the commits before it don't build from a clean clone).
- Config (stage 4): `config.rs`, documented in `docs/FORMATTER_CONFIG.md`. The YAML reader is ~60 lines for nested mappings of scalars, no crate. erb_lint's defaults count only with `EnableDefaultLinters: true`; haml-lint's and slim-lint's mapped linters count unless disabled. A disabled linter has no say. `SpaceAroundErbTag` is listed as ignored (clarification 3), so are haml-lint `Indentation` (Haml keeps its unit) and slim-lint `EmptyLines`/`TrailingBlankLines` (Slim compiles blank lines).
- Editor (stage 5): ERB with `erb.formatter: builtin`, Haml (`reindent`) and the _Re-indent Haml_ command go through client providers that forward to the server's `textDocument/formatting` / `rangeFormatting`; Slim stays dynamically registered by the server. All three use `format::format`. Range formatting leaves out the steps that add or remove lines.
- Kept on purpose (stage 3 review): in the editor, our defaults (trim trailing whitespace, add a final newline) apply when neither the editor nor a project file says otherwise, which is decision 4's order; VS Code only sends those options when they are on. Haml and Slim _Format Document_ therefore now also add a missing final newline.
- npm (stage 6): `npm/ruby-templates` (main package, name set only there), `npm/assemble.js` (platform packages from `cli-<platform>` artifacts), `test/npm.js` (pack, offline install, run). CI: `npm` job uploads the packages; `publish-npm` on `v*` tags uses trusted publishing. Not run. **The first publish probably needs a manual token**: npm lets you configure a trusted publisher only for a package that exists, so the first version of each of the six packages has to be published once with an `NPM_TOKEN` (or by hand) before the OIDC job can take over. `@x-ror` is a placeholder scope.
- Stage 3 review: fixed CRLF range edits, range formatting on files without a final newline, silent no-ops of _Re-indent Haml_, ASCII word characters in the Haml regexps, and the compile check skipping Haml without the slim gem.
- Stage 4/5 review: fixed an editorconfig brace range that ran out of memory, YAML booleans and flow mappings, `tab_width` with spaces, erb_lint `FinalNewline` and extra trailing lines, unreported ignored options, and the client not sending the editor's file options for ERB/Haml.
- Stage 6 review: fixed non-resumable publishing, a missing-artifact check, `libc`/`preferUnplugged` in platform manifests, the wrapper's message, Windows quoting in `test/npm.js`. Not done: static musl builds for npm (glibc 2.39 from ubuntu-24.04 runners is required; `libc: glibc` keeps musl systems from installing a binary that can't run); worth it if Alpine users ask. Rejected: splitting the CLAUDE.md line out of `b8dd298` (it would mean rewriting pushed history).
- Branch review: fixed docs that over-claimed (script/style bodies and multi-line tags move as a block; Haml indentation errors are re-indented; built-in formatting needs the server; exit 2 for non-templates is `check` only), the changelog, TODO paths, dead `fixed_end`, `lang.rs` tests, the Windows binary name in `test/slim_format.js`. Kept: Slim diagnostics parse the file a second time for the render checks (linear, debounced; sharing the parse would thread it through `lsp::diagnostics`); commit messages over 72 characters, the `npm`/`build` scopes and the non-building commits before `e6b6ec4` stay, since fixing them means rewriting pushed history — squash-merge or a rebase by the owner can clean that up.
- Corpora (outside the repo, shallow sparse clones of `app/views` and `app/components`): Forem, Mastodon, rubygems.org, Whitehall, freeats. The network allowed it.

## Report

**Built.** A whitespace-only formatter in the Rust server's new library target, used by three front ends: the language server (Format Document for Slim, and for ERB/Haml through the client's providers, plus Format Selection for ERB and Haml), a `ruby-templates` CLI (`check`, `format [--check|--write|--stdin-filepath] [--verbose]`, exit codes 0/1/2) and an npm package (`@x-ror/ruby-templates`, a placeholder scope) that ships the CLI binary per platform. ERB is new (`erb_format.rs` on the `tags.rs` token stream); Haml is the client's re-indenter ported to Rust (`haml_format.rs`, JS copy deleted); Slim is the existing `slim_format.rs`. Settings: editor → `.editorconfig` → mapped erb_lint/haml-lint/slim-lint linters → defaults (`config.rs`, `docs/FORMATTER_CONFIG.md`). `rubyTemplates.erb.formatter` gained `builtin` (default still `none`).

**Verified, per stage.**

- 0: decision D12, this plan.
- 1: unit tests (args, folder walk), `test/cli.js` (all commands, stdin, `--write`, `--verbose`, exit codes) in CI on Linux/macOS/Windows; `ruby-templates-ls --check/--format` diffed against `main` on every sample and corpus file; `.vsix` file list unchanged (`vsce ls`).
- 2: unit tests per construct in `erb_format.rs`; invariants on unguarded output over `test/sample.*` × 5 indentation variants × 5 settings (`format.rs` `invariants_hold_on_the_samples`) and the same over the corpora (`corpus`, ignored test); runtime guard refuses any result that changes non-whitespace, isn't stable or adds a diagnostic; `htmlbeautifier` comparison.
- 3: Rust tests ported from the JS ones; `test/e2e.js` document and range formatting; `bun run test:format` compiles Slim and Haml fixtures with the gems before and after; all Mastodon Haml views compile the same after formatting.
- 4: tests per source and for precedence in `config.rs`, `with_editor` test, CLI `--verbose` test.
- 5: `test/vscode/suite.js` checks for ERB `builtin`, Haml Format Document, Re-indent Haml and Slim, under `xvfb-run` with VS Code 1.143.1.
- 6: `test/npm.js` (assemble, `npm pack`, offline install, run through the wrapper), in the CI test jobs; publish job written, not run.
- 7: the whole `CLAUDE.md` check list passes: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` (238), `build:server`, `test:server`, `test/e2e.js`, `test/cli.js`, `test/npm.js`, `test:tokens`, `test:unit`, `test:slim`, `test:snippets`, `test:format`, `format:check`, `build`, `test:vscode`. Nothing skipped except the VS Code checks for erb_lint, herb-lint, haml-lint and slim-lint, which skip themselves when those tools aren't installed (they aren't here). Windows and macOS were not run locally; CI covers them.

**Corpus.** Forem, Mastodon, rubygems.org, Whitehall, freeats (`app/views`, `app/components`): 2,163 templates, 46,064 formatted variants, 0 invariant violations, 0 panics. Found on the way and fixed: two Slim indentation-check false positives, a `final_newline = false` instability, a js-view-as-HTML bug in `html_host`.

**Intentional differences from htmlbeautifier** (1,167 of 1,425 views identical): no line breaks added or joined; continuation lines of multi-line tags keep their offset; optional-end elements without an end tag don't indent; ERB inside tags and a `>` at column 0 don't shift the nesting; trailing whitespace trimmed; files in doubt left alone. Details in Notes.

**Blockers and defaults chosen.** No blockers. Defaults taken: "first line only" for multi-line tags read as "the first line gets the level, the rest move with it"; Haml keeps its own indent unit; branches that disagree keep indentation from there to the end of the file; Slim blank lines are content; in the editor our defaults (trim, final newline) apply when nothing else decides. The first npm publish needs a token or a manual publish before trusted publishing can be configured.

**Where it lives.** Branch `feat/formatter` on `x-ror/vscode-ruby-templates`, pushed; draft PR opened against `main`. Nothing published, nothing merged, no tags.
