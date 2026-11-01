# Contributing

Issues and pull requests are welcome. Coding standards, checks and commit rules are in [CLAUDE.md](CLAUDE.md); open tasks and questions are [GitHub issues](https://github.com/x-ror/vscode-ruby-templates/issues).

## Development

```sh
bun install           # also enables .githooks (pre-commit formats staged JS and Rust)
bun run build:server  # cargo build --release, copies the binary into bin/<platform>/
bun run test:tokens   # token scopes of test/sample.* vs. test/__snapshots__, with VS Code's HTML/Ruby/JS/CSS grammars (UPDATE=1 rewrites them)
bun run test:server   # cargo test + diagnostics check on the samples
bun run test:unit     # pure JS helpers (ERB tags, language configs, Haml toggle)
bun run test:snippets # snippets: unique prefixes, valid bodies, no unescaped `$`
bun test/e2e.js       # LSP end-to-end test against a fake Rails tree (exits 1 on a mismatch)
bun test/cli.js       # the ruby-templates command line on a temp folder
bun test/npm.js       # the npm packages: assembled from the release build, packed, installed offline, run
bun run test:slim     # Slim editing helpers and onEnterRules
bun run test:vscode   # commands, formatter and the server inside a real VS Code (downloads it into .vscode-test/; `xvfb-run -a` first on Linux without a display)
bun run test:format   # Slim/Haml formatters vs. the gems: test/format/*.{slim,haml} compile the same before and after (needs `ruby -rslim` / `ruby -rhaml`)
bun run build         # bundle the client into dist/extension.js (what VS Code loads)
bun run package       # bundle, then build the .vsix
```

Grammars live in `syntaxes/`, language configs in `language-configuration/`, snippets in `snippets/`, the VS Code client in `src/`, and the language server in `server/src/` (a library shared by `ruby-templates-ls` and the `ruby-templates` CLI: `lsp.rs`, `cli.rs`, `format.rs` with `erb_format.rs`, `haml_format.rs`, `slim_format.rs` and `config.rs` for the formatter, `erb.rs`, `haml_indent.rs` and `slim_indent.rs` for Haml/Slim structure, `slim_parser.rs` (a port of `Slim::Parser` used by Slim completion, hover and formatting), `partials.rs`, `ruby.rs`). CI (`.github/workflows/build.yml`) builds the server for Linux, macOS and Windows, packages one `.vsix` per platform, and assembles the npm packages (`npm/`, `npm/assemble.js`); tags publish them.
