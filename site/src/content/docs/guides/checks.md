---
title: Errors as you type
description: What the language server checks in ERB, Haml and Slim templates, and how to turn single checks or whole languages off.
sidebar:
  order: 2
---

The language server checks every template as you type and reports problems in the editor and the _Problems_ panel. The checks follow Erubi, Rails, Haml 7 and Slim 5, and Haml, Slim and Rails errors use those projects' own messages. No gems are needed.

Every check has a code. The [diagnostics reference](/vscode-ruby-templates/reference/diagnostics/) lists each code with what it means.

## ERB

- Ruby syntax errors inside tags, checked with Prism, Ruby's own parser, for your project's Ruby version.
- An unclosed `<%`.
- Blocks without `<% end %>`.
- Mismatched or unclosed HTML elements.
- `<img>` without `alt`.
- Duplicate ids.

In this view the block has no `<% end %>` and the image has no `alt`:

```erb title="app/views/users/index.html.erb"
<% @users.each do |user| %>
  <div class="card">
    <img src="<%= user.avatar_url %>">
  </div>
```

Both are reported: the first as an error from Prism (`unexpected end-of-input, assuming it is closing the parent top level context`), the second as a warning (`` `<img>` without `alt`: the alt attribute must be specified (HTML 4.8.4.4.1) ``).

The HTML checks (tags, `alt`, duplicate `id`) apply only to ERB files with an HTML host, such as `.html.erb`. `.js.erb`, `.yml.erb` and the other host types get the ERB and Ruby checks; see [File types](/vscode-ruby-templates/guides/file-types/). At most 20 tag diagnostics are reported per file.

## Haml and Slim

The errors Haml and Slim themselves would raise, at the same line and column:

- indentation
- illegal nesting
- unclosed attributes
- unknown filters (Haml) and engines (Slim)
- an explicit `end`

Ruby syntax errors in code lines, attribute hashes and `#{}` are reported too.

```haml title="app/views/users/show.html.haml"
%ul
  %li= @user.name
     %span Admin
```

The third line is reported with Haml's own message: `Inconsistent indentation: 5 spaces used for indentation, but the rest of the document was indented using 2 spaces.`

Slim engines your app registers itself (for example `mjml:`) can be accepted with [`rubyTemplates.slim.extraEngines`](/vscode-ruby-templates/guides/settings/#slim).

## Rails checks

Two Rails checks run in ERB, Haml and Slim views, and in Jbuilder and Builder views. They report with Rails' own messages:

- `render` of a partial that doesn't exist (`unresolved-render`), with a quick fix that creates the partial.
- Mismatches with a partial's strict locals (`strict-locals`), declared with a comment such as `<%# locals: (user:) %>`.

```erb title="app/views/users/_card.html.erb"
<%# locals: (user:) %>
<p><%= user.name %></p>
```

```erb title="app/views/users/show.html.erb"
<%= render "users/card", user: @user %>
```

Here the `render` passes the `user:` the partial declares. A `render` of `users/card` that passes something else, or a partial name with no file behind it, is reported.

How partials are looked up in engines, packs, components and gems, and when the missing-partial check stays off, is described in [Compatibility](/vscode-ruby-templates/guides/compatibility/#partials-and-formats).

The [rules reference](/vscode-ruby-templates/reference/rules/) also covers the checks that are planned, such as a `render` in a tag that outputs nothing, and unsafe output.

## Turning checks off

Turn single checks off in a `.ruby-templates.toml` at the project root, by rule name. The editor and `ruby-templates check` read it, and it wins over the settings:

```toml
[Html.ImgAlt]
enabled = false

[Html.DuplicateId]
exclude = ["app/views/legacy/**"]
```

Or by code, per language, in your user or workspace settings (the kebab-case codes of earlier versions, such as `img-missing-alt`, still work):

```json
{
  "rubyTemplates.erb.disabledDiagnostics": ["Html/ImgAlt", "Html/DuplicateId"],
  "rubyTemplates.haml.disabledDiagnostics": ["Syntax/UnknownFilter"],
  "rubyTemplates.slim.disabledDiagnostics": ["Syntax/UnknownEngine"],
  "rubyTemplates.jbuilder.disabledDiagnostics": ["Rails/MissingPartial"]
}
```

`rubyTemplates.jbuilder.disabledDiagnostics` covers Jbuilder and Builder views.

Turn a whole language's diagnostics off:

```json
{
  "rubyTemplates.haml.diagnostics.enabled": false
}
```

`rubyTemplates.slim.diagnostics.enabled` also covers Skim files. All settings are listed in [Settings](/vscode-ruby-templates/guides/settings/).

The same checks run outside the editor with [the command line](/vscode-ruby-templates/guides/command-line/).
