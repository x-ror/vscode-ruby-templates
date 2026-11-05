---
title: Getting started
description: Install Ruby Templates, open a Rails view and, if you want, turn on format on save.
sidebar:
  order: 1
---

Ruby Templates is one VS Code extension for Rails views in ERB, Haml and Slim. It highlights your templates, finds errors as you type, formats them, and understands your Rails app: partials, components, helpers, routes, instance variables, translations and Stimulus controllers.

## Install

Install **Ruby Templates** from the [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=x-ror.ruby-templates) or from [Open VSX](https://open-vsx.org/extension/x-ror/ruby-templates).

There are no gems to install. The checks and the formatter are a language server written in Rust and bundled with the extension.

## Open a view

Open a Rails project and any view. Highlighting, checks, navigation and formatting work immediately.

While a template is open, the status bar shows its template language (`ERB`, `HAML`, `SLIM`, `ERB-JS`, …). If that item is missing, the file isn't one of the extension's; see [Troubleshooting](/vscode-ruby-templates/guides/troubleshooting/).

## Format on save

Format on save is optional. Turn it on per language in your settings:

```json
{
  "[erb]": { "editor.formatOnSave": true },
  "[haml]": { "editor.formatOnSave": true },
  "[slim]": { "editor.formatOnSave": true }
}
```

The formatter changes only whitespace; [Formatting](/vscode-ruby-templates/guides/formatting/) explains what it does and doesn't touch.

## Several Rails apps in one window

Multi-root workspaces and monorepos work side by side: each file resolves partials, routes, translations and gems against the app it belongs to.

## Requirements

- VS Code 1.100 or newer.
- Linux (x64, arm64), macOS (Apple silicon, Intel) or Windows (x64).
- Ruby only for _Preview Rendered HTML_, _Show Compiled Ruby Source_ and the converters, which call Ruby gems you install only if you use them. Everything else works without Ruby. See [Commands](/vscode-ruby-templates/guides/commands/).

## Next steps

- [Errors as you type](/vscode-ruby-templates/guides/checks/) and the [list of diagnostics](/vscode-ruby-templates/reference/diagnostics/)
- [Rails navigation and completion](/vscode-ruby-templates/guides/rails-navigation/)
- [Command line for CI](/vscode-ruby-templates/guides/command-line/)
- [Settings](/vscode-ruby-templates/guides/settings/)
