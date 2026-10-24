# Changelog

## 0.1.0

- Rust language server: diagnostics for ERB/Haml/Slim, go to partial/component, hover, completions for partials, form builder methods and Rails helpers, `--check` CLI mode.

- Syntax highlighting for ERB (`.erb`, `.html.erb`, `.rhtml`), Haml (`.haml`) and Slim (`.slim`, `.skim`).
- Embedded Ruby, JavaScript, CSS, SCSS, Sass, CoffeeScript and Markdown inside filters / embedded blocks.
- Language configuration: brackets, auto-closing pairs, comment toggling, indentation rules, folding.
- Rails-oriented snippets for all three languages.
- Commands: toggle ERB tag, wrap selection in ERB, HTML preview, show compiled Ruby, ERB→Haml, ERB/Haml→Slim converters.
- Slim commands: toggle line marker (`-` / `=` / `==` / `/`), wrap selection in `=` / `#{}`, extract selection to partial, Slim→ERB (`slimrb --erb`); `rubyTemplates.preview.useRails` renders the preview through `bin/rails runner`, `rubyTemplates.slim.compileForRails` compiles with `slimrb --rails`.
- Slim `onEnterRules`: indent after `|` / `'` text lines, keep indent after `,` / `\` continuations.
