# Slim support: TODO

What full Slim support in this extension needs, based on an audit of the code at commit `ca15db5`.

Legend: `[x]` means implemented and checked in the code (the file is named next to it). `[ ]` means missing, partial, or not yet confirmed. `(?)` means it probably works but has no test.

## 1. Slim syntax: grammar coverage

- [x] `doctype html` / `doctype 5` / `doctype xml` (`syntaxes/slim.tmLanguage.json` `doctype`, `test/sample.html.slim`)
- [x] Tag names, including `ns:tag` (`syntaxes/slim.tmLanguage.json` `tag`)
- [x] `.class` / `#id` shorthand and implicit `div` (`syntaxes/slim.tmLanguage.json` `tag`, `test/sample.html.slim` `body.main#page`, `.user`)
- [x] Inline attributes `attr="string"` and `attr=ruby_expr` (`syntaxes/slim.tmLanguage.json` `attribute`, `test/sample.html.slim`)
- [x] Unescaped attributes `attr=="..."` (`syntaxes/slim.tmLanguage.json` `attribute` `(==?)`)
- [x] Wrapped attributes `()` and `[]` with boolean attributes (`syntaxes/slim.tmLanguage.json` `wrapped-attributes`, `test/sample.html.slim`)
- [ ] (?) Wrapped attributes `{}`: in the grammar (`wrapped-attributes`), not in the sample
- [ ] (?) Wrapped attributes spanning several lines (`a(href="/"` newline `class="x")`): not in the sample
- [x] Splat attributes `*{...}` and `*attrs` (`syntaxes/slim.tmLanguage.json` `splat`, `test/sample.html.slim`)
- [x] Inline tags `li: a href="/" Home` (`syntaxes/slim.tmLanguage.json` `inline-tag`, `test/sample.html.slim`)
- [ ] (?) Whitespace modifiers on tags `a>`, `a<`, `a<>`: in the grammar (`tag` `keyword.operator.whitespace`), not in the sample
- [ ] (?) Self-closing tag `img/`: in the grammar (`tag` `self-closing`), not in the sample
- [x] Control code `- ruby` (`syntaxes/slim.tmLanguage.json` `control-line`, `test/sample.html.slim`)
- [x] Output `= ruby` and unescaped `== ruby` (`syntaxes/slim.tmLanguage.json` `output-line`, `test/sample.html.slim`)
- [ ] (?) Output with whitespace modifiers `=<`, `=>`, `==<>`: in the grammar, not in the sample
- [ ] Legacy trailing-whitespace output `='` (deprecated but still parsed by Slim)
- [x] Output after a tag `p = expr` (`syntaxes/slim.tmLanguage.json` `inline-output`, `test/sample.html.slim`)
- [x] Inline text after a tag (`syntaxes/slim.tmLanguage.json` `inline-text`, `test/sample.html.slim`)
- [x] Text blocks `|` and `'` with nested continuation lines (`syntaxes/slim.tmLanguage.json` `text-block`, `test/sample.html.slim`)
- [ ] Text continuation lines indented under `p Some text` highlighted as text, not as tags
- [x] Code comment `/` and its indented block (`syntaxes/slim.tmLanguage.json` `code-comment`, `test/sample.html.slim`)
- [x] HTML comment `/!` (`syntaxes/slim.tmLanguage.json` `html-comment`, `test/sample.html.slim`)
- [ ] (?) IE conditional comment `/[if IE]`: in the grammar (`html-comment`), not in the sample
- [x] Interpolation `#{}` and escaped `\#{` in text, attributes and comments (`syntaxes/slim.tmLanguage.json` `interpolation`, `test/sample.html.slim`)
- [ ] Unescaped interpolation `#{{ expr }}` (grammar ends the interpolation at the first `}`)
- [ ] Multi-line Ruby after `,` or `\` stays Ruby on the next line (snapshot shows `class: "form" do |f|` scoped as a tag: `test/__snapshots__/sample.html.slim.tokens.txt`)
- [ ] Inline HTML lines starting with `<` (`<div class="x">`) highlighted as HTML, not plain text
- [ ] Attribute names with `@`/`:`/`.` (Alpine/Vue: `@click`, `:class`, `x-on:click.prevent`) in the sample
- [ ] Custom shortcuts (`Slim::Parser` `:shortcut`, e.g. `&text` → `input type="text"`, `@` → `role`)
- [ ] `include` directive (`slim/include`) highlighted as a keyword with its path
- [ ] Smart text (`slim/smart`) lines starting with `>` and the implicit text it allows
- [ ] Logic-less mode (`slim/logic_less`): `- section` / `= var` highlighted as dictionary access, not Ruby
- [ ] Translator plugin (`slim/translator`) needs nothing extra: confirm and note in docs
- [ ] `.skim` files: code lines are CoffeeScript, not Ruby (today `.skim` uses the Ruby grammar)

## 2. Embedded engines

- [x] `javascript:`, `css:`, `ruby:`, `scss:`, `sass:`, `coffee:`, `markdown:`, `erb:` highlighted with the target grammar (`syntaxes/slim.tmLanguage.json` `embedded`, `package.json` `embeddedLanguages`)
- [x] `textile:`, `rdoc:`, `less:`, `styl:`, `stylus:`, `builder:`, `nokogiri:`, `wiki:`, `creole:`, `mediawiki:`, `asciidoc:`, `org:` as plain text with interpolation (`syntaxes/slim.tmLanguage.json` `embedded`)
- [ ] `less:` and `stylus:` highlighted with their grammars when installed (they are plain text today)
- [ ] `typescript:`, `livescript:`, `babel:`, `haml:` engines: accepted by the server (`server/src/indented.rs` `SLIM_ENGINES`) but no grammar rule
- [ ] Embedded engines with attributes (`scss class="x":`, `javascript nonce=content_security_policy_nonce:`)
- [ ] (?) `#{}` interpolation inside `javascript:` / `css:` blocks highlighted as Ruby
- [ ] Token test loads real JS/CSS/Ruby grammars instead of stubs (`test/tokenize.js` shows `stub.source.*`), so embedding is actually checked

## 3. File types and language configuration

- [x] `.slim`, `.html.slim`, `.skim` map to the `slim` language id (`package.json` `contributes.languages`)
- [x] Server picks Slim from the `slim` id or the `.slim`/`.skim` extension (`server/src/main.rs` `TemplateLang::from_uri_or_id`)
- [x] Line comment toggle with `/` (`language-configuration/slim.json` `comments.lineComment`)
- [x] Brackets, auto-closing and surrounding pairs (`language-configuration/slim.json`)
- [x] Indentation-based folding (`language-configuration/slim.json` `folding.offSide`; server returns `None` so VS Code's fallback applies: `server/src/main.rs` `folding_range`)
- [x] Indent after tags, `- if/unless/...`, `do |x|`, engines, `/`, `|` lines; dedent on `- else/elsif/when/rescue/ensure/end` (`language-configuration/slim.json` `indentationRules`)
- [x] Word pattern includes `@ivar`, `$global`, `predicate?`, `kebab-case` (`language-configuration/slim.json` `wordPattern`)
- [x] Default `tabSize: 2`, spaces, no indent detection (`package.json` `configurationDefaults["[slim]"]`)
- [ ] `onEnterRules`: keep indent after a `,`/`\` continuation line and indent after a `|` text line
- [ ] (?) Emmet in Slim: VS Code enables Emmet for the `slim` id by default; not tested or documented here
- [ ] Host-specific Slim files (`.text.slim`, `.xml.slim`, `.js.slim`) recognized and not checked as HTML

## 4. Snippets

- [x] Control flow `if`, `ife`, `unless`, `each`, `eachi`, `case` (`snippets/slim.json`)
- [x] `doctype`, `div`, `tag`, `layout` skeleton (`snippets/slim.json`)
- [x] `render`, `renderc` (`snippets/slim.json`)
- [x] Links/forms: `link_to`, `link_tob`, `button_to`, `image_tag`, `form_with`, `ff`, `flf`, `fsubmit`, `fields_for` (`snippets/slim.json`)
- [x] `content_for`, `yield`, `turbo_frame`, `stimulus`, `t`, `flash`, `errors` (`snippets/slim.json`)
- [x] Engine blocks `js`, `css`, `ruby`, `md`, text `|`, HTML comment `/!` (`snippets/slim.json`)
- [x] Snippet files validated for duplicate prefixes and stray `$` (`test/snippets.js`, CI `npm run test:snippets`)
- [ ] Parity with ERB: `renderl`, `renderlayout`, `renderall`, `rendercomp`, `form_with_url`, `fselect`, `fcselect`, `cache`, `debug`, `content_tag`, `tag.`, `l`, `currency`, `pluralize`, `time_ago`, `meta_tags`, `assets`, `javascript_include_tag`, `favicon_link_tag`, `turbo_stream`, `ts-*`
- [ ] `elsif` / `else` / `when` single-line snippets
- [ ] Strict locals magic comment `/# locals: (name:, default: nil)` (Rails scans the raw source, so it works in Slim)
- [ ] `scss`, `sass`, `coffee`, `erb` engine block snippets and `'` text block
- [ ] Wrapped-attribute tag snippet `tag(attr="")` and splat `*{}`

## 5. Commands

- [x] Preview rendered HTML with `slimrb --pretty` (`src/extension.js` `renderToHtml`)
- [x] Show compiled Ruby with `slimrb --compile` (`src/extension.js` `compileToRuby`)
- [x] `bundle exec` when `rubyTemplates.preview.useBundler` and a Gemfile exist (`src/extension.js` `run`)
- [x] `rubyTemplates.commands.slimrb` setting (`package.json`)
- [x] Convert ERB → Slim (`erb2slim`) and Haml → Slim (`haml2slim`) (`src/extension.js` `convert`, `package.json` commands/menus)
- [ ] Convert Slim → ERB (`slimrb --erb`)
- [ ] Preview/compile with `slimrb --rails` so Rails-specific output (`form_with` capture, safe buffers) matches the app
- [ ] Preview inside a Rails app (`bin/rails runner` / `ApplicationController.render`) so helpers work
- [ ] Slim equivalent of Toggle ERB Tag: cycle a line between `-`, `=`, `==` and `/` (`toggleErbTag` is ERB-only in `package.json`)
- [ ] Wrap selection in `= ...` / `#{...}` for Slim
- [ ] Extract selection to partial (`_name.html.slim` + `= render "name"`)

## 6. Diagnostics (server)

- [x] Mixed tabs and spaces in indentation (`server/src/indented.rs` `diagnostics`)
- [x] Inconsistent indentation step (warning for Slim) (`server/src/indented.rs` `detect_unit`; checked with `--check` on a probe file)
- [x] Dedent to a level that never existed (`server/src/indented.rs`; `test/broken/bad.html.slim` line 11)
- [x] Unexpected indentation at top level (`server/src/indented.rs`)
- [x] Nothing nested under `doctype` (`server/src/indented.rs`; `test/broken/bad.html.slim` line 2)
- [x] Unclosed `(`/`[`/`{` in attributes, across lines (`server/src/indented.rs` `skip_tag_head`; `test/broken/bad.html.slim` line 5)
- [x] Explicit `- end` is an error (`server/src/indented.rs` `check_explicit_end`; `test/broken/bad.html.slim` line 8)
- [x] Unknown embedded engine warning (`server/src/indented.rs` `SLIM_ENGINES`; `test/broken/bad.html.slim` line 9)
- [x] `,` / `\` continuation lines joined before analysis (`server/src/indented.rs` `join_continuation`)
- [x] Unresolved `= render "dir/name"` warning (`server/src/main.rs` `unresolved_renders`; checked from a `.slim` document)
- [x] CLI `ruby-templates-ls --check file.slim` (`server/src/main.rs`, CI runs it on `test/sample.*`)
- [ ] Tabs counted as Slim does (`tabsize` default 4; the server counts a tab as 2 columns)
- [ ] `- else` / `- elsif` / `- when` without a matching `- if` / `- case` (not reported today)
- [ ] Ruby syntax errors in `-`/`=` lines and attribute values (e.g. `= foo(` is not reported)
- [ ] Children under a void element (`input`, `br`, `img/`) or under an inline-output tag that doesn't open a block
- [ ] `=` with a block helper but missing `do`, and `-` on an output block helper (`- form_with do` renders nothing)
- [ ] Unknown engine check accepts engines registered by the app (setting for extra engine names)
- [ ] slim-lint integration (`slim-lint --reporter json`, mapped to diagnostics)
- [ ] Strict locals: missing/unknown locals passed to a Slim partial
- [ ] Settings to enable/disable each built-in Slim check

## 7. Navigation, completion, hover

- [x] Go to definition on `= render "x"` and components from a Slim file (`server/src/main.rs` `goto_definition`; checked with an LSP probe on a `.slim` document)
- [x] Slim partials (`_x.html.slim`) found when rendering from any template (`server/src/partials.rs` `matching_templates`, `test/e2e.js` `_card.html.slim`)
- [x] Document links on render strings (`server/src/main.rs` `document_link`; checked on a `.slim` document)
- [x] Hover on render shows resolved files; hover on Rails helpers shows the signature (`server/src/main.rs` `hover`; checked on a `.slim` document)
- [x] Completion: partial names, form builder methods, Rails helpers on `-`/`=` lines (`server/src/main.rs` `completion`, `in_ruby_context`; checked on a `.slim` document)
- [ ] `in_ruby_context` for Slim: exact Ruby spans (attribute values, `#{}`, wrapped attributes) instead of the `"= "` / `(` / `{` heuristic
- [ ] Completion of embedded engine names at line start (`javascript:`, `css:`, ...)
- [ ] HTML tag and attribute completion in tag position
- [ ] Locals, instance variables, block variables, i18n keys, route helpers (same as ERB TODO sections 9-11)
- [ ] Hover on Slim line markers explaining them (`==` unescaped, `=>` trailing space, `/!` HTML comment)

## 8. Outline, folding, symbols, rename

- [ ] Document symbols / outline for Slim (`server/src/main.rs` `document_symbol` returns `None` for non-ERB)
- [ ] Breadcrumbs from the tag tree (`tag#id`, `content_for`, `form_with`, `turbo_frame_tag`)
- [ ] Server folding ranges for multi-line wrapped attributes and `,` continuations (indentation folding splits them)
- [ ] Rename a Slim partial and update every `render` pointing at it

## 9. Formatting and semantic tokens

- [ ] Document formatting provider for Slim (no mature standalone formatter; at least normalize indentation and attribute spacing)
- [ ] Format on save via the provider
- [ ] Semantic tokens from the server for Ruby spans the TextMate grammar can't delimit (continuations, nested brackets)

## 10. Tests

- [x] Token snapshot for the Slim sample (`test/sample.html.slim`, `test/__snapshots__/sample.html.slim.tokens.txt`, CI `npm run test:tokens`)
- [x] Broken-file fixture (`test/broken/bad.html.slim`)
- [x] Slim sample passes `--check` in CI (`.github/workflows/build.yml`)
- [ ] Unit tests in `server/src/indented.rs` (it has no `#[cfg(test)]` module), one per Slim diagnostic with positions
- [ ] Assert the expected diagnostics for `test/broken/bad.html.slim` (today nothing checks its output)
- [ ] `test/e2e.js` opens a `.slim` document (definition, hover, completion, diagnostics); today it only opens ERB
- [ ] Sample lines for every `(?)` item in sections 1 and 2
- [ ] Fixture corpus of real-world Slim views with no false-positive diagnostics

## 11. Docs

- [x] README lists Slim highlighting, diagnostics, commands, gems (`README.md`)
- [x] CHANGELOG mentions Slim (`CHANGELOG.md`)
- [ ] Document unsupported Slim options (custom shortcuts, logic-less, smart text, `.skim` CoffeeScript)
- [ ] Document which engines get full highlighting vs plain text
