// Pure part of the herb-lint diagnostics (no vscode dependency; tested in test/clientPure.js).

/**
 * A `herb-lint --json` report as 0-based offenses, or the reason it didn't run: herb-lint exits 0 or 1 on
 * a broken config or an unmatched path alike, so `completed` is the only signal. Columns stay in characters.
 */
function herbLintReport(json) {
  const { offenses = [], completed, message } = JSON.parse(json);
  if (completed === false)
    return { offenses: [], error: message ?? "herb-lint did not complete" };
  return {
    offenses: offenses.map(({ code, message, severity, location }) => {
      const line = location.start.line - 1;
      const endLine = location.end.line - 1;
      const column = location.start.column;
      const endColumn =
        endLine === line
          ? Math.max(location.end.column, column + 1)
          : location.end.column;
      return { line, column, endLine, endColumn, code, message, severity };
    }),
    error: null,
  };
}

export { herbLintReport };
