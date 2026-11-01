<p align="center"><img src="images/icon.png" alt="Ruby Templates" width="128"></p>

# Ruby Templates: ERB, Haml, Slim

[![VS Marketplace](https://img.shields.io/visual-studio-marketplace/v/x-ror.ruby-templates?label=VS%20Marketplace)](https://marketplace.visualstudio.com/items?itemName=x-ror.ruby-templates)
[![Open VSX](https://img.shields.io/open-vsx/v/x-ror/ruby-templates?label=Open%20VSX)](https://open-vsx.org/extension/x-ror/ruby-templates)
[![npm](https://img.shields.io/npm/v/@x-ror/ruby-templates?label=npm%20CLI)](https://www.npmjs.com/package/@x-ror/ruby-templates)
[![build](https://github.com/x-ror/vscode-ruby-templates/actions/workflows/build.yml/badge.svg)](https://github.com/x-ror/vscode-ruby-templates/actions/workflows/build.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Rails view tooling for VS Code: one extension for ERB, Haml and Slim.** It highlights your templates, finds errors as you type, formats them, and understands your Rails app: partials, components, helpers, routes, instance variables, translations and Stimulus controllers.

Built for Rails developers who work in `app/views` every day, and for teams that want the same template checks and formatting in the editor and in CI.

- **No gems to install.** The checks and the formatter are a fast language server written in Rust and bundled with the extension. They follow Erubi, Rails, Haml 7 and Slim 5 and report errors with those projects' own messages.
- **Rails-aware.** <kbd>F12</kbd> on `render "users/card"` opens the partial; completion knows your partials, ViewComponents, model columns, `*_path` helpers, `@ivars` from the controller and `t(".keys")`.
- **Safe formatting.** It changes only whitespace and checks that the template still compiles to the same thing; if it is unsure, it leaves the file alone.
- **Works with Ruby LSP.** Install both: Ruby LSP keeps your `.rb` files, this extension adds what templates need.

## Screenshots

**Errors as you type**: unclosed blocks, mismatched tags, missing `alt`, unresolved partials.

![Diagnostics in an ERB view](images/screenshots/diagnostics.png)

**Completion** of partial names inside `render "…"`.

![Partial completion](images/screenshots/completion.png)

**Hover** on a Rails helper shows its signature and links to the API docs.

![Hover on link_to in Slim](images/screenshots/hover.png)

**Highlighting** for Slim with embedded JavaScript and `#{}` interpolation, and the same for ERB and Haml.

![Slim highlighting](images/screenshots/slim.png)

## Features

### Highlighting and editing

- Full syntax highlighting for ERB (`.html.erb`, `.erubi`, `.js.erb`, `.css.erb`, `.json.erb`, `.yml.erb`, …), Haml, Slim and Skim, including embedded JavaScript, CSS, SCSS, CoffeeScript and Markdown blocks.
- Comment toggling, bracket matching, auto-closing `<% %>`, smart indentation and folding.
- Typing `>` in ERB closes the HTML tag; renaming `<div>` renames its `</div>` (linked editing).
- 50+ snippets per language for Rails, Hotwire and ViewComponent: `link_to`, `render`, `form_with`, `ff`, `turbo_frame`, `ts-append`, `stimulus`, `locals`, `flash`, …
- Emmet in ERB and Slim.

### Errors as you type

- **ERB**: Ruby syntax errors inside tags (checked with Prism, Ruby's own parser, for your project's Ruby version), unclosed `<%`, blocks without `<% end %>`, mismatched or unclosed HTML elements, `<img>` without `alt`, duplicate ids.
- **Haml and Slim**: the errors Haml and Slim themselves would raise (indentation, illegal nesting, unclosed attributes, unknown filters and engines, explicit `end`) at the same line and column, plus Ruby syntax errors in code lines, attribute hashes and `#{}`.
- **Rails**: `render` of a partial that doesn't exist (with a quick fix that creates it) and strict locals (`<%# locals: (user:) %>`) mismatches, with Rails' own messages.
- Every check has a code you can turn off; whole languages can be switched off too.

### Navigation and completion that know Rails

| Where                                 | What you get                                                                           |
| ------------------------------------- | -------------------------------------------------------------------------------------- |
| `render "users/card"`, `render @user` | Go to the partial, hover shows the file, completion of partial names, find all renders |
| `render ButtonComponent.new`          | Completion of ViewComponents, go to class and template, `with_*` slot completion       |
| `@user`                               | Completion, hover and go to definition from the action and its `before_action`s        |
| `users_path`, `edit_user_url`         | Completion, hover with the URL and `controller#action`, go to `config/routes.rb`       |
| `t(".title")`                         | Completion, hover with each locale's value, go to the locale file, find references     |
| `format_price(…)`                     | Go to the helper's `def` in `app/helpers` or `helper_method`                           |
| `f.text_field :`                      | Form builder methods and model columns from `db/schema.rb`                             |
| `data-controller`, `data-action`      | Stimulus controller names, actions and targets                                         |
| `link_to`, `turbo_frame_tag`, …       | Rails helpers with snippet arguments and hover docs                                    |
| `def show` in a controller            | Go to the action's templates                                                           |

Plus: rename or move a partial in the Explorer and every `render` that points to it is updated; <kbd>F2</kbd> on a partial's strict local renames it in the partial and in every `render` that passes it; <kbd>Ctrl+T</kbd> finds templates by render name (`uscard` → `users/card`); outline and breadcrumbs for every template; Jbuilder and Builder views get partial navigation too.

### Formatting

_Format Document_, _Format Selection_ and format on save for HTML ERB views, Haml and Slim. The formatter changes **only whitespace**: leading indentation, trailing whitespace, blank lines and the final newline. It never wraps or joins lines, never reorders attributes and never touches Ruby code.

- **ERB**: one indent per open HTML element and ERB block (`if`/`else`/`end`, `do … end`, `case`/`when`); multi-line tags move with their first line. `<script>`, `<style>`, `<pre>`, `<textarea>` and `<title>` bodies, quoted attribute values and the continuation lines of multi-line ERB tags keep every byte. Only HTML views are formatted, not `.js.erb`, `.json.erb`, `.text.erb`, Jbuilder or generator templates.
- **Haml** and **Slim**: every level becomes the indentation width in spaces; filter and engine bodies, text blocks and continuation lines move with their line. A file Haml or Slim would reject is left unchanged, and so is any result that wouldn't compile to the same output.
- **Safe by construction**: every result must be stable when formatted again, keep every non-whitespace character and add no diagnostic; otherwise the file comes back unchanged and the reason is logged.
- **Settings**: the indentation width comes from the editor's tab size, then `.editorconfig` (`indent_size`; `tab_width` for `indent_size = tab`), then 2. Indentation is always written in spaces and each line keeps its ending. `rubyTemplates.erb.format.alignComments` (`--align-comments` on the command line) moves the lines inside HTML and multi-line `<%#` comments with the comment's first line; by default they keep every byte.
- **Not supported**: reflowing long lines, indenting embedded JavaScript or CSS by their own syntax, Ruby formatting (use RuboCop), Haml files with errors and tab-indented Slim files.

### Commands

| Command                                  | Keys         | What it does                                                                         |
| ---------------------------------------- | ------------ | ------------------------------------------------------------------------------------ |
| Toggle ERB Tag / Haml Prefix / Slim Line | `Ctrl+Alt+E` | `<% %>` → `<%= %>` → `<%# %>`, and the same for `-` / `=` / comment in Haml and Slim |
| Wrap Selection in ERB / Slim Output      | `Ctrl+Alt+W` | `foo` → `<%= foo %>`, `= foo` or `#{foo}`                                            |
| Extract Selection to Partial             |              | Moves the selection into `_name.html.erb` (or Haml, Slim) and leaves a `render`      |
| Switch to Related File                   |              | View ↔ other formats ↔ controller action; ViewComponent class ↔ template ↔ preview   |
| Preview Rendered HTML                    | `Ctrl+Alt+P` | Renders the template in a side panel, optionally through your Rails app              |
| Show Compiled Ruby Source                |              | The Ruby the template engine generates, for debugging                                |
| Convert ERB ↔ Haml ↔ Slim                |              | Converts the current file between template languages                                 |

Preview and the converters call Ruby gems (`slim`, `haml`, `erubi`, `html2haml`, `haml_to_erb`, `html2slim`, `haml2slim`), installed only if you use them. Everything else works without Ruby.

### Command line for CI

The same checks and formatter run outside the editor, from npm:

```sh
npm install --save-dev @x-ror/ruby-templates
npx ruby-templates check app/views            # errors and warnings; exit 1 if any
npx ruby-templates format --check app/views   # files that would change; exit 1 if any
npx ruby-templates format --write app/views   # format them
npx ruby-templates format --stdin-filepath show.html.erb < show.html.erb   # from stdin, for editors
npx ruby-templates format --verbose --check app/views   # also the settings and .editorconfig files used
```

Exit codes: 0 ok, 1 diagnostics or files that would change, 2 usage or I/O errors. Folders are walked for template files, skipping hidden folders and `node_modules`.

## Getting started

1. Install **Ruby Templates** from the [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=x-ror.ruby-templates) or [Open VSX](https://open-vsx.org/extension/x-ror/ruby-templates).
2. Open a Rails project and any view. That's it: highlighting, checks, navigation and formatting work immediately.
3. Optional: turn on format on save, e.g. `"[erb]": { "editor.formatOnSave": true }` (also `[haml]`, `[slim]`).

Requires VS Code 1.100 or newer, on Linux (x64, arm64), macOS (Apple silicon, Intel) or Windows (x64).

## Settings

| Setting                                                           | Default                    | Purpose                                                            |
| ----------------------------------------------------------------- | -------------------------- | ------------------------------------------------------------------ |
| `rubyTemplates.<erb\|haml\|slim>.diagnostics.enabled`             | `true`                     | Show a language's diagnostics                                      |
| `rubyTemplates.<erb\|haml\|slim\|jbuilder>.disabledDiagnostics`   | `[]`                       | Turn off single checks by code, e.g. `["unclosed-element"]`        |
| `rubyTemplates.<erb\|haml\|slim>.format.enabled`                  | `true`                     | Use the built-in formatter (off lets another extension format)     |
| `rubyTemplates.erb.format.alignComments`                          | `false`                    | Move lines inside ERB comments with the comment's first line       |
| `rubyTemplates.erb.autoCloseTags`                                 | `true`                     | Close HTML tags when typing `>`                                    |
| `rubyTemplates.routes.enabled`                                    | `true`                     | Read routes with `bin/rails runner` (trusted workspaces only)      |
| `rubyTemplates.slim.extraEngines`                                 | `[]`                       | Extra Slim embedded engines your app registers (e.g. `mjml`)       |
| `rubyTemplates.preview.useRails` / `useBundler` / `refreshOnSave` | `false` / `false` / `true` | Render the preview through the app, through `bundle exec`, on save |
| `rubyTemplates.commands.*`                                        |                            | Paths of the Ruby tools used by preview and converters             |
| `rubyTemplates.server.enabled` / `server.path`                    | `true` / bundled           | Run the language server, or use your own build                     |

## Compatibility

- **Ruby LSP** and this extension both register the `erb` language id and work side by side: Ruby LSP for `.rb` files, this extension for templates.
- **Other Haml or Slim extensions**: VS Code picks one grammar per language, so disable the other extension to avoid surprises. To let another extension format, turn off `rubyTemplates.<lang>.format.enabled`.
- **Emmet** works in ERB out of the box (`"emmet.includeLanguages": { "erb": "html" }` is a default); VS Code's Emmet expands to Slim syntax in Slim and Skim.
- **Partials and formats**: partials resolve by Rails' default formats plus the ones your app registers with `Mime::Type.register` in `config/initializers`; renders into a gem's views (`devise/…`) aren't flagged when the gem is in `Gemfile.lock`.
- **Generator templates** (`lib/templates/**/*.tt`, `lib/generators/**/templates/**`) open in their template language, without diagnostics or formatting, since the generator's own ERB is mixed in.
- **Slim options set per app** aren't reflected in highlighting: custom shortcuts and attribute delimiters, smart text's implicit text lines, logic-less mode.

## Learn more

- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md): building, testing, project layout.

Found a bug or missing a Rails feature? [Open an issue](https://github.com/x-ror/vscode-ruby-templates/issues).

## License

[MIT](LICENSE)
