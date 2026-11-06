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

The [rules reference](/vscode-ruby-templates/reference/rules/) lists the formatting rules the extension has and the ones planned, which are configured in a `.ruby-templates.toml`.

## Blank lines

Rules in a `.ruby-templates.toml` decide what happens to blank lines; none runs on _Format Selection_.

- `Layout/TrailingEmptyLines` is on for Haml and Slim: the file ends with exactly one line break. The blank lines at the end go, and a missing final line break is added in the file's line ending. It is off for ERB, because Rails renders ERB byte for byte and the end of a partial is output (a final line break shows as a space between inline elements); turn it on with `enabled = true`.
- `Layout/EmptyLines` is off. Turn it on to cut every run of blank lines longer than `max` (1 by default) down to `max`.
- `Layout/EmptyLinesAroundBlockBody`, `Layout/EmptyLinesAroundElementBody` and `Layout/EmptyLinesAroundBranches` are off. Each takes `style`: `no_empty_lines` (the default) removes the blank lines at the start and end of a body (`<% if %>`, `<% x.each do %>`, `<div>`, `%div`, `div` and what they contain) or before and after `else`, `elsif`, `when`, `in`, `rescue` and `ensure`; `empty_lines` keeps exactly one there and adds it where there is none. The end of a body is the closing tag or `<% end %>`, so it only exists in ERB: in Haml and Slim the rules set the blank lines after the opening line and around branch lines, and leave the ones before a dedent alone, since they separate siblings. A body with nothing in it (`<div>` directly above `</div>`, `- if a` above `- else`) loses its blank lines under `no_empty_lines` and is left alone under `empty_lines`; a body that ends the file has no closing line, so nothing is added there.
- `max` of `Layout/EmptyLines` is applied after these rules, so with `max = 0` no blank line is left and `empty_lines` has no effect.

```toml
[Layout.EmptyLines]
enabled = true
max = 1

[Layout.EmptyLinesAroundBlockBody]
enabled = true
style = "no_empty_lines"

[Layout.TrailingEmptyLines]
enabled = false
```

Blank lines stay where whitespace counts: `<pre>`, `<xmp>`, `<listing>`, `<textarea>`, `<script>`, `<style>`, quoted attribute values, comments and the Ruby inside ERB tags; Haml filters; Slim text blocks and engine bodies. For Haml and Slim, the formatter also compares what the template compiles to before and after, ignoring line numbers, and keeps the blank lines of the file when removing or adding them would change it.

## Safe by construction

Every result must be stable when formatted again, keep every non-whitespace character and add no diagnostic. Otherwise the file comes back unchanged and the reason is logged in the _Ruby Templates_ channel of the Output panel.

## Indentation width and line endings

`format.enabled` and `format.align_comments` under `[erb]`, `[haml]` and `[slim]` in a `.ruby-templates.toml` turn formatting off or on and align comments, and win over the `rubyTemplates.*` settings; its global `exclude` leaves files unformatted.

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
