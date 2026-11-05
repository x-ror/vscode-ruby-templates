---
title: Compatibility
description: How Ruby Templates works with Ruby LSP, other template extensions, Emmet, engines and gems, generator templates and Slim options.
sidebar:
  order: 11
---

## Ruby LSP

Ruby LSP and this extension both register the `erb` language id and work side by side: Ruby LSP for `.rb` files, this extension for templates. If both add diagnostics, completion or formatting to ERB, see [Troubleshooting](/vscode-ruby-templates/guides/troubleshooting/#other-extensions-for-the-same-languages).

## Other Haml or Slim extensions

VS Code picks one grammar per language, so disable the other extension to avoid surprises (see [Troubleshooting](/vscode-ruby-templates/guides/troubleshooting/)). To let another extension format, turn off `rubyTemplates.<lang>.format.enabled`.

## Emmet

Emmet works in ERB out of the box (`"emmet.includeLanguages": { "erb": "html" }` is a default). VS Code's Emmet expands to Slim syntax in Slim and Skim.

## Partials and formats

How a `render` is resolved for links, go to definition, hover and the missing-partial check:

- A `dir/name` render missing from `app/views` is looked up in the app's engines, packs and components (`*/app/views`), then in the installed gems' views.
- Gem files are read-only: no diagnostics or rename edits.
- Partials resolve by Rails' default formats plus the ones your app registers with `Mime::Type.register` in `config/initializers`.
- Renders into a gem's views (`devise/…`) aren't flagged when the gem is in `Gemfile.lock`.
- A folder that exists nowhere (`uesrs/card`) is flagged only once every view root, gems included, is known.
- Gems are found in `vendor/bundle`, `BUNDLE_PATH` (environment, `.bundle/config`, `~/.bundle/config`) or `GEM_HOME`/`GEM_PATH`. With Ruby elsewhere (rbenv, asdf) the check stays off.

```erb title="app/views/users/edit.html.erb"
<%= render "devise/shared/links" %>
<%= render "uesrs/card", user: @user %>
```

With Devise in `Gemfile.lock`, the first line isn't flagged. The second names a folder that exists nowhere; it is flagged once every view root, gems included, is known.

## Generator templates

Generator templates (`lib/templates/**/*.tt`, `lib/generators/**/templates/**`) open in their template language, without diagnostics or formatting, since the generator's own ERB is mixed in.

## Slim options

Slim options set per app aren't reflected in highlighting: custom shortcuts and attribute delimiters, smart text's implicit text lines, logic-less mode.
