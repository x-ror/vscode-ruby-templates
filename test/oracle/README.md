# Oracle harness

Compares the official template engine with `ruby-templates` (see `docs/LINT_PLAN.md`, "How we compare"). Stage 0 measures today's code: mismatches are printed, not failed.

## Install

Isolated gems, so the pinned versions don't touch the system Ruby:

```sh
export GEM_HOME=/tmp/oracle-gems GEM_PATH=/tmp/oracle-gems PATH=/tmp/oracle-gems/bin:$PATH
gem install slim:5.2.2 haml:7.5.1 erubi:1.13.1
```

Embedded engines (Slim README "Embedded engines", the Haml filters) need their own gems; without one the case is reported as "dependency missing", never as a template error.

## Run

```sh
cargo build --release --manifest-path server/Cargo.toml
bun test/oracle.js slim          # VERBOSE=1 lists every mismatch
```

Without Slim 5.2.2 on `ruby` it prints a skip message and exits 0.

- `test/oracle/slim.rb`: compiles the template on stdin with Slim's defaults and prints `{"error": {message, line, column, slim_column}}`, `{"dependency_missing": gem}` or `{"source": ruby}`.
- `test/oracle/slim_cases.rb`: replays `test/core/*.rb` of the Slim checkout against a recording stub and prints the templates they render. Tests that need non-default options are skipped and counted.
- `format:` line: for each case Slim compiles and we don't flag, `ruby-templates format` runs, then `slim.rb --rails` compiles the text before and after (the Railtie's generator and options, without ActionView: only source is generated) and they must be identical; formatting the result again must change nothing. Cases with an error of ours aren't formatted and are counted apart.
- The Slim checkout (tag `v5.2.2`, shallow) is cloned once into `$ORACLE_CACHE`, default `$TMPDIR/ruby-templates-oracle`, as `slim-5.2.2`.
- `error.column` is a 0-based UTF-16 offset produced by `slim.rb` from Slim's character column (`slim_column`); the CLI prints 1-based columns, so they are compared with a +1.
