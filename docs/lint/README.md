# Lint rules: ERB, Haml, Slim

> Superseded by D13 (`docs/DECISIONS.md`) and `docs/LINT_PLAN.md`: this catalogue is kept only as a record of what third-party tools check, not as a source of rules. The checks follow only the official projects; their codes and sources are listed in `docs/LINT_PLAN.md`.

A catalogue of what the template linters check, per language, to decide which rules this extension's language server should implement. Nothing here is a commitment: the built-in checks are listed as they are today, the external linters as they are in the versions below.

| Language | File               | Sources                                                                                                                                                  |
| -------- | ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| ERB      | [ERB.md](ERB.md)   | built-in (18 codes), [erb_lint](https://github.com/Shopify/erb_lint) 0.9.0 (23 linters), [Herb](https://herb-tools.dev/linter/rules/) 0.11.1 (170 rules) |
| Haml     | [HAML.md](HAML.md) | built-in (13 codes), [haml-lint](https://github.com/sds/haml-lint) 0.78.0 (35 linters)                                                                   |
| Slim     | [SLIM.md](SLIM.md) | built-in (12 codes), [slim-lint](https://github.com/sds/slim-lint) 0.37.0 (20 linters)                                                                   |

Herb parses HTML+ERB only; it has no Haml or Slim support. The extension runs erb_lint, herb-lint, haml-lint and slim-lint when they are installed and enabled (`rubyTemplates.{erbLint,herbLint,hamlLint,slimLint}.enabled`); the built-in checks run without any gem.

## The same check across languages

Where a check exists, by rule name. `—` means no source has it for that language; "built-in" means this extension's server.

| Check                                        | ERB                                                                                                                                | Haml                                                    | Slim                                            |
| -------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- | ----------------------------------------------- |
| Unclosed / mismatched HTML tags              | built-in `unclosed-element`, `mismatched-close-tag`; Herb `html-require-closing-tags`, `parser-no-errors`; erb_lint `ParserErrors` | (structure is indentation)                              | (structure is indentation)                      |
| Unbalanced Ruby blocks                       | built-in `unclosed-block`, `stray-end`, `unmatched-branch`                                                                         | built-in `unmatched-branch`, `explicit-end`             | built-in `unmatched-branch`, `explicit-end`     |
| Ruby syntax errors                           | built-in `ruby-syntax`; Herb (Prism, in diagnostics); erb_lint `Rubocop`                                                           | haml-lint `Syntax`, `RuboCop`                           | built-in `unclosed-ruby`; slim-lint `RuboCop`   |
| Helper in a silent tag renders nothing       | built-in `silent-output-helper`; Herb `actionview-no-silent-helper`, `actionview-no-silent-render`                                 | —                                                       | built-in `silent-output-helper`                 |
| Output tag around control flow               | built-in `output-control-flow`; Herb `erb-no-output-control-flow`                                                                  | —                                                       | —                                               |
| Unresolved `render`                          | built-in `unresolved-render`                                                                                                       | built-in `unresolved-render`                            | built-in `unresolved-render`                    |
| Strict locals: required magic comment        | Herb `erb-strict-locals-required`; erb_lint `StrictLocals`                                                                         | haml-lint `StrictLocals`                                | slim-lint `StrictLocalsMissing`                 |
| Strict locals: callers pass the right locals | built-in `strict-locals`; Herb `actionview-no-mistyped-locals`, `actionview-no-unused-strict-locals`                               | built-in `strict-locals`                                | built-in `strict-locals`                        |
| Instance variables in partials               | Herb `erb-no-instance-variables-in-partials`; erb_lint `PartialInstanceVariable`                                                   | haml-lint `InstanceVariables`                           | slim-lint `InstanceVariables`                   |
| `<img>` without `alt`                        | built-in `img-missing-alt`; Herb `html-img-require-alt` (also `image_tag`)                                                         | haml-lint `AltText` (off)                               | —                                               |
| Duplicate ids                                | built-in `duplicate-id`; Herb `html-no-duplicate-ids`                                                                              | haml-lint `RepeatedId`                                  | —                                               |
| Inline `style` attributes                    | Herb `html-no-style-attributes` (off)                                                                                              | haml-lint `InlineStyles`                                | —                                               |
| Unescaped / raw output                       | built-in `unsafe-output`; Herb `erb-no-unsafe-raw`, `erb-no-raw-output-in-attribute-value`; erb_lint `ErbSafety`                   | haml-lint `UnescapedHtml`                               | —                                               |
| Ruby interpolated into `<script>`            | Herb `erb-no-unsafe-script-interpolation`, `erb-no-unsafe-js-attribute`; erb_lint `ErbSafety`                                      | —                                                       | —                                               |
| Script `nonce`                               | Herb `html-require-script-nonce`; erb_lint `RequireScriptNonce`                                                                    | —                                                       | —                                               |
| `<input>` without `autocomplete`             | Herb `html-input-require-autocomplete`; erb_lint `RequireInputAutocomplete`                                                        | —                                                       | —                                               |
| Uppercase tag names                          | Herb `html-tag-name-lowercase`                                                                                                     | haml-lint `TagName`                                     | slim-lint `TagCase`                             |
| Redundant `div`                              | —                                                                                                                                  | haml-lint `ImplicitDiv`                                 | slim-lint `RedundantDiv`                        |
| Void element with content / self-closing     | Herb `actionview-no-void-element-content`, `html-no-self-closing`; erb_lint `SelfClosingTag`                                       | built-in `self-closing-content`                         | built-in `void-children`                        |
| Spacing inside ERB tags / after `=` `-`      | Herb `erb-require-whitespace-inside-tags`, `erb-no-extra-whitespace-inside-tags`; erb_lint `SpaceAroundErbTag`                     | haml-lint `SpaceBeforeScript`                           | slim-lint `ControlStatementSpacing`             |
| Comment syntax                               | Herb `erb-comment-syntax`; erb_lint `CommentSyntax`                                                                                | haml-lint `RubyComments`, `LeadingCommentSpace`         | slim-lint `CommentControlStatement`             |
| Indentation character / width                | Herb `source-indentation`; erb_lint `SpaceIndentation`                                                                             | built-in `inconsistent-indent`; haml-lint `Indentation` | built-in `inconsistent-indent`; slim-lint `Tab` |
| Trailing whitespace                          | Herb `erb-no-trailing-whitespace`; erb_lint `TrailingWhitespace`                                                                   | haml-lint `TrailingWhitespace`                          | slim-lint `TrailingWhitespace`                  |
| Final newline / blank lines                  | Herb `erb-require-trailing-newline`, `erb-no-extra-newline`; erb_lint `FinalNewline`, `ExtraNewline`                               | haml-lint `FinalNewline`, `TrailingEmptyLines`          | slim-lint `TrailingBlankLines`, `EmptyLines`    |
| Line / file length                           | —                                                                                                                                  | haml-lint `LineLength`, `ViewLength`                    | slim-lint `LineLength`, `FileLength`            |
| Hard-coded (untranslated) text               | erb_lint `HardCodedString`                                                                                                         | —                                                       | —                                               |

## HTML tag or Rails helper?

Herb is the only source with rules on this choice; they apply the same way to Haml (`%img{src: image_path("x")}`) and Slim (`img src=image_path("x")`), where no linter checks it yet. Quotes are from Herb's rule docs at commit `fc3ca23`.

### `<img>` vs `image_tag`

Rule `erb-prefer-image-tag-helper` (on, no autofix, needs `framework: actionview` in `.herb.yml`); built in as `prefer-image-tag` (info): use `image_tag` when `src` is computed; a static path is fine.

> Prefer using Rails' `image_tag` helper over manual `<img>` tags with dynamic ERB expressions like `image_path` or `asset_path`.
>
> The `image_tag` helper […] properly escapes the `src` value to prevent XSS vulnerabilities and ensures consistent rendering across different contexts. Using `image_tag` also reduces template complexity by eliminating the need for manual string interpolation and makes it easier to add additional attributes like `alt`, `class`, or `data-*` attributes in a clean, readable way.

|                 | ERB                                                    | Haml                                             | Slim                                        |
| --------------- | ------------------------------------------------------ | ------------------------------------------------ | ------------------------------------------- |
| Fine            | `<img src="/static/logo.png" alt="Logo">`              | `%img{src: "/static/logo.png", alt: "Logo"}`     | `img src="/static/logo.png" alt="Logo"`     |
| Flagged by Herb | `<img src="<%= image_path("logo.png") %>" alt="Logo">` | `%img{src: image_path("logo.png"), alt: "Logo"}` | `img src=image_path("logo.png") alt="Logo"` |
| Preferred       | `<%= image_tag "logo.png", alt: "Logo" %>`             | `= image_tag "logo.png", alt: "Logo"`            | `= image_tag "logo.png", alt: "Logo"`       |

`html-img-require-alt` (on, warning) checks `image_tag` calls for `alt:` as well as `<img>` tags. Herb's `a11y-no-redundant-image-alt` (off) flags alt text like "image of…".

### `<a>` vs `link_to`

Rule `actionview-prefer-link-to-helper` (on, info, autofix for most offenses, `framework: actionview`): an `<a>` whose `href` is one ERB output tag becomes `link_to`. Literal, fragment and external URLs stay `<a>`.

> `link_to` […] runs the URL through `url_for`, so the same call accepts a route helper, a plain String, a model, an array, or an options Hash, and it builds the rest of the tag from a Ruby Hash, so classes, `data:` options and conditional attributes are expressed as Ruby rather than as interpolation inside markup.

Not flagged: an `href` with more than the expression (`"<%= root_path %>#section"`), or an expression with its own modifier (`<%= @event.url if @event.published? %>`).

### `form_for` / `form_tag` vs `form_with`

Rule `actionview-prefer-form-with` (on, info, `framework: actionview`) reports `form_for`, which the Rails guides discourage in favour of `form_with`; `fields_for` is never reported. A plain `<form>` is not flagged. `html-no-nested-forms` treats `form_with`, `form_for`, `form_tag` and `button_to` as the `<form>` they render.

### `<script>`, `javascript_tag`, `javascript_include_tag`

- `erb-no-javascript-tag-helper` (on; erb_lint's `NoJavascriptTagHelper` agrees): prefer a plain `<script>` over `javascript_tag do`, because the helper renders its block as raw text and hides unsafe interpolation from `erb-no-unsafe-script-interpolation`.
- `html-no-inline-script-elements` (off): for a strict CSP, no inline scripts at all, `javascript_include_tag` instead; JSON and JSON-LD scripts are allowed.
- `html-require-script-nonce` (on; erb_lint `RequireScriptNonce`): inline scripts need a `nonce`; `javascript_include_tag` is not flagged. `tag.script nonce: true` does not resolve the request's nonce; pass `request.content_security_policy_nonce`.

### `<style>`, `<link>`, `stylesheet_link_tag`

`html-no-style-elements` (off): no inline `<style>` (also through `content_tag`/`tag`), `stylesheet_link_tag` instead; `<style scoped>` is exempt. No rule prefers `stylesheet_link_tag` over a hand-written `<link>`.

### `tag.*` / `content_tag`

No rule prefers helpers over plain HTML in general. The helper rules catch misuse:

- `actionview-no-unnecessary-tag-attributes` (autofix): `tag.attributes` covering every attribute of a plain tag; use `tag.div` or plain attributes.
- `actionview-no-void-element-content`: `tag.img "/x.png"` raises at runtime.
- `actionview-no-content-argument-with-block`: a content argument next to a block is ignored.
- `actionview-no-helper-shadowing`: a block variable named like a helper (`link_to`, `tag`, …).
- HTML rules (`html-no-nested-links`, `html-tag-name-lowercase`, `html-no-unknown-tag`, …) treat `content_tag` / `tag.*` / `link_to` like the elements they render.
