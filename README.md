<p align="center"><img src="images/icon.png" alt="Ruby Templates" width="128"></p>

# Ruby Templates: ERB, Haml, Slim

[![VS Marketplace](https://vsmarketplacebadges.dev/version-short/x-ror.ruby-templates.svg)](https://marketplace.visualstudio.com/items?itemName=x-ror.ruby-templates)
[![Open VSX](https://img.shields.io/open-vsx/v/x-ror/ruby-templates?label=Open%20VSX)](https://open-vsx.org/extension/x-ror/ruby-templates)
[![npm](https://img.shields.io/npm/v/@x-ror/ruby-templates?label=npm%20CLI)](https://www.npmjs.com/package/@x-ror/ruby-templates)
[![Socket](https://socket.dev/api/badge/npm/package/@x-ror/ruby-templates)](https://socket.dev/npm/package/@x-ror/ruby-templates)
[![build](https://github.com/x-ror/vscode-ruby-templates/actions/workflows/build.yml/badge.svg)](https://github.com/x-ror/vscode-ruby-templates/actions/workflows/build.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Rails view tooling for VS Code: one extension for ERB, Haml and Slim.** It highlights your templates, finds errors as you type, formats them, and understands your Rails app: partials, components, helpers, routes, instance variables, translations and Stimulus controllers.

Built for Rails developers who work in `app/views` every day, and for teams that want the same template checks and formatting in the editor and in CI.

- **No gems to install.** The checks and the formatter are a fast language server written in Rust and bundled with the extension. They follow Erubi, Rails, Haml 7 and Slim 5 and report errors with those projects' own messages.
- **Rails-aware.** <kbd>F12</kbd> on `render "users/card"` opens the partial; completion knows your partials, ViewComponents, model columns, `*_path` helpers, `@ivars` from the controller and `t(".keys")`.
- **Safe formatting.** It changes only whitespace and checks that the template still compiles to the same thing; if it is unsure, it leaves the file alone.
- **Works with Ruby LSP.** Install both: Ruby LSP keeps your `.rb` files, this extension adds what templates need.

## Getting started

1. Install **Ruby Templates** from the [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=x-ror.ruby-templates) or [Open VSX](https://open-vsx.org/extension/x-ror/ruby-templates).
2. Open a Rails project and any view. That's it: highlighting, checks, navigation and formatting work immediately.
3. Optional: turn on format on save, e.g. `"[erb]": { "editor.formatOnSave": true }` (also `[haml]`, `[slim]`).

Requires VS Code 1.100 or newer, on Linux (x64, arm64), macOS (Apple silicon, Intel) or Windows (x64).

## Documentation

The full documentation is at [x-ror.github.io/vscode-ruby-templates](https://x-ror.github.io/vscode-ruby-templates/):

- [Getting started](https://x-ror.github.io/vscode-ruby-templates/getting-started/)
- Guides: [errors as you type](https://x-ror.github.io/vscode-ruby-templates/guides/checks/), [Rails navigation and completion](https://x-ror.github.io/vscode-ruby-templates/guides/rails-navigation/), [file types](https://x-ror.github.io/vscode-ruby-templates/guides/file-types/), [formatting](https://x-ror.github.io/vscode-ruby-templates/guides/formatting/), [commands](https://x-ror.github.io/vscode-ruby-templates/guides/commands/), [command line for CI](https://x-ror.github.io/vscode-ruby-templates/guides/command-line/), [settings](https://x-ror.github.io/vscode-ruby-templates/guides/settings/), [supported syntax](https://x-ror.github.io/vscode-ruby-templates/guides/syntax/), [compatibility](https://x-ror.github.io/vscode-ruby-templates/guides/compatibility/) and [troubleshooting](https://x-ror.github.io/vscode-ruby-templates/guides/troubleshooting/)
- [Diagnostics reference](https://x-ror.github.io/vscode-ruby-templates/reference/diagnostics/): every check, with its code
- [Changelog](https://x-ror.github.io/vscode-ruby-templates/changelog/)

[Contributing](CONTRIBUTING.md) covers building, testing and the project layout. Found a bug or missing a Rails feature? [Open an issue](https://github.com/x-ror/vscode-ruby-templates/issues).

## License

[MIT](LICENSE)
