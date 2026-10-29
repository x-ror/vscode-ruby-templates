# Formatter settings

The built-in formatter (`ruby-templates format`, and the language server's _Format Document_) has two settings. The indentation width comes from, highest precedence first:

1. The editor's tab size, when the language server formats.
2. `.editorconfig`, from the file's folder up to the first `root = true`; nearer files and later sections win. `indent_size` counts; `indent_size = tab` (or `indent_size` unset with `indent_style = tab`) takes `tab_width`.
3. The default, 2.

ERB comments are the other: by default the lines of an HTML comment after the one holding `<!--`, and of a multi-line `<%# … %>` after its first line, keep every byte. `rubyTemplates.erb.format.alignComments` in VS Code, or `--align-comments` on the command line, moves them with the line the comment opens on, keeping their relative indentation. A comment's lines keep their bytes, as without the setting, when one of them is left of its first line or the comment holds a multi-line ERB tag. Format Selection never moves them, since the selection may leave out the line they move with. `<![CDATA[`, `<!DOCTYPE` and `<?…?>` aren't comments and never move.

Everything else is fixed, per language. Indentation is always written in spaces, even for a tab-indenting editor or `indent_style = tab`; each line keeps its ending.

- **ERB** (HTML views only): keeps the end of the file as it is, and trims trailing whitespace of text lines. Content whitespace (`<pre>`, `<textarea>`, Ruby strings…) is never trimmed or collapsed.
- **Haml**: trims trailing whitespace only where Haml ignores it (not in filter bodies), keeps blank lines before the last line (Haml compiles them), and collapses the empty and whitespace-only lines at the end into one line ending, since Haml strips the template. A file with any Haml error is left unchanged.
- **Slim**: trims trailing whitespace only where Slim ignores it, keeps blank lines, since Slim compiles every line, and ends the file with one line ending. A format selection leaves the end of the file as it is.

`ruby-templates format --verbose` prints the settings used for each file, the `.editorconfig` files read and every property ignored: `end_of_line`, `insert_final_newline`, `trim_trailing_whitespace`, `charset`, `max_line_length`, any other key, values that don't parse (`indent_size = 4 # four`), and sections whose glob can't be read.
