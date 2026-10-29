# Changelog

## Unreleased

- Haml and Slim: the strict locals comment (`-# locals: (…)`, `/# locals: (…)`) gets the same `ruby-syntax` checks as ERB with `actionview` 7.1 or newer: a signature Ruby can't parse, and Rails' "`a` set as non-keyword argument for app/views/users/_card.html.haml. Locals can only be set as keyword arguments." for locals that aren't keywords.

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

- Diagnostics for ERB, Haml and Slim, as the official implementations report them: Erubi 1.13.1 with Rails 8.1.4's options, Haml 7.5.1 and Slim 5.2.2, with their messages and positions. Every diagnostic has a code; `rubyTemplates.erb.disabledDiagnostics`, `rubyTemplates.haml.disabledDiagnostics` and `rubyTemplates.slim.disabledDiagnostics` turn checks off, and `rubyTemplates.<erb|haml|slim>.diagnostics.enabled` (on by default) all of a language's; Skim follows `slim`, and the `ruby-templates` command line is unaffected. A diagnostic's `source` is the file's language (`erb`, `haml`, `slim`; `ruby` for Jbuilder and Builder views), HTML and Rails checks included.
- The extension runs no third-party linters or formatters: every check and the formatter are built into the server.
- Ruby in ERB and Slim is checked with Prism (Ruby's own parser) as Rails compiles the template, under `ruby-syntax`, with Ruby's messages. ERB builds the method source Erubi generates (`trim: true`, `@output_buffer`, the `locals:` magic comment as the method's parameters) and reports every error at the template line it comes from (exactly, when it is in a tag's code), whatever else is wrong with the file: `<%= if x %>` gets ``expected `then` or `;` or '\n'``. The grammar is the project's Ruby, from the nearest `.ruby-version`, else the `RUBY VERSION` of the nearest `Gemfile.lock`, else the latest.
- ERB: `unclosed-tag` warns about a `<%` with no `%>` after it in HTML templates, which Rails compiles as text but the HTML parser rejects (`invalid-first-character-of-tag-name`, HTML 13.2.5.6); it is found with Erubi's own scan, and `.text.erb` or `.js.erb` templates get none.
- ERB's HTML checks for views cite the HTML standard: unexpected, mismatched and unclosed elements ("Unexpected end tag `</span>`: no open element of that name (HTML 13.2.6.4.7)", "End tag `</div>` doesn't close the open `<span>` (line 2)", "`<div>` is never closed"), end tags of void elements (HTML 13.1.2) and a `</p>` after a block start tag closed the `p` (not inside `button`, `object`, `template`, `table` cells, SVG/MathML or where ERB branches disagree); `img-missing-alt` and `duplicate-id` quote the standard's own sentence (4.8.4.4.1, 3.2.6). `<caption>` may omit its end tag, and `<img generator-unable-to-provide-required-alt="">` is accepted, as the standard says.
- Haml: indentation errors (`Indentation can't use both tabs and spaces.`, `Inconsistent indentation: …`, `The line was indented 2 levels deeper than the previous line.`, `Indenting at the beginning of the document is illegal.`; only the first is reported), illegal nesting on the nested line (content both on the line and nested, nesting in plain text, a self-closing tag, a tag that already has content, or a doctype), `Illegal element: classes and ids must have values.`, `Unbalanced brackets.` and `Invalid attribute list: …` (`unclosed-attribute`), `Invalid tag: …` and `There's no Ruby code for = to evaluate.` (`syntax-error`), `You don't need to use "- end" in Haml.` (`explicit-end`), `Invalid filter name …` and `FilterCompiler for 'nope' was not found`, and `- else`/`elsif`/`when` placement (`Got "else" with no preceding "if"`, `"else" is indented at wrong level: …`), all with Haml's messages. Content in a void element (`%br` with children or text) is the warning `void-children`.
- Slim: the first error Slim's parser raises (`Malformed indentation`, `Unexpected indentation`, `Expected closing delimiter )`, `Unknown line indicator`, `Illegal shortcut`, `Expected tag`, …) at Slim's line and column, Temple's `Multiple id attributes specified` and `Invalid doctype`, and `Explicit end statements are forbidden`. Smart text (`> text`, `slim/smart`) is an unknown line indicator, as in Slim's default parser.
- Partials: go to definition, links, hover, completion, find references and rename for `render "x"`, `render partial:`, `template:`, `layout:`, hash-rocket forms, `render @user` and `render FooComponent.new`. Partials resolve in the view's format and variant, across symlinked folders, and with the formats an app registers in `config/initializers` (`Mime::Type.register`, `register_alias`), so `_card.mobile.erb` is the partial `card`. Unresolved renders and strict-locals mismatches are warnings with Rails' own messages (`Missing partial users/_card`, `missing local: :b`, `unknown keywords: :c, :d`, `no locals accepted`, with ` for app/views/…` from Rails 7.2), and a quick fix creates the missing partial.
- Rails checks follow the project's `Gemfile.lock`: `unresolved-render` runs when it lists `actionview`, `strict-locals` with `actionview` 7.1 or newer; Sinatra projects and folders without a lockfile get neither. Diagnostics refresh when `Gemfile.lock` changes.
- Strict locals (`<%# locals: (…) %>`): completion, hover and diagnostics. With `actionview` 7.1 or newer, locals must be keywords: `<%# locals: (a, b:) -%>` gets Rails' "`a` set as non-keyword argument for app/views/users/_card.html.erb. Locals can only be set as keyword arguments." under `ruby-syntax`.
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
- Find references, file renames and the definition of a partial's local scan the app on their own thread, so hover, completion and diagnostics answer while they run.
- `ruby-templates-ls --check` runs the diagnostics from the command line.

### Commands and formatting

- Toggle ERB tag, wrap selection in ERB; toggle Haml `-`/`=`/`-#` and Slim line markers; wrap in `=` / `#{}`.
- Extract selection to a partial (ERB, Haml, Slim).
- Switch to Related File: a view's other formats, its controller or mailer action, and a ViewComponent's class, template and preview.
- Preview rendered HTML and show compiled Ruby, through Erubi like Rails (`rubyTemplates.commands.ruby`), or inside the Rails app (`rubyTemplates.preview.useRails`).
- Converters: ERB → Haml / Slim, Haml → ERB / Slim, Slim → ERB, HTML → Haml.
- Command line: `ruby-templates check` and `ruby-templates format [--check|--write]` for CI and editors, shipped next to the server and on npm as `@x-ror/ruby-templates` with prebuilt binaries for Linux and Windows; `ruby-templates-ls --check` takes folders too.
- Formatting: a built-in whitespace-only formatter for HTML ERB, Haml and Slim, on by default for Format Document and format on save, with Format Selection for ERB and Haml. `rubyTemplates.<erb|haml|slim>.format.enabled` (on by default) switches it off, and the extension then registers no formatter for that language, so another extension's can be used.
- The formatter takes only the indentation width from the editor or `.editorconfig` (default 2): it always writes spaces and keeps each line's ending. ERB keeps the end of the file as it is and trims trailing whitespace of text lines; a file with an ERB error or an HTML structure error is left unchanged.
- Haml and Slim formatting never changes what the template compiles: the formatter compares what Haml reads (nodes, depths, filter bodies) or the Ruby `Slim::Engine` generates, before and after, and leaves the file as it is if they differ, or if it has any Haml or Slim error. Empty lines at the end collapse into the final line ending; in Slim, whitespace-only lines there stay, as Slim compiles them.

### Packaging

- One `.vsix` per platform (Linux x64 / arm64, Windows x64; macOS builds are paused until GitHub's macOS runners pick up jobs again), each with only its own server binary.

### Development

- `bun test/oracle.js slim|haml|erb` compares the diagnostics and formatter (also on perturbed copies of each file) with Slim 5.2.2, Haml 7.5.1 and Rails 8.1.4's ERB, on their test suites and on public apps' views (GitLab's too, for Haml); `bun test/oracle.js context` compares the render checks with actionview 7.0 to 8.0 (`test/oracle/README.md`).
