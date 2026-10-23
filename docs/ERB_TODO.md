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
- [ ] Diagnostic: `%>` inside a Ruby string or `<%#` comment closes the tag early (ERB ends a tag at the first `%>`)
- [ ] Multi-line `<% ... %>` tags containing heredocs

## 2. File types and host languages

- [x] `.erb`, `.rhtml`, `.html.erb` map to the `erb` language id (`package.json` `contributes.languages`)
- [x] Server picks ERB from the language id, or from `.erb`/`.rhtml` when there is none (`server/src/main.rs` `TemplateLang::from_uri_or_id`)
- [x] `.js.erb` gets a JavaScript host grammar (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.css.erb` / `.scss.erb` get a CSS/SCSS host grammar (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.json.erb` gets a JSON host grammar (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.text.erb` / `.txt.erb` (mailer text parts) get a plain-text host grammar (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.xml.erb` / `.rss.erb` / `.atom.erb` / `.svg.erb` get an XML host grammar (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.md.erb` gets a Markdown host grammar (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.rb.erb` / `.yml.erb` (generators, `database.yml`) get Ruby/YAML host grammars (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)
- [x] `.turbo_stream.erb` gets the `erb` id and HTML host via the `.erb` suffix (`package.json`); Turbo snippets tracked separately
- [x] `.html+variant.erb` (e.g. `show.html+phone.erb`) recognized via the `.erb` suffix (`package.json`, `server/src/main.rs`); variant-aware lookup tracked under navigation
- [ ] `filenamePatterns: ["*.tt"]` for Rails generator templates (e.g. `Gemfile.tt`)
- [ ] Host-specific language ids (e.g. `erb-js`) or a host-detection grammar so `editor.*` settings and comment toggling follow the host
- [x] Server handles the same host variants (diagnostics shouldn't assume HTML) (`package.json` `erb-*` ids, `syntaxes/erb-*.tmLanguage.json`)

## 3. Grammar and highlighting

- [x] HTML host via `text.html.basic` with ERB injected everywhere outside ERB tags (`syntaxes/erb.tmLanguage.json` `injections`)
- [x] Embedded language mapping for ruby/js/css (`package.json` `grammars[erb].embeddedLanguages`)
- [x] Token snapshot script for the sample (`test/tokenize.js`, `test/sample.html.erb`)
- [ ] (?) ERB inside HTML attribute values (`class="<%= x %>"`): not in the sample
- [ ] (?) ERB inside `<script>` and `<style>` blocks: not in the sample
- [ ] (?) ERB inside HTML comments `<!-- <%= x %> -->`
- [ ] ERB in attribute-name position, e.g. `<div <%= attrs %>>`
- [ ] `test/tokenize.js` loads the real HTML/Ruby grammars instead of stubs so embedding is actually checked
- [ ] Token snapshot assertions (diff against a committed baseline) instead of a dump
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
- [ ] `onEnterRules` for `do |x| %>` and `else`/`elsif`/`when` lines (only `if/unless/...` are covered)
- [ ] `onEnterRules` that keep the cursor between `<% %>` on Enter
- [ ] Auto-close of `<%=` / `<%#` that doesn't double the ` %>` when the user types `=` right after `<%`
- [ ] Auto-close HTML tags (`</div>`) in `erb` files (VS Code's HTML auto-close only runs for `html`)
- [ ] Linked editing of HTML tag pairs
- [ ] Folding for `<% if %>…<% end %>` and HTML elements independent of indentation (server `foldingRange`)
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
- [ ] Strict locals magic comment `<%# locals: (name:, default: nil) -%>`
- [ ] `elsif`, `else`, `end`, `when` single-tag snippets
- [ ] `turbo_stream.*` action snippets for `.turbo_stream.erb`
- [ ] `render @collection` / `render partial:, locals:` / `render layout: do`
- [ ] `javascript_include_tag`, `favicon_link_tag`, `turbo_include_tags`, `asset_path`
- [ ] Mailer text-part skeleton snippet (`mail_to` is already a completion in `server/src/helpers.rs`)
- [ ] Snippets for ViewComponent templates (`content`, `helpers.`, slots `with_*`)

## 6. Commands and editing helpers

- [x] Toggle tag `<% %>` → `<%= %>` → `<%# %>`, multi-cursor (`src/extension.js` `toggleErbTag`, `package.json` keybinding)
- [x] Wrap selection in `<%= %>` (`src/extension.js` `wrapInErb`)
- [x] Preview rendered HTML via `erb -T -` (`src/extension.js` `renderToHtml`)
- [x] Show compiled Ruby via `erb -x` (`src/extension.js` `compileToRuby`)
- [x] Convert ERB → Haml / Slim through gems (`src/extension.js` `convert`)
- [ ] Toggle tag keeps `-%>` / `_%>` trim markers correctly for every variant (`<%-` maps to `<%=` and drops the `-`)
- [ ] Preview/compile with Erubi (Rails' engine) instead of stdlib `erb`, so `<%==` and Rails trim behave the same
- [ ] Preview inside a Rails app (`bin/rails runner` / `ApplicationController.render`) so helpers work
- [ ] Extract selection to partial (create `_name.html.erb`, replace with `render "name"`)
- [ ] Create a missing partial from an unresolved `render "x"` (code action)

## 7. Formatting

- [ ] Document formatting provider for ERB (Herb formatter, `htmlbeautifier`, or `erb-formatter`)
- [ ] Range formatting
- [ ] Configurable formatter executable plus `bundle exec` (reuse `rubyTemplates.preview.useBundler` pattern)
- [ ] Normalize spaces inside tags (`<%=foo%>` → `<%= foo %>`) as a code action or format option
- [ ] Format on save works (`editor.formatOnSave` via the provider)

## 8. Diagnostics and linting

- [x] Unclosed `<%` tag (`server/src/erb.rs`, tree-sitter-embedded-template)
- [x] Malformed tag / parse error node (`server/src/erb.rs`)
- [x] Block opened without `<% end %>` (`server/src/erb.rs` + `server/src/ruby.rs` `analyze`)
- [x] Stray `end` with no open block (`server/src/erb.rs`)
- [x] `else`/`elsif`/`when` outside a block (`server/src/erb.rs` `BlockEffect::Continue`)
- [x] `if`/`end` inside `<%= %>` warning (`server/src/erb.rs`)
- [x] Modifier `if`, `x = if … end`, `while … do`, keywords in strings/comments/symbol keys ignored (`server/src/ruby.rs` tests)
- [x] CLI `ruby-templates-ls --check`, exit 1 on errors (`server/src/main.rs`)
- [ ] Unclosed or mismatched HTML tags (`<div>` … `</span>`), including tags that cross ERB block boundaries
- [ ] Ruby syntax errors inside tags (e.g. `<%= foo( %>`): via Prism or tree-sitter-ruby over the compiled source
- [ ] `<%= %>` with a block helper missing `do` / `<% %>` on a block helper (`<% form_with do %>` renders nothing)
- [ ] Duplicate HTML `id` attributes and other HTML validity checks
- [ ] Unresolved `render "partial"` reported as a warning
- [ ] Strict locals: missing required local / unknown local passed to `render`
- [ ] `raw` / `html_safe` / `<%==` security hints (better-html / erb_lint `SafeErbTester` rules)
- [ ] erb_lint integration (run `erb_lint --format json` on save, map results to diagnostics)
- [ ] Herb (`herb` / `@herb-tools/linter`) integration as an alternative linter
- [ ] Settings to enable/disable each built-in check and pick external linters
- [ ] Incremental text sync instead of `TextDocumentSyncKind::FULL` for large files
- [ ] Debounce diagnostics on change

## 9. Navigation

- [x] Go to definition on `render "x"`, `render partial:`, `template:`, `layout:` (`server/src/partials.rs` `resolve`, `server/src/main.rs` `goto_definition`)
- [x] Partial lookup in the same dir, `app/views/<dir>`, `app/views/application`, and `app/views/layouts` for `layout:` (`server/src/partials.rs` `resolve`)
- [x] Go to `FooComponent` `.rb` and sidecar template from `render FooComponent.new` (`server/src/partials.rs` `resolve_component`)
- [x] Document links on render strings (`server/src/main.rs` `document_link`)
- [ ] `render @user` / `render @users` → `users/_user` by model naming convention
- [ ] `render partial: name_var` (non-literal partial name): show nothing, don't guess
- [ ] `render(...)` with `formats:`, `variants:`, `handlers:` picking the right file
- [ ] View paths outside `app/views` (engines, `prepend_view_path`, `app/components` templates)
- [ ] Layout navigation: `layout "admin"` in controllers → `app/views/layouts/admin.html.erb`
- [ ] Template → controller action and controller action → template
- [ ] Helper method → definition in `app/helpers/**` (and gem helpers through Ruby LSP)
- [ ] Locals: go to where a local is passed (`render … locals:`) from its use in the partial
- [ ] Instance variables → assignment in the controller action
- [ ] i18n key `t(".key")` / `t("a.b")` → `config/locales/*.yml` (lazy lookup scope from the view path)
- [ ] Route helpers `*_path` / `*_url` → `config/routes.rb`
- [ ] Asset paths in `image_tag` / `stylesheet_link_tag` → file
- [ ] Stimulus `data-controller="x"` → `app/javascript/controllers/x_controller.js`
- [ ] ViewComponent: component `.rb` ↔ template file switch command
- [ ] Find references: who renders this partial (workspace scan)
- [ ] Workspace symbol search for partials/templates

## 10. Completion

- [x] Partial names inside `render "…"` (`server/src/partials.rs` `partial_candidates`, `server/src/main.rs`)
- [x] Form builder methods after `f.` / `form.` / `ff.` / `builder.` / `fields.` (`server/src/helpers.rs` `FORM_BUILDER`)
- [x] ~78 Rails view helpers with snippet arguments, only inside `<% %>` (`server/src/helpers.rs` `HELPERS`, `server/src/main.rs` `in_ruby_context`)
- [ ] `render template:` / `layout:` completion lists templates/layouts (today it always lists `_partials`)
- [ ] Instance variables used in the matching controller action
- [ ] Locals from `<%# locals: %>` or from callers' `render … locals:`
- [ ] Block variables (`do |user|`) in scope
- [ ] i18n keys from `config/locales`
- [ ] Route helpers from `bin/rails routes` (cached)
- [ ] Model attributes after `f.text_field :` (from `db/schema.rb`)
- [ ] ViewComponent class names after `render `
- [ ] Stimulus controller / action / target names in `data-*` attributes
- [ ] Turbo stream actions after `turbo_stream.`
- [ ] HTML tag/attribute completion in ERB (VS Code's HTML language service, or `emmet`)
- [ ] `in_ruby_context` handles tags spanning several lines (it only looks at the current line)

## 11. Hover

- [x] Hover on render call lists resolved file(s) or "No matching template found" (`server/src/main.rs` `hover`)
- [x] Hover on a known Rails helper shows its signature (`server/src/main.rs`, `server/src/helpers.rs`)
- [ ] Partial hover shows its strict locals signature
- [ ] i18n key hover shows the translations
- [ ] Route helper hover shows verb + path
- [ ] Hover on ERB tag punctuation explains the tag type (`<%==` = raw, `-%>` = trim)
- [ ] Helper docs link to api.rubyonrails.org

## 12. Rename, references, symbols

- [ ] Rename a partial file and update every `render "…"` that points at it
- [ ] Rename a local across a partial and its callers
- [ ] Document symbols / outline: `content_for` blocks, forms, `turbo_frame_tag`, HTML landmarks with ids
- [ ] Breadcrumbs follow the symbol tree
- [ ] Find all references to an i18n key

## 13. Embedded HTML/CSS/JS language features

- [ ] Request forwarding to VS Code's HTML language service for positions outside ERB tags (completion, hover, formatting)
- [ ] Same for CSS in `<style>` and JS in `<script>`
- [ ] Emmet enabled by default (`emmet.includeLanguages` contributed via `configurationDefaults`; today it is only a README tip)
- [ ] HTML custom data (Stimulus/Turbo attributes) for attribute completion
- [ ] Ruby features inside tags delegated to Ruby LSP (README notes coexistence; no actual integration)

## 14. Rails-specific

- [ ] Strict locals `<%# locals: (…) -%>`: highlight the comment as a signature, not a plain comment
- [ ] Strict locals: completion, hover, and diagnostics (see sections 8, 10, 11)
- [ ] ViewComponent slots (`renders_one` / `renders_many` → `with_*` completion)
- [ ] ViewComponent previews (`spec/components/previews`) navigation
- [ ] Phlex/other component frameworks: explicitly out of scope, or supported
- [ ] Mailer views (`app/views/*_mailer/*.text.erb` / `.html.erb` pairs) switch command
- [ ] Rails engines and `app/views` in gems found via `bundle show`
- [ ] Jbuilder/other templates in the same dir don't confuse partial resolution

## 15. Server and client infrastructure

- [x] Client starts the bundled per-platform binary, falls back to dev builds and PATH (`src/client.js` `findServer`)
- [x] `rubyTemplates.server.enabled` / `server.path` settings, restart command, restart on config change (`package.json`, `src/extension.js`)
- [x] Client watches `**/app/views/**` (`src/client.js`)
- [ ] Server uses those watched-file events (no `did_change_watched_files` handler; partial list is rescanned per request)
- [ ] Cache the partial index per workspace instead of walking `app/views` on every completion
- [ ] Multi-root workspaces: resolve each file against its own Rails root
- [ ] Non-`file://` URIs (untitled, remote) degrade gracefully everywhere
- [ ] Windows paths in `resolve` / `partial_candidates` (separator handling only partly covered)

## 16. Tests and CI

- [x] Rust unit tests for render refs and block analysis (`server/src/partials.rs`, `server/src/ruby.rs`)
- [x] LSP end-to-end smoke test: definition, hover, completion, diagnostics on a fake Rails tree (`test/e2e.js`)
- [x] Broken-file fixture (`test/broken/bad.html.erb`)
- [x] CI builds the server for 5 targets, runs `cargo test` on 4 (skipped for cross-built linux-arm64) and `test:tokens`, packages per-platform `.vsix` (`.github/workflows/build.yml`)
- [ ] Unit tests for `erb.rs` diagnostics (each message, positions with multibyte text)
- [ ] `test/e2e.js` asserts results (it only prints them) and runs in CI
- [ ] `test:server` (`--check` on samples) runs in CI
- [ ] Token test asserts scopes for every tag variant (`<%_`, `<%==`, `-%>`, attribute/script contexts)
- [ ] Extension-host tests (`@vscode/test-electron`) for commands: toggle, wrap, convert
- [ ] Fixture corpus of real-world Rails views (no false-positive diagnostics)

## 17. Docs

- [x] README covers features, commands, settings, development (`README.md`)
- [x] CHANGELOG (`CHANGELOG.md`)
- [ ] Per-feature screenshots/GIFs in the README
- [ ] Docs on file-type/host-language support once section 2 lands
- [ ] Docs on the trim modes and tag variants supported by grammar vs server
- [ ] Troubleshooting: conflicts with other ERB extensions (e.g. Ruby LSP, `aliariff.vscode-erb-beautify`, Shopify ERB grammar)
