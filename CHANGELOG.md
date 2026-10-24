# Changelog

## Unreleased

- **Breaking:** requires VS Code 1.100 or newer: the extension is now an ES module (`vscode-languageclient` 11 alone needs 1.91).
- Partials: hash-rocket `render :partial => "x"`, `:template =>`, `:layout =>` get navigation, completion and diagnostics.
- Partials resolve only in the view's format: `render "users/user"` from `index.html.erb` no longer offers `_user.json.jbuilder`. Format-less partials still match; JS and turbo_stream views also take HTML partials.
- Erubi / Erubis: `.erubi` and `.erubis` files open as ERB, with the same diagnostics and navigation; `x.html.erubi` gets the HTML tag checks.
- Jbuilder / Builder views (`.jbuilder`, `.builder`, kept on the `ruby` language): go to definition, hover, links, completion and unresolved / strict-locals warnings for `json.partial! "x"`, `json.array! @xs, partial: "x"` and `xml << render(...)`; renaming a partial updates them too.
- Every ERB and Haml diagnostic has a code; `rubyTemplates.erb.disabledDiagnostics` and `rubyTemplates.haml.disabledDiagnostics` turn checks off like the Slim setting. Strict-locals warnings are coded `strict-locals` in all languages.
- Quick fix: create the missing partial or template of an unresolved `render "dir/name"`, in the view's format and template language.
- Find All References on a partial lists the `render` calls that resolve to it.
- Completion of Turbo Stream actions after `turbo_stream.` (`append`, `replace`, `remove`, `refresh`, `*_all`, `action`).
- Completion of Stimulus controller names in `data-controller="…"`, `data-action="…->…"` and `data: { controller: "…" }`.
- ERB: warn when `%>` inside a Ruby string ends the tag early (`<%= "50%>" %>`).
- Fewer false warnings, found on real apps: `=begin`/`=end` comments in ERB tags, `.rss`/`.atom` (and font) formats, and `render "devise/…"` views shipped by a gem in `Gemfile.lock`.
- ERB and Slim generator templates (`*.erb.tt`, `*.slim.tt` under `lib/templates` or `lib/generators`) open as ERB / Slim, without diagnostics or formatting.
- ERB: linked editing of HTML open/close tag names (`editor.linkedEditing`).
- Go to Symbol in Workspace lists partials and templates by their render name.
- Command *Switch to Related File*: a view's other formats (mailer HTML ↔ text) and a ViewComponent's class ↔ template.
- Haml highlighting: a nested hash (`%a{data: {id: 1}} text`) no longer ends the attribute hash at its inner `}`.
- ERB: Ruby syntax errors inside tags, from tree-sitter-ruby over the template's Ruby as Erubi joins it; an open tag swallowing the next ones is reported where it opens.
- ERB formatting through `htmlbeautifier` or `erb-format` (`rubyTemplates.erb.formatter`), with `bundle exec` support.
- erb_lint diagnostics on open and save (`rubyTemplates.erbLint.enabled`).
- Server: tree-sitter 0.27, `once_cell` replaced by the standard library; builds need Rust 1.82.

## 0.1.0

- Rust language server: diagnostics for ERB/Haml/Slim, go to partial/component, hover, completions for partials, form builder methods and Rails helpers, `--check` CLI mode.

- Syntax highlighting for ERB (`.erb`, `.html.erb`, `.rhtml`), Haml (`.haml`) and Slim (`.slim`, `.skim`).
- Embedded Ruby, JavaScript, CSS, SCSS, Sass, CoffeeScript and Markdown inside filters / embedded blocks.
- Language configuration: brackets, auto-closing pairs, comment toggling, indentation rules, folding.
- Rails-oriented snippets for all three languages.
- Commands: toggle ERB tag, wrap selection in ERB, HTML preview, show compiled Ruby, ERB→Haml, ERB/Haml→Slim converters.
- Language server, Haml: completion of filters, tags, attribute names and Haml helpers; hover on filters and line prefixes; outline and folding. Renaming a partial updates `render` calls in ERB, Haml and Slim views. `*.haml.tt` generator templates open as Haml (no structure diagnostics).
- Haml: toggle `-`/`=`/`-#` (`Ctrl+Alt+E`), extract selection to a partial (also ERB), re-indent formatter, optional `haml-lint` formatter and diagnostics, Haml→ERB (`haml_to_erb`) and HTML→Haml converters.
- Slim commands: toggle line marker (`-` / `=` / `==` / `/`), wrap selection in `=` / `#{}`, extract selection to partial, Slim→ERB (`slimrb --erb`); `rubyTemplates.preview.useRails` renders the preview through `bin/rails runner`, `rubyTemplates.slim.compileForRails` compiles with `slimrb --rails`.
- Slim `onEnterRules`: indent after `|` / `'` text lines, keep indent after `,` / `\` continuations.
