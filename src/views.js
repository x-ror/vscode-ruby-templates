// Where a Rails app keeps its views (no vscode dependency; tested in test/clientPure.js).

// Either separator, so a Windows path matches on any platform. Greedy: the innermost app/views wins.
const VIEWS_ROOT = /^(.*[/\\]app[/\\]views)[/\\]/;

/** The `…/app/views` folder that holds `file`, or undefined outside app/views. */
const viewsRoot = (file) => VIEWS_ROOT.exec(file)?.[1];

/** `admin/users` for `…/app/views/admin/users/show.html.erb`; undefined outside app/views or right in it. */
function viewDirOf(file) {
  const root = viewsRoot(file);
  if (!root) return undefined;
  const dirs = file
    .slice(root.length + 1)
    .split(/[/\\]/)
    .slice(0, -1);
  return dirs.length ? dirs.join("/") : undefined;
}

export { viewsRoot, viewDirOf };
