# Changelog

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
