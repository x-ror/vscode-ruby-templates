# Formatter settings

Where the built-in formatter (`ruby-templates format`, and the language server's _Format Document_) takes its settings from, highest precedence first:

1. The editor's options, when the language server formats: tab size and spaces/tabs always; trim trailing whitespace, insert final newline and trim final newlines only when the editor turns them on.
2. `.editorconfig`, from the file's folder up to the first `root = true`; nearer files and later sections win.
3. The template language's linter config, the nearest one from the file's folder up: `.erb_lint.yml` or `.erb-lint.yml` for ERB, `.haml-lint.yml` for Haml, `.slim-lint.yml` for Slim and Skim. Only the linters below are read, and only for the setting they check.
4. Defaults: 2 spaces, trailing whitespace trimmed, a final newline, line endings and blank lines as they are.

`ruby-templates format --verbose` prints the settings used for each file, the files read and every key or linter ignored.

## Settings

| Setting                  | `.editorconfig`                           | erb_lint                                   | haml-lint                  | slim-lint            | Default |
| ------------------------ | ----------------------------------------- | ------------------------------------------ | -------------------------- | -------------------- | ------- |
| Indent with spaces/tabs  | `indent_style`                            | `SpaceIndentation` → spaces                | —                          | `Tab` → spaces       | spaces  |
| Indent width             | `indent_size` (`tab` uses `tab_width`)    | —                                          | —                          | —                    | 2       |
| Line endings             | `end_of_line` (`lf`, `crlf`)              | —                                          | —                          | —                    | kept    |
| Final newline            | `insert_final_newline` (`false` drops it) | `FinalNewline` (`present: false` drops it) | `FinalNewline` (`present`) | —                    | added   |
| Trim trailing whitespace | `trim_trailing_whitespace`                | `TrailingWhitespace`                       | `TrailingWhitespace`       | `TrailingWhitespace` | on      |
| At most one blank line   | —                                         | `ExtraNewline`                             | —                          | —                    | off     |
| No blank lines at EOF    | —                                         | —                                          | `TrailingEmptyLines`       | —                    | off     |

A linter counts when the config enables it, or doesn't mention it and the tool enables it by default: haml-lint and slim-lint enable these by default; erb_lint only with `EnableDefaultLinters: true`. A linter set to `enabled: false` has no say (the next source decides), since turning a check off doesn't ask for the opposite.

What each language does with them:

- **ERB** (HTML views only): all of them. Content whitespace (`<pre>`, `<textarea>`, Ruby strings…) is never trimmed or collapsed.
- **Haml**: keeps the file's own indent unit (its first indented line, as Haml reads it), so indent settings don't apply; whitespace is trimmed on structural lines only, as Haml does itself.
- **Slim**: spaces only (a tab-indented editor or `indent_style = tab` leaves the file alone); blank lines are kept, since Slim compiles every line.

## Ignored on purpose

Listed by `--verbose` when present:

- `.editorconfig`: `charset`, `max_line_length`, `end_of_line = cr`, and any other key.
- erb_lint `SpaceAroundErbTag`: it changes what is inside ERB tags, which the formatter never does.
- haml-lint `Indentation`: Haml files keep their own unit.
- slim-lint `EmptyLines` and `TrailingBlankLines`: Slim compiles blank lines.
- Every other linter, `inherit_from`, `exclude`, and other top-level keys. The YAML reader handles nested mappings of plain or quoted scalars; anything else is skipped.

This is not erb_lint, haml-lint or slim-lint compatibility: the formatter reads these files only to agree with them on whitespace. Run the linters themselves for everything they check.
