# Formatter settings

Where the built-in formatter (`ruby-templates format`, and the language server's _Format Document_) takes its settings from, highest precedence first:

1. The editor's options, when the language server formats: tab size and spaces/tabs always; trim trailing whitespace, insert final newline and trim final newlines only when the editor turns them on.
2. `.editorconfig`, from the file's folder up to the first `root = true`; nearer files and later sections win.
3. The template language's linter config, the nearest one from the file's folder up: `.erb_lint.yml` or `.erb-lint.yml` for ERB, `.haml-lint.yml` for Haml, `.slim-lint.yml` for Slim and Skim. Only the linters below are read, and only for the setting they check.
4. Defaults: 2 spaces, trailing whitespace trimmed, a final newline, line endings and blank lines as they are.

`ruby-templates format --verbose` prints the settings used for each file, the files read and every key or linter ignored.

## Settings

| Setting                  | `.editorconfig`                                        | erb_lint                                   | haml-lint | slim-lint            | Default |
| ------------------------ | ------------------------------------------------------ | ------------------------------------------ | --------- | -------------------- | ------- |
| Indent with spaces/tabs  | `indent_style`                                         | `SpaceIndentation` → spaces                | —         | `Tab` → spaces       | spaces  |
| Indent width             | `indent_size` (`tab`, or unset with tabs: `tab_width`) | —                                          | —         | —                    | 2       |
| Line endings             | `end_of_line` (`lf`, `crlf`)                           | —                                          | —         | —                    | kept    |
| Final newline            | `insert_final_newline` (`false` drops it)              | `FinalNewline` (`present: false` drops it) | —         | —                    | added   |
| Trim trailing whitespace | `trim_trailing_whitespace`                             | `TrailingWhitespace`                       | —         | `TrailingWhitespace` | on      |
| At most one blank line   | —                                                      | `ExtraNewline`                             | —         | —                    | off     |
| No blank lines at EOF    | —                                                      | `FinalNewline` (`present: true`)           | —         | —                    | off     |

A linter counts when the config enables it, or doesn't mention it and the tool enables it by default: slim-lint enables these by default; erb_lint only with `EnableDefaultLinters: true`. A linter set to `enabled: false` has no say (the next source decides), since turning a check off doesn't ask for the opposite; neither has one whose settings can't be read. Booleans are read as Ruby reads YAML (`true`/`false`, `yes`/`no`, `on`/`off`). A linter's other options, `include` and `exclude` among them, are ignored: a mapped linter applies to every file of its language.

What each language does with them:

- **ERB** (HTML views only): only the indentation width counts. It always writes spaces, keeps each line's ending and the end of the file, and trims trailing whitespace of text lines. Content whitespace (`<pre>`, `<textarea>`, Ruby strings…) is never trimmed or collapsed.
- **Haml**: as Slim: only the indentation width counts. It always writes spaces, keeps each line's ending, trims trailing whitespace only where Haml ignores it (not in filter bodies), keeps blank lines before the last line (Haml compiles them) and collapses the empty and whitespace-only lines at the end into one line ending, since Haml strips the template. `end_of_line`, `insert_final_newline`, `trim_trailing_whitespace`, the editor's file options and `.haml-lint.yml` change nothing. A file with any Haml error is left unchanged.
- **Slim**: only the indentation width counts (the editor's tab size or `indent_size`, default 2). It always writes spaces, even for a tab-indenting editor or `indent_style = tab`, keeps each line's ending, ends the file with one line ending, trims trailing whitespace only where Slim ignores it and keeps blank lines, since Slim compiles every line. `end_of_line`, `insert_final_newline`, `trim_trailing_whitespace`, the editor's file options and `.slim-lint.yml` change nothing.

## Ignored on purpose

Listed by `--verbose` when present:

- `.editorconfig`: `charset`, `max_line_length`, `end_of_line = cr`, any other key, values that don't parse (`indent_size = 4 # four`), and sections whose glob can't be read.
- erb_lint `SpaceAroundErbTag`: it changes what is inside ERB tags, which the formatter never does.
- slim-lint `EmptyLines` and `TrailingBlankLines`: Slim compiles blank lines.
- haml-lint: every linter. Haml takes only the indentation width, so none applies; the file is still read (`--verbose` lists it).
- Every other linter, `inherit_from`, `exclude`, and other top-level keys. The YAML reader handles nested mappings of plain or quoted scalars; anything else is skipped.

This is not erb_lint, haml-lint or slim-lint compatibility: the formatter reads these files only to agree with them on whitespace. Run the linters themselves for everything they check.
