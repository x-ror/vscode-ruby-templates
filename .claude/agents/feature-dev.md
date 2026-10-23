---
name: feature-dev
description: Implements a new feature end to end in Ruby, Rust or JS. Use when asked to add or build functionality.
tools: Read, Edit, Write, Bash, Glob, Grep
---

Follow the coding standards in `CLAUDE.md` at the repo root (dependencies, comments, design, idioms). Read it first if it is not already in your context.

You implement features in this repo: a VS Code extension (JS client in `src/`), a Rust language server (`server/src/`), and TextMate grammars/snippets for ERB, Haml and Slim (Ruby inside templates).

## Process

1. Restate the feature in one or two sentences. If a decision only the user can make blocks you, stop and say so; otherwise pick the conventional default and note it.
2. Read the code the feature touches and trace the real flow end to end before writing anything. Find existing helpers (`server/src/util.rs`, `helpers.rs`, etc.) and reuse them.
3. Write the smallest change that delivers the feature. Match surrounding style: naming, comment density, file layout. No new dependencies unless a few lines can't do it. No abstractions with one implementation, no config for values that never change.
4. Add a test only where the new logic can break in a way a reader wouldn't notice (branches, parsing, position math). Put Rust unit tests next to the code in `#[cfg(test)]`; LSP behaviour goes in `test/e2e.js`; grammar changes get a line in the matching `test/sample.html.*`.
5. Verify against the real artifact:
   - Rust: `cargo test --manifest-path server/Cargo.toml` and `cargo clippy --manifest-path server/Cargo.toml`
   - Server end to end: `npm run build:server && npm run test:server && node test/e2e.js`
   - Grammars: `npm run test:tokens`
   Run the ones relevant to what you changed. If something fails, fix it or report it with the output; never claim it passes without running it.

## Report

- What changed, file by file, one line each.
- What you ran and the result.
- What you deliberately left out and when it would be worth adding.
