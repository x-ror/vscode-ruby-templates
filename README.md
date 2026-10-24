# Ruby Templates: ERB, Haml, Slim

One extension for the three most common Ruby HTML template languages.

## Features

**Syntax highlighting**
- ERB: `<% %>`, `<%= %>`, `<%== %>`, `<%# %>`, `<%- -%>`, `<%%` literals, on top of the built-in HTML grammar.
- Haml: `!!!`, `%tag.class#id`, `{ruby: hash}` / `(html attrs)` / `[object ref]` attributes, `- = != &= ~` lines, `-#` and `/` comments, `#{}` interpolation, filters (`:javascript`, `:css`, `:ruby`, `:scss`, `:sass`, `:coffee`, `:markdown`, `:erb`, …).
- Slim: `doctype`, `tag.class#id`, inline `attr="x"` / `attr=ruby`, wrapped `()` `[]` `{}` attributes (also across lines) with boolean attrs, `*splat`, `li: a` inline tags, `a>` / `a<>` / `img/`, `|` / `'` text blocks and text continued under `p Some text`, `- = == =< =>` lines with `,` / `\` continuations, `/`, `/!` and `/[if IE]` comments, `#{}` and `#{{}}` interpolation, inline HTML `<div>` lines, `include` (`slim/include`), `>` text (`slim/smart`).
- Slim embedded engines (attributes such as `javascript nonce=...:` allowed): `javascript:`, `css:`, `ruby:`, `scss:`, `sass:`, `coffee:`, `markdown:` and `erb:` use the target grammar, with `#{}` highlighted as Ruby in `javascript:`, `css:` and `markdown:`; `textile:`, `rdoc:`, `asciidoc:` and older engines (`less:`, `styl:`, `builder:`, …) are plain text with `#{}`. Slim 5 registers only `asciidoc`, `markdown`, `textile`, `rdoc`, `coffee`, `sass`, `scss`, `javascript`, `css` and `ruby`, so `typescript:`, `babel:`, `livescript:` and `haml:` get no engine rule.
- Skim (`.skim`): the Slim grammar with CoffeeScript instead of Ruby in code lines, attributes and `#{}`.

**Editing**
- Bracket matching, auto-closing pairs (`<%` → `<% %>`), comment toggling (`Ctrl+/`), indentation rules for `if/each/do … end`, indentation-based folding for Haml/Slim.
- Snippets for Rails helpers: `link_to`, `render`, `form_with`, `ff` (form field), `each`, `if`, `turbo_frame`, `ts-append` (Turbo Stream actions), `locals` (strict locals), `stimulus`, `flash`, `errors`, `layout`, and more. Type the prefix and press Tab.

**Language server (Rust)**

A small language server written in Rust (`server/`, built on tower-lsp and tree-sitter) adds what a TextMate grammar can't:

- Diagnostics as you type:
  - ERB: unclosed `<%` tags, blocks without `<% end %>`, stray `end`, `if`/`end` inside `<%= %>`.
  - Haml / Slim: mixed tabs and spaces, inconsistent indentation, dedent to a level that doesn't exist, explicit `- end`, unclosed attribute brackets, unknown filters / embedded engines, nesting under a doctype, Haml illegal nesting (inline content + children).
- Go to definition (`F12` / Ctrl+click) on `render "users/card"`, `render partial: "row"`, `render template:`, `render layout:` and `render FooComponent.new` → opens the partial, template or component.
- Hover on a render call shows the resolved file(s); hover on a Rails helper shows its signature. In Slim, hover on a line marker (`-`, `=`, `==`, `=<`, `=>`, `/`, `/!`, `/[if IE]`, `|`, `'`, `javascript:` and other engines) explains it.
- Completion: partial names inside `render "…"`, form builder methods after `f.`, and Rails view helpers (`link_to`, `form_with`, `turbo_frame_tag`, `number_to_currency`, …) with snippet arguments. In Slim, helpers are offered only in Ruby (after `-`/`=`, in `attr=value`, `*splat`, `#{}`, `ruby:` bodies and continuation lines, not in text); HTML tag and embedded engine names (including `rubyTemplates.slim.extraEngines`) where a tag starts; common HTML attribute names where an attribute may start (`a hr`, `input(ty`).
- Slim formatting (*Format Document*): re-indents each Slim line to the editor's tab size in spaces, moves text blocks, engine bodies and continuation lines with their line (keeping their relative indentation), and strips trailing whitespace except where it is part of the output (`|`/`'` text, inline text, quoted attribute values, engine bodies). A file Slim would reject (or that uses `slim/smart` `>` lines) is left unchanged. Format on save: `"[slim]": { "editor.formatOnSave": true }`. The formatter is registered for Slim and Skim only, so ERB and Haml keep their own formatters.
- `ruby-templates-ls --check file.slim …` runs the same diagnostics from the command line (exit code 1 on errors), handy for CI. `ruby-templates-ls --format file.slim` prints the file formatted with 2 spaces (exit code 1 if it can't be formatted safely).

The binary for your platform is bundled in `bin/<platform>/`. To build it yourself: `npm run build:server` (needs a Rust toolchain). Settings: `rubyTemplates.server.enabled`, `rubyTemplates.server.path`. Command: *Ruby Templates: Restart Language Server*.

**Commands** (Command Palette, right-click menu, or keybinding)

| Command | Keys | What it does |
|---|---|---|
| Toggle ERB Tag | `Ctrl+Alt+E` | Cycles the tag under the cursor: `<% %>` → `<%= %>` → `<%# %>` |
| Wrap Selection in ERB Output Tag | `Ctrl+Alt+W` | `foo` → `<%= foo %>` |
| Toggle Slim Line | `Ctrl+Alt+E` | Cycles the marker of each selected line: `-` → `=` → `==` → `/` |
| Wrap Selection in Slim Output | `Ctrl+Alt+W` | A whole line becomes `= foo`, part of a line `#{foo}` |
| Extract Selection to Slim Partial | | Moves the selected lines (dedented) to `_name.html.slim` and leaves `= render "name"`; `dir/name` goes under `app/views/dir/` |
| Preview Rendered HTML | `Ctrl+Alt+P` | Renders the current file with `slimrb` / `haml` / `erb` in a side panel, refreshes on save |
| Show Compiled Ruby Source | | Shows the Ruby code the engine generates (great for debugging Slim/Haml) |
| Convert ERB → Haml | | Runs `html2haml --erb` |
| Convert ERB/Haml → Slim | | Runs `erb2slim` / `haml2slim` |
| Convert Slim → ERB | | Runs `slimrb --erb`. The output is Slim's compiled form (`<%= ::Temple::Utils.escape_html((x)) %>`), correct for plain ERB but escaped twice under Rails' auto-escaping, so review it before committing |

The preview and converters shell out to Ruby gems, which you install once:

```sh
gem install slim haml          # rendering
gem install html2haml          # ERB → Haml
gem install html2slim haml2slim  # ERB → Slim, Haml → Slim
```

Executable paths are configurable under `rubyTemplates.commands.*`. Set `rubyTemplates.preview.useBundler` to run them through `bundle exec` in projects with a Gemfile.

Note that a standalone render has no Rails helpers (`link_to`, `render`, …). The preview is most useful for layout fragments and plain-Ruby templates. Set `rubyTemplates.preview.useRails` to render through the app instead: when a `bin/rails` is found above the file, the preview runs `bin/rails runner` with `ApplicationController.render(inline: source, type: :slim)` (or `:erb` / `:haml`), so helpers and routes work. It boots the app on each refresh and gives up after 60 seconds; relative `render "x"` calls resolve against `application/`, as in any inline render. Set `rubyTemplates.slim.compileForRails` to show the Slim compiled source as Rails compiles it (`slimrb --rails`: ActionView output buffer, no capture).

## Tips

- Emmet works in `.html.erb` out of the box (the extension defaults `"emmet.includeLanguages": { "erb": "html" }`). VS Code's Emmet supports the `slim` id natively and expands to Slim syntax (`ul>li` → `ul` / `  li`); the extension maps `skim` to it too.
- Host-specific Slim files (`.text.slim`, `.xml.slim`, `.js.slim`, …) open as Slim; the void-element check (`img` with children) only runs for HTML hosts (`.html.slim`, `.html+variant.slim`, `.turbo_stream.slim` and bare `.slim`).
- Ruby LSP (Shopify) and this extension both register the `erb` language id, so they work together.
- If you also have another Slim or Haml extension installed, VS Code picks one grammar; disable the other to avoid surprises.
- Slim options set per app are not reflected in highlighting: custom `:shortcut`s (`&text`, `@role`), custom `attr_list_delims` / `code_attr_delims`, smart text's implicit text lines (only explicit `>` lines are recognized) and logic-less mode (`- section` / `= var` still show as Ruby). The translator plugin (`slim/translator`) changes nothing in the syntax, so it needs no support.

## Development

```sh
npm install
npm run build:server  # cargo build --release, copies the binary into bin/<platform>/
npm run test:tokens   # dump token scopes for the sample files in test/
npm run test:server   # cargo test + diagnostics check on the samples
node test/e2e.js      # LSP smoke test against a fake Rails tree
npm run test:slim     # Slim editing helpers and onEnterRules
npm run test:format   # Slim formatter vs. the slim gem: test/format/*.slim compile the same before and after (needs `ruby -rslim`)
npm run package       # build the .vsix
```

Grammars live in `syntaxes/`, language configs in `language-configuration/`, snippets in `snippets/`, the VS Code client in `src/`, and the language server in `server/src/` (`erb.rs`, `indented.rs` for Slim/Haml, `slim_parser.rs` (a port of `Slim::Parser` used by Slim completion, hover and formatting), `partials.rs`, `ruby.rs`). CI (`.github/workflows/build.yml`) builds the server for Linux, macOS and Windows and packages one `.vsix` per platform.
