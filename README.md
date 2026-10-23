# Ruby Templates: ERB, Haml, Slim

One extension for the three most common Ruby HTML template languages.

## Features

**Syntax highlighting**
- ERB: `<% %>`, `<%= %>`, `<%== %>`, `<%# %>`, `<%- -%>`, `<%%` literals, on top of the built-in HTML grammar.
- Haml: `!!!`, `%tag.class#id`, `{ruby: hash}` / `(html attrs)` / `[object ref]` attributes, `- = != &= ~` lines, `-#` and `/` comments, `#{}` interpolation, filters (`:javascript`, `:css`, `:ruby`, `:scss`, `:sass`, `:coffee`, `:markdown`, `:erb`, …).
- Slim: `doctype`, `tag.class#id`, inline `attr="x"` / `attr=ruby`, wrapped `()` `[]` `{}` attributes with boolean attrs, `*splat`, `li: a` inline tags, `|` / `'` text blocks, `- = ==` lines, `/` and `/!` comments, `#{}` interpolation, embedded `javascript:`, `css:`, `ruby:`, `scss:`, `sass:`, `coffee:`, `markdown:`, `erb:` blocks.

**Editing**
- Bracket matching, auto-closing pairs (`<%` → `<% %>`), comment toggling (`Ctrl+/`), indentation rules for `if/each/do … end`, indentation-based folding for Haml/Slim.
- Snippets for Rails helpers: `link_to`, `render`, `form_with`, `ff` (form field), `each`, `if`, `turbo_frame`, `stimulus`, `flash`, `errors`, `layout`, and more. Type the prefix and press Tab.

**Commands** (Command Palette, right-click menu, or keybinding)

| Command | Keys | What it does |
|---|---|---|
| Toggle ERB Tag | `Ctrl+Alt+E` | Cycles the tag under the cursor: `<% %>` → `<%= %>` → `<%# %>` |
| Wrap Selection in ERB Output Tag | `Ctrl+Alt+W` | `foo` → `<%= foo %>` |
| Preview Rendered HTML | `Ctrl+Alt+P` | Renders the current file with `slimrb` / `haml` / `erb` in a side panel, refreshes on save |
| Show Compiled Ruby Source | | Shows the Ruby code the engine generates (great for debugging Slim/Haml) |
| Convert ERB → Haml | | Runs `html2haml --erb` |
| Convert ERB/Haml → Slim | | Runs `erb2slim` / `haml2slim` |

The preview and converters shell out to Ruby gems, which you install once:

```sh
gem install slim haml          # rendering
gem install html2haml          # ERB → Haml
gem install html2slim haml2slim  # ERB → Slim, Haml → Slim
```

Executable paths are configurable under `rubyTemplates.commands.*`. Set `rubyTemplates.preview.useBundler` to run them through `bundle exec` in projects with a Gemfile.

Note that a standalone render has no Rails helpers (`link_to`, `render`, …). The preview is most useful for layout fragments and plain-Ruby templates.

## Tips

- Emmet in ERB: add `"emmet.includeLanguages": { "erb": "html" }` to your settings.
- Ruby LSP (Shopify) and this extension both register the `erb` language id, so they work together.
- If you also have another Slim or Haml extension installed, VS Code picks one grammar; disable the other to avoid surprises.

## Development

```sh
npm install
npm run test:tokens   # dump token scopes for the sample files in test/
npm run package       # build the .vsix
```

Grammars live in `syntaxes/`, language configs in `language-configuration/`, snippets in `snippets/`, commands in `src/extension.js`.
