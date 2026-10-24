# Ruby Templates: ERB, Haml, Slim

One extension for the three most common Ruby HTML template languages.

## Features

**Syntax highlighting**
- ERB: `<% %>`, `<%= %>`, `<%== %>`, `<%# %>`, `<%- -%>`, `<%%` literals, on top of the built-in HTML grammar.
- Haml: `!!!`, `%tag.class#id`, `{ruby: hash}` / `(html attrs)` / `[object ref]` attributes, `- = != &= ~` lines, `-#` and `/` comments, `#{}` interpolation, filters (`:javascript`, `:css`, `:ruby`, `:scss`, `:sass`, `:coffee`, `:markdown`, `:erb`, …).
- Slim: `doctype`, `tag.class#id`, inline `attr="x"` / `attr=ruby`, wrapped `()` `[]` `{}` attributes with boolean attrs, `*splat`, `li: a` inline tags, `|` / `'` text blocks, `- = ==` lines, `/` and `/!` comments, `#{}` interpolation, embedded `javascript:`, `css:`, `ruby:`, `scss:`, `sass:`, `coffee:`, `markdown:`, `erb:` blocks.

**Editing**
- Bracket matching, auto-closing pairs (`<%` → `<% %>`), comment toggling (`Ctrl+/`), indentation rules for `if/each/do … end`, indentation-based folding for Haml/Slim.
- Snippets for Rails helpers: `link_to`, `render`, `form_with`, `ff` (form field), `each`, `if`, `turbo_frame`, `ts-append` (Turbo Stream actions), `locals` (strict locals), `stimulus`, `flash`, `errors`, `layout`, and more. Type the prefix and press Tab.

**Language server (Rust)**

A small language server written in Rust (`server/`, built on tower-lsp and tree-sitter) adds what a TextMate grammar can't:

- Diagnostics as you type:
  - ERB: unclosed `<%` tags, blocks without `<% end %>`, stray `end`, `if`/`end` inside `<%= %>`.
  - Haml / Slim: mixed tabs and spaces, inconsistent indentation, dedent to a level that doesn't exist, explicit `- end`, unclosed attribute brackets, unknown filters / embedded engines, nesting under a doctype, Haml illegal nesting (inline content + children).
- Go to definition (`F12` / Ctrl+click) on `render "users/card"`, `render partial: "row"`, `render template:`, `render layout:` and `render FooComponent.new` → opens the partial, template or component.
- Hover on a render call shows the resolved file(s); hover on a Rails helper shows its signature.
- Completion: partial names inside `render "…"`, form builder methods after `f.`, and Rails view helpers (`link_to`, `form_with`, `turbo_frame_tag`, `number_to_currency`, …) with snippet arguments.
- Haml: completion of filter names after `:`, tags after `%`, attribute names in `{}` / `()` (including `data:` / `aria:` keys) and `succeed` / `precede` / `surround` / `preserve`; hover on filters and on `=`, `&=`, `!=`, `~`, `==`, `&`, `!`, `<`, `>`; an outline (`content_for`, forms, Turbo Frames, `render`, `#id` and landmark tags) and folding for multi-line attribute hashes and `|` blocks.
- Renaming or moving a partial in the Explorer updates the `render "…"` calls that point to it in ERB, Haml and Slim views. Limits: only views under the same `app/views` are searched (controllers, helpers, engines and `.rb` files are not); a `render` is changed only when every variant Rails could pick for it (`_card.html.erb`, `_card.html+phone.erb`, …) moves to the same new name; renaming a folder and moving a partial directly into `app/views` change nothing; `render` text inside a Ruby string, comment or template text is left alone.
- `ruby-templates-ls --check file.slim …` runs the same diagnostics from the command line (exit code 1 on errors), handy for CI.

The binary for your platform is bundled in `bin/<platform>/`. To build it yourself: `npm run build:server` (needs a Rust toolchain). Settings: `rubyTemplates.server.enabled`, `rubyTemplates.server.path`. Command: *Ruby Templates: Restart Language Server*.

**Commands** (Command Palette, right-click menu, or keybinding)

| Command | Keys | What it does |
|---|---|---|
| Toggle ERB Tag | `Ctrl+Alt+E` | Cycles the tag under the cursor: `<% %>` → `<%= %>` → `<%# %>` |
| Wrap Selection in ERB Output Tag | `Ctrl+Alt+W` | `foo` → `<%= foo %>` |
| Preview Rendered HTML | `Ctrl+Alt+P` | Renders the current file with `slimrb` / `haml` / `erb` in a side panel, refreshes on save |
| Show Compiled Ruby Source | | Shows the Ruby code the engine generates (great for debugging Slim/Haml) |
| Toggle Haml Line Prefix | `Ctrl+Alt+E` | Cycles the current line(s) in Haml: `- x` → `= x` → `-# x` |
| Extract Selection to Partial | | Moves the selection into `_name.html.haml` (or `.erb` / `.slim`) next to the file, re-indented to column 0, and puts `render "dir/name"` in its place |
| Re-indent Haml | | Re-indents the document or selection so every level is the file's indent unit (also *Format Selection*, and *Format Document* by default) |
| Convert ERB/HTML → Haml | | Runs `html2haml --erb` (ERB) or `html2haml --no-erb` (plain `.html`) |
| Convert Haml → ERB | | Runs the `haml_to_erb` library through `ruby` |
| Convert ERB/Haml → Slim | | Runs `erb2slim` / `haml2slim` |

The preview and converters shell out to Ruby gems, which you install once:

```sh
gem install slim haml          # rendering
gem install html2haml          # ERB/HTML → Haml
gem install haml_to_erb        # Haml → ERB
gem install html2slim haml2slim  # ERB → Slim, Haml → Slim
gem install haml_lint          # Haml lint diagnostics, haml-lint formatter
```

**haml-lint**: when `haml-lint` is installed, Haml files are linted on open and save and its offenses show up as diagnostics (`rubyTemplates.hamlLint.enabled`, `rubyTemplates.commands.hamlLint`, `rubyTemplates.hamlLint.useBundler`). Set `rubyTemplates.haml.formatter` to `haml-lint` to make *Format Document* run `haml-lint --auto-correct-only`; note that it applies your RuboCop config's safe corrections too, which can rewrite or drop Ruby code (for example a useless assignment).

Executable paths are configurable under `rubyTemplates.commands.*`. Set `rubyTemplates.preview.useBundler` to run them through `bundle exec` in projects with a Gemfile.

Note that a standalone render has no Rails helpers (`link_to`, `render`, …). The preview is most useful for layout fragments and plain-Ruby templates.

## Tips

- Emmet works in `.html.erb` out of the box (the extension defaults `"emmet.includeLanguages": { "erb": "html" }`).
- Ruby LSP (Shopify) and this extension both register the `erb` language id, so they work together.
- If you also have another Slim or Haml extension installed, VS Code picks one grammar; disable the other to avoid surprises.

## Development

```sh
npm install
npm run build:server  # cargo build --release, copies the binary into bin/<platform>/
npm run test:tokens   # dump token scopes for the sample files in test/
npm run test:server   # cargo test + diagnostics check on the samples
npm run test:unit     # pure JS helpers (Haml toggle, re-indent, haml-lint JSON)
node test/e2e.js      # LSP smoke test against a fake Rails tree
npm run package       # build the .vsix
```

Grammars live in `syntaxes/`, language configs in `language-configuration/`, snippets in `snippets/`, the VS Code client in `src/`, and the language server in `server/src/` (`erb.rs`, `indented.rs` for Slim/Haml, `partials.rs`, `ruby.rs`). CI (`.github/workflows/build.yml`) builds the server for Linux, macOS and Windows and packages one `.vsix` per platform.
