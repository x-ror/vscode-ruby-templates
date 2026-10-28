# ruby-templates

A whitespace-only formatter and the structure checks for ERB, Haml and Slim templates, from the [Ruby Templates](https://github.com/x-ror/vscode-ruby-templates) VS Code extension's language server.

```sh
npm install --save-dev @x-ror/ruby-templates
npx ruby-templates check app/views
npx ruby-templates format --check app/views
npx ruby-templates format --write app/views
npx ruby-templates format --stdin-filepath app/views/users/show.html.erb < show.html.erb
```

The formatter changes only indentation, trailing whitespace, blank lines, the final newline and line endings, and leaves a file it is in doubt about unchanged, saying why on stderr. The only setting is the indentation width, from `.editorconfig` (`indent_size`, default 2); see [the settings](https://github.com/x-ror/vscode-ruby-templates/blob/main/docs/FORMATTER_CONFIG.md). Exit codes: 0 ok, 1 diagnostics or files that would change, 2 usage or I/O error.

The binary comes from a per-platform package (Linux x64/arm64, Windows x64) installed as an optional dependency; `RUBY_TEMPLATES_BINARY` points the wrapper at another build.
