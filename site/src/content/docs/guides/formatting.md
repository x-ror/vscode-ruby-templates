---
title: Formatting
description: How the built-in formatter re-indents ERB, Haml and Slim templates while changing only whitespace.
sidebar:
  order: 6
---

_Format Document_, _Format Selection_ and format on save work for HTML ERB views, Haml and Slim. The formatter changes **only whitespace**: leading indentation, trailing whitespace, blank lines and the final newline. It never wraps or joins lines, never reorders attributes and never touches Ruby code.

## ERB

One indent per open HTML element and ERB block (`if`/`else`/`end`, `do … end`, `case`/`when`); multi-line tags move with their first line.

```erb title="Before"
<ul>
<% @users.each do |user| %>
<li><%= user.name %></li>
<% end %>
</ul>
```

```erb title="After"
<ul>
  <% @users.each do |user| %>
    <li><%= user.name %></li>
  <% end %>
</ul>
```

`<script>`, `<style>`, `<pre>`, `<textarea>` and `<title>` bodies, quoted attribute values and the continuation lines of multi-line ERB tags keep every byte.

Only HTML views are formatted, not `.js.erb`, `.json.erb`, `.text.erb`, Jbuilder or generator templates. [File types](/vscode-ruby-templates/guides/file-types/) shows which files that covers.

## Haml and Slim

Every level becomes the indentation width in spaces; filter and engine bodies, text blocks and continuation lines move with their line.

```slim title="Before"
ul
    li
        a href="/" Home
```

```slim title="After"
ul
  li
    a href="/" Home
```

A file Haml or Slim would reject is left unchanged, and so is any result that wouldn't compile to the same output.

The [rules reference](/vscode-ruby-templates/reference/rules/) lists the formatting rules the extension has and the ones planned, which will be configured in a `.ruby-templates.toml`.

## Safe by construction

Every result must be stable when formatted again, keep every non-whitespace character and add no diagnostic. Otherwise the file comes back unchanged and the reason is logged in the _Ruby Templates_ channel of the Output panel.

## Indentation width and line endings

The indentation width comes from the editor's tab size, then `.editorconfig` (`indent_size`; `tab_width` for `indent_size = tab`), then 2. Indentation is always written in spaces and each line keeps its ending.

```text title=".editorconfig"
[*.{erb,haml,slim}]
indent_size = 2
```

## Comments in ERB

`rubyTemplates.erb.format.alignComments` (`--align-comments` on the command line) moves the lines inside HTML and multi-line `<%#` comments with the comment's first line. By default they keep every byte.

```json
{
  "rubyTemplates.erb.format.alignComments": true
}
```

## Format on save and other formatters

Turn on format on save per language:

```json
{
  "[erb]": { "editor.formatOnSave": true },
  "[haml]": { "editor.formatOnSave": true },
  "[slim]": { "editor.formatOnSave": true }
}
```

If another extension also formats these languages, pick this one with `"[erb]": { "editor.defaultFormatter": "x-ror.ruby-templates" }` (also `[haml]`, `[slim]`). To let the other one format instead, set `rubyTemplates.<erb|haml|slim>.format.enabled` to `false`. See [Troubleshooting](/vscode-ruby-templates/guides/troubleshooting/).

The formatter also runs in CI with `npx ruby-templates format --check`; see [Command line for CI](/vscode-ruby-templates/guides/command-line/).

## Not supported

- Reflowing long lines.
- Indenting embedded JavaScript or CSS by their own syntax.
- Ruby formatting (use RuboCop).
- Haml files with errors.
- Tab-indented Slim files.
