# Slim lint rules

What the sources check in Slim (and Skim, where noted). Reference only: nothing here is implemented beyond the built-in checks. Index, cross-language matrix and helper guidance: [README.md](README.md). Herb covers HTML+ERB only.

## Built-in (this extension's language server)

| Code | What it flags |
|---|---|
| `inconsistent-indent` | Indentation that doesn't match its siblings |
| `bad-dedent` | A dedent to a level that never existed |
| `doctype-children` | Anything nested under `doctype` |
| `void-children` | Content nested under a void element (`img`, `br`, …), HTML hosts only |
| `unknown-engine` | `name:` that isn't a Slim engine (or one listed in `rubyTemplates.slim.extraEngines`) |
| `unclosed-attribute` | `(`, `[` or `{` left open in attributes, or in a quoted attribute value |
| `unclosed-ruby` | A bracket or string left open in Ruby code (not in Skim) |
| `explicit-end` | `- end`: Slim closes blocks by indentation (not in Skim) |
| `unmatched-branch` | `- else` / `- elsif` / `- when` without its opener at the same level (not in Skim) |
| `silent-output-helper` | `- form_with … do`: the block's HTML is discarded; use `=` (not in Skim) |
| `unresolved-render`, `strict-locals` | As in ERB |

Turned off per code with `rubyTemplates.slim.disabledDiagnostics`.

## slim-lint

Version 0.37.0 (`config/default.yml`, class docs in `lib/slim_lint/linter/*.rb`). Wired in through `rubyTemplates.slimLint.enabled` (on save). Autofix = `--auto-correct`.

| Linter | On | Autofix | Default options | What it flags |
|---|---|---|---|---|
| `CommentControlStatement` | yes | yes | | `-` lines holding only a comment (use `/`) |
| `ConsecutiveControlStatements` | yes | no | `max_consecutive: 2` | `-` lines that could be a `ruby:` block |
| `ControlStatementSpacing` | yes | yes | | Missing or extra space around `=` / `-` |
| `EmbeddedEngines` | no | no | `forbidden_engines: []` | Forbidden embedded engines |
| `EmptyControlStatement` | yes | yes | | `-` with no code |
| `EmptyLines` | yes | yes | | Two or more blank lines, or a blank first line |
| `FileLength` | no | no | `max: 300` | Long files |
| `InstanceVariables` | no | no | `include: app/views/**/_*.html.slim` | Instance variables in partials |
| `LineLength` | yes | no | `max: 80` | Long lines |
| `QuoteConsistency` | no | yes | | Mixed `'` / `"` in attribute values |
| `RedundantDiv` | yes | yes | | `div.x` where `.x` implies the div |
| `RuboCop` | yes | no | Layout and some Style cops ignored | RuboCop offenses in the Ruby |
| `StrictLocalsMissing` | no | no | `include: app/views/**/_*.html.slim` | Partials without a `/# locals: (…)` magic comment |
| `Tab` | yes | no | | Tab indentation |
| `Tag` | no | no | `forbidden_tags: []` | Forbidden tags |
| `TagAttribute` | no | no | `forbidden_attributes: []` | Forbidden attributes |
| `TagCase` | yes | yes | | Tag names with uppercase letters |
| `TrailingBlankLines` | yes | yes | | Trailing blank lines / missing final newline |
| `TrailingWhitespace` | yes | yes | | Trailing whitespace |
| `Zwsp` | no | yes | | Zero-width spaces (U+200B) |
