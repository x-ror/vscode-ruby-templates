---
title: Settings
description: Every rubyTemplates setting with its default value and what it does.
sidebar:
  order: 9
---

All settings live under `rubyTemplates.` and can be set per user or per workspace. For example, in `.vscode/settings.json`:

```json
{
  "rubyTemplates.erb.disabledDiagnostics": ["Html/ImgAlt"],
  "rubyTemplates.slim.extraEngines": ["mjml"],
  "rubyTemplates.preview.useRails": true
}
```

A `.ruby-templates.toml` in the project sets the diagnostics, formatting and Slim engine settings below too, and wins over them where it says something; see the [rules reference](/vscode-ruby-templates/reference/rules/#configuration).

Editor settings for one template language use its language id, such as `[erb]`, `[haml]`, `[slim]` or `[erb-js]`; [File types](/vscode-ruby-templates/guides/file-types/) lists the ids.

## Diagnostics

| Setting                                      | Default | Description                                                                                  |
| -------------------------------------------- | ------- | -------------------------------------------------------------------------------------------- |
| `rubyTemplates.erb.disabledDiagnostics`      | `[]`    | Built-in ERB diagnostics to turn off, by rule name.                                          |
| `rubyTemplates.haml.disabledDiagnostics`     | `[]`    | Built-in Haml diagnostics to turn off, by rule name.                                         |
| `rubyTemplates.jbuilder.disabledDiagnostics` | `[]`    | Built-in diagnostics to turn off in Jbuilder and Builder views, by rule name.                |
| `rubyTemplates.slim.disabledDiagnostics`     | `[]`    | Built-in Slim diagnostics to turn off, by rule name.                                         |
| `rubyTemplates.erb.diagnostics.enabled`      | `true`  | Show the language server's diagnostics in ERB files. Turn off to silence them all.           |
| `rubyTemplates.haml.diagnostics.enabled`     | `true`  | Show the language server's diagnostics in Haml files. Turn off to silence them all.          |
| `rubyTemplates.slim.diagnostics.enabled`     | `true`  | Show the language server's diagnostics in Slim and Skim files. Turn off to silence them all. |

Each `disabledDiagnostics` setting takes the rule names of that language's checks (`Html/ImgAlt`); the [diagnostics reference](/vscode-ruby-templates/reference/diagnostics/) lists them. [Errors as you type](/vscode-ruby-templates/guides/checks/#turning-checks-off) shows examples.

## Formatting

| Setting                                  | Default | Description                                                                                                                                                                                                        |
| ---------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `rubyTemplates.erb.format.enabled`       | `true`  | Let the language server format ERB files (Format Document and Format Selection with the built-in formatter). Turn off for no edits.                                                                                |
| `rubyTemplates.erb.format.alignComments` | `false` | Move the lines inside an ERB template's HTML comments (`<!-- … -->`) and multi-line `<%# … %>` comments with the line the comment opens on, keeping their relative indentation. Off: those lines keep their bytes. |
| `rubyTemplates.haml.format.enabled`      | `true`  | Let the language server format Haml files (Format Document and Format Selection). Turn off for no edits.                                                                                                           |
| `rubyTemplates.slim.format.enabled`      | `true`  | Let the language server format Slim and Skim files (Format Document and Format Selection). Turn off for no edits.                                                                                                  |

The indentation width isn't a setting of its own: it comes from the editor's tab size, then `.editorconfig`, then 2. See [Formatting](/vscode-ruby-templates/guides/formatting/#indentation-width-and-line-endings).

## Editing

| Setting                           | Default | Description                                                                                                                 |
| --------------------------------- | ------- | --------------------------------------------------------------------------------------------------------------------------- |
| `rubyTemplates.erb.autoCloseTags` | `true`  | Typing the `>` of an HTML start tag in ERB inserts its end tag after the cursor (like html.autoClosingTags for plain HTML). |

## Rails

| Setting                        | Default | Description                                                                                                                                                                                                          |
| ------------------------------ | ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rubyTemplates.routes.enabled` | `true`  | Read the app's routes with `bin/rails runner` (boots the app once, and again when config/routes.rb or config/routes/*.rb changes) for completion, hover and go to definition of `*_path` / `*_url` helpers in views. |

Routes are read in trusted workspaces only.

## Slim

| Setting                           | Default | Description                                                                                             |
| --------------------------------- | ------- | ------------------------------------------------------------------------------------------------------- |
| `rubyTemplates.slim.extraEngines` | `[]`    | Embedded engine names registered by your app (e.g. `mjml`) that the unknown-engine check should accept. |

`.skim` files use the `slim` settings.

## Preview and compiled source

| Setting                               | Default | Description                                                                                                                                                                                                                                       |
| ------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rubyTemplates.slim.compileForRails`  | `false` | Show Compiled Ruby Source for Slim runs `slimrb --rails`: ActionView output buffer and no capture, as Rails compiles it.                                                                                                                          |
| `rubyTemplates.preview.refreshOnSave` | `true`  | Re-render the HTML preview whenever the template is saved.                                                                                                                                                                                        |
| `rubyTemplates.preview.useBundler`    | `false` | Prefix render commands with `bundle exec` when a Gemfile exists in the workspace.                                                                                                                                                                 |
| `rubyTemplates.preview.useRails`      | `false` | Render the preview through the app's `bin/rails runner` (`ApplicationController.render(inline:, type:)`) when the file is inside a Rails app, so helpers, routes and partials work. Boots the app on every refresh and gives up after 60 seconds. |

## Ruby tools

| Setting                            | Default       | Description                                                                                                                                                                 |
| ---------------------------------- | ------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rubyTemplates.commands.slimrb`    | `"slimrb"`    | Executable used to render Slim (gem install slim).                                                                                                                          |
| `rubyTemplates.commands.haml`      | `"haml"`      | Executable used to render Haml (gem install haml).                                                                                                                          |
| `rubyTemplates.commands.html2haml` | `"html2haml"` | Executable for ERB → Haml (gem install html2haml).                                                                                                                          |
| `rubyTemplates.commands.haml2slim` | `"haml2slim"` | Executable for Haml → Slim (gem install haml2slim).                                                                                                                         |
| `rubyTemplates.commands.erb2slim`  | `"erb2slim"`  | Executable for ERB → Slim (gem install html2slim).                                                                                                                          |
| `rubyTemplates.commands.ruby`      | `"ruby"`      | Ruby used to preview and compile ERB through Erubi (gem install erubi; stdlib ERB without it) and for Haml → ERB through the haml_to_erb library (gem install haml_to_erb). |

These are the executables that preview, _Show Compiled Ruby Source_ and the converters call; see [Commands](/vscode-ruby-templates/guides/commands/#ruby-gems).

## Language server

| Setting                        | Default | Description                                                                                                                    |
| ------------------------------ | ------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `rubyTemplates.server.enabled` | `true`  | Run the Rust language server (diagnostics, go to partial, completions, hover, and the built-in ERB, Haml and Slim formatting). |
| `rubyTemplates.server.path`    | `""`    | Path to the ruby-templates-ls binary. Leave empty to use the bundled one.                                                      |
