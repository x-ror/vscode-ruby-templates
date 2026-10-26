# Changelog

## Unreleased

### Language server

- Rails checks follow the project: the framework and gem versions come from the nearest `Gemfile.lock`. `unresolved-render` runs only when it lists `actionview`, `strict-locals` only with `actionview` 7.1 or newer; Sinatra projects and folders without a lockfile get neither. Diagnostics refresh when `Gemfile.lock` changes.
- Both messages are Rails' own: `Missing partial users/_card` / `Missing template users/show`, and one strict locals error per render (`missing local: :b`, `unknown keywords: :c, :d`, `no locals accepted`, with ` for app/views/…` from Rails 7.2).
- Formats an app registers in `config/initializers` (`Mime::Type.register`, `register_alias`) count as formats in template names, so `_card.mobile.erb` is the partial `card`.
- Slim errors are Slim 5.2.2's own: the first error its parser raises (`Malformed indentation`, `Unexpected indentation`, `Expected closing delimiter )`, `Unknown line indicator`, `Illegal shortcut`, `Expected tag`, …) at Slim's line and column, Temple's `Multiple id attributes specified` and `Invalid doctype`, and `Explicit end statements are forbidden`. `action_path('[')` in an attribute is no longer an unclosed bracket.
- Slim's Ruby is checked with Prism (Ruby's own parser) as Slim compiles it, under the new code `ruby-syntax`, with Ruby's messages.
- **Breaking:** Slim's `inconsistent-indent` and `silent-output-helper` are gone (Slim accepts what they reported), and `unclosed-ruby` and `unmatched-branch` are folded into `ruby-syntax`; their entries in `rubyTemplates.slim.disabledDiagnostics` are ignored. Smart text (`> text`, `slim/smart`) is reported as an unknown line indicator, as Slim's default parser does.
- Haml indentation errors are Haml 7.5.1's own, at the line it reports: `Indentation can't use both tabs and spaces.`, `Inconsistent indentation: …`, `The line was indented 2 levels deeper than the previous line.`, `Indenting at the beginning of the document is illegal.`; only the first is reported.
- Haml nesting errors are reported on the nested line, as Haml does, with its messages (`Illegal nesting: content can't be both given on the same line as %p and nested within it.`, `…nesting within plain text is illegal.`, `…a self-closing tag…`, `…a tag that already has content…`), and a doctype with children gets Haml's `…nesting within a header command is illegal.`; `Illegal element: classes and ids must have values.` follows Haml's rule (`.`/`#` followed by nothing, `.` or `#`), and `#$x`/`#@x` lines are plain text.
- Haml attribute errors are Haml's own: `Unbalanced brackets.` for a `{` (balanced as Haml lexes the Ruby: strings, comments, `%w()`, regexps and `?}` don't count) or `[` left open, and `Invalid attribute list: "(foo 'bar')".` for a bad `(…)` list, under `unclosed-attribute`; `%p(a=1` is accepted, as in Haml.
- Haml's `Invalid tag: "% a".`, `There's no Ruby code for = to evaluate.` (bare `=`, `~`, `&=`, `%p=`, a multi-line hash before `=`) and `You don't need to use "- end" in Haml. Un-indent to close a block:` (with Haml's example) are reported as Haml raises them; the first two under the new code `syntax-error`, the last under `explicit-end`. `- x; def f; end` is accepted, as in Haml.
- Haml filters: `Invalid filter name ":java-script".` and `FilterCompiler for 'nope' was not found` (Haml raises the latter at compile, so a parser error in the file comes first; names are case-insensitive, `:maruku` and `:textile` are not filters in 7.5.1). The unknown filter is now an error, not a warning.
- **Breaking:** Haml's "Nested content under Ruby code that doesn't open a block" is gone (Haml compiles `= x` with children); it was reported under `illegal-nesting`.
- **Breaking:** Haml's `bad-dedent` is gone (Haml reports a dedent to a level that never existed as inconsistent indentation); its entry in `rubyTemplates.haml.disabledDiagnostics` is ignored.

### Formatter

- Slim formatting never changes what Slim compiles: before writing, the formatter compares the Ruby Slim::Engine would generate for the text before and after, and leaves the file as it is if they differ. A file with any Slim error is left unchanged too.
- Empty lines at the end of a Slim file collapse into the final newline (Slim drops them); whitespace-only lines there stay, as Slim compiles them.
- **Breaking:** Slim takes only the indentation width from the editor or `.editorconfig` (default 2): it always writes spaces, also in editors that indent with tabs, keeps each line's ending, and ignores `end_of_line`, `insert_final_newline` and `trim_trailing_whitespace`.

### Development

- `bun test/oracle.js slim|haml|erb` compares our diagnostics and formatter (also on perturbed copies of each file) with the official Slim 5.2.2, Haml 7.5.1 and Rails 8.1.4 ERB implementations, on their test suites and on public apps' views; `bun test/oracle.js context` compares the render checks with actionview 7.0 to 8.0 (`test/oracle/README.md`).

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
