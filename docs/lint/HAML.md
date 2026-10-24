# Haml lint rules

What the sources check in Haml. Reference only: nothing here is implemented beyond the built-in checks. Index, cross-language matrix and helper guidance: [README.md](README.md). Herb covers HTML+ERB only.

## Built-in (this extension's language server)

| Code                                 | What it flags                                                                                                              |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| `inconsistent-indent`                | Mixed tabs and spaces, an indent that isn't the file's unit, tabs in a spaces file (Haml's own messages)                   |
| `bad-dedent`                         | A dedent to a level that never existed                                                                                     |
| `top-level-indent`                   | The first line indented                                                                                                    |
| `illegal-nesting`                    | Content nested under a tag with inline content, under plain text, under a self-closing tag, under Ruby that opens no block |
| `doctype-children`                   | Anything nested under `!!!`                                                                                                |
| `unmatched-branch`                   | `- else` / `- elsif` / `- when` without its `- if` / `- case`, or at the wrong level                                       |
| `explicit-end`                       | `- end`: Haml closes blocks by indentation                                                                                 |
| `illegal-element`                    | `.` / `#` with no class or id name                                                                                         |
| `unknown-filter`                     | `:name` that isn't a Haml 6/7 filter                                                                                       |
| `self-closing-content`               | `%br/ text`                                                                                                                |
| `unclosed-attribute`                 | `{`, `(` or `[` left open in tag attributes                                                                                |
| `unresolved-render`, `strict-locals` | As in ERB                                                                                                                  |

Turned off per code with `rubyTemplates.haml.disabledDiagnostics`.

## haml-lint

Version 0.78.0 (`config/default.yml`, class docs in `lib/haml_lint/linter/*.rb`). Wired in through `rubyTemplates.hamlLint.enabled`. Autofix = `--auto-correct` (`unsafe` needs `--auto-correct-all`); the extension's haml-lint formatter runs `--auto-correct-only`.

| Linter                          | On       | Autofix | Default options                | What it flags                                                              |
| ------------------------------- | -------- | ------- | ------------------------------ | -------------------------------------------------------------------------- |
| `AlignmentTabs`                 | yes      | unsafe  |                                | Tabs placed to align tag content                                           |
| `AltText`                       | no       | no      |                                | `%img` without `alt`                                                       |
| `ClassAttributeWithStaticValue` | yes      | yes     |                                | `%tag{ class: 'x' }` instead of `%tag.x` (templating placeholders allowed) |
| `ClassesBeforeIds`              | yes      | yes     |                                | `%div#id.class` instead of `.class#id`                                     |
| `ConsecutiveComments`           | yes      | unsafe  | `max_consecutive: 1`           | Comment lines that can be one block                                        |
| `ConsecutiveSilentScripts`      | yes      | unsafe  | `max_consecutive: 2`           | `-` lines that could be a `:ruby` filter                                   |
| `EmptyObjectReference`          | yes      | yes     |                                | `%div[]`                                                                   |
| `EmptyScript`                   | yes      | unsafe  |                                | `-` / `=` with no code                                                     |
| `FinalNewline`                  | yes      | yes     | `present: true`                | Missing final newline                                                      |
| `HtmlAttributes`                | yes      | unsafe  |                                | `%tag(lang=en)` HTML-style attributes                                      |
| `IdNames`                       | yes      | no      | `style: lisp_case`             | Ids not in the configured case                                             |
| `ImplicitDiv`                   | yes      | yes     |                                | `%div.x` where `.x` implies the div                                        |
| `Indentation`                   | yes      | no      | `character: space`, `width: 2` | The other indentation character / width                                    |
| `InlineStyles`                  | yes      | no      |                                | `style` attributes                                                         |
| `InstanceVariables`             | yes      | no      | `file_types: partials`         | Instance variables in partials                                             |
| `LeadingCommentSpace`           | yes      | yes     |                                | `-#comment` without a space                                                |
| `LineLength`                    | yes      | no      | `max: 80`                      | Long lines                                                                 |
| `MultilinePipe`                 | yes      | no      |                                | `\|` multiline blocks                                                      |
| `MultilineScript`               | yes      | unsafe  |                                | Ruby split across lines after an operator (`\|\|`, `+`, `if`, `?`, …)      |
| `NoPlaceholders`                | no       | no      |                                | `placeholder` attributes                                                   |
| `ObjectReferenceAttributes`     | yes      | no      |                                | `%div[@user]` object references                                            |
| `RepeatedId`                    | yes      | no      | `severity: error`              | The same id twice in a file                                                |
| `RuboCop`                       | yes      | yes     | `ignored_cops: []`             | RuboCop offenses in the Ruby (with autocorrect)                            |
| `RubyComments`                  | yes      | yes     |                                | `- # x` where `-# x` would do                                              |
| `SpaceBeforeScript`             | yes      | yes     |                                | `=x` / `-x` without a space                                                |
| `SpaceInsideHashAttributes`     | yes      | yes     | `style: space`                 | `{a: 1}` vs `{ a: 1 }`                                                     |
| `StrictLocals`                  | no       | no      | `file_types: partials`         | Partials without a `-# locals: (…)` magic comment                          |
| `Syntax`                        | (always) | no      |                                | Haml parser errors                                                         |
| `TagName`                       | yes      | yes     |                                | Tag names with uppercase letters                                           |
| `TrailingEmptyLines`            | yes      | yes     |                                | Empty lines at the end                                                     |
| `TrailingWhitespace`            | yes      | yes     |                                | Trailing whitespace                                                        |
| `UnescapedHtml`                 | yes      | no      |                                | `!=`, `!~` and `!` unescaped output (XSS risk, like `raw`)                 |
| `UnnecessaryInterpolation`      | yes      | yes     |                                | `%tag #{x}` instead of `%tag= x`                                           |
| `UnnecessaryStringOutput`       | yes      | unsafe  |                                | `%tag= "text #{x}"` instead of `%tag text #{x}`                            |
| `ViewLength`                    | yes      | no      | `max: 100`                     | Views longer than the maximum                                              |
