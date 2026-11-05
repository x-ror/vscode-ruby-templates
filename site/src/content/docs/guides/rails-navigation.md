---
title: Rails navigation and completion
description: Go to definition, hover, completion and renames that follow your Rails app's partials, components, instance variables, routes, translations, helpers and Stimulus controllers.
sidebar:
  order: 3
---

The language server reads your Rails app, so navigation and completion in templates point at real files: partials, ViewComponents, controllers, routes, locale files, helpers, `db/schema.rb` and Stimulus controllers. Go to definition is <kbd>F12</kbd>; hover, completion and find all references work as everywhere else in VS Code.

| Where                                 | What you get                                                                                                 |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `render "users/card"`, `render @user` | Go to the partial, hover shows the file, completion of partial names, find all renders                       |
| `render ButtonComponent.new`          | Completion of ViewComponents, go to class and template, `with_*` slot completion                             |
| `@user`                               | Completion, hover and go to definition from the action and its `before_action`s                              |
| `users_path`, `edit_user_url`         | Completion, hover with the URL and `controller#action`, go to `config/routes.rb`                             |
| `t(".title")`                         | Completion, hover with each locale's value, go to the locale file, find references                           |
| `format_price(…)`                     | Go to the helper's `def` in `app/helpers` or `helper_method`, hover with its signature and comment           |
| `f.text_field :`                      | Form builder methods and model columns from `db/schema.rb`                                                   |
| `data-controller`, `data-action`      | Stimulus controller names, actions, targets, and the element's `-target`/`-value`/`-class` attributes        |
| `link_to`, `turbo_frame_tag`, …       | Rails helpers with snippet arguments and hover docs                                                          |
| `def show` in a controller            | Go to the action's templates                                                                                 |
| `user` in `@users.each do \|user\|`   | Go to the block parameter or the assignment that declares it                                                 |
| `user`, `item` (locals)               | Semantic colours for uses of strict locals (`parameter`) and block variables (`variable`) over the grammar's |
| `report.title`                        | Column type, `null:` and `default:` on hover, go to `db/schema.rb`                                           |

The sections below go through each one.

## Partials and `render`

```erb title="app/views/users/show.html.erb"
<%= render "users/card", user: @user %>
<%= render @user %>
```

- <kbd>F12</kbd> on the render name opens the partial.
- Hover shows the file.
- Inside `render "…"`, completion offers your partial names.
- Find all references on a partial lists every `render` of it.

A `dir/name` render missing from `app/views` is also looked up in the app's engines, packs and components, then in installed gems' views; [Compatibility](/vscode-ruby-templates/guides/compatibility/#partials-and-formats) has the details. Jbuilder and Builder views get partial navigation too.

### Rename or move a partial

Rename or move a partial in the Explorer and every `render` that points to it is updated.

### Rename a strict local

<kbd>F2</kbd> on a partial's strict local renames it in the partial and in every `render` that passes it.

```erb title="app/views/users/_card.html.erb"
<%# locals: (user:) %>
<p><%= user.name %></p>
```

Renaming `user` here also renames the `user:` key in `render "users/card", user: @user`.

## ViewComponents

```erb
<%= render ButtonComponent.new(label: "Save") %>
```

Completion offers your ViewComponents. Go to definition opens the component class or its template, and `with_*` slots are completed. _Switch to Related File_ moves between a component's class, template and preview; see [Commands](/vscode-ruby-templates/guides/commands/).

## Instance variables

`@user` gets completion, hover and go to definition from the controller action and its `before_action`s.

```slim title="app/views/users/show.html.slim"
h1 = @user.name
```

Views in a folder without a controller (`shared/`, `layouts/`) get the instance variables that `ApplicationController` sets in `before_action`s that run for every action; mailer layouts and `*_mailer` folders get `ApplicationMailer`'s.

## Routes

`users_path`, `edit_user_url` and the other route helpers get completion, hover with the URL and `controller#action`, and go to definition in `config/routes.rb`.

```haml
= link_to "Edit", edit_user_path(@user)
```

Routes are read with `bin/rails runner`, in trusted workspaces only. Turn this off with [`rubyTemplates.routes.enabled`](/vscode-ruby-templates/guides/settings/#rails).

## Translations

`t(".title")` gets completion, hover with each locale's value, go to the locale file, and find references.

```erb title="app/views/users/index.html.erb"
<h1><%= t(".title") %></h1>
```

## Helpers

Your own helpers, such as `format_price(…)`, go to their `def` in `app/helpers` or to a controller's `helper_method`. Hover shows the signature and the comment above it.

Rails helpers (`link_to`, `turbo_frame_tag`, …) complete with snippet arguments and have hover docs. In Haml files, `succeed`, `precede`, `surround`, `preserve` and `find_and_preserve` get completion and hover too.

## Form builder

In a form builder block, completion at `f.text_field :` offers the form builder methods and the model's columns from `db/schema.rb`.

```erb title="app/views/users/_form.html.erb"
<%= form_with model: @user do |f| %>
  <%= f.text_field :email %>
<% end %>
```

## Stimulus

In `data-controller` and `data-action`, completion offers your Stimulus controller names, actions and targets. Once an element has a `data-controller`, its `-target`, `-value` and `-class` attributes are completed too.

```erb
<div data-controller="search">
  <input data-search-target="query" data-action="input->search#filter">
</div>
```

## Controller actions

Go to definition on `def show` in a controller opens the action's templates.

## Block parameters and locals

Go to definition on `user` in `@users.each do |user|` jumps to the block parameter; on a local the template assigns, it jumps to the assignment.

Uses of strict locals get the `parameter` semantic colour and block variables the `variable` colour, over the grammar's colours.

## Model columns

Hover on `report.title` shows the column type, `null:` and `default:`. Go to definition opens its line in `db/schema.rb`.

```erb title="app/views/reports/index.html.erb"
<% @reports.each do |report| %>
  <h2><%= report.title %></h2>
<% end %>
```

This works when the controller makes the record plain to see, for example `@reports = Report.order(:title)` or `@report = Report.find(params[:id])`.

## Find templates by name

<kbd>Ctrl</kbd>+<kbd>T</kbd> (<kbd>Cmd</kbd>+<kbd>T</kbd> on macOS) finds templates by render name: `uscard` finds `users/card`.

## Outline and breadcrumbs

Every template has an outline and breadcrumbs.
