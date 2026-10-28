# Changelog

## Unreleased

### Language server

- **Breaking:** Format Document and format on save now re-indent HTML ERB views with the built-in formatter. Set `rubyTemplates.erb.format.enabled: false` to keep the old behaviour.
- `rubyTemplates.erb|haml|slim.diagnostics.enabled` and `rubyTemplates.erb|haml|slim.format.enabled` (both on by default) switch a language's diagnostics and the built-in formatter off; Skim follows `slim`. The `ruby-templates` command line is unaffected.
- Find references, file renames and the definition of a partial's local scan the app on their own thread, so hover, completion and diagnostics answer while they run (a hover behind references on 5000 templates: 0.3 ms instead of 130 ms).
- Rails checks follow the project: the framework and gem versions come from the nearest `Gemfile.lock`. `unresolved-render` runs only when it lists `actionview`, `strict-locals` only with `actionview` 7.1 or newer; Sinatra projects and folders without a lockfile get neither. Diagnostics refresh when `Gemfile.lock` changes.
- Both messages are Rails' own: `Missing partial users/_card` / `Missing template users/show`, and one strict locals error per render (`missing local: :b`, `unknown keywords: :c, :d`, `no locals accepted`, with ` for app/views/…` from Rails 7.2).
- Formats an app registers in `config/initializers` (`Mime::Type.register`, `register_alias`) count as formats in template names, so `_card.mobile.erb` is the partial `card`.
- Slim errors are Slim 5.2.2's own: the first error its parser raises (`Malformed indentation`, `Unexpected indentation`, `Expected closing delimiter )`, `Unknown line indicator`, `Illegal shortcut`, `Expected tag`, …) at Slim's line and column, Temple's `Multiple id attributes specified` and `Invalid doctype`, and `Explicit end statements are forbidden`. `action_path('[')` in an attribute is no longer an unclosed bracket.
- Slim's Ruby is checked with Prism (Ruby's own parser) as Slim compiles it, under the new code `ruby-syntax`, with Ruby's messages.
- **Breaking:** Slim's `inconsistent-indent` and `silent-output-helper` are gone (Slim accepts what they reported), and `unclosed-ruby` and `unmatched-branch` are folded into `ruby-syntax`; their entries in `rubyTemplates.slim.disabledDiagnostics` are ignored. Smart text (`> text`, `slim/smart`) is reported as an unknown line indicator, as Slim's default parser does.
- ERB's Ruby is checked with Prism as Rails compiles it: the server builds the method source Erubi 1.13.1 generates with Rails 8.1.4's options (`trim: true`, `@output_buffer`, the `locals:` magic comment as the method's parameters) and reports every error Prism gives, with Ruby's messages, at the template line it comes from (exactly, when the error is in a tag's code), under `ruby-syntax`. It runs on every template, whatever else is wrong with it.
- **Breaking:** ERB's `stray-end`, `unmatched-branch`, `unclosed-block`, `output-control-flow` and `unterminated-string`, and the "missing `%>` before the next `<%`" case of `unclosed-tag`, are folded into `ruby-syntax` (they were guesses at what Ruby rejects; `<%= if x %>` is now ``expected `then` or `;` or '\n'`` and the `<% end %>` closing it a second error); their entries in `rubyTemplates.erb.disabledDiagnostics` are ignored.
- ERB's `unclosed-tag` (a `<%` with no `%>` after it) is a warning, not an error, and only for HTML templates: Erubi leaves it as text and Rails compiles it, but the HTML parser rejects `<` followed by `%` (`invalid-first-character-of-tag-name`, HTML 13.2.5.6), which the message names. It is found with Erubi's own scan, so `<% a <% b %>` (a Ruby error) is not reported twice, and `.text.erb` or `.js.erb` templates get none.
- **Breaking:** ERB's `malformed-tag`, `silent-output-helper`, `prefer-image-tag` and `unsafe-output` are gone (Erubi has no malformed state and accepts `<% form_with … do %>`, `<%==`, `raw` and a computed `<img src>`); their entries in `rubyTemplates.erb.disabledDiagnostics` are ignored.
- ERB's HTML checks cite the HTML standard in their messages, e.g. "Unexpected end tag `</span>`: no open element of that name (HTML 13.2.6.4.7)", "End tag `</div>` doesn't close the open `<span>` (line 2) (HTML 13.2.6.4.7)", "`<div>` is never closed (HTML 13.2.6.4.7)", and for `img-missing-alt` and `duplicate-id` the standard's own sentence (4.8.4.4.1, 3.2.6). `unexpected-close-tag` now also reports the end tag of a void element ("End tag `</br>` for a void element (HTML 13.1.2)") and a `</p>` after a block start tag closed the `p` (`<p>a<div>b</div></p>`; not checked inside `button`, `object`, `template`, `table` cells, SVG/MathML or where ERB branches disagree); `<caption>` may omit its end tag, and `<img generator-unable-to-provide-required-alt="">` is not reported, as the standard says. The ERB formatter leaves a file with such an error alone.
- ERB strict locals must be keywords, as Rails 7.1+ requires: `<%# locals: (a, b:) -%>` is an error with Rails' own message ("`a` set as non-keyword argument for app/views/users/_card.html.erb. Locals can only be set as keyword arguments.") at the comment's line, under `ruby-syntax`, once the Ruby is valid and only when the `Gemfile.lock` lists `actionview` 7.1 or newer (otherwise, as in Rails before 7.1, the `locals:` line is a plain comment and its parameters are not parsed).
- The Ruby checks of ERB and Slim parse with the project's Ruby, as Rails evals the template: the version comes from the nearest `.ruby-version`, else the `RUBY VERSION` of the nearest `Gemfile.lock`, and the latest grammar applies when there is none (`<% x = a\n  && b %>` is an error on Ruby 3.3 and 3.4 only).
- Haml indentation errors are Haml 7.5.1's own, at the line it reports: `Indentation can't use both tabs and spaces.`, `Inconsistent indentation: …`, `The line was indented 2 levels deeper than the previous line.`, `Indenting at the beginning of the document is illegal.`; only the first is reported.
- Haml nesting errors are reported on the nested line, as Haml does, with its messages (`Illegal nesting: content can't be both given on the same line as %p and nested within it.`, `…nesting within plain text is illegal.`, `…a self-closing tag…`, `…a tag that already has content…`), and a doctype with children gets Haml's `…nesting within a header command is illegal.`; `Illegal element: classes and ids must have values.` follows Haml's rule (`.`/`#` followed by nothing, `.` or `#`), and `#$x`/`#@x` lines are plain text.
- Haml attribute errors are Haml's own: `Unbalanced brackets.` for a `{` (balanced as Haml lexes the Ruby: strings, comments, `%w()`, regexps and `?}` don't count) or `[` left open, and `Invalid attribute list: "(foo 'bar')".` for a bad `(…)` list, under `unclosed-attribute`; `%p(a=1` is accepted, as in Haml.
- Haml's `Invalid tag: "% a".`, `There's no Ruby code for = to evaluate.` (bare `=`, `~`, `&=`, `%p=`, a multi-line hash before `=`) and `You don't need to use "- end" in Haml. Un-indent to close a block:` (with Haml's example) are reported as Haml raises them; the first two under the new code `syntax-error`, the last under `explicit-end`. `- x; def f; end` is accepted, as in Haml.
- Haml filters: `Invalid filter name ":java-script".` and `FilterCompiler for 'nope' was not found` (Haml raises the latter at compile, so a parser error in the file comes first; names are case-insensitive, `:maruku` and `:textile` are not filters in 7.5.1). The unknown filter is now an error, not a warning.
- Haml's `- else`/`elsif`/`when` placement follows Haml's own bookkeeping (`Got "else" with no preceding "if"`, `"else" is indented at wrong level: expected 0, but was at 1.`), and content in a void element (`%br` with children or text, which Haml compiles to `<br>x</br>`) gets the warning `void-children`, a new Haml code.
- **Breaking:** Haml's "Nested content under Ruby code that doesn't open a block" is gone (Haml compiles `= x` with children); it was reported under `illegal-nesting`.
- **Breaking:** Haml's `bad-dedent` is gone (Haml reports a dedent to a level that never existed as inconsistent indentation); its entry in `rubyTemplates.haml.disabledDiagnostics` is ignored.
- Every diagnostic's `source` is the file's language (`erb`, `haml`, `slim`; `ruby` for Jbuilder and Builder views), HTML and Rails checks included, instead of `ruby-templates`.

### Removed tools

- **Breaking:** the external linters are gone; the built-in server checks replace them. Removed settings: `rubyTemplates.erbLint.enabled`, `rubyTemplates.commands.erbLint`, `rubyTemplates.herbLint.enabled`, `rubyTemplates.commands.herbLint`, `rubyTemplates.slimLint.enabled`, `rubyTemplates.commands.slimLint` and `rubyTemplates.hamlLint.enabled`; erb_lint, herb-lint, slim-lint and haml-lint no longer show diagnostics.
- **Breaking:** the external formatters are gone too (htmlbeautifier, erb-format and `haml-lint --auto-correct-only`); the built-in formatter is the only one. Removed settings: `rubyTemplates.erb.formatter`, `rubyTemplates.haml.formatter`, `rubyTemplates.commands.htmlbeautifier`, `rubyTemplates.commands.erbFormat`, `rubyTemplates.commands.hamlLint` and `rubyTemplates.hamlLint.useBundler`. `rubyTemplates.<erb|haml|slim>.format.enabled` is the switch: off, the extension registers no formatter for that language and another extension's formatter can be used.

### Formatter

- Format Selection and format on save in "modifications" mode work in Haml and Slim files that end in empty lines.
- Slim formatting never changes what Slim compiles: before writing, the formatter compares the Ruby Slim::Engine would generate for the text before and after, and leaves the file as it is if they differ. A file with any Slim error is left unchanged too.
- Empty lines at the end of a Slim file collapse into the final newline (Slim drops them); whitespace-only lines there stay, as Slim compiles them.
- **Breaking:** Slim takes only the indentation width from the editor or `.editorconfig` (default 2): it always writes spaces, also in editors that indent with tabs, keeps each line's ending, and ignores `end_of_line`, `insert_final_newline` and `trim_trailing_whitespace`.

- Haml formatting never changes what Haml compiles: before writing, the formatter compares the nodes, depths and filter bodies Haml reads from the text before and after, and leaves the file as it is if they differ. A file with any Haml error is left unchanged.
- Empty and whitespace-only lines at the end of a Haml file collapse into the final line ending (Haml strips the template).
- **Breaking:** Haml takes only the indentation width from the editor or `.editorconfig` (default 2): it always writes spaces, so tab-indented Haml is now written with spaces, keeps each line's ending, and ignores `end_of_line`, `insert_final_newline`, `trim_trailing_whitespace` and the `.haml-lint.yml` formatting keys (`FinalNewline`, `TrailingWhitespace`, `TrailingEmptyLines`).
- **Breaking:** files with Haml indentation errors are no longer re-indented; fix the error first.
- **Breaking:** ERB takes only the indentation width from the editor or `.editorconfig` (default 2): it always writes spaces, so tab-indented ERB is now written with spaces, keeps each line's ending and the end of the file as it is, always trims trailing whitespace of text lines, and ignores `end_of_line`, `insert_final_newline`, `trim_trailing_whitespace` and the `.erb_lint.yml` formatting keys (`SpaceIndentation`, `FinalNewline`, `TrailingWhitespace`, `ExtraNewline`).

### Development

- `bun test/oracle.js slim|haml|erb` compares our diagnostics and formatter (also on perturbed copies of each file) with the official Slim 5.2.2, Haml 7.5.1 and Rails 8.1.4 ERB implementations, on their test suites and on public apps' views; the Haml oracle also runs on GitLab's views; `bun test/oracle.js context` compares the render checks with actionview 7.0 to 8.0 (`test/oracle/README.md`).

## 0.1.0

First release. Requires VS Code 1.100 or newer.

### Languages

- ERB (`.erb`, `.html.erb`, `.rhtml`, `.html+variant.erb`), Haml (`.haml`) and Slim (`.slim`, `.skim`), with host grammars for `.js.erb`, `.css.erb`, `.json.erb`, `.text.erb`, `.xml.erb`, `.md.erb`, `.rb.erb` and `.yml.erb`.
- Erubi / Erubis (`.erubi`, `.erubis`) open as ERB, with the same diagnostics and navigation.
- Jbuilder and Builder views (`.jbuilder`, `.builder`, kept on the `ruby` language): partial navigation, hover, completion and warnings for `json.partial!`, `json.array! @xs, partial: "x"` and `xml << render(...)`.
- Generator templates (`*.erb.tt`, `*.slim.tt`, `*.haml.tt` under `lib/templates` or `lib/generators`) open as ERB / Slim / Haml, without diagnostics or formatting.
- Embedded Ruby, JavaScript, CSS, SCSS, Sass, CoffeeScript and Markdown inside filters and embedded blocks; language configuration (brackets, auto-closing pairs, comment toggling, indentation, folding, `onEnterRules`).
- Rails-oriented snippets for all three languages, including Hotwire and ViewComponent.

### Language server

- Diagnostics for ERB, Haml and Slim: unclosed and malformed tags, unbalanced or stray `end`, `else` outside a block, block helpers in the wrong tag, unclosed or mismatched HTML tags, Ruby syntax errors in ERB tags, `%>` inside a string, and HTML checks for views (`img-missing-alt`, `duplicate-id`, `prefer-image-tag`, `unsafe-output`). Every ERB and Haml diagnostic has a code; `rubyTemplates.erb.disabledDiagnostics`, `rubyTemplates.haml.disabledDiagnostics` and the Slim setting turn checks off. Haml and Slim also report their own structure errors, such as a line indented several levels at once.
- Partials: go to definition, links, hover, completion, find references and rename for `render "x"`, `render partial:`, `template:`, `layout:`, hash-rocket forms, `render @user` and `render FooComponent.new`. Partials resolve in the view's format and variant, and across symlinked folders. Unresolved renders and strict-locals mismatches are warnings, with a quick fix that creates the missing partial.
- Strict locals (`<%# locals: (…) %>`): completion, hover and diagnostics.
- Controllers and helpers: go to definition from `def show` to its templates, from `layout "admin"` and `render :edit` to their views, and from a helper call to its `def` in `app/helpers` or a `helper_method`.
- Instance variables: completion after `@` and go to definition, from the view's controller action and its `before_action` callbacks.
- Route helpers: completion, hover and go to definition from the app's routes (`rubyTemplates.routes.enabled`).
- i18n: completion, hover, go to definition and find references for `t("…")` keys, from `config/locales`.
- Rails helpers: completion with snippet arguments, form builder methods after `f.`, model attributes after `f.text_field :` from `db/schema.rb`, Turbo Stream actions after `turbo_stream.`, asset paths.
- Stimulus: controller names in `data-controller` and `data-action`, action methods after `hello#`, and `static targets` names.
- ViewComponent: class completion after `render `, `with_*` slot completion and slot readers, go to the component's class and template.
- HTML in ERB: element, closing tag and attribute completion; typing `>` closes the start tag (`rubyTemplates.erb.autoCloseTags`); linked editing of tag names; code actions that add spaces inside `<%=foo%>` tags.
- Haml: completion of filters, tags, attributes and helpers; hover on filters and line prefixes. Slim: completion, hover and formatting through a port of `Slim::Parser`.
- Outline, breadcrumbs, folding, and Go to Symbol in Workspace by render name.
- `ruby-templates-ls --check` runs the diagnostics from the command line.

### Commands and formatting

- Toggle ERB tag, wrap selection in ERB; toggle Haml `-`/`=`/`-#` and Slim line markers; wrap in `=` / `#{}`.
- Extract selection to a partial (ERB, Haml, Slim).
- Switch to Related File: a view's other formats, its controller or mailer action, and a ViewComponent's class, template and preview.
- Preview rendered HTML and show compiled Ruby, through Erubi like Rails (`rubyTemplates.commands.ruby`), or inside the Rails app (`rubyTemplates.preview.useRails`).
- Converters: ERB → Haml / Slim, Haml → ERB / Slim, Slim → ERB, HTML → Haml.
- Command line: `ruby-templates check` and `ruby-templates format [--check|--write]` for CI and editors, shipped next to the server and on npm as `@x-ror/ruby-templates` with prebuilt binaries for Linux, macOS and Windows; `ruby-templates-ls --check` takes folders too.
- Formatting: a built-in whitespace-only formatter in the language server for HTML ERB (`rubyTemplates.erb.formatter: builtin`), Haml (re-indent, the default of `rubyTemplates.haml.formatter`) and Slim, with document and selection formatting for ERB and Haml; settings from the editor, `.editorconfig` and the whitespace linters of erb_lint, haml-lint and slim-lint. ERB can also go through `htmlbeautifier` or `erb-format`, Haml through `haml-lint`.
- Linters on open and save: `erb_lint`, `herb-lint`, `haml-lint` and `slim-lint`.

### Packaging

- One `.vsix` per platform (Linux x64 / arm64, macOS x64 / arm64, Windows x64), each with only its own server binary.
