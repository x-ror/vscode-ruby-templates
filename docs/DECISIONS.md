# Decisions

Ideas and questions waiting for an accept or reject, plus the decisions already made and the known open bugs.

To decide an item, change its `Status:` to `accepted` or `rejected` and add a short reason. Accepted items then move to "Decided", with the commit that implements them.

## Pending

### D1. HTML diagnostics for locale-only ERB views

`show.de.erb` currently behaves like a bare `show.erb`: no HTML tag diagnostics, folding or outline, because its output format is unknown. In `app/views`, such files are almost always localized HTML.

- **A:** keep as is.
- **B:** treat `<name>.<locale>.erb` under `app/views` as HTML.

**Recommendation:** B. A one-line change in `html_host`, plus a test.
Status: pending

### D2. turbo_stream views falling back to HTML partials

`render "x"` inside `create.turbo_stream.erb` currently finds `_x.html.erb`, so no "not found" warning appears. ActionView itself only adds this fallback for JS views. It is unverified whether turbo-rails adds it for turbo_stream views.

- **A:** keep the fallback.
- **B:** drop it.
- **C:** check the turbo-rails source first, then pick A or B.

**Recommendation:** C. Keep A until it is checked, since the worst case is a missed warning rather than a false one.
Status: pending

### D3. Extension-host smoke test (`@vscode/test-electron`)

One test that launches real VS Code, opens an ERB file and checks that the server reports ready and sends a diagnostic. It would have caught the vscode-languageclient 11 startup failure, which no current test noticed. The cost is a new dev dependency and a VS Code download in CI, run under xvfb on Linux. It would also cover the restart queue and the preview escaping, which have no tests today.

- **A:** add it.
- **B:** don't add it, and keep checking by hand with F5.

**Recommendation:** A, with one test only.
Status: pending

### D4. More checks in CI

These run locally but not in CI:
- `cargo fmt --check` and `npm run format:check`: seconds each.
- `node test/e2e.js`: needs the built server, which the `server` job already produces.
- `npm run test:format`: needs Ruby and the slim gem.

- **A:** add `fmt --check`, `format:check` and e2e now. Add `test:format` with `ruby/setup-ruby` later.
- **B:** add all four now.
- **C:** none.

**Recommendation:** A.
Status: pending

### D5. Next template languages

The plans are in `docs/*_TODO.md`:

| Language | Plan | Size |
|---|---|---|
| Erubi / Erubis | `ERUBI_TODO.md` | XS |
| Jbuilder | `JBUILDER_TODO.md` | S, the format filter is already done |
| Builder | `BUILDER_TODO.md` | XS, on top of Jbuilder |
| Liquid | `LIQUID_TODO.md` | L, phased |

Accept or reject each one separately.

**Recommendation:** accept Erubi, Jbuilder and Builder. Defer Liquid until someone asks for it, since Shopify already ships its own extension.
Status: pending

### D6. Migrate from tower-lsp to tower-lsp-server

tower-lsp 0.20 is unmaintained. Its maintained fork, tower-lsp-server, has a slightly different API.

- **A:** migrate now.
- **B:** migrate when a bug or a missing LSP 3.18 feature forces it.

**Recommendation:** B. Nothing is broken today, and `cargo audit` in CI will flag any advisory.
Status: pending

### D7. Symlinked folders under `app/views`

`resolve` follows symlinks. Completion and partial rename don't, because walkdir doesn't follow links by default.

- **A:** turn on `follow_links(true)`. Walkdir detects loops.
- **B:** leave it as is.

**Recommendation:** A, with a temp-dir test.
Status: pending

### D8. Form-builder completion and helper hover in plain text

Typing `form.` in `<p>Please fill in the form.` pops up 42 FormBuilder methods. Hovering "cache" in `<p>Clear the cache</p>` shows the Rails `cache` helper.

- **A:** gate both on the same "cursor is in Ruby code" check that the rest of completion uses.
- **B:** leave it.

**Recommendation:** A.
Status: pending

### D9. Windows check

The cmd.exe path for Ruby tools was checked by code review only. The Windows CI job runs only the Rust tests.

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

## Decided

- **Coding rules live in `CLAUDE.md`, and the agents point to it.** Bugs go through the cycle feature-dev → refactorer → test-writer. feature-dev and test-writer run on Sonnet.
- **Conventional Commits, one logical change per commit, formatting in separate `style:` commits.** Tests are written only where they catch a plausible regression, and each is seen failing first.
- **rustfmt** with `max_width = 140` and `use_small_heuristics = "Max"`; **Prettier** with default rules. A plain git **pre-commit hook** formats staged files, with no husky.
- **The JS is ES modules, and the extension requires VS Code ≥ 1.100.** vscode-languageclient is on 11 with a log output channel.
- **Standard library before crates.** `once_cell` was replaced by `LazyLock`, `serde` was dropped, the minimum Rust version is 1.82, and `cargo audit` runs in CI.
- **Partial names follow ActionView**: `name[.locale][.format][+variant].handler`. A partial must be in the view's format or have none; JS views also find HTML partials.
- **Jbuilder and Builder keep the `ruby` language id**, because Ruby LSP owns those files. **Liquid** gets no Shopify-specific features.
- **On Windows, Ruby tools run through `cmd.exe`.** Arguments containing `"` are refused. A `%VAR%` in an argument would still expand.

## Open bugs

- `test/broken/bad.html.erb` is not asserted by any test. The Haml and Slim fixtures are.
- For a jump of several levels, the Haml message ("expected N spaces, found M") differs from Haml's own ("indented N levels deeper").
- `KNOWN_FORMATS` is a fixed list, so a custom-registered MIME format is treated as a variant.
- See also D7 (symlinks) and D8 (completion in plain text). The fixed bugs are in `git log --grep "^fix"` and `CHANGELOG.md`.
