---
title: Supported syntax
description: What the grammars highlight and what the language server parses in ERB, Haml, Slim and Skim.
sidebar:
  order: 10
---

What the grammars highlight and what the language server parses. The server follows Erubi as Rails configures it, Haml 7.5 and Slim 5.2.

## ERB

| Syntax                   | Highlighting                          | Server                                    |
| ------------------------ | ------------------------------------- | ----------------------------------------- |
| `<% %>`                  | Ruby                                  | Code, no output                           |
| `<%- %>`                 | Ruby                                  | Same as `<%`                              |
| `<%= %>`                 | Ruby                                  | Escaped output                            |
| `<%== %>`                | Ruby                                  | Unescaped output                          |
| `<%# %>`                 | Comment                               | Comment                                   |
| `<%# locals: (user:) %>` | Strict locals, as Ruby                | Strict locals checks, renames, completion |
| `-%>`, `=%>`             | Tag end                               | Tag end (`=%>` as `-%>`, as in Erubi)     |
| `<%%`, `%%>`             | Literal `<%` / `%>`                   | Text, not a tag                           |
| `<%_`, `_%>`             | Not special: `_` starts the Ruby code | Not special, as in Erubi (code `_ x _`)   |
| `%` lines                | Only in `erb-percent`                 | Only in `erb-percent` (`trim_mode: "%"`)  |

```erb title="app/views/posts/_post.html.erb"
<%# locals: (post:) %>
<article id="<%= dom_id(post) %>">
  <h2><%= post.title %></h2>
  <%== post.body_html %>
  <% if post.draft? -%>
    <p>Draft</p>
  <% end -%>
</article>
```

### Trim modes

Erubi's `trim: true`, the Rails default, is what the server models, so `-` before `%>` changes nothing in the checks. `trim_mode: "%"` is the opt-in [`erb-percent` language id](/vscode-ruby-templates/guides/file-types/#erb-percent); other stdlib modes (`<>`, `>`) are not modelled.

## Haml

| Syntax                                                                      | Highlighting      | Server                                                          |
| --------------------------------------------------------------------------- | ----------------- | --------------------------------------------------------------- |
| `%tag`, `.class`, `#id`, `!!!`, `/`, `-#`, `=`, `-`, `~`, `&=`, `!=`, `#{}` | yes               | Parsed as Haml 7.5.1 does, with its error messages              |
| Attributes `{…}`, `(…)`, `[…]`                                              | yes               | Parsed; Ruby inside is checked                                  |
| Filters `:plain` `:escaped` `:preserve` `:cdata`                            | Text              | Accepted                                                        |
| Filters `:javascript` `:css` `:sass` `:scss` `:less` `:markdown` `:erb`     | Embedded language | Accepted                                                        |
| Filters `:coffee` `:coffeescript`                                           | CoffeeScript      | Accepted                                                        |
| Filter `:ruby`                                                              | Ruby              | Accepted, body checked as Ruby                                  |
| Any other filter (Haml 5's `:maruku`, `:textile`, …)                        | Text              | `unknown-filter`                                                |
| `succeed`, `precede`, `surround`, `preserve`, `find_and_preserve`           | Ruby              | Completion and hover (Haml 6/7 `RailsHelpers`), Haml files only |

```haml title="app/views/posts/show.html.haml"
%article{ id: dom_id(@post) }
  %h2.title= @post.title
  -# Rendered from Markdown
  :markdown
    #{@post.summary}
  = succeed "." do
    = link_to "Read more", post_path(@post)
```

Haml 6 and 7 are targeted (Haml 7.5.1 is the reference); Haml 5 is not. Whether a filter's gem (`sass`, `coffee-script`, …) is installed isn't checked.

## Slim and Skim

| Syntax                                                                                                    | Highlighting                            | Server                                                              |
| --------------------------------------------------------------------------------------------------------- | --------------------------------------- | ------------------------------------------------------------------- |
| Tags, `.class`/`#id` shortcuts, `=`, `==`, `-`, `/`, `\|`, `'`, `doctype`, `#{}`                          | yes                                     | Parsed as Slim 5.2 does, with its error messages                    |
| Attribute wrappers `(…)`, `[…]`, `{…}`, or none                                                           | yes                                     | Parsed; Ruby inside is checked                                      |
| Engines `ruby:` `javascript:` `css:` `sass:` `scss:` `coffee:` `markdown:` `textile:` `rdoc:` `asciidoc:` | Embedded language where VS Code has one | Accepted                                                            |
| Other engines: `erb:`, `less:`, `stylus:`, …                                                              | Highlighted                             | `unknown-engine` unless listed in `rubyTemplates.slim.extraEngines` |
| Engines your app registers (`mjml:`)                                                                      | Text                                    | Accepted once added to `rubyTemplates.slim.extraEngines`            |

```slim title="app/views/posts/show.html.slim"
article id=dom_id(@post)
  h2.title = @post.title
  / Rendered from Markdown
  markdown:
    #{@post.summary}
  | Posted by #{@post.author.name}
```

`.skim` files use the Slim syntax and the `slim` settings. Slim options set per app (custom shortcuts, attribute delimiters, smart text, logic-less mode) are not read.
