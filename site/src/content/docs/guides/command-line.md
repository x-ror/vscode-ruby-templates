---
title: Command line for CI
description: Run the same template checks and formatter outside the editor with the ruby-templates command from npm.
sidebar:
  order: 8
---

The same checks and formatter run outside the editor, from npm, with no gems to install.

## Install

```sh
npm install --save-dev @x-ror/ruby-templates
```

## Check templates

```sh
npx ruby-templates check app/views
```

`check` prints the diagnostics (errors and warnings) of ERB, Haml, Slim, Jbuilder and Builder templates, one per line with the file, line and column:

```text
app/views/users/index.html.erb:4:3: error: unexpected end-of-input, assuming it is closing the parent top level context
app/views/users/index.html.erb:3:5: warning: `<img>` without `alt`: the alt attribute must be specified (HTML 4.8.4.4.1)
```

It exits with 1 if there are any. The checks are the ones described in [Errors as you type](/vscode-ruby-templates/guides/checks/); the command line checker reads ERB as Erubi does, so `erb-percent` `%` lines are not modelled.

## Format templates

```sh
npx ruby-templates format --check app/views   # files that would change; exit 1 if any
npx ruby-templates format --write app/views   # format them
```

Without `--check` or `--write`, the result goes to standard output. A file whose structure is in doubt is left as it is, with the reason on standard error. See [Formatting](/vscode-ruby-templates/guides/formatting/) for what the formatter changes.

### Options

| Option                    | What it does                                                                       |
| ------------------------- | ---------------------------------------------------------------------------------- |
| `--check`                 | Lists the files that would change; exits with 1 if any                             |
| `--write`                 | Writes the formatted files                                                         |
| `--verbose`               | Also prints the settings and `.editorconfig` files used                            |
| `--align-comments`        | Moves the lines inside ERB comments with the line the comment opens on             |
| `--stdin-filepath <path>` | Formats the template read from standard input as the file at `<path>`, for editors |

```sh
npx ruby-templates format --stdin-filepath show.html.erb < show.html.erb   # from stdin, for editors
npx ruby-templates format --verbose --check app/views   # also the settings and .editorconfig files used
```

## Files and folders

Pass files or folders. Folders are walked for template files, skipping hidden folders and `node_modules`.

## Exit codes

| Code | Meaning                                                                                  |
| ---- | ---------------------------------------------------------------------------------------- |
| `0`  | OK                                                                                       |
| `1`  | Diagnostics (errors or warnings), or files that would change                             |
| `2`  | Usage or I/O error, a formatter bug, or (for `check`) a named file that isn't a template |

## GitHub Actions

With `@x-ror/ruby-templates` in your `package.json` dev dependencies, add steps like these to a job that has checked out the repository:

```yaml
- run: npm ci
- run: npx ruby-templates check app/views
- run: npx ruby-templates format --check app/views
```

Each step fails the job when the command exits with a non-zero code.
