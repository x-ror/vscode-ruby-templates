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

**Language server (Rust)**

A small language server written in Rust (`server/`, built on tower-lsp and tree-sitter) adds what a TextMate grammar can't:

- Diagnostics as you type:
  - ERB: unclosed `<%` tags, blocks without `<% end %>`, stray `end`, `if`/`end` inside `<%= %>`.
  - Haml / Slim: mixed tabs and spaces, inconsistent indentation, dedent to a level that doesn't exist, explicit `- end`, unclosed attribute brackets, unknown filters / embedded engines, nesting under a doctype, Haml illegal nesting (inline content + children).
- Go to definition (`F12` / Ctrl+click) on `render "users/card"`, `render partial: "row"`, `render template:`, `render layout:` and `render FooComponent.new` → opens the partial, template or component.
- Hover on a render call shows the resolved file(s); hover on a Rails helper shows its signature.
- Completion: partial names inside `render "…"`, form builder methods after `f.`, and Rails view helpers (`link_to`, `form_with`, `turbo_frame_tag`, `number_to_currency`, …) with snippet arguments.
- `ruby-templates-ls --check file.slim …` runs the same diagnostics from the command line (exit code 1 on errors), handy for CI.

The binary for your platform is bundled in `bin/<platform>/`. To build it yourself: `npm run build:server` (needs a Rust toolchain). Settings: `rubyTemplates.server.enabled`, `rubyTemplates.server.path`. Command: *Ruby Templates: Restart Language Server*.

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
npm run build:server  # cargo build --release, copies the binary into bin/<platform>/
npm run test:tokens   # dump token scopes for the sample files in test/
npm run test:server   # cargo test + diagnostics check on the samples
node test/e2e.js      # LSP smoke test against a fake Rails tree
npm run package       # build the .vsix
```

Grammars live in `syntaxes/`, language configs in `language-configuration/`, snippets in `snippets/`, the VS Code client in `src/`, and the language server in `server/src/` (`erb.rs`, `indented.rs` for Slim/Haml, `partials.rs`, `ruby.rs`). CI (`.github/workflows/build.yml`) builds the server for Linux, macOS and Windows and packages one `.vsix` per platform.
