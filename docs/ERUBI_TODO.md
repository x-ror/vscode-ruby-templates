# Erubi / Erubis support: TODO

Goal: open `.erubi` / `.erubis` (and `.html.erubi`, …) files as ERB, with the same grammar and server features. Same tag syntax as Rails ERB, so this is file-type plumbing, not a new language.

Legend as in `ERB_TODO.md`.

## Tasks

- [x] Add `.erubi`, `.erubis` to the `erb` language's `extensions` (`package.json` `contributes.languages[id=erb]`). The client needs no change: `documentSelector` is built from every `contributes.languages` id (`src/client.js` `startClient`), and the server accepts the `erb` language id (`server/src/main.rs` `TemplateLang::from_uri_or_id`).
- [x] Path fallback for documents without a language id, i.e. `--check` (`server/src/main.rs` `TemplateLang::from_uri_or_id`: add `.erubi`/`.erubis` next to `.erb`/`.rhtml`). Without it `ruby-templates-ls --check x.erubi` prints "unknown template type" and exits 1.
- [x] HTML-host detection for `x.html.erubi` so tag diagnostics, folding and HTML outline apply (`server/src/main.rs` `html_host`: it only checks `host_format(&file, ".erb")`; accept `.erubi`/`.erubis` too).
- [x] Converter file name (`src/extension.js` `convert`: `replace(/\.(erb|haml|rhtml|slim)$/, …)`): add `erubi|erubis`. Without it `newName === fileName` on `show.html.erubi`, and "Save as … → Overwrite" writes the Haml/Slim output over the source.
- [x] Nothing else: toggle/wrap/preview key off `editorLangId =~ /^erb/` and `engineOf` (`package.json` `keybindings`/`menus`, `src/extension.js`), which already cover the `erb` id.

## Tests

- [x] Extend `host_specific_erb_ids` with `x.erubi` → `Some(TemplateLang::Erb)` and `html_host_by_file_name` with `/v/show.html.erubi` → `true` (`server/src/main.rs` tests). These are the only new branches.
- No token sample: the grammar is unchanged. No e2e case: same code path as `.erb`.

## Open questions

- Erubis-only syntax (`<%=== %>` debug-print, custom `pattern:` like `[% %]`) is not supported by `syntaxes/erb.tmLanguage.json` or tree-sitter-embedded-template. Default: don't support it (Erubis is unmaintained since 2011; Erubi replaced it in Rails 5.1).
- Erubi's `CaptureBlockEngine`/`CaptureEndEngine` add `<%|` tags; opt-in engines, same default: skip.
- Preview and compile run Erubi as Rails does (`src/erbEngine.js`), so `<%==` works; stdlib ERB only when the gem is missing.
- Rails' ActionView only registers the `:erb` handler, so `.erubi` files are mostly Roda/Tilt/Sinatra views outside `app/views`; Rails partial navigation won't apply there (`partials::views_root` returns `None`). Roda's `render("x")` → `views/x.erubi` lookup: out of scope until someone asks.

## Size

XS: ~5 lines in `package.json` + `main.rs` + `extension.js`, 2 test assertions.
