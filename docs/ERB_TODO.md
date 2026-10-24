# ERB support: TODO

What full ERB support in this extension needs, based on an audit of the code at commit `085b5ba`.

Legend: `[x]` means implemented and checked in the code (the file is named next to it). `[ ]` means missing, partial, or not yet confirmed. `(?)` means it probably works but has no test.

## 1. Tag syntax

- [x] `<% code %>` highlighted as embedded Ruby (`syntaxes/erb.tmLanguage.json` `erb-code`)
- [x] `<%= output %>` (`syntaxes/erb.tmLanguage.json` `erb-output`)
- [x] `<%== raw output %>` for Erubi/Rails (`syntaxes/erb.tmLanguage.json` `<%==?-?`)
- [x] `<%# comment %>` scoped as `comment.block.erb` (`syntaxes/erb.tmLanguage.json` `erb-comment`)
- [x] `<%-` / `-%>` trim markers (`syntaxes/erb.tmLanguage.json`; parser accepts them, checked with `--check`)
- [x] `<%%` / `%%>` literal escapes (`syntaxes/erb.tmLanguage.json` `erb-literal`)
- [x] `# comment` inside a code tag ends at `%>`, not at end of line (`syntaxes/erb.tmLanguage.json` `ruby-inner`)
- [x] `=%>` closing tag scoped as punctuation; `<%_`/`_%>` deliberately not supported (Erubi only allows `-`/`=` before `%>`) (`syntaxes/erb.tmLanguage.json`)
- [ ] `trim_mode: '%'` lines (lines starting with `%` are Ruby; `%%` escapes it) for plain-Ruby ERB files
- [x] Diagnostic: `%>` inside a Ruby string closes the tag early (`server/src/erb.rs` `UNTERMINATED_STRING`; skipped for tags with a heredoc). A `%>` inside `<%#` leaves no trace to detect.
- [ ] Multi-line `<% ... %>` tags containing heredocs

## 2. File types and host languages

- [x] `.erb`, `.rhtml`, `.html.erb` map to the `erb` language id (`package.json` `contributes.languages`)
- [x] Server picks ERB from the language id, or from `.erb`/`.rhtml` when there is none (`server/src/lang.rs` `TemplateLang::from_uri_or_id`)
- [x] `.js.erb` gets a JavaScript host grammar (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.css.erb` / `.scss.erb` get a CSS/SCSS host grammar (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.json.erb` gets a JSON host grammar (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.text.erb` / `.txt.erb` (mailer text parts) get a plain-text host grammar (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.xml.erb` / `.rss.erb` / `.atom.erb` / `.svg.erb` get an XML host grammar (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.md.erb` gets a Markdown host grammar (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.rb.erb` / `.yml.erb` (generators, `database.yml`) get Ruby/YAML host grammars (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.turbo_stream.erb` gets the `erb` id and HTML host via the `.erb` suffix (`package.json`); Turbo snippets tracked separately
- [x] `.html+variant.erb` (e.g. `show.html+phone.erb`) recognized via the `.erb` suffix (`package.json`, `server/src/lsp.rs`); variant-aware lookup tracked under navigation
- [x] Rails generator templates open as their language: `lib/templates/{erb,slim}/**/*.{erb,slim}.tt` and `lib/generators/**/templates/**` (`package.json` `filenamePatterns`, like Haml's); no diagnostics, formatting or slim-lint there, since the generator's ERB is mixed in (`server/src/lsp.rs` `diagnostics`). Non-template `.tt` files (`Gemfile.tt`) are left alone
- [ ] Host-specific language ids (e.g. `erb-js`) or a host-detection grammar so `editor.*` settings and comment toggling follow the host
- [x] Server handles the same host variants (diagnostics shouldn't assume HTML) (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)

## 3. Grammar and highlighting

- [x] HTML host via `text.html.basic` with ERB injected everywhere outside ERB tags (`syntaxes/erb.tmLanguage.json` `injections`)
- [x] Embedded language mapping for ruby/js/css (`package.json` `grammars[erb].embeddedLanguages`)
- [x] Token snapshot script for the sample (`test/tokenize.js`, `test/sample.html.erb`)
- [x] ERB inside HTML attribute values (`class="<%= x %>"`, single and double quotes) (`test/sample.html.erb`)
- [x] ERB inside `<script>` and `<style>` blocks (`test/sample.html.erb`). A `<script>` whose JS holds `a < b` on the `</script>` line runs on as JS; that is VS Code's HTML/JS grammar, plain `.html` does the same
- [x] ERB inside HTML comments `<!-- <%= x %> -->` (`test/sample.html.erb`)
- [x] ERB in attribute-name position, e.g. `<div <%= attrs %>>` (`test/sample.html.erb`)
- [x] `test/tokenize.js` loads the real HTML/Ruby grammars instead of stubs so embedding is actually checked (VS Code's grammars from the pinned `tm-grammars` dev dependency)
- [x] Token snapshot assertions (diff against a committed baseline) instead of a dump (`test/tokenize.js`, `test/__snapshots__/`)
- [ ] Semantic tokens from the server (instance vars, locals, helpers) to fix what TextMate gets wrong

## 4. Language configuration

- [x] Block comment toggle `<%# %>` (`language-configuration/erb.json` `comments.blockComment`)
- [x] Brackets including `<% %>` (`language-configuration/erb.json` `brackets`)
- [x] Auto-closing `<%` → `<% %>`, quotes, brackets, `<!--` (`language-configuration/erb.json` `autoClosingPairs`)
- [x] Surrounding pairs (`language-configuration/erb.json` `surroundingPairs`)
- [x] `#region` / `<%# region %>` folding markers (`language-configuration/erb.json` `folding.markers`)
- [x] Indentation rules for Ruby blocks and HTML tags (`language-configuration/erb.json` `indentationRules`)
- [x] Indent on Enter after an opening `<% if/unless/... %>` (`language-configuration/erb.json` `onEnterRules`)
- [x] Word pattern includes `@ivar`, `$global`, `predicate?` (`language-configuration/erb.json` `wordPattern`)
- [x] Default `tabSize: 2`, spaces (`package.json` `configurationDefaults["[erb]"]`)
- [x] `onEnterRules` for `do |x| %>` and `else`/`elsif`/`when` lines (only `if/unless/...` are covered) (`language-configuration/erb.json`, `test/langconfig.js`)
- [x] `onEnterRules` that keep the cursor between `<% %>` on Enter (`language-configuration/erb.json`: Enter in an empty tag splits it onto 3 lines)
- [x] Typing a whole tag (`<%= x %>`) after auto-closing inserted ` %>` no longer ends in `%> %>` (`src/autoClose.js` `doubledErbClose`; checked in `test/vscode/suite.js`)
- [x] Auto-close HTML tags (`</div>`) in `erb` files on typing `>` (`rubyTemplates.erb.autoCloseTags`, `src/autoClose.js`)
- [x] Linked editing of HTML tag pairs, across ERB tags between them (`server/src/tags.rs` `linked_names`; needs `editor.linkedEditing`)
- [x] Folding for `<% if %>…<% end %>` and HTML elements independent of indentation (server `foldingRange`) (`server/src/outline.rs` `folding_ranges`, merged with indentation folds)
- [ ] Line comment toggle for a single line inside a code tag (`# `) vs whole-tag toggle

## 5. Snippets

- [x] Tag snippets `pe`, `pc`, `pcc` (`snippets/erb.json`)
- [x] Control flow `if`, `ife`, `unless`, `each`, `eachi`, `case` (`snippets/erb.json`)
- [x] `render`, `renderc`, `rendercomp` (`snippets/erb.json`)
- [x] Links/forms: `link_to`, `link_tob`, `button_to`, `form_with`, `form_with_url`, `ff`, `flf`, `fsubmit`, `fselect`, `fcselect`, `fields_for` (`snippets/erb.json`)
- [x] Layout helpers: `content_for`, `yield`, `layout`, `meta_tags`, `assets` (`snippets/erb.json`)
- [x] Hotwire: `turbo_frame`, `turbo_stream`, `stimulus`, `data-action` (`snippets/erb.json`)
- [x] i18n/format: `t`, `l`, `currency`, `pluralize`, `time_ago` (`snippets/erb.json`)
- [x] `flash`, `errors`, `cache`, `debug`, `image_tag`, `content_tag`, `tag.` (`snippets/erb.json`)
- [x] Strict locals magic comment `<%# locals: (name:, default: nil) -%>` (`snippets/erb.json` `locals`)
- [x] `elsif`, `else`, `end`, `when` single-tag snippets (`snippets/erb.json` prefixed `p*` to avoid hijacking prose)
- [x] `turbo_stream.*` action snippets for `.turbo_stream.erb` (`snippets/erb.json` `ts-*`)
- [x] `render @collection` / `render partial:, locals:` / `render layout: do` (`snippets/erb.json` `renderall`/`renderl`/`renderlayout`)
- [x] `javascript_include_tag`, `favicon_link_tag`, `turbo_include_tags`, `asset_path` (`snippets/erb.json` `turbo_include_tags` is deprecated → `turbo_script`)
- [x] Mailer text-part skeleton snippet (`mail_to` is already a completion in `server/src/helpers.rs`) (`snippets/erb.json` `mailer_text`)
- [x] Snippets for ViewComponent templates: `rendercompb` (render with a slot block), `slot` (`with_*`), `slot?`, `content` (`pcontent` in ERB, like the other one-tag snippets), in ERB, Haml and Slim (`snippets/*.json`)

## 6. Commands and editing helpers

- [x] Toggle tag `<% %>` → `<%= %>` → `<%# %>`, multi-cursor (`src/extension.js` `toggleErbTag`, `package.json` keybinding)
- [x] Wrap selection in `<%= %>` (`src/extension.js` `wrapInErb`)
- [x] Preview rendered HTML via `erb -T -` (`src/extension.js` `renderToHtml`)
- [x] Show compiled Ruby (`src/extension.js` `compileToRuby`)
- [x] Convert ERB → Haml / Slim through gems (`src/extension.js` `convert`)
- [x] Toggle tag keeps `-%>` / `_%>` trim markers correctly for every variant (`<%-` maps to `<%=` and drops the `-`) (`src/erbTags.js`, `test/erbTags.js`; `<%-` opener trim can't combine with `=`/`#`, closers kept)
- [x] Preview/compile with Erubi (Rails' engine, `escape: true`) instead of stdlib `erb`, so `<%==` and Rails trim behave the same; stdlib ERB when the gem is missing (`src/erbEngine.js`)
- [x] Preview inside a Rails app (`bin/rails runner` / `ApplicationController.render`) so helpers work (`rubyTemplates.preview.useRails`, `src/extension.js`)
- [x] Extract selection to partial (create `_name.html.erb`, replace with `render "name"`) (`rubyTemplates.extractPartial`, `src/extension.js`, `src/partial.js`)
- [x] Create a missing partial from an unresolved `render "x"` (code action) (`server/src/lsp.rs` `code_action`, `partials::new_template_path`)

## 7. Formatting

- [x] Document formatting provider for ERB: `rubyTemplates.erb.formatter` = `builtin`, `htmlbeautifier` or `erb-format`, registered only when chosen (`src/extension.js` `registerErbFormatting`, `src/erbFormat.js`)
- [x] Built-in whitespace-only formatter for HTML ERB (also Erubi, Erubis, `.rhtml`): HTML and ERB-block nesting, `<script>`/`<style>`/multi-line tags moved as units, `<pre>`/`<textarea>`/Ruby strings untouched, files in doubt left alone; settings from `.editorconfig` and `.erb_lint.yml` (`server/src/erb_format.rs`, `format.rs`, `config.rs`; invariant tests in `format.rs`, corpus results in `docs/FORMATTER_PLAN.md`)
- [x] Range formatting, with `builtin` (`server/src/lsp.rs` `range_formatting`)
- [x] Formatting from the command line: `ruby-templates format [--check|--write]`, also on npm (`server/src/cli.rs`, `npm/`; `test/cli.js`, `test/npm.js`)
- [x] Configurable formatter executable plus `bundle exec` (`rubyTemplates.commands.htmlbeautifier` / `.erbFormat`, `rubyTemplates.preview.useBundler`)
- [x] Normalize spaces inside tags (`<%=foo%>` → `<%= foo %>`): code action for the tag, and source action `source.erbSpacing` for the file (`server/src/erb.rs` `spacing_edits`)
- [x] Format on save works (`editor.formatOnSave` via the provider)

## 8. Diagnostics and linting

- [x] Unclosed `<%` tag (`server/src/erb.rs`, tree-sitter-embedded-template)
- [x] Malformed tag / parse error node (`server/src/erb.rs`)
- [x] Block opened without `<% end %>` (`server/src/erb.rs` + `server/src/ruby.rs` `analyze`)
- [x] Stray `end` with no open block (`server/src/erb.rs`)
- [x] `else`/`elsif`/`when` outside a block (`server/src/erb.rs` `BlockEffect::Continue`)
- [x] `if`/`end` inside `<%= %>` warning (`server/src/erb.rs`)
- [x] Modifier `if`, `x = if … end`, `while … do`, keywords in strings/comments/symbol keys ignored (`server/src/ruby.rs` tests)
- [x] CLI `ruby-templates-ls --check`, exit 1 on errors (`server/src/lsp.rs`)
- [x] Unclosed or mismatched HTML tags (`<div>` … `</span>`), including tags that cross ERB block boundaries (`server/src/tags.rs`; stops where ERB branches disagree, skips JS block helpers, partials report mismatches only)
- [x] Ruby syntax errors inside tags (e.g. `<%= foo( %>`): tree-sitter-ruby over the tags joined like Erubi, offsets kept (`server/src/ruby_syntax.rs`); runs once tag and block structure is sound, first 3 errors. No false positives on the Forem / rubygems.org / Mastodon / diaspora corpus (1082 ERB views). A tag left open before the next `<%` is reported at its own opener (`unclosed-tag`)
- [x] `<%= %>` with a block helper missing `do` / `<% %>` on a block helper (`<% form_with do %>` renders nothing) (`server/src/erb.rs` `OUTPUT_BLOCK_HELPER`)
- [x] Duplicate HTML `id` attributes, `<img>`/`image_tag` without `alt`, `image_tag` for computed `src` (`server/src/html_lint.rs`); more HTML checks are catalogued in `docs/lint/ERB.md`
- [x] Unresolved `render "partial"` reported as a warning (`server/src/lsp.rs` `unresolved_renders`; only `dir/name` with an existing dir — bare names need controller lookup)
- [x] Strict locals: missing required local / unknown local passed to `render` (`server/src/lsp.rs` `strict_locals_mismatches`; one-line literal `dir/name` renders)
- [x] `raw` / `html_safe` / `<%==` security hints (`unsafe-output`, info, `server/src/html_lint.rs`)
- [x] erb_lint integration (`erb_lint --format json --stdin` on open and save, `rubyTemplates.erbLint.enabled`; `src/extension.js` `registerErbLint`, `src/erbLint.js`)
- [x] Herb (`@herb-tools/linter`) as an optional linter: `herb-lint --json` on open and save, the project's `node_modules/.bin` first (`rubyTemplates.herbLint.enabled`, `src/herbLint.js`)
- [x] Settings to enable/disable each built-in check (`rubyTemplates.erb.disabledDiagnostics`, codes in `server/src/erb.rs`/`tags.rs` `code`)
- [ ] Settings to pick external linters
- [x] Incremental text sync instead of `TextDocumentSyncKind::FULL` for large files (`server/src/util.rs` `apply_change`)
- [x] Debounce diagnostics on change (`server/src/lsp.rs` `publish_later`, 200 ms)

## 9. Navigation

- [x] Go to definition on `render "x"`, `render partial:`, `template:`, `layout:` (`server/src/partials.rs` `resolve`, `server/src/lsp.rs` `goto_definition`)
- [x] Partial lookup in the same dir, `app/views/<dir>`, `app/views/application`, and `app/views/layouts` for `layout:` (`server/src/partials.rs` `resolve`)
- [x] Go to `FooComponent` `.rb` and sidecar template from `render FooComponent.new` (`server/src/partials.rs` `resolve_component`)
- [x] Document links on render strings (`server/src/lsp.rs` `document_link`)
- [x] `render @user` / `render @users` → `users/_user` by model naming convention (`server/src/partials.rs` `resolve_model`; namespaced models skipped)
- [x] `render partial: name_var` (non-literal partial name): show nothing, don't guess (`RENDER_STR` only takes string literals)
- [ ] `render(...)` with `formats:`, `variants:`, `handlers:` picking the right file
- [ ] View paths outside `app/views` (engines, `prepend_view_path`, `app/components` templates)
- [x] Layout navigation: `layout "admin"` in controllers → `app/views/layouts/admin.html.erb` (`server/src/controllers.rs`)
- [x] Template → controller action (_Switch to Related File_, `src/related.js` `actionSource`) and controller action → template (`def show`, `render :edit`; `server/src/controllers.rs`)
- [x] Helper method → definition in `app/helpers/**` and controllers' `helper_method`s (`server/src/helper_defs.rs`); gem helpers stay with Ruby LSP
- [x] Locals: go to where a local is passed (`render "card", user: u`) from its use in the partial, else its strict-locals comment (`partial_rename::local_passes`)
- [x] Instance variables → assignment in the controller action or a `before_action` callback that runs for it (`server/src/ivars.rs`)
- [x] i18n key `t(".key")` / `t("a.b")` → `config/locales/*.yml` (lazy lookup scope from the view path) (`server/src/i18n.rs`)
- [x] Route helpers `*_path` / `*_url` → `config/routes.rb` (the route's source location, Rails 7.1+; `server/src/routes.rs`)
- [x] Asset paths in `image_tag` / `stylesheet_link_tag` → file (`server/src/assets.rs`)
- [x] Stimulus `data-controller="x"` → `app/javascript/controllers/x_controller.js` (`server/src/stimulus.rs`)
- [x] ViewComponent: component `.rb` ↔ template file switch command, sidecar folder included (`rubyTemplates.switchRelated`, `src/related.js`)
- [x] Find references: who renders this partial (workspace scan) (`server/src/lsp.rs` `references`, `partial_rename::references`)
- [x] Workspace symbol search for partials/templates by render name (`server/src/lsp.rs` `symbol`, `partials::named_templates`)

## 10. Completion

- [x] Partial names inside `render "…"` (`server/src/partials.rs` `partial_candidates`, `server/src/lsp.rs`)
- [x] Form builder methods after `f.` / `form.` / `ff.` / `builder.` / `fields.` (`server/src/helpers.rs` `FORM_BUILDER`)
- [x] ~78 Rails view helpers with snippet arguments, only inside `<% %>` (`server/src/helpers.rs` `HELPERS`, `server/src/lsp.rs` `in_ruby_context`)
- [x] `render template:` / `layout:` completion lists templates/layouts (today it always lists `_partials`) (`server/src/partials.rs` `render_candidates`; `render layout:` resolves to a partial like Rails)
- [x] Instance variables assigned in the matching controller action and its `before_action` callbacks (`only:` / `except:`, ApplicationController's too); every action's for a partial
- [x] Locals from `<%# locals: %>` or from callers' `render … locals:` (`server/src/lsp.rs` `local_completions`; callers' `render … locals:` still open)
- [ ] Block variables (`do |user|`) in scope
- [x] i18n keys from `config/locales` (`server/src/i18n.rs` `completion_prefix`)
- [x] Route helpers from the app's routes (`bin/rails runner src/routes.rb`, cached per app, reloaded when config/routes* change; `rubyTemplates.routes.enabled`)
- [x] Model attributes after `f.text_field :` (from `db/schema.rb`, model from the innermost `form_with model:` / `form_for` above) (`server/src/schema.rs`)
- [x] ViewComponent class names after `render ` (from `app/components/**/*_component.rb`, inserted as `Name.new(…)`)
- [x] Stimulus controller names in `data-controller` / `data-action` values and `data: { controller: }` hashes (`server/src/stimulus.rs` `completion_prefix`, `controllers`)
- [x] Stimulus action method and target names: methods after `id#` in `data-action`, `static targets` in `data-id-target` / `id_target:` (`stimulus.rs` `methods`, `targets`)
- [x] Turbo stream actions after `turbo_stream.` (`server/src/helpers.rs` `TURBO_STREAM`)
- [x] HTML tag/attribute completion in ERB: elements after `<`, the open element after `</`, attributes (with `data-*`/`aria-*`) in a start tag, outside ERB tags of HTML-host views (`server/src/erb_complete.rs`)
- [x] `in_ruby_context` handles tags spanning several lines (it only looks at the current line) (`server/src/erb.rs` `code_spans`/`span_at`)

## 11. Hover

- [x] Hover on render call lists resolved file(s) or "No matching template found" (`server/src/lsp.rs` `hover`)
- [x] Hover on a known Rails helper shows its signature (`server/src/lsp.rs`, `server/src/helpers.rs`)
- [x] Partial hover shows its strict locals signature (`server/src/lsp.rs` `render_hover`)
- [x] i18n key hover shows the translations (`server/src/i18n.rs`; missing locales listed)
- [x] Route helper hover shows verb + path, with every verb on that path and its controller#action
- [x] Hover on ERB tag punctuation explains the tag type (`<%==` = raw, `-%>` = trim) (`server/src/erb.rs` `tag_doc`)
- [x] Helper docs link to api.rubyonrails.org (`server/src/helpers.rs` `doc_url`)

## 12. Rename, references, symbols

- [x] Rename a partial file and update every `render "…"` that points at it (`server/src/partial_rename.rs`, `workspace/willRenameFiles`)
- [ ] Rename a local across a partial and its callers
- [x] Document symbols / outline: `content_for` blocks, forms, `turbo_frame_tag`, HTML landmarks with ids (`server/src/outline.rs` `document_symbols`)
- [x] Breadcrumbs follow the symbol tree (VS Code builds them from `textDocument/documentSymbol`)
- [x] Find all references to an i18n key: `t("…")` calls with the same full key (lazy `.x` keys resolved per view) plus its locale entries (`partial_rename::i18n_references`)

## 13. Embedded HTML/CSS/JS language features

- [ ] Request forwarding to VS Code's HTML language service for positions outside ERB tags (completion, hover, formatting)
- [ ] Same for CSS in `<style>` and JS in `<script>`
- [x] Emmet enabled by default (`emmet.includeLanguages` contributed via `configurationDefaults`; today it is only a README tip) (`package.json` `configurationDefaults`)
- [ ] HTML custom data (Stimulus/Turbo attributes) for attribute completion
- [ ] Ruby features inside tags delegated to Ruby LSP (README notes coexistence; no actual integration)

## 14. Rails-specific

- [x] Strict locals `<%# locals: (…) -%>`: highlight the comment as a signature, not a plain comment (`syntaxes/erb.tmLanguage.json` `erb-strict-locals`)
- [x] Strict locals: completion, hover, and diagnostics (see sections 8, 10, 11) (see items above)
- [x] ViewComponent slots: `with_*` after `c.` in a `render X.new do |c|` block, `header` / `header?` in the component's own template (`server/src/slots.rs`)
- [x] ViewComponent previews (`{test,spec}/components/previews`) ↔ component in _Switch to Related File_ (`src/related.js` `previewPartners`)
- [ ] Phlex/other component frameworks: explicitly out of scope, or supported
- [x] Mailer views (`app/views/*_mailer/*.text.erb` / `.html.erb` pairs) switch command (`rubyTemplates.switchRelated`: any view's other formats)
- [ ] Rails engines and `app/views` in gems found via `bundle show`
- [x] Jbuilder/Builder views get partial navigation (`docs/JBUILDER_TODO.md`, `TemplateLang::Ruby`)
- [x] Jbuilder/other templates in the same dir don't confuse partial resolution (`server/src/partials.rs` `renders_format`)
- [x] Hash-rocket `render :partial => "x"`, `:template =>`, `:layout =>` (`server/src/partials.rs` `RENDER_STR`)

## 15. Server and client infrastructure

- [x] Client starts the bundled per-platform binary, falls back to dev builds and PATH (`src/client.js` `findServer`)
- [x] `rubyTemplates.server.enabled` / `server.path` settings, restart command, restart on config change (`package.json`, `src/extension.js`)
- [x] Client watches `**/app/views/**` (`src/client.js`)
- [x] Server uses those watched-file events (no `did_change_watched_files` handler; partial list is rescanned per request) (`server/src/lsp.rs` `did_change_watched_files`, `stale`)
- [x] Cache the partial index per workspace instead of walking `app/views` on every completion (`server/src/lsp.rs` template/locale caches)
- [ ] Multi-root workspaces: resolve each file against its own Rails root
- [ ] Non-`file://` URIs (untitled, remote) degrade gracefully everywhere
- [ ] Windows paths in `resolve` / `partial_candidates` (separator handling only partly covered)

## 16. Tests and CI

- [x] Rust unit tests for render refs and block analysis (`server/src/partials.rs`, `server/src/ruby.rs`)
- [x] LSP end-to-end test: definition, hover, completion, diagnostics on a fake Rails tree (`test/e2e.js`)
- [x] Broken-file fixture (`test/broken/bad.html.erb`)
- [x] CI builds the server for 5 targets, runs `cargo test` on 3 (skipped for cross-built linux-arm64 and darwin-x64) and `test:tokens`, packages per-platform `.vsix` (`.github/workflows/build.yml`)
- [x] Unit tests for `erb.rs` diagnostics (each message, positions with multibyte text) (`server/src/erb.rs` tests)
- [x] `test/e2e.js` asserts results and exits non-zero on a mismatch
- [x] `test/e2e.js` runs in CI on linux-x64, darwin-arm64 and win32-x64 against each job's own build (`.github/workflows/build.yml`, `RUBY_TEMPLATES_LS`)
- [x] `test/broken/bad.html.erb` asserted like the Haml/Slim fixtures (`server/src/erb.rs` `broken_fixture`)
- [x] `test:server` (`--check` on samples) runs in CI (`.github/workflows/build.yml`)
- [ ] Token test asserts scopes for every tag variant (`<%_`, `<%==`, `-%>`, attribute/script contexts)
- [x] Extension-host tests (`@vscode/test-electron`): toggle, related-file switch, ERB formatter, erb_lint, server diagnostics and definition in a real VS Code (`test/vscode/`, `bun run test:vscode`, in CI)
- [x] Fixture corpus of real-world Rails views (no false-positive diagnostics): checked by hand on Forem and rubygems.org `app/views` (the fixes it led to are listed under the corpus item in `docs/HAML_TODO.md`); the remaining HTML tag warnings on Forem are real mismatches (`<h3>…</h2>`).

## 17. Docs

- [x] README covers features, commands, settings, development (`README.md`)
- [x] CHANGELOG (`CHANGELOG.md`)
- [ ] Per-feature screenshots/GIFs in the README
- [ ] Docs on file-type/host-language support once section 2 lands
- [ ] Docs on the trim modes and tag variants supported by grammar vs server
- [ ] Troubleshooting: conflicts with other ERB extensions (e.g. Ruby LSP, `aliariff.vscode-erb-beautify`, Shopify ERB grammar)
