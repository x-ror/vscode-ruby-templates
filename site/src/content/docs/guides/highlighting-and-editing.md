---
title: Highlighting and editing
description: Syntax highlighting, semantic colours, tag closing, linked editing, snippets and Emmet for ERB, Haml, Slim, Skim and Liquid.
sidebar:
  order: 4
---

## Syntax highlighting

Full syntax highlighting for ERB (`.html.erb`, `.erubi`, `.js.erb`, `.css.erb`, `.json.erb`, `.yml.erb`, …), Haml, Slim, Skim and Liquid, including embedded JavaScript, CSS, SCSS, CoffeeScript and Markdown blocks. Every code example on this site is highlighted with the extension's own grammars.

```slim title="app/views/users/show.html.slim"
.card
  h2 = @user.name
  p Joined #{time_ago_in_words(@user.created_at)} ago
javascript:
  document.querySelector(".card").classList.add("is-loaded")
```

Which file gets which grammar is listed in [File types](/vscode-ruby-templates/guides/file-types/); what each grammar recognises is in [Supported syntax](/vscode-ruby-templates/guides/syntax/).

## Semantic colours

Some Ruby locals look the same to a grammar, so the language server colours them in ERB, Haml and Slim:

- uses of strict locals get the `parameter` token type;
- block variables (`|item|`) and their uses get the `variable` token type.

```erb title="app/views/orders/_summary.html.erb"
<%# locals: (order:) %>
<% order.items.each do |item| %>
  <li><%= item.name %></li>
<% end %>
```

Here `order` is coloured as a parameter and `item` as a variable; the declaration in the `locals:` comment is left alone. Only those identifiers get a token, so the grammar's colours of everything else stay. Slim has no `locals:` comment, so in Slim only block variables and assignments are coloured. Turn semantic colours off with VS Code's `editor.semanticHighlighting.enabled`.

## Editing

- Comment toggling, bracket matching, auto-closing `<% %>`, smart indentation and folding.
- Typing `>` in ERB closes the HTML tag. Turn this off with [`rubyTemplates.erb.autoCloseTags`](/vscode-ruby-templates/guides/settings/#editing).
- Renaming `<div>` renames its `</div>` (linked editing).

The comment syntax depends on the file type: <kbd>Ctrl</kbd>+<kbd>/</kbd> uses `//` in `.js.erb`, `#` in `.yml.erb` and so on, and inside a multi-line `<% %>` tag it toggles a Ruby `# `. [File types](/vscode-ruby-templates/guides/file-types/#comment-toggling) has the full list.

## Snippets

More than 50 snippets per language for Rails, Hotwire and ViewComponent, for example `link_to`, `render`, `form_with`, `ff`, `turbo_frame`, `ts-append`, `stimulus`, `locals` and `flash`.

## Emmet

Emmet works in ERB out of the box (`"emmet.includeLanguages": { "erb": "html" }` is a default). In Slim and Skim, VS Code's Emmet expands to Slim syntax.

## Commands for editing

_Toggle ERB Tag_, _Wrap Selection in ERB Output_ and the Haml and Slim equivalents switch between code, output and comment tags; see [Commands](/vscode-ruby-templates/guides/commands/).
