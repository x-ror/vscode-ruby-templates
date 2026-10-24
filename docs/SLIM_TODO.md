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
- [x] Wrapped attributes `{}` (`syntaxes/slim.tmLanguage.json` `wrapped-attributes`, `test/sample.html.slim` `span{...}`)
- [x] Wrapped attributes spanning several lines (`syntaxes/slim.tmLanguage.json` `wrapped-attributes`, `test/sample.html.slim` `a(href="/"`)
- [x] Splat attributes `*{...}` and `*attrs` (`syntaxes/slim.tmLanguage.json` `splat`, `test/sample.html.slim`)
- [x] Inline tags `li: a href="/" Home` (`syntaxes/slim.tmLanguage.json` `inline-tag`, `test/sample.html.slim`)
- [x] Whitespace modifiers on bare tags `a>`, `a<`, `a<>` and legacy `a'`, also before wrapped attributes `a>(...)`, output `a>= x` and text `a>text`; a `'` after a quoted value is text, as in Slim (`p title='it''= x`) (`syntaxes/slim.tmLanguage.json` `tag`, `tag-line-with-text`, `test/sample.html.slim`)
- [x] Self-closing tag `img/` (`syntaxes/slim.tmLanguage.json` `tag`, `test/sample.html.slim`)
- [x] Control code `- ruby` (`syntaxes/slim.tmLanguage.json` `control-line`, `test/sample.html.slim`)
- [x] Output `= ruby` and unescaped `== ruby` (`syntaxes/slim.tmLanguage.json` `output-line`, `test/sample.html.slim`)
- [x] Output with whitespace modifiers `=<`, `=>`, `==<>` (`syntaxes/slim.tmLanguage.json` `output-line`, `test/sample.html.slim`)
- [x] Legacy trailing-whitespace output `='` after a tag (`p ='x'`; Slim 5 parses `'` as a modifier only there, a line-start `='x'` is Ruby `'x'`) (`syntaxes/slim.tmLanguage.json` `inline-output`, `test/sample.html.slim`)
- [x] Output after a tag `p = expr`, also attached `p= x`, `p=x`, `p==x`, `a(href="/")= x` (`syntaxes/slim.tmLanguage.json` `inline-output`, `test/sample.html.slim`)
- [x] Inline text after a tag (`syntaxes/slim.tmLanguage.json` `inline-text`, `test/sample.html.slim`)
- [x] Text blocks `|` and `'` with nested continuation lines, modifiers `|<`, `|>`, `'>` and `|text` without a space (`syntaxes/slim.tmLanguage.json` `text-block`, `test/sample.html.slim`)
- [x] Text continuation lines indented under `p Some text` / `li: a Home` highlighted as text, not as tags (Slim 5 `parse_text_block` makes every deeper line text) (`syntaxes/slim.tmLanguage.json` `tag-line-with-text`, `test/sample.html.slim`). Falls back to the old tag scoping when the tag head can't be matched on one line (unclosed wrapped attributes, nested brackets two levels deep)
- [x] Code comment `/` and its indented block (`syntaxes/slim.tmLanguage.json` `code-comment`, `test/sample.html.slim`)
- [x] HTML comment `/!` (`syntaxes/slim.tmLanguage.json` `html-comment`, `test/sample.html.slim`)
- [x] IE conditional comment `/[if IE]`: own rule, children use the top-level patterns (`syntaxes/slim.tmLanguage.json` `conditional-comment`, `test/sample.html.slim`)
- [x] Interpolation `#{}` and escaped `\#{` in text, double-quoted attributes and comments (`syntaxes/slim.tmLanguage.json` `interpolation`, `test/sample.html.slim`)
- [x] Unescaped interpolation `#{{ expr }}`, only when the whole body is `{...}` on the line (one level of inner braces, `#{{ {a: 1}.to_json }}`), as Slim's `Interpolation` checks; `#{{a: 1}.to_json}` stays escaped. Balanced braces inside `#{}` and `*{}` stay Ruby (`ruby-braces`) (`syntaxes/slim.tmLanguage.json` `interpolation-unescaped`, `test/sample.html.slim`)
- [x] Multi-line Ruby after `,` or `\` stays Ruby on the next line: the end lookbehind also rejects the `\n` vscode-textmate appends (`syntaxes/slim.tmLanguage.json` `control-line`, `output-line`, `inline-output`; `test/__snapshots__/sample.html.slim.tokens.txt` `class: "form" do |f|`). A `,` followed by trailing spaces still ends the line
- [x] Inline HTML lines starting with `<` highlighted as tags, attributes, strings and `#{}`; quotes count only inside `<...>` and never carry past the line (`<p>Don't</p>`) (`syntaxes/slim.tmLanguage.json` `inline-html`, `test/sample.html.slim`)
- [x] Single-quoted attribute values interpolate `#{}` (`syntaxes/slim.tmLanguage.json` `string-single`, `test/sample.html.slim`)
- [x] Attribute names with `@`/`:`/`.` (`@click`, `:class`, `x-on:click.prevent`) (`syntaxes/slim.tmLanguage.json` `attribute`, `test/sample.html.slim`)
- [x] Custom shortcuts (`:shortcut`): per-app runtime config, documented as unsupported (`README.md` Tips)
- [x] Custom `attr_list_delims` / `code_attr_delims`: per-app runtime config, documented as unsupported (`README.md` Tips)
- [x] `include` directive (`slim/include`) highlighted as a keyword with its path (`syntaxes/slim.tmLanguage.json` `include`, `test/sample.html.slim`)
- [x] Smart text (`slim/smart`): explicit `>` lines are text blocks (`syntaxes/slim.tmLanguage.json` `text-block`, `test/sample.html.slim`; without the plugin Slim rejects them, so this is safe). Implicit text depends on `implicit_text` and is documented as unsupported (`README.md` Tips)
- [x] Logic-less mode (`slim/logic_less`): enabled per app, documented as unsupported (`README.md` Tips)
- [x] Translator plugin (`slim/translator`) translates static text at compile time and adds no syntax; noted in `README.md` Tips
- [x] `.skim` files: own `skim` language and `text.skim` grammar, the Slim grammar with `source.coffee` outside engine blocks (`syntaxes/skim.tmLanguage.json`, generated and kept in sync by `test/tokenize.js`; `package.json`; `test/sample.skim`). Slim snippets are not registered for `skim`: they insert Ruby helpers

## 2. Embedded engines

- [x] `javascript:`, `css:`, `ruby:`, `scss:`, `sass:`, `coffee:`, `markdown:`, `erb:` highlighted with the target grammar (`syntaxes/slim.tmLanguage.json` `embedded`; `package.json` `embeddedLanguages` for all but `erb:`). Slim 5 itself registers only `asciidoc`, `markdown`, `textile`, `rdoc`, `coffee`, `sass`, `scss`, `javascript`, `css`, `ruby` (`slim/embedded.rb`); `erb:` and the plain-text engines below are kept for older Slim
- [x] `erb:` in `embeddedLanguages`: not a Slim 5 engine (removed in Slim 4), so not mapped; the existing highlighting rule stays for older Slim
- [x] `textile:`, `rdoc:`, `less:`, `styl:`, `stylus:`, `builder:`, `nokogiri:`, `wiki:`, `creole:`, `mediawiki:`, `asciidoc:`, `org:` as plain text with interpolation (`syntaxes/slim.tmLanguage.json` `embedded`)
- [x] `less:` / `stylus:` with their grammars: not Slim 5 engines, not supported; they stay plain text with `#{}` for older Slim (`README.md`)
- [x] `typescript:`, `livescript:`, `babel:`, `haml:`: not Slim 5 engines, not supported and no grammar rule (`README.md`). The server still accepts them (`server/src/slim_indent.rs` `SLIM_ENGINES`), see section 6
- [x] Embedded engines with attributes (`scss class="x":`, `javascript nonce=content_security_policy_nonce:`) (`syntaxes/slim.tmLanguage.json` `embedded`, `test/sample.html.slim`)
- [x] `#{}` inside `javascript:` / `css:` / `markdown:` blocks highlighted as Ruby, also inside the embedded grammar's strings (`syntaxes/slim.tmLanguage.json` `injections`, `test/sample.html.slim`; strings checked with a begin/end stub grammar, the snapshot stubs are single-token)
- [x] Token test loads real JS/CSS/Ruby grammars instead of stubs (`test/tokenize.js`, from the pinned `tm-grammars` dev dependency, so snapshots are the same on every machine)

## 3. File types and language configuration

- [x] `.slim`, `.html.slim` map to the `slim` language id, `.skim` to `skim` (`package.json` `contributes.languages`)
- [x] Server picks Slim from the `slim` id or `.slim`, Skim from the `skim` id or `.skim`; Skim gets the structure checks but not the Ruby ones (`unclosed-ruby`, `explicit-end`, `unmatched-branch`, `silent-output-helper`), and no Ruby context for helper completion or render warnings (`server/src/main.rs` `TemplateLang::from_uri_or_id`, `diagnostics`, `in_ruby_context`, tests `skim_skips_ruby_checks`, `unresolved_render_warnings`; `server/src/slim_indent.rs` `code::RUBY`)
- [x] Line comment toggle with `/` (`language-configuration/slim.json` `comments.lineComment`)
- [x] Brackets, auto-closing and surrounding pairs (`language-configuration/slim.json`)
- [x] Indentation-based folding (`language-configuration/slim.json` `folding.offSide`; server returns `None` so VS Code's fallback applies: `server/src/main.rs` `folding_range`)
- [x] Indent after tags, `- if/unless/...`, `do |x|`, engines (except `asciidoc:`, `org:`), `/`, `|` lines; dedent on `- else/elsif/when/rescue/ensure/end` (`language-configuration/slim.json` `indentationRules`)
- [x] Word pattern includes `@ivar`, `$global`, `predicate?`, `kebab-case` (`language-configuration/slim.json` `wordPattern`)
- [x] Default `tabSize: 2`, spaces, no indent detection (`package.json` `configurationDefaults["[slim]"]`)
- [x] `onEnterRules`: indent after a `|`/`'` text line, keep indent after a `-`/`=`/`tag =` line ending in `,`/`\` (`language-configuration/slim.json`, shared by `skim`; regexes checked in `test/slim.js`). The line after the last continuation line still keeps the continued indent
- [x] (?) Emmet in Slim: VS Code's Emmet lists `slim` among its native modes (Slim-syntax output); `skim` mapped to it (`package.json` `emmet.includeLanguages`), documented (`README.md` Tips). Not exercised in an editor here
- [x] Host-specific Slim files (`.text.slim`, `.xml.slim`, `.js.slim`) open as `slim` (`.slim` extension), and `void-children` only runs for HTML hosts, bare `.slim` and untitled (`server/src/main.rs` `slim_html_host`, `file_name` decodes `%2B`; test `void_children_only_for_html_slim`). Slim itself renders `img` children (`<img>x</img>`), so that is the only HTML-only Slim check; `doctype-children` is a Slim parse error in every host

## 4. Snippets

- [x] Control flow `if`, `ife`, `unless`, `each`, `eachi`, `case` (`snippets/slim.json`)
- [x] `doctype`, `div`, `tag`, `layout` skeleton (`snippets/slim.json`)
- [x] `render`, `renderc` (`snippets/slim.json`)
- [x] Links/forms: `link_to`, `link_tob`, `button_to`, `image_tag`, `form_with`, `ff`, `flf`, `fsubmit`, `fields_for` (`snippets/slim.json`)
- [x] `content_for`, `yield`, `turbo_frame`, `stimulus`, `t`, `flash`, `errors` (`snippets/slim.json`)
- [x] Engine blocks `js`, `css`, `ruby`, `md`, text `|`, HTML comment `/!` (`snippets/slim.json`)
- [x] Snippet files validated for duplicate prefixes and stray `$` (`test/snippets.js`, CI `bun run test:snippets`)
- [x] Parity with ERB: `renderl`, `renderlayout`, `renderall`, `rendercomp`, `form_with_url`, `fselect`, `fcselect`, `cache`, `debug`, `content_tag`, `tag.`, `l`, `currency`, `pluralize`, `time_ago`, `meta_tags`, `assets`, `javascript_include_tag`, `favicon_link_tag`, `turbo_stream`, `ts-*`, `asset_path`, `data-action`, `turbo_script` (`snippets/slim.json`; `erb` engine and ERB-only `pe`/`pc`/`pend` left out)
- [x] `elsif` / `else` / `when` single-line snippets (`snippets/slim.json` `elsif`, `else`, `when`)
- [x] Strict locals magic comment `/# locals: (name:, default: nil)` (Rails scans the raw source, so it works in Slim) (`snippets/slim.json` `locals`)
- [x] `scss`, `sass`, `coffee`, `erb` engine block snippets and `'` text block (`snippets/slim.json`; no `erb` snippet: Slim 5 has no `erb:` engine)
- [x] Wrapped-attribute tag snippet `tag(attr="")` and splat `*{}` (`snippets/slim.json` `tagw`, `splat`)

## 5. Commands

- [x] Preview rendered HTML with `slimrb --pretty` (`src/extension.js` `renderToHtml`)
- [x] Show compiled Ruby with `slimrb --compile` (`src/extension.js` `compileToRuby`)
- [x] `bundle exec` when `rubyTemplates.preview.useBundler` and a Gemfile exist (`src/extension.js` `run`)
- [x] `rubyTemplates.commands.slimrb` setting (`package.json`)
- [x] Convert ERB → Slim (`erb2slim`) and Haml → Slim (`haml2slim`) (`src/extension.js` `convert`, `package.json` commands/menus)
- [x] Convert Slim → ERB with `slimrb --erb` (Slim 5.2.2 `Slim::ERBConverter`) (`src/extension.js` `convert`, `package.json` `rubyTemplates.convertToErb`). Output keeps `::Temple::Utils.escape_html`, documented as double-escaping under Rails (`README.md`)
- [x] Compile with `slimrb --rails` behind `rubyTemplates.slim.compileForRails` (`src/extension.js` `compileToRuby`). `--rails` implies `--compile` in Slim 5.2.2, so it can't drive the HTML preview; the Rails preview below covers that
- [x] Preview inside a Rails app: `rubyTemplates.preview.useRails` runs `ruby bin/rails runner` with `render(inline: $stdin, type:)` from the nearest `bin/rails` on an `ApplicationController` subclass whose `controller_path` is the template's `app/views` directory (so bare `render "card"` resolves next to the file), 60 s timeout, errors in the preview panel (`src/extension.js` `renderInRails`; checked against a stub `bin/rails`, not a real app)
- [x] Toggle Slim Line: cycles `-` → `=` → `==` → `/` on every selected line, keeps `<`/`>` between `=` and `==`, skips `/!`, `/[`, `-#` and lines inside engine/comment/text blocks; a selection ending at column 0 leaves that line out (`src/slim.js` `toggleSlimMarker`, `inVerbatimBlock`, `src/extension.js` `toggleSlimTag`, `Ctrl+Alt+E` for `slim`; `test/slim.js`). Inline `p = x` markers are not toggled
- [x] Wrap selection for Slim: a whole plain line's content becomes `= x`; part of a line, or a line starting with `-`/`=`/`|`/`'`/`/`/`<`/`>`, becomes `#{x}`; empty selection inserts `#{}` (`src/slim.js` `wrapSlim`, `src/extension.js` `wrapInSlim`, `Ctrl+Alt+W` for `slim`). Multi-line selections are skipped with a message
- [x] Extract selection to partial: whole selected lines, dedented by their common whitespace prefix, to `_name.<format>.slim` (the file's format without `+variant`, else `html`) next to the file (or under `app/views/dir/` for `dir/name`), replaced by `= render "name"` in one `WorkspaceEdit`; refuses to overwrite and reports a failed edit (`src/slim.js` `dedent`, `partialPath`, `src/extension.js` `extractSlimPartial`; `test/slim.js`). ERB has no extract command to mirror

## 6. Diagnostics (server)

- [x] Mixed tabs and spaces in indentation (`server/src/slim_indent.rs` `diagnostics`)
- [x] Mixed tabs and spaces reported as an error for Slim, though Slim expands tabs and accepts it (downgrade or drop): now Haml-only (`server/src/haml_indent.rs`)
- [x] Inconsistent indentation step (warning for Slim) (`server/src/slim_indent.rs` `detect_unit`; checked with `--check` on a probe file)
- [x] Dedent to a level that never existed (`server/src/slim_indent.rs`; `test/broken/bad.html.slim` line 11)
- [x] Unexpected indentation at top level: not an error for Slim, which accepts it (Haml rejects it)
- [x] Nothing nested under `doctype` (`server/src/slim_indent.rs`; `test/broken/bad.html.slim` line 2)
- [x] Unclosed `(`/`[`/`{` in attributes, across lines (`server/src/slim_indent.rs` `skip_tag_head`; `test/broken/bad.html.slim` line 5)
- [x] Explicit `- end` is an error (`server/src/slim_indent.rs` `check_explicit_end`; `test/broken/bad.html.slim` line 8)
- [x] Unknown embedded engine warning (`server/src/slim_indent.rs` `SLIM_ENGINES`; `test/broken/bad.html.slim` line 9)
- [x] `,` / `\` continuation lines joined before analysis (`server/src/slim_indent.rs` `join_continuation`)
- [x] Unresolved `= render "dir/name"` warning (`server/src/main.rs` `unresolved_renders`; checked from a `.slim` document)
- [x] CLI `ruby-templates-ls --check file.slim` (`server/src/main.rs`, CI runs it on `test/sample.*`)
- [x] Tabs counted as Slim does (`tabsize` default 4; the server counts a tab as 2 columns) (`server/src/slim_indent.rs` `indent_width`, tab stops of 4 for Slim)
- [x] `- else` / `- elsif` / `- when` without a matching `- if` / `- case` (not reported today) (`server/src/slim_indent.rs` `unmatched-branch`)
- [x] Ruby syntax errors in `-`/`=` lines and attribute values (e.g. `= foo(` is not reported): bracket/quote balance only, as a warning; no Ruby parser (`server/src/slim_indent.rs` `unclosed-ruby`)
- [x] Children under a void element (`input`, `br`, `img/`) or under an inline-output tag that doesn't open a block: void elements and `tag/` (warning, HTML hosts only: `server/src/slim_indent.rs` `void-children`, `server/src/main.rs` `slim_html_host`); inline-output tags with children are valid Slim (DoInserter adds `do`), so not reported
- [x] `=` with a block helper but missing `do`, and `-` on an output block helper (`- form_with do` renders nothing): `- helper do` warns (`server/src/slim_indent.rs` `silent-output-helper`); `= helper` without `do` is valid
- [x] Unknown engine check accepts engines registered by the app (setting for extra engine names) (`rubyTemplates.slim.extraEngines`, `src/client.js` `initializationOptions.slim`)
- [x] `SLIM_ENGINES` lists engines Slim 5 no longer registers (`erb`, `less`, `styl`, `stylus`, `builder`, `nokogiri`, `wiki`, `creole`, `mediawiki`, `org`, `typescript`, `livescript`, `babel`, `haml`), so their typos and removals go unreported (`server/src/slim_indent.rs`): trimmed to Slim 5's list (`server/src/slim_indent.rs` `SLIM_ENGINES`)
- [x] slim-lint integration (`slim-lint --reporter json`, mapped to diagnostics) (`src/extension.js` `registerSlimLint`, `rubyTemplates.slimLint.enabled`)
- [x] Strict locals: missing/unknown locals passed to a Slim partial (`server/src/main.rs` `strict_locals_mismatches`, language-independent)
- [x] Settings to enable/disable each built-in Slim check (`rubyTemplates.slim.disabledDiagnostics`, codes in `server/src/slim_indent.rs` `code`)

## 7. Navigation, completion, hover

- [x] Go to definition on `= render "x"` and components from a Slim file (`server/src/main.rs` `goto_definition`; checked with an LSP probe on a `.slim` document)
- [x] Slim partials (`_x.html.slim`) found when rendering from any template (`server/src/partials.rs` `matching_templates`, `test/e2e.js` `_card.html.slim`)
- [x] Document links on render strings (`server/src/main.rs` `document_link`; checked on a `.slim` document)
- [x] Hover on render shows resolved files; hover on Rails helpers shows the signature (`server/src/main.rs` `hover`; checked on a `.slim` document)
- [x] Completion: partial names, form builder methods, Rails helpers on `-`/`=` lines (`server/src/main.rs` `completion`, `in_ruby_context`; checked on a `.slim` document)
- [x] Exact Ruby spans for Slim completion and unresolved-render warnings: `-`/`=`/`==` (also `p= x`), `attr=ruby` in inline and wrapped attributes, splats, `#{}` in text, quoted values and interpolating engines, `ruby:` bodies, `,`/`\` continuations; Skim never counts as Ruby (`server/src/slim_parser.rs` `parse`, `in_ruby`, a port of `Slim::Parser`; `server/src/main.rs` `completion`, `unresolved_renders`). An unclosed `#{` before the cursor counts as Ruby for completion only (`in_open_interpolation`)
- [x] Completion of embedded engine names (Slim 5 engines plus `rubyTemplates.slim.extraEngines`) where a tag starts (`server/src/slim_complete.rs`, `test/e2e.js`)
- [x] HTML tag and attribute completion where a tag or attribute name may start (`server/src/slim_complete.rs`, `server/src/html.rs`; a modest static list, ERB has none to reuse)
- [ ] Locals, instance variables, block variables, i18n keys, route helpers (same as ERB TODO sections 9-11). Nothing to port: ERB has none of these yet
- [x] Hover on Slim line markers: `-`, `=`, `==` with `<`/`>`, `/`, `/!`, `/[if]`, `|`, `'`, engines (`server/src/slim_hover.rs`, `test/e2e.js`)

## 8. Outline, folding, symbols, rename

- [x] Document symbols / outline for Slim (`server/src/main.rs` `document_symbol` returns `None` for non-ERB): done (`server/src/slim_outline.rs`)
- [x] Breadcrumbs from the tag tree (`tag#id`, `content_for`, `form_with`, `turbo_frame_tag`) (`server/src/slim_outline.rs` hierarchical symbols)
- [x] Server folding ranges for multi-line wrapped attributes and `,` continuations (indentation folding splits them) (`server/src/slim_outline.rs` logical lines)
- [x] Rename a Slim partial and update every `render` pointing at it (`server/src/partial_rename.rs`, `workspace/willRenameFiles`; also folder renames and qualified refs in `.rb` files)

## 9. Formatting and semantic tokens

- [x] Document formatting provider for Slim/Skim: indentation to the tab size in spaces, blocks and continuations moved with their line, trailing whitespace stripped only where Slim ignores it; files Slim rejects and `insertSpaces: false` are left alone (`server/src/slim_format.rs`; `test/slim_format.js` compiles `test/format/*.slim` with the slim gem before and after). Attribute spacing is not normalized: `p a = b` is an attribute but `b = c` is output, so the rewrite isn't safe without more parsing than it is worth
- [x] Format on save via the provider (`README.md`; registered dynamically for slim/skim only, `server/src/main.rs` `initialized`)
- [ ] Semantic tokens from the server for Ruby spans the TextMate grammar can't delimit (continuations, nested brackets). Skipped: there is no semantic tokens provider, and one token type per span would override the TextMate Ruby colours instead of adding to them; useful tokens need a Ruby lexer. `slim_parser::parse` already has the spans

## 10. Tests

- [x] Token snapshot for the Slim sample (`test/sample.html.slim`, `test/__snapshots__/sample.html.slim.tokens.txt`, CI `bun run test:tokens`)
- [x] Broken-file fixture (`test/broken/bad.html.slim`)
- [x] Slim sample passes `--check` in CI (`.github/workflows/build.yml`)
- [x] Unit tests in `server/src/slim_indent.rs` (it has no `#[cfg(test)]` module), one per Slim diagnostic with positions (`server/src/slim_indent.rs` `mod tests`)
- [x] Assert the expected diagnostics for `test/broken/bad.html.slim` (today nothing checks its output) (`server/src/slim_indent.rs` `broken_fixture`)
- [x] `test/e2e.js` opens `.slim` documents: diagnostics, definition, hover, completion, formatting (`test/e2e.js`)
- [x] Slim formatter keeps the slim gem's compiled output, and is idempotent (`test/slim_format.js`, `test/format/`, `bun run test:format`)
- [x] Sample lines for every `(?)` item in sections 1 and 2 (`test/sample.html.slim`, parsed with Slim 5.2.2 except the plugin-only `include` and `>` lines)
- [x] Fixture corpus of real-world Slim views with no false-positive diagnostics. Checked by hand with `--check` on the `app/views` of Mastodon, diaspora (Haml), Ruby Toolbox (Slim), Forem and rubygems.org (ERB): the false positives found (`=begin` blocks, multi-line `<% %>` with `if` on its own line, `key: if …`, gem view folders, `.rss`/`.atom`) are fixed with unit tests; the rest are real template bugs or app-registered formats (`:mobile`). The Slim formatter keeps the compiled output of all 86 Ruby Toolbox views, re-indented to 4 spaces first.

## 11. Docs

- [x] README lists Slim highlighting, diagnostics, commands, gems (`README.md`)
- [x] CHANGELOG mentions Slim (`CHANGELOG.md`)
- [x] Document unsupported Slim options (custom shortcuts, attribute delimiters, logic-less, implicit smart text) (`README.md` Tips; `.skim` is now supported)
- [x] Document which engines get full highlighting vs plain text (`README.md` Features)
