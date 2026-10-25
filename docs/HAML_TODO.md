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
- [x] Multi-line Ruby after a trailing `,`, including the last continuation line (`  class: "y"`) (`syntaxes/haml.tmLanguage.json` `inline-ruby-multiline`); the server joins it after a tag too (`server/src/haml_indent.rs` `join_continuation`)
- [x] Plain text ending in `,` or `|` (`Dear Sir,`) stays plain text (`syntaxes/haml.tmLanguage.json` `text`, `inline-text-multiline`; `test/sample.html.haml`)
- [x] `==` / `&==` / `!==` interpolated text (`%p== hi #{x}`, `== top #{x}`) is plain text with interpolation (`syntaxes/haml.tmLanguage.json`; server: `server/src/haml_indent.rs` `strip_output_prefix`)
- [x] Multiline `|`: trailing `|` is punctuation, except block arguments `do | x |` (`syntaxes/haml.tmLanguage.json` `multiline-pipe`, `punctuation.separator.continuation.multiline.haml`); the server groups `|` lines for plain text, tags and Ruby (`server/src/haml_indent.rs` `multiline_len`, `is_haml_multiline`)

## 2. Attributes

- [x] Ruby hash `{}` with Ruby 1.9 `key: v`, `"data-x": v`, and hashrocket `:a => v` (embedded `source.ruby`) (`syntaxes/haml.tmLanguage.json` `hash-attributes`)
- [x] Nested `data: {…}` hash (`test/sample.html.haml`)
- [x] Nested `aria: {…}` / `data: {…}` hashes keep the attribute hash open past their `}` (`syntaxes/haml.tmLanguage.json` `ruby-braces`, also in `(…)` values and `#{}`; `test/sample.html.haml`)
- [x] Multiline `{}` attribute hash spanning lines (`syntaxes/haml.tmLanguage.json` begin/end region; server joins lines in `server/src/haml_indent.rs` `skip_tag_head`)
- [x] HTML-style `(name="v" other=ruby_expr)` attributes (`syntaxes/haml.tmLanguage.json` `html-attributes`, `test/sample.html.haml`)
- [x] `#{}` inside double-quoted `()` attribute values (`syntaxes/haml.tmLanguage.json` `string-double`)
- [x] Boolean attributes in `()` without a value (`%input(checked type="x")`) (`syntaxes/haml.tmLanguage.json` `html-attributes`; `test/sample.html.haml`)
- [x] Object reference `[@user, :prefix]` (`syntaxes/haml.tmLanguage.json` `object-ref`)
- [x] Attribute methods `%div{attrs_hash}` and combined `{…}(…)` on one tag (`test/sample.html.haml`)
- [x] `class: [a, b]` / `id: [a, b]` arrays and `data: false`-style boolean values (`test/sample.html.haml`)

## 3. Filters

- [x] Embedded highlighting for `:javascript`, `:css`, `:ruby`, `:scss`, `:sass`, `:coffee`, `:markdown`, `:erb` (`syntaxes/haml.tmLanguage.json` `filters`)
- [x] `:plain`, `:escaped`, `:preserve`, `:cdata`, `:less` as plain text with `#{}` (`syntaxes/haml.tmLanguage.json` `filters`)
- [x] Embedded language mapping for ruby/js/css/scss/sass/coffee/markdown (`package.json` `grammars[haml].embeddedLanguages`)
- [x] Filters Haml 6+ doesn't register (`:textile`, `:maruku`, `:styl`, `:stylus`, `:builder`, `:nokogiri`, `:wiki`, `:creole`, `:mediawiki`) removed; `HAML_FILTERS` now matches `Haml::Filters` in 6.4.0 and 7.5.1, so they warn as unknown (`server/src/haml_indent.rs` `HAML_FILTERS`; `syntaxes/haml.tmLanguage.json` `filters`)
- [x] `#{}` interpolation highlighted inside `:javascript`, `:css`, `:markdown`, `:scss`, `:sass`, `:coffee`, `:less` (`syntaxes/haml.tmLanguage.json` `filters`)
- [x] `:less` embeds `source.css.less` (`syntaxes/haml.tmLanguage.json` `filters`; `package.json` `embeddedLanguages`)
- [x] `:coffeescript` alias for `:coffee` (`server/src/haml_indent.rs` `HAML_FILTERS`; `syntaxes/haml.tmLanguage.json` `filters`)
- [x] `text.html.erb` → `erb` in `embeddedLanguages` (`package.json` `grammars[haml].embeddedLanguages`)

## 4. File types

- [x] `.haml` / `.html.haml` map to the `haml` language id (`package.json` `contributes.languages`)
- [x] Server picks Haml from the language id, or from `.haml` when there is none (`server/src/lang.rs` `TemplateLang::from_uri_or_id`)
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
- [x] Convert Haml → ERB via the `haml_to_erb` gem's library through `ruby` (its CLI only converts in place) (`src/extension.js` `CONVERTERS['haml>erb']`)
- [x] Convert plain `.html` → Haml via `html2haml --no-erb` (`src/extension.js` `CONVERTERS['html>haml']`)
- [x] Extract selection to a partial (ERB/Haml/Slim), re-indenting the extracted block to column 0 (`src/extension.js` `extractPartial`)
- [x] Toggle `=` ↔ `-` ↔ `-#` on the current line, Ctrl/Cmd+Alt+E (`src/haml.js` `toggleHamlPrefix`)

## 8. Formatting

- [x] Document formatting provider: built-in re-indenter (in the language server) or `haml-lint`, chosen by `rubyTemplates.haml.formatter` (`src/extension.js`, `server/src/haml_format.rs`)
- [x] Re-indent a file or selection to the detected indent unit, trailing whitespace dropped on structural lines; files with errors other than indentation are left alone (`server/src/haml_format.rs`, range formatting in `server/src/lsp.rs`; compiled output checked in `bun run test:format`)
- [x] Formatting from the command line and npm: `ruby-templates format`, settings from `.editorconfig` and `.haml-lint.yml` (`server/src/cli.rs`, `config.rs`)
- [x] Configurable formatter executable plus `bundle exec` (`package.json` `rubyTemplates.commands.hamlLint`, `rubyTemplates.hamlLint.useBundler`)

## 9. Diagnostics and linting

- [x] Mixed tabs and spaces within one line's indentation; filter and comment bodies aren't checked (`server/src/haml_indent.rs` `visit_lines`)
- [x] Inconsistent indentation against the first indented line, with Haml's message (`server/src/haml_indent.rs` `check_structure`)
- [x] Dedent to a level that never existed (`server/src/haml_indent.rs`; `test/broken/bad.html.haml`)
- [x] Unexpected indentation at top level (`server/src/haml_indent.rs`)
- [x] Explicit `- end` rejected (`server/src/haml_indent.rs` `check_explicit_end`; `test/broken/bad.html.haml`)
- [x] Unclosed `{` `(` `[` in tag attributes, across lines (`server/src/haml_indent.rs` `skip_tag_head`; `test/broken/bad.html.haml`)
- [x] Unknown filter warning (`server/src/haml_indent.rs` `HAML_FILTERS`; `test/broken/bad.html.haml`)
- [x] Nothing nested under a doctype (`server/src/haml_indent.rs`)
- [x] Illegal nesting: content on a tag's line plus children, including `%p= form_with do |f|` (Haml 5–7 reject it); allowed only when the line's value ends in `,` (Haml's `is_ruby_multiline?`). Message names the tag as Haml does (`... as %p and nested within it.`) (`server/src/haml_indent.rs` `classify`, `is_ruby_multiline`; `test/broken/bad.html.haml`)
- [x] Illegal nesting under plain text (`server/src/haml_indent.rs` `LineKind::Text`)
- [x] Multiline `|` and trailing-`,` Ruby on line-start `-`/`=` lines joined before analysis (`server/src/haml_indent.rs` `join_continuation`)
- [x] `,`/`|` continuation after a tag (`%p= link_to "x",` + `  class: "y"`), and `|` multiline plain text, are not reported as nesting (`server/src/haml_indent.rs` `classify`, `multiline_len`; unit test `no_false_positives`)
- [x] Unresolved `render "dir/partial"` warning on `=`/`-`/`!=`/`&=` lines (`server/src/lsp.rs` `unresolved_renders`, unit test `unresolved_render_warnings`)
- [x] Line-start `~ code` is Ruby (`server/src/haml_indent.rs` `strip_output_prefix`)
- [x] `#{x}` at line start is plain text: nesting under it is reported as within plain text (`server/src/haml_indent.rs` `classify`)
- [x] Tabs: consistent tabs are accepted; a line indented with the other character than the first indented line gets Haml's `Inconsistent indentation: 1 tab used for indentation, but the rest of the document was indented using 2 spaces.` The reference is the first indented line the structure pass visits, so continuation lines (multi-line `{}`, `,`, `|`) don't count; indentation is spaces and tabs only, not other Unicode whitespace (`server/src/haml_indent.rs` `check_structure`, `human_indentation`; `server/src/util.rs` `leading_ws`)
- [x] `- else` / `- elsif` / `- when` with no open `if`/`case`/`unless` (`Got "else" with no preceding "if"`) or at the wrong level (`"else" is indented at wrong level: expected 0, but was at 1.`), following Haml's `@script_level_stack` (`server/src/haml_indent.rs` `track_block_keyword`)
- [ ] `- rescue` / `- ensure` / `- in` / a second `- else` out of place: Haml itself accepts them and only the generated Ruby fails to parse; belongs with the Ruby syntax check below
- [x] `- code` / `= code` / `~ code` with nested children but no block opener (`do`, `if`, …); Haml appends `end`, so Ruby fails with `unexpected 'end'` (`server/src/haml_indent.rs` `opens_block`)
- [x] Self-closing tag with content (`Self-closing tags can't have content.`) or children (`Illegal nesting: nesting within a self-closing tag is illegal.`) (`server/src/haml_indent.rs` `LineKind::SelfClosing`)
- [x] `/ comment text` and `/[if IE] text` with nested content (`Illegal nesting: nesting within a tag that already has content is illegal.`); `/[if IE]` and `/![if !IE]` alone may have children (`server/src/haml_indent.rs` `LineKind::TextComment`)
- [x] `#`/`.` with no name (`# heading`, `...more`): `Illegal element: classes and ids must have values.` (`server/src/haml_indent.rs` `classify`)
- [x] Indentation of `,` continuation lines: nothing to report. Haml 7.5 compiles `%p= link_to x,\n\t  y` and spaces-in-a-tab-file continuations without error (checked with the gem), so a check would be a false positive
- [x] Indentation inside filter bodies relative to the filter: nothing to report. Haml 7.5 accepts bodies that dedent below their first line or use spaces in a tab file (checked with the gem)
- [x] Unresolved `render` after a tag (`%div= render "x"`, `.a=`, `#b!=`, `%p&=`) or on a `~` / `&~` / `!~` line (`server/src/lsp.rs` `unresolved_renders`, `server/src/haml_indent.rs` `haml_code_start`; unit test `unresolved_render_warnings`)
- [ ] Ruby syntax errors in `-`/`=` lines and `{}` attribute hashes (Prism or tree-sitter-ruby over `haml compile` output). ERB has it now (`server/src/ruby_syntax.rs`); Haml and Slim need the implicit `end`s of indented blocks synthesized, with an offset map back to the template
- [x] haml-lint integration (`haml-lint --reporter json`, mapped to diagnostics; `*.haml.tt` generator templates skipped) (`src/extension.js`, `src/haml.js` `hamlLintOffenses`; `test/haml.js`, `bun run test:unit`)
- [x] Settings to enable/disable each built-in check: `rubyTemplates.haml.disabledDiagnostics`, codes in `server/src/haml_indent.rs` `code` (`server/src/lsp.rs` `Options`)

## 10. Completion

- [x] Partial names inside `render "…"` (`server/src/lsp.rs` `completion` is language-agnostic)
- [x] Form builder methods after `f.` (`server/src/lsp.rs` `completion`)
- [x] Rails helpers on Ruby lines (`server/src/lsp.rs` `in_ruby_context`)
- [x] Ruby context only in the code part: after `-`/`=`/`~`/`!=`/`&=` (line start or after a tag head), inside a tag's `{}` hash or `[]` object ref, or an open `#{`; not in `()` attributes, plain text, or after a Ruby `#` comment (`server/src/haml.rs` `in_ruby`, unit test `ruby_only_in_code`)
- [x] Completion and hover use each line's role: Haml completion and hover only on structural lines (never in filter or `-#` bodies or `|` text), Rails helpers also on continuation lines of `-`/`=` code and multi-line `{}` hashes and in `:ruby` bodies (`server/src/haml_indent.rs` `haml_line_roles`, unit test `line_roles`; `server/src/lsp.rs` `completion`, `hover`; `test/e2e.js`)
- [x] Attribute-name completion on continuation lines of a multi-line `{}` / `()` hash, `data: {` and `aria: {` included (`server/src/haml.rs` `continued_head`)
- [x] Filter names after `:` at line start (`server/src/haml.rs` `completion_context`, `server/src/haml_indent.rs` `HAML_FILTERS`)
- [x] `succeed` / `precede` / `surround` / `preserve` / `find_and_preserve` completion and hover, Haml files only (`server/src/helpers.rs` `HAML_HELPERS`, `server/src/lsp.rs` `helpers_for`)
- [x] Attribute-name completion inside `{}` (`name: `, quoted `"http-equiv": `, `data: {}` / `aria: {}`, and old-style `:name => `), inside nested `data: {…}` (Rails/Turbo/Stimulus keys) and `aria: {…}`, and inside `()` (`name=""`, `aria-*`, `data-*`) (`server/src/haml.rs` `completion_context`, `server/src/helpers.rs` `HTML_ATTRIBUTES`/`DATA_KEYS`/`ARIA_KEYS`; unit test `completion_contexts`)
- [x] Tag-name completion after `%` (`server/src/haml.rs` `completion_context`, `server/src/helpers.rs` `HTML_TAGS`); `%`, `:`, `{`, `(` added as trigger characters (`server/src/lsp.rs` `initialize`)

## 11. Hover

- [x] Hover on `render "x"` lists the resolved file(s) (`server/src/lsp.rs` `hover` is language-agnostic)
- [x] Hover on a known Rails helper shows its signature (`server/src/lsp.rs`, `server/src/helpers.rs`)
- [x] Hover on a filter name explains it (escaping, interpolation, required gem), checked against the Haml 6.4.0 filter sources (`server/src/haml.rs` `FILTER_DOCS`)
- [x] Hover on line prefixes (`-`, `-#`, `=`, `&=`, `!=`, `~`, `&~`, `!~`, `==`, `&==`, `!==`, `&`, `!`) and on `<` / `>` after a tag head; `&`/`!` only before a space or `#{`, as Haml reads them (`server/src/haml.rs` `hover`, unit test `hover_docs`)

## 12. Navigation, symbols, folding

- [x] Go to definition on `render "x"`, `render partial:`, `render FooComponent.new` (`server/src/lsp.rs` `goto_definition` is language-agnostic)
- [x] `.haml` partials found by resolution and completion (`server/src/partials.rs` `matching_templates` matches any extension)
- [x] Document links on render strings (`server/src/lsp.rs` `document_link`)
- [x] Folding by indentation: the server returns VS Code's off-side indentation ranges itself, since any provider answer replaces them (`server/src/outline.rs` `haml_folding_ranges`, `indentation_folds`)
- [x] Document symbols / outline: `content_for` blocks, forms, `turbo_frame_tag`, `render`, `#id` and landmark tags, with ERB's symbol kinds, nested by indentation; filter and comment bodies skipped (`server/src/outline.rs` `haml_document_symbols`, unit test `haml_symbols_nest_by_indentation`)
- [x] Outline: literal ids given as attributes (`%div{id: "x"}`, `{:id => "x"}`, `%div(id="x")`) on the tag's first line, top level only (`server/src/outline.rs` `haml_attribute_id`)
- [x] Folding for multiline `{}` attribute hashes, `|` blocks and `,` continuations whose lines aren't indented deeper, plus `-# region` markers (`server/src/outline.rs` `haml_folding_ranges`, `server/src/haml_indent.rs` `haml_structure`; unit test `haml_folds_indentation_regions_and_continuations`)
- [x] Rename a partial and update every `render` in ERB, Haml and Slim files under `app/views` (`workspace/willRenameFiles`). A render changes only when every file Rails could pick for it (the first resolved directory, all variants/formats) moves, all to one name; one edit per render however many files move; `render` inside Ruby strings, comments, template text and filter bodies is left alone (`server/src/partial_rename.rs`, with `render_in_code` from `server/src/lsp.rs` for Haml/ERB; unit tests `partial_rename_updates_renders`, `partial_rename_needs_every_candidate_to_move`; `test/e2e.js`; limits in `README.md`)
- [x] Rename a directory of partials, and `render partial:` strings in controllers/helpers (`.rb`) (`server/src/partial_rename.rs`, from the Slim work)
- [ ] Moves directly into `app/views`

## 13. Embedded languages

- [ ] Semantic tokens from the server (instance vars, locals, helpers)
- [ ] CSS/JS language features inside `:css` / `:javascript` filters

## 14. Rails-specific

- [x] Strict locals `-# locals: (…)` highlighted as a signature (`syntaxes/haml.tmLanguage.json` `strict-locals`)
- [x] Strict locals: completion and diagnostics (`server/src/lsp.rs` `local_completions`, `strict_locals_mismatches`, shared with ERB)
- [x] ViewComponent sidecar `.html.haml` templates (`server/src/partials.rs` `resolve_component`, unit test `component_haml_sidecar`; `test/e2e.js`)
- [x] `haml-rails` generator templates recognized as Haml: `lib/templates/haml/**/*.haml` by extension, `*.haml.tt` under `lib/templates/haml/` and `lib/generators/**/templates/` by `filenamePatterns` (`package.json` `contributes.languages[haml]`); structure diagnostics are skipped for `.tt`, whose column-0 ERB breaks Haml indentation (`server/src/lsp.rs` `diagnostics`, unit test `thor_templates_skip_haml_structure`)

## 15. Tests and CI

- [x] Token snapshot for the sample (`test/tokenize.js`, `test/sample.html.haml`, `test/__snapshots__/sample.html.haml.tokens.txt`)
- [x] `--check` on `test/sample.html.haml` runs in CI with no errors (`.github/workflows/build.yml`)
- [x] Broken-file fixture (`test/broken/bad.html.haml`)
- [x] Snippet lint (duplicate prefixes, stray `$`) covers `snippets/haml.json` (`test/snippets.js`, CI `test:snippets`)
- [x] Sample covers `<`/`>`, self-closing `/`, `[]`, `/[if IE]`, `&`/`!` lines, `|` multiline, multiline `{}`, `()` boolean attributes, every filter (`test/sample.html.haml`)
- [x] Rust unit tests for `server/src/haml_indent.rs`: each Haml message and its range, plus no-false-positive cases (`server/src/haml_indent.rs` `tests`)
- [x] `test/broken/bad.html.haml` asserted (expected diagnostics) via `include_str!` in `cargo test`, which CI and `bun run test:server` run (`server/src/haml_indent.rs` `broken_fixture`)
- [x] Haml file in `test/e2e.js`: definition, hover, completion (including filter bodies), diagnostics, symbols, folding, rename; every result is asserted and a mismatch exits non-zero (`test/e2e.js` `check`)
- [x] Fixture corpus of real-world Haml views (no false-positive diagnostics). Checked by hand with `--check` on the `app/views` of Mastodon, diaspora (Haml), Ruby Toolbox (Slim), Forem and rubygems.org (ERB): the false positives found (`=begin` blocks, multi-line `<% %>` with `if` on its own line, `key: if …`, gem view folders, `.rss`/`.atom`) are fixed with unit tests; the rest are real template bugs or app-registered formats (`:mobile`). The Slim formatter keeps the compiled output of all 86 Ruby Toolbox views, re-indented to 4 spaces first.

## 16. Docs

- [x] README lists Haml highlighting, diagnostics, commands and required gems (`README.md`)
- [x] CHANGELOG mentions Haml highlighting and diagnostics (`CHANGELOG.md`)
- [ ] Which Haml version the grammar and server target (Haml 5 vs 6/7 filters and helpers)
- [ ] Troubleshooting: conflicts with other Haml extensions (README has one line on picking a grammar)
