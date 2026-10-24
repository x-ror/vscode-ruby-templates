# Changelog

## 0.1.0

- Rust language server: diagnostics for ERB/Haml/Slim, go to partial/component, hover, completions for partials, form builder methods and Rails helpers, `--check` CLI mode.

- Syntax highlighting for ERB (`.erb`, `.html.erb`, `.rhtml`), Haml (`.haml`) and Slim (`.slim`, `.skim`).
- Embedded Ruby, JavaScript, CSS, SCSS, Sass, CoffeeScript and Markdown inside filters / embedded blocks.
- Language configuration: brackets, auto-closing pairs, comment toggling, indentation rules, folding.
- Rails-oriented snippets for all three languages.
- Commands: toggle ERB tag, wrap selection in ERB, HTML preview, show compiled Ruby, ERB→Haml, ERB/Haml→Slim converters.
- Language server, Haml: completion of filters, tags, attribute names and Haml helpers; hover on filters and line prefixes; outline and folding. Renaming a partial updates `render` calls in ERB, Haml and Slim views. `*.haml.tt` generator templates open as Haml (no structure diagnostics).
- Haml: toggle `-`/`=`/`-#` (`Ctrl+Alt+E`), extract selection to a partial (also ERB/Slim), re-indent formatter, optional `haml-lint` formatter and diagnostics, Haml→ERB (`haml_to_erb`) and HTML→Haml converters.
