// Where a Rails app keeps its views (no vscode dependency; tested in test/clientPure.js).

// Either separator, so a Windows path matches on any platform. Greedy: the innermost app/views wins.
const VIEWS_ROOT = /^(.*[/\\]app[/\\]views)[/\\]/;

/** The `…/app/views` folder that holds `file`, or undefined outside app/views. */
const viewsRoot = (file) => VIEWS_ROOT.exec(file)?.[1];

export { viewsRoot };
