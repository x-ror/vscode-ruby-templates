---
title: Commands
description: The Ruby Templates commands, their keyboard shortcuts, and which ones need Ruby gems.
sidebar:
  order: 7
---

Run any command from the Command Palette (<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd>, or <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd> on macOS); they are listed under _Ruby Templates_.

## Editing commands

| Command                                  | Keys (Windows, Linux)                       | Keys (macOS)                               | What it does                                                                         |
| ---------------------------------------- | ------------------------------------------- | ------------------------------------------ | ------------------------------------------------------------------------------------ |
| Toggle ERB Tag / Haml Prefix / Slim Line | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>E</kbd> | <kbd>Cmd</kbd>+<kbd>Alt</kbd>+<kbd>E</kbd> | `<% %>` → `<%= %>` → `<%# %>`, and the same for `-` / `=` / comment in Haml and Slim |
| Wrap Selection in ERB / Slim Output      | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>W</kbd> | <kbd>Cmd</kbd>+<kbd>Alt</kbd>+<kbd>W</kbd> | `foo` → `<%= foo %>`, `= foo` or `#{foo}`                                            |
| Extract Selection to Partial             | none                                        | none                                       | Moves the selection into `_name.html.erb` (or Haml, Slim) and leaves a `render`      |
| Switch to Related File                   | none                                        | none                                       | View ↔ other formats ↔ controller action; ViewComponent class ↔ template ↔ preview   |
| Re-indent Haml (Document or Selection)   | none                                        | none                                       | Re-indents a Haml document or the selection                                          |

In ERB, <kbd>Ctrl</kbd>+<kbd>/</kbd> (<kbd>Cmd</kbd>+<kbd>/</kbd> on macOS) is bound to the extension's line comment toggle, which follows the file's host language; see [Comment toggling](/vscode-ruby-templates/guides/file-types/#comment-toggling).

### Toggle a tag

_Toggle ERB Tag_ cycles the tag under the cursor:

```erb
<% user.name %>
<%= user.name %>
<%# user.name %>
```

In Haml it cycles the line prefix (`-`, `=`, `-#`), and in Slim the line indicator (`-`, `=`, `==`, `/`).

### Extract a partial

_Extract Selection to Partial_ works the same in ERB, Haml and Slim: the new partial and the `render` call are one edit, so a single undo reverts both. The name must be a Ruby identifier, optionally prefixed by folders (`shared/card`), and the render path is qualified (`render "users/card"`). An existing partial is overwritten only after you confirm.

## Preview and compiled source

| Command                   | Keys (Windows, Linux)                       | Keys (macOS)                               | What it does                                                            |
| ------------------------- | ------------------------------------------- | ------------------------------------------ | ----------------------------------------------------------------------- |
| Preview Rendered HTML     | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>P</kbd> | <kbd>Cmd</kbd>+<kbd>Alt</kbd>+<kbd>P</kbd> | Renders the template in a side panel, optionally through your Rails app |
| Show Compiled Ruby Source | none                                        | none                                       | The Ruby the template engine generates, for debugging                   |

The preview is controlled by three settings:

- `rubyTemplates.preview.useRails` renders through the app's `bin/rails runner` when the file is inside a Rails app, so helpers, routes and partials work. It boots the app on every refresh and gives up after 60 seconds.
- `rubyTemplates.preview.useBundler` prefixes render commands with `bundle exec` when a Gemfile exists in the workspace.
- `rubyTemplates.preview.refreshOnSave` re-renders the preview whenever the template is saved.

`rubyTemplates.slim.compileForRails` makes _Show Compiled Ruby Source_ for Slim run `slimrb --rails`, as Rails compiles it.

## Converters

| Command                 | What it does                      |
| ----------------------- | --------------------------------- |
| Convert ERB/HTML → Haml | Converts the current file to Haml |
| Convert Haml/Slim → ERB | Converts the current file to ERB  |
| Convert ERB/Haml → Slim | Converts the current file to Slim |

## Language server

_Restart Language Server_ restarts the language server. Its log is the _Ruby Templates_ channel in the Output panel.

## Ruby gems

Preview and the converters call Ruby gems, installed only if you use them:

| Gem           | Used for                                                           | Setting for the executable         |
| ------------- | ------------------------------------------------------------------ | ---------------------------------- |
| `slim`        | Rendering Slim                                                     | `rubyTemplates.commands.slimrb`    |
| `haml`        | Rendering Haml                                                     | `rubyTemplates.commands.haml`      |
| `erubi`       | Previewing and compiling ERB through Erubi (stdlib ERB without it) | `rubyTemplates.commands.ruby`      |
| `haml_to_erb` | Haml → ERB                                                         | `rubyTemplates.commands.ruby`      |
| `html2haml`   | ERB → Haml                                                         | `rubyTemplates.commands.html2haml` |
| `html2slim`   | ERB → Slim                                                         | `rubyTemplates.commands.erb2slim`  |
| `haml2slim`   | Haml → Slim                                                        | `rubyTemplates.commands.haml2slim` |

Everything else works without Ruby.
