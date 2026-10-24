// Pure part of the erb_lint diagnostics (no vscode dependency; tested in test/clientPure.js).

/** erb_lint exits 1 when it found offenses; anything else non-zero is a failure. */
const ERB_LINT_OK_CODES = new Set([0, 1]);

/**
 * Offenses of an `erb_lint --format json` report, with 0-based lines. erb_lint's columns are 0-based already;
 * a zero-length offense (a missing space) is widened to one character so the editor can show it.
 */
function erbLintOffenses(json) {
  return JSON.parse(json)
    .files.flatMap((f) => f.offenses ?? [])
    .map(({ linter, message, location }) => {
      const line = location.start_line - 1;
      const endLine = location.last_line - 1;
      const column = location.start_column;
      const endColumn =
        endLine === line
          ? Math.max(location.last_column, column + 1)
          : location.last_column;
      return { line, column, endLine, endColumn, linter, message };
    });
}

export { ERB_LINT_OK_CODES, erbLintOffenses };
