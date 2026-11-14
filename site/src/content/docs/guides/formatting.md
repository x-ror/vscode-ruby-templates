---
title: Formatting
description: How the built-in formatter re-indents ERB, Haml and Slim templates while changing only whitespace.
sidebar:
  order: 6
---

_Format Document_, _Format Selection_ and format on save work for HTML ERB views, Haml and Slim. The formatter changes **only whitespace**: leading indentation, trailing whitespace, blank lines and the final newline, and, when you turn the rules on, the spaces at the edges of ERB tags and after Haml and Slim script markers. It never wraps or joins lines, never reorders attributes and never touches Ruby code.

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

- `Layout/TrailingEmptyLines` is on for Haml and Slim: the file ends with exactly one line break. The blank lines at the end go, and a missing final line break is added in the file's line ending. It is off for ERB, because Rails renders ERB byte for byte and the end of a partial is output (a final line break shows as a space between inline elements); turn it on with `enabled = true`, or with `insert_final_newline = true` in `.editorconfig` (a rule you turn off stays off).
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

## Spaces inside tags and after markers

Four rules change the spaces on a line and never add or remove one; all are off until enabled, and they run before the re-indentation, in _Format Selection_ too.

- `Layout/SpaceInsideErbTag` gives an ERB tag one space inside its delimiters: `<%=x%>` and `<%=  x  %>` become `<%= x %>`, `<%-x-%>` becomes `<%- x -%>`. Only tags on one line are changed; `<%#` comments, `<%%` literals and tags that span lines stay. The "Add spaces inside this ERB tag" code action and the `source.erbSpacing` source action do the same without the rule.
- `Layout/SpaceInHtmlTag` tidies the start and end tags of an ERB template: one space between the name and the attributes and between attributes, none around `=` or before `>`. `<div  class = "card"   id="u" >` becomes `<div class="card" id="u">` and `</div  >` becomes `</div>`; `<br  />` keeps one space before the `/>`. A blank run with a newline stays (attributes on their own lines are layout), as do quoted and unquoted values, the bodies of `<script>`, `<style>`, `<textarea>`, `<title>`, `<xmp>`, `<iframe>`, `<noscript>`, `<noembed>`, `<noframes>` and `<plaintext>`, comments, CDATA and text such as `a < b`. Beside an ERB tag in a tag, `<div <%= attrs %>  class="a">` loses only the extra blank: the space between an ERB tag and what follows is never added or removed, since `<%= x %>class` and `<%= x %> class` render differently.
- `Style/ErbCommentSyntax` turns `<% # note %>` into `<%# note %>`. The tag has to be nothing but comment, on every line if it has several, and stand alone on its lines: beside other content Rails comments out what follows a `<% #` tag, which `<%#` would bring back. `<%= # x %>`, `<%- # x -%>` (skipped to be conservative) and tags with Ruby before or after the comment stay.
- `Layout/SpaceAfterScriptMarker` adds a space after the marker of a Ruby line: `-foo` and `=foo` become `- foo` and `= foo` (also `~`, `!=`, `&=`, `!~` and `&~` in Haml, `==` in Slim), and after a tag (`%p=x` and `p=x` become `%p= x` and `p= x`). Comments (`-#`, Slim's `/`), text, filters, attribute values and continuation lines stay, as do Haml's `==` and `\=` lines, which are text.

```toml
[Layout.SpaceInsideErbTag]
enabled = true

[Layout.SpaceInHtmlTag]
enabled = true

[Style.ErbCommentSyntax]
enabled = true

[Layout.SpaceAfterScriptMarker]
enabled = true
```

A result is kept only when the Ruby of every ERB tag and the name, attributes and values of every HTML tag are the same afterwards, and when Haml or Slim compile it to the same template; otherwise the file is formatted without these rules and `--verbose` says so.

## Quotes, boolean attributes, void tags and tag names

Four rules rewrite the markup of ERB templates (`Style/TagNameCase` also Slim's tag names) without changing what the browser makes of it; all are off until enabled, and they run before the other rules, in _Format Selection_ too.

- `Style/AttributeQuotes` puts attribute values in double quotes (`style = "single_quotes"` for single) and quotes bare ones: `<div class='card' data-controller=toggle>` becomes `<div class="card" data-controller="toggle">`. A value that holds the quote (`title='say "hi"'`), spans lines or holds an output tag (`title='<%= link_to … %>'`: helpers return text with quotes of their own) stays, and so does a bare value with any ERB tag (`value=<%= q %>`), whose output may be empty or hold blanks.
- `Style/BooleanAttributes` writes `disabled="disabled"` and `disabled=""` as `disabled`, for the boolean attributes of the HTML standard on the elements they belong to. `disabled="false"` (still disabled), `hidden="until-found"`, values with ERB, custom elements and `<svg>` and `<math>` stay.
- `Style/VoidTagClosing` writes `<br>` and `<img src="x">` without the slash (`style = "no_slash"`) or as `<br />` (`style = "slash"`). A slash inside an unquoted value (`src=a.png/`) or after an ERB tag, `slash` after a bare value that ends in an ERB tag, tags over several lines, `<svg>` and `<math>` stay.
- `Style/TagNameCase` lower-cases tag names (`<DIV>` and `</DIV>`, also the end tag of `<script>` and `<title>`; `DIV.icon` in Slim), with the camel-case SVG elements as the SVG standard spells them (`linearGradient`, `foreignObject`, `clipPath`). In Slim, a name that lower case would turn into `doctype` or an embedded engine (`Javascript:`) stays, and so does a template that does not output HTML, like `index.xml.slim`.

Tags in `<script>`, `<style>`, `<textarea>` and `<title>`, comments and ERB tags are not markup and stay. The formatter reads every tag again after the rewrite and keeps it only when each tag has the same name, attributes and values apart from what the rules change (for Slim, when Slim parses the file into the same pieces); otherwise the file is formatted without these rules and `--verbose` says so.

```toml
[Style.AttributeQuotes]
enabled = true
style = "double_quotes"

[Style.BooleanAttributes]
enabled = true

[Style.VoidTagClosing]
enabled = true
style = "no_slash"

[Style.TagNameCase]
enabled = true
```

## Haml and Slim tags

Three rules rewrite the tag lines of Haml (`Style/ImplicitDiv` also Slim's); all are off until enabled, they run before the other rules, in _Format Selection_ too, and every rewrite stays on its line.

- `Style/ImplicitDiv` drops the `div` where a class or id follows it: `%div.card` becomes `.card` and `%div#main` becomes `#main` (Slim: `div.card`, `div#main`). A `div` with only attributes (`%div{class: "a"}`, `div class="a"`), a bare `div`, a name in other case (`%DIV.card`, which is output as written) and a line over several lines with `|` stay. For Slim it assumes the default tag is `div`; an app that sets `Slim::Engine.set_options default_tag: "span"` should leave the rule off.
- `Style/ClassesBeforeIds` writes the classes of a Haml tag before its id: `%p#intro.lead` becomes `%p.lead#intro`, `#a.b.c` becomes `.b.c#a`. Haml renders `class` before `id` whichever way round they are written.
- `Style/HashAttributes` writes attributes as a Ruby hash (`style = "ruby_hash"`, the default) or in HTML style (`style = "html"`): `%a(href="/users" checked)` becomes `%a{href: "/users", checked: true}`, with `"data-n": …` for a name that needs quotes. It converts a tag only when every attribute is a string without escapes or `#{}`, a number, `true`, `false`, `nil` or a variable; a tag with both a hash and a list, a nested hash (`data: {…}`), `data` or `aria` with a value that is no hash, a splat, any other expression and attributes over several lines stay.

The formatter reads every rewritten file again. Haml must generate the same Ruby (apart from the attribute hashes, which `Style/HashAttributes` changes) and each tag must have the same name, classes, ids and attributes; Slim must parse into the same pieces. Otherwise the file is formatted without these rules and `--verbose` says so.

```toml
[Style.ImplicitDiv]
enabled = true

[Style.ClassesBeforeIds]
enabled = true

[Style.HashAttributes]
enabled = true
style = "ruby_hash"
```

## Content on its own line

`Layout/ElementContentOnOwnLine` (off until enabled) moves the content of an element that is written next to its tags to a line of its own, one level in. It applies to the names in `tags` (`div`, `section`, `article`, `header`, `footer`, `ul` and `ol` unless you list others, in any case), runs before the other rules, which indent the new line, and is left out of _Format Selection_ because it adds lines.

```erb
<div class="card" id="<%= dom_id(user) %>"><%= user.name %></div>
```

becomes

```erb
<div class="card" id="<%= dom_id(user) %>">
  <%= user.name %>
</div>
```

In Haml `%section.card= x` becomes `%section.card` and `  = x`, and `%div text` becomes `%div` and `  text`. In Slim `section.card = x` becomes `section.card` and `  = x`, and `div text` becomes `div` and `  | text`. Elements inside each other all move.

This is the one rule that adds whitespace to the page, so it is for tags where a line break shows nowhere: inside a `<span>` or `<a>` it would show as a space, and `pre`, `textarea`, `script` and `style` (and anything inside them or inside an inline element) are never touched. It also stays out of:

- ERB: a `<% %>` tag in the start tag or the content (output tags and comments are fine), a `-%>` or `<%-`, an element or content over several lines, an empty element.
- Haml: `<`, `>` and `/` on the tag, a line joined with `|`, content that starts with a character that would make the new line something else (`-`, `%`, `.`, `=` and the like; `#{…}` is fine), and a tag with lines nested under it or Ruby that goes on to the next line.
- Slim: `div: span x`, `='`, and a tag with lines nested under it or code that goes on to the next line.

The formatter reads the result again. ERB must keep every character and element, so only whitespace is new; Haml must generate the same Ruby and read the moved content one level below its tag; Slim must compile to the same template (Slim's output does not change at all, Haml's gains line breaks inside the element). Otherwise the file is formatted without the rule and `--verbose` says so.

```toml
[Layout.ElementContentOnOwnLine]
enabled = true
tags = ["div", "section", "article", "header", "footer", "ul", "ol"]
```

## One attribute per line

`Layout/AttributesPerLine` (ERB only, off until enabled) splits a start tag whose line is longer than `max_line_length` characters (100 unless you set another; counted like `Layout/LineLength`, a tab is one). Each attribute goes on a line of its own, one indent level in, and the `>` (or `/>`) stays after the last attribute:

```erb
<input type="email" name="user[email]" id="user_email" class="form-control form-control-lg" placeholder="Email" required>
```

becomes

```erb
<input
  type="email"
  name="user[email]"
  id="user_email"
  class="form-control form-control-lg"
  placeholder="Email"
  required>
```

The rule runs after `Layout/ElementContentOnOwnLine` and measures the line where the re-indenter leaves it, so the result is stable. Whitespace between attributes is not content, so the page does not change. A tag stays as it is when it has fewer than two attributes, is already over several lines, does not start its line, has an ERB tag between its attributes or in a name, has `<%` control flow in it, has a value over several lines, or sits inside `<pre>` or `<textarea>`. The formatter reads every tag again and keeps the rewrite only when names, attributes, values and slashes are the same; otherwise `--verbose` says why. It adds lines, so _Format Selection_ leaves it out.

```toml
[Layout.AttributesPerLine]
enabled = true
max_line_length = 100
```

## Safe by construction

Every result must be stable when formatted again, keep every non-whitespace character and add no diagnostic. Otherwise the file comes back unchanged and the reason is logged in the _Ruby Templates_ channel of the Output panel. The exceptions are `Turbo/PreferTurboAttributes`, below, and the rules above, whose rewrites are the characters the formatter changes besides whitespace.

## Rails UJS attributes to Turbo's

`Turbo/PreferTurboAttributes` is the one rule that changes the page, so it is off until enabled and its renames are the only characters the formatter changes besides whitespace. It turns `data-method` into `data-turbo-method` and `data-confirm` into `data-turbo-confirm` on links, and `data-disable-with` into `data-turbo-submits-with` on submit buttons (`<button>` with no `type` or `type="submit"`, `<input type="submit">`), in markup and in the `data: { }` options of `link_to`, `tag`, `content_tag`, `submit_tag`, `button_tag`, `button_to` and `submit`/`button` on a form builder (`confirm:` becomes `turbo_confirm:`, `method:` `turbo_method:`, `disable_with:` `turbo_submits_with:`). They work only in an app that runs Turbo; the old ones work only with Rails UJS.

Left alone: a `data-confirm` on a link that sends no method (Turbo asks to confirm only a link with `data-turbo-method`), the confirm of `button_to` (Turbo reads it from the form), `link_to … method: :delete` (not a data attribute), an element that has the Turbo attribute already, other elements and button types, and anything that is not written out as a name or a literal key. The renames are made first. The formatter then reads every element's attributes before and after and keeps the rewrite only when nothing but the renamed attributes differs; its usual checks apply to the renamed text, so the rest of the file is guarded as for any other rule. _Format Selection_ applies the rule to the lines it covers.

```toml
[Turbo.PreferTurboAttributes]
enabled = true
```

## Indentation width and line endings

`format.enabled` and `format.align_comments` under `[erb]`, `[haml]` and `[slim]` in a `.ruby-templates.toml` turn formatting off or on and align comments, and win over the `rubyTemplates.*` settings; its global `exclude` leaves files unformatted.

The indentation width comes from the editor's tab size, then `.editorconfig` (`indent_size`; `tab_width` for `indent_size = tab`), then 2. Each line keeps its ending.

Haml and Slim are indented with spaces. ERB can use tabs: one tab per indent level, whatever the width. The style comes from the first of these that sets it:

1. `style` of `[Layout.IndentationStyle]` in a `.ruby-templates.toml` (`"spaces"` or `"tabs"`); `enabled = false` always gives spaces.
2. The editor's `editor.insertSpaces` for the file. In an editor this always decides when the toml is silent, because the language server protocol always sends it (as it does the tab size). `.editorconfig` only counts there through an EditorConfig extension, which sets `insertSpaces` for the file.
3. `.editorconfig`: `indent_style = tab` or `space`. Only the command line, which has no editor, reads it directly.
4. Spaces.

```text title=".editorconfig"
[*.{erb,haml,slim}]
indent_style = space
indent_size = 2
```

```toml title=".ruby-templates.toml"
[Layout.IndentationStyle]
style = "tabs"
```

Only the leading whitespace the formatter writes becomes tabs. `<pre>`, `<textarea>`, `<script>`, `<style>`, `<xmp>` and `<listing>` bodies, Ruby inside tags, quoted attribute values and comments keep every byte. The lines of a tag that spans several lines move with its first line and keep their indentation relative to it exactly as written: only the first line's level becomes tabs, so an attribute lined up with spaces under the tag name stays lined up in any tab width. A file indented with tabs is left as it is under `style = "tabs"` and converted to spaces under `"spaces"`. _Format Selection_ applies the style to the lines it covers.

## Trailing whitespace

`Layout/TrailingWhitespace` is on for ERB, Haml and Slim: spaces and tabs at the end of a line go where they don't count, and whitespace-only lines become empty. With `[Layout.TrailingWhitespace] enabled = false` they stay as written; the places where they count (a `<pre>` body, an ERB comment, a Haml filter, Slim text) are never touched either way.

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

- Reflowing long lines (`Layout/LineLength` reports them; off until enabled, with `max` for the limit).
- Indenting embedded JavaScript or CSS by their own syntax.
- Ruby formatting (use RuboCop).
- Haml files with errors.
- Tab-indented Slim and Haml files.
