# Haml support: TODO

What full Haml support in this extension needs, based on an audit of the code at commit `ca15db5`. Language reference: [haml.info](https://haml.info/docs/yardoc/file.REFERENCE.html) (Haml 6/7; latest gem checked: 7.5.1).

Legend: `[x]` means implemented and checked in the code (the file is named next to it). `[ ]` means missing, partial, or not yet confirmed. `(?)` means it probably works but has no test.

Rails-wide features that don't depend on the template language (i18n, routes, helpers, controllers, Stimulus) are tracked once in `docs/ERB_TODO.md`; this file only lists what is Haml-specific or needs Haml-specific work.

## 1. Line syntax

- [x] Doctype `!!!`, `!!! 5`, `!!! XML`, `!!! Strict` (`syntaxes/haml.tmLanguage.json` `doctype`)
- [x] `%tag`, including namespaced `%fb:login` (`syntaxes/haml.tmLanguage.json` `tag-line`)
- [x] `.class` / `#id` shorthand, chained, and implicit `div` (`syntaxes/haml.tmLanguage.json` `tag-line`)
- [x] Self-closing `/` after a tag (`syntaxes/haml.tmLanguage.json` `punctuation.definition.tag.self-closing.haml`)
- [x] Whitespace removal `>` / `<` / `<>` after a tag (`syntaxes/haml.tmLanguage.json` `keyword.operator.whitespace.haml`)
- [x] `- code` silent script (`syntaxes/haml.tmLanguage.json` `ruby-line`)
- [x] `= code`, `!= code`, `&= code`, `~ code` at line start and after a tag (`syntaxes/haml.tmLanguage.json` `ruby-line`, `inline-ruby`)
- [x] `& text` / `! text` (escaped / unescaped plain text with interpolation): `&`/`!` scoped as punctuation (`syntaxes/haml.tmLanguage.json` `text`, `punctuation.definition.text.haml`)
- [x] `%p& text` / `%p! text`: the `&`/`!` after a tag is punctuation (`syntaxes/haml.tmLanguage.json` `tag-line`)
- [x] `\` escape at line start (`\= not code`) (`syntaxes/haml.tmLanguage.json` `plain-line`, `test/sample.html.haml`)
- [x] `#{}` interpolation in plain text and inline text, `\#{` escape (`syntaxes/haml.tmLanguage.json` `interpolation`, `escape`)
- [x] `-#` silent comment, including its nested block (`syntaxes/haml.tmLanguage.json` `silent-comment`)
- [x] `/` HTML comment with nested content, `/[if IE]` conditional comment (`syntaxes/haml.tmLanguage.json` `html-comment`)
- [x] Bare `- else` / `- ensure` / `- end` scoped as `keyword.control`; keywords followed by an expression (`- elsif x`, `- when x`, `- rescue Foo`) get their scope from `source.ruby` (`syntaxes/haml.tmLanguage.json` `ruby-line`)
- [x] Multi-line Ruby after a trailing `,`, including the last continuation line (`  class: "y"`) (`syntaxes/haml.tmLanguage.json` `inline-ruby-multiline`); the server joins it after a tag too (`server/src/indented.rs` `join_continuation`)
- [x] Plain text ending in `,` or `|` (`Dear Sir,`) stays plain text (`syntaxes/haml.tmLanguage.json` `text`, `inline-text-multiline`; `test/sample.html.haml`)
- [x] `==` / `&==` / `!==` interpolated text (`%p== hi #{x}`, `== top #{x}`) is plain text with interpolation (`syntaxes/haml.tmLanguage.json`; server: `server/src/indented.rs` `strip_output_prefix`)
- [x] Multiline `|`: trailing `|` is punctuation, except block arguments `do | x |` (`syntaxes/haml.tmLanguage.json` `multiline-pipe`, `punctuation.separator.continuation.multiline.haml`); the server groups `|` lines for plain text, tags and Ruby (`server/src/indented.rs` `multiline_len`, `is_haml_multiline`)

## 2. Attributes

- [x] Ruby hash `{}` with Ruby 1.9 `key: v`, `"data-x": v`, and hashrocket `:a => v` (embedded `source.ruby`) (`syntaxes/haml.tmLanguage.json` `hash-attributes`)
- [x] Nested `data: {…}` hash (`test/sample.html.haml`)
- [ ] (?) Nested `aria: {…}` hash: same rule as `data:`, not in the sample
- [x] Multiline `{}` attribute hash spanning lines (`syntaxes/haml.tmLanguage.json` begin/end region; server joins lines in `server/src/indented.rs` `skip_tag_head`)
- [x] HTML-style `(name="v" other=ruby_expr)` attributes (`syntaxes/haml.tmLanguage.json` `html-attributes`, `test/sample.html.haml`)
- [x] `#{}` inside double-quoted `()` attribute values (`syntaxes/haml.tmLanguage.json` `string-double`)
- [x] Boolean attributes in `()` without a value (`%input(checked type="x")`) (`syntaxes/haml.tmLanguage.json` `html-attributes`; `test/sample.html.haml`)
- [x] Object reference `[@user, :prefix]` (`syntaxes/haml.tmLanguage.json` `object-ref`)
- [ ] (?) Attribute methods `%div{attrs_hash}` and combined `{…}(…)` on one tag: plain Ruby, not in the sample
- [ ] (?) `class: [a, b]` / `id: [a, b]` arrays and `data: false`-style boolean values: plain Ruby, not in the sample

## 3. Filters

- [x] Embedded highlighting for `:javascript`, `:css`, `:ruby`, `:scss`, `:sass`, `:coffee`, `:markdown`, `:erb` (`syntaxes/haml.tmLanguage.json` `filters`)
- [x] `:plain`, `:escaped`, `:preserve`, `:cdata`, `:less` as plain text with `#{}` (`syntaxes/haml.tmLanguage.json` `filters`)
- [x] Embedded language mapping for ruby/js/css/scss/sass/coffee/markdown (`package.json` `grammars[haml].embeddedLanguages`)
- [x] Filters Haml 6+ doesn't register (`:textile`, `:maruku`, `:styl`, `:stylus`, `:builder`, `:nokogiri`, `:wiki`, `:creole`, `:mediawiki`) removed; `HAML_FILTERS` now matches `Haml::Filters` in 6.4.0 and 7.5.1, so they warn as unknown (`server/src/indented.rs` `HAML_FILTERS`; `syntaxes/haml.tmLanguage.json` `filters`)
- [x] `#{}` interpolation highlighted inside `:javascript`, `:css`, `:markdown`, `:scss`, `:sass`, `:coffee`, `:less` (`syntaxes/haml.tmLanguage.json` `filters`)
- [x] `:less` embeds `source.css.less` (`syntaxes/haml.tmLanguage.json` `filters`; `package.json` `embeddedLanguages`)
- [x] `:coffeescript` alias for `:coffee` (`server/src/indented.rs` `HAML_FILTERS`; `syntaxes/haml.tmLanguage.json` `filters`)
- [x] `text.html.erb` → `erb` in `embeddedLanguages` (`package.json` `grammars[haml].embeddedLanguages`)

## 4. File types

- [x] `.haml` / `.html.haml` map to the `haml` language id (`package.json` `contributes.languages`)
- [x] Server picks Haml from the language id, or from `.haml` when there is none (`server/src/main.rs` `TemplateLang::from_uri_or_id`)
- [x] Variants (`show.html+phone.haml`) and other formats (`.xml.haml`, `.text.haml`) get the `haml` id via the `.haml` suffix (`package.json`)

## 5. Language configuration

- [x] Line comment toggle `-#` (`language-configuration/haml.json` `comments.lineComment`)
- [x] Brackets, auto-closing and surrounding pairs for `{}` `[]` `()` and quotes (`language-configuration/haml.json`)
- [x] Indent after `- if/else/when/...`, line-start `-`/`=`/`~ … do |x|`, a filter, or an empty `/` / `-#`; dedent on `- else/elsif/when/rescue/ensure` (`language-configuration/haml.json` `indentationRules`)
- [x] Indentation rules: no indent after a tag with inline content (`%p text`, `%a(href="/x") Things to do`, `%a{href: x}= foo`, and `%div= form_with do |f|`, which Haml rejects with children); indent after line-start `= foo do |f|` / `!= foo do |x|`; the patterns stay linear on long attribute lists (`language-configuration/haml.json` `indentationRules`)
- [x] Word pattern includes `@ivar`, `$global`, `predicate?` (`language-configuration/haml.json` `wordPattern`)
- [x] Default `tabSize: 2`, spaces, no indent detection (`package.json` `configurationDefaults["[haml]"]`)
- [x] `onEnterRules`: keep indent after `|` multiline lines (not `do | x |`) and trailing-`,` Ruby lines, with `=`/`~` right after the tag head (`language-configuration/haml.json` `onEnterRules`)
- [x] `-# region` / `-# endregion` folding markers (`language-configuration/haml.json` `folding.markers`); no `#region`, which is a Haml `#id` div

## 6. Snippets

- [x] Control flow `if`, `ife`, `unless`, `each`, `eachi`, `case` (`snippets/haml.json`)
- [x] `!!!`, `div`, `tag`, `layout` skeleton (`snippets/haml.json`)
- [x] `render`, `renderc`, `link_to`, `link_tob`, `button_to`, `image_tag`, `form_with`, `ff`, `flf`, `fsubmit`, `fields_for` (`snippets/haml.json`)
- [x] `content_for`, `yield`, `turbo_frame`, `stimulus`, `t`, `flash`, `errors` (`snippets/haml.json`)
- [x] Filters `js`, `css`, `ruby`, `md`; silent comment `-#` (`snippets/haml.json`)
- [x] Filters `:plain`, `:erb`, `:scss`, `:preserve` (`snippets/haml.json`)
- [x] `succeed` / `precede` / `surround` helper snippets (`snippets/haml.json`)
- [x] `turbo_stream.*` action snippets for `.turbo_stream.haml` (`snippets/haml.json` `ts-*`)
- [x] Single-line `- elsif` / `- else` / `- when` snippets (`snippets/haml.json`)

## 7. Commands and editing helpers

- [x] Preview rendered HTML on Haml 6/7 via `haml render -`; falls back to the Haml < 6 CLI only when it opened `render` as a file (`ENOENT`), and treats Thor's exit-0 usage errors (stderr, no stdout) as failures (`src/extension.js` `runHaml`; checked against Haml 5.2.2, 6.4.0, 7.5.1)
- [x] Show compiled Ruby on Haml 6/7 via `haml compile -`, Haml < 6 via `haml --debug` (`src/extension.js` `runHaml`, `compileToRuby`)
- [x] Convert ERB → Haml via `html2haml --erb` (`src/extension.js` `convert`)
- [x] Convert Haml → Slim via `haml2slim` (`src/extension.js` `convert`)
- [ ] Convert Haml → ERB (no converter wired up; e.g. `haml2erb`)
- [ ] Convert plain `.html` → Haml (`html2haml` without `--erb`)
- [ ] Extract selection to a `_partial.html.haml`, re-indenting the extracted block to column 0
- [ ] Toggle `=` ↔ `-` ↔ `-#` on the current line (the Haml counterpart of the ERB tag toggle)

## 8. Formatting

- [ ] Document formatting provider (`haml-lint --auto-correct`, or a built-in re-indenter)
- [ ] Re-indent a file or selection to the detected indent unit
- [ ] Configurable formatter executable plus `bundle exec`

## 9. Diagnostics and linting

- [x] Mixed tabs and spaces within one line's indentation; filter and comment bodies aren't checked (`server/src/indented.rs` `visit_lines`)
- [x] Inconsistent indentation step against the file's detected unit (`server/src/indented.rs` `detect_unit`; `test/broken/bad.html.haml`)
- [x] Dedent to a level that never existed (`server/src/indented.rs`; `test/broken/bad.html.haml`)
- [x] Unexpected indentation at top level (`server/src/indented.rs`)
- [x] Explicit `- end` rejected (`server/src/indented.rs` `check_explicit_end`; `test/broken/bad.html.haml`)
- [x] Unclosed `{` `(` `[` in tag attributes, across lines (`server/src/indented.rs` `skip_tag_head`; `test/broken/bad.html.haml`)
- [x] Unknown filter warning (`server/src/indented.rs` `HAML_FILTERS`; `test/broken/bad.html.haml`)
- [x] Nothing nested under a doctype (`server/src/indented.rs`)
- [x] Illegal nesting: content on a tag's line plus children, including `%p= form_with do |f|` (Haml 5–7 reject it); allowed only when the line's value ends in `,` (Haml's `is_ruby_multiline?`). Message names the tag as Haml does (`... as %p and nested within it.`) (`server/src/indented.rs` `classify`, `is_ruby_multiline`; `test/broken/bad.html.haml`)
- [x] Illegal nesting under plain text (`server/src/indented.rs` `LineKind::Text`)
- [x] Multiline `|` and trailing-`,` Ruby on line-start `-`/`=` lines joined before analysis (`server/src/indented.rs` `join_continuation`)
- [x] `,`/`|` continuation after a tag (`%p= link_to "x",` + `  class: "y"`), and `|` multiline plain text, are not reported as nesting (`server/src/indented.rs` `classify`, `multiline_len`; unit test `no_false_positives`)
- [x] Unresolved `render "dir/partial"` warning on `=`/`-`/`!=`/`&=` lines (`server/src/main.rs` `unresolved_renders`, unit test `unresolved_render_warnings`)
- [x] Line-start `~ code` is Ruby (`server/src/indented.rs` `strip_output_prefix`)
- [x] `#{x}` at line start is plain text: nesting under it is reported as within plain text (`server/src/indented.rs` `classify`)
- [x] Tabs: consistent tabs are accepted; a line indented with the other character than the first indented line gets Haml's `Inconsistent indentation: 1 tab used for indentation, but the rest of the document was indented using 2 spaces.` The reference is the first indented line the structure pass visits, so continuation lines (multi-line `{}`, `,`, `|`) don't count; indentation is spaces and tabs only, not other Unicode whitespace (`server/src/indented.rs` `check_structure`, `human_indentation`; `server/src/util.rs` `leading_ws`)
- [x] `- else` / `- elsif` / `- when` with no open `if`/`case`/`unless` (`Got "else" with no preceding "if"`) or at the wrong level (`"else" is indented at wrong level: expected 0, but was at 1.`), following Haml's `@script_level_stack` (`server/src/indented.rs` `track_block_keyword`)
- [ ] `- rescue` / `- ensure` / `- in` / a second `- else` out of place: Haml itself accepts them and only the generated Ruby fails to parse; belongs with the Ruby syntax check below
- [x] `- code` / `= code` / `~ code` with nested children but no block opener (`do`, `if`, …); Haml appends `end`, so Ruby fails with `unexpected 'end'` (`server/src/indented.rs` `opens_block`)
- [x] Self-closing tag with content (`Self-closing tags can't have content.`) or children (`Illegal nesting: nesting within a self-closing tag is illegal.`) (`server/src/indented.rs` `LineKind::SelfClosing`)
- [x] `/ comment text` and `/[if IE] text` with nested content (`Illegal nesting: nesting within a tag that already has content is illegal.`); `/[if IE]` and `/![if !IE]` alone may have children (`server/src/indented.rs` `LineKind::TextComment`)
- [x] `#`/`.` with no name (`# heading`, `...more`): `Illegal element: classes and ids must have values.` (`server/src/indented.rs` `classify`)
- [ ] Indentation of `,` continuation lines (Haml checks them: `"\t    " used for indentation` in a tab file); continuation lines are skipped today
- [ ] Indentation inside filter bodies relative to the filter (Haml requires the body to start with the filter's indent plus one unit); bodies are skipped today
- [x] Unresolved `render` after a tag (`%div= render "x"`, `.a=`, `#b!=`, `%p&=`) or on a `~` / `&~` / `!~` line (`server/src/main.rs` `unresolved_renders`, `server/src/indented.rs` `haml_code_start`; unit test `unresolved_render_warnings`)
- [ ] Ruby syntax errors in `-`/`=` lines and `{}` attribute hashes (Prism or tree-sitter-ruby over `haml compile` output)
- [ ] haml-lint integration (`haml-lint --reporter json` on save, mapped to diagnostics)
- [ ] Settings to enable/disable each built-in check

## 10. Completion

- [x] Partial names inside `render "…"` (`server/src/main.rs` `completion` is language-agnostic)
- [x] Form builder methods after `f.` (`server/src/main.rs` `completion`)
- [x] Rails helpers on Ruby lines (`server/src/main.rs` `in_ruby_context`)
- [ ] `in_ruby_context` false positives: any `(` or `{` earlier on the line counts as Ruby, so plain text like `%p see (link_` offers helpers
- [ ] Filter names after `:` at line start (from `HAML_FILTERS`)
- [ ] `succeed` / `precede` / `surround` / `preserve` in the helper list (`server/src/helpers.rs`)
- [ ] Attribute-name completion inside `{}` / `()` (HTML attributes, `data:`/`aria:` keys)
- [ ] Tag-name completion after `%`

## 11. Hover

- [x] Hover on `render "x"` lists the resolved file(s) (`server/src/main.rs` `hover` is language-agnostic)
- [x] Hover on a known Rails helper shows its signature (`server/src/main.rs`, `server/src/helpers.rs`)
- [ ] Hover on a filter name explains it (escaping, interpolation, required gem)
- [ ] Hover on line prefixes (`&=`, `!=`, `~`, `<`, `>`) explains escaping / whitespace behaviour

## 12. Navigation, symbols, folding

- [x] Go to definition on `render "x"`, `render partial:`, `render FooComponent.new` (`server/src/main.rs` `goto_definition` is language-agnostic)
- [x] `.haml` partials found by resolution and completion (`server/src/partials.rs` `matching_templates` matches any extension)
- [x] Document links on render strings (`server/src/main.rs` `document_link`)
- [x] Folding by indentation (server returns `None` for Haml so VS Code's `offSide` folding applies) (`server/src/main.rs` `folding_range`)
- [ ] Document symbols / outline for Haml: `content_for` blocks, forms, `turbo_frame_tag`, `#id` tags (`server/src/main.rs` `document_symbol` returns `None` for non-ERB)
- [ ] Folding for multiline `{}` attribute hashes and `|` blocks, which indentation folding misses
- [ ] Rename a partial and update every `render` in `.haml` files

## 13. Embedded languages

- [ ] Semantic tokens from the server (instance vars, locals, helpers)
- [ ] CSS/JS language features inside `:css` / `:javascript` filters

## 14. Rails-specific

- [x] Strict locals `-# locals: (…)` highlighted as a signature (`syntaxes/haml.tmLanguage.json` `strict-locals`)
- [ ] Strict locals: completion and diagnostics as in ERB
- [ ] ViewComponent sidecar `.html.haml` templates (resolution already matches any extension; not tested)
- [ ] `haml-rails` generator templates (`lib/templates/haml/**/*.haml`) recognized as Haml

## 15. Tests and CI

- [x] Token snapshot for the sample (`test/tokenize.js`, `test/sample.html.haml`, `test/__snapshots__/sample.html.haml.tokens.txt`)
- [x] `--check` on `test/sample.html.haml` runs in CI with no errors (`.github/workflows/build.yml`)
- [x] Broken-file fixture (`test/broken/bad.html.haml`)
- [x] Snippet lint (duplicate prefixes, stray `$`) covers `snippets/haml.json` (`test/snippets.js`, CI `test:snippets`)
- [x] Sample covers `<`/`>`, self-closing `/`, `[]`, `/[if IE]`, `&`/`!` lines, `|` multiline, multiline `{}`, `()` boolean attributes, every filter (`test/sample.html.haml`)
- [x] Rust unit tests for `server/src/indented.rs`: each Haml message and its range, plus no-false-positive cases (`server/src/indented.rs` `tests`)
- [x] `test/broken/bad.html.haml` asserted (expected diagnostics) via `include_str!` in `cargo test`, which CI and `npm run test:server` run (`server/src/indented.rs` `broken_fixture`)
- [ ] Haml file in `test/e2e.js` (definition, hover, completion, diagnostics)
- [ ] Fixture corpus of real-world Haml views (no false-positive diagnostics)

## 16. Docs

- [x] README lists Haml highlighting, diagnostics, commands and required gems (`README.md`)
- [x] CHANGELOG mentions Haml highlighting and diagnostics (`CHANGELOG.md`)
- [ ] Which Haml version the grammar and server target (Haml 5 vs 6/7 filters and helpers)
- [ ] Troubleshooting: conflicts with other Haml extensions (README has one line on picking a grammar)
