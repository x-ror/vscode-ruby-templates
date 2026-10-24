// Pure part of "Switch to Related File" (no vscode dependency; tested in test/clientPure.js).

const stemOf = (name) => name.split(".")[0];

/**
 * Files in `siblings` (names in the same folder) that share `name`'s stem: the other formats of a view
 * (`welcome.html.erb` ↔ `welcome.text.erb`, `show.json.jbuilder`) and a ViewComponent's class and template
 * (`card_component.rb` ↔ `card_component.html.erb`). Sorted, without `name` itself.
 */
function relatedFiles(name, siblings) {
  const stem = stemOf(name);
  return siblings
    .filter((s) => s !== name && s.includes(".") && stemOf(s) === stem)
    .sort();
}

/**
 * Folders that may hold related files of `dir/name`, besides `dir`: a component's sidecar folder
 * (`card_component/`) next to `card_component.rb`, and the folder holding the class for a template inside it.
 */
function relatedDirs(dir, name, sep = "/") {
  const stem = stemOf(name);
  const parts = dir.split(sep);
  if (parts.at(-1) === stem) return [parts.slice(0, -1).join(sep)];
  return name.endsWith(".rb") ? [`${dir}${sep}${stem}`] : [];
}

/**
 * The controller (or mailer) file and action that render the view at `file`: `app/views/admin/users/show.html.erb`
 * → `app/controllers/admin/users_controller.rb` `show`, `app/views/user_mailer/welcome.text.erb` →
 * `app/mailers/user_mailer.rb` `welcome`. Undefined for partials, layouts and files outside app/views.
 */
function actionSource(file, sep = "/") {
  const parts = file.split(sep);
  const views = parts.lastIndexOf("views");
  if (views < 1 || parts[views - 1] !== "app") return undefined;
  const dirs = parts.slice(views + 1, -1);
  const action = stemOf(parts.at(-1));
  if (!dirs.length || dirs[0] === "layouts" || !/^[a-z]\w*$/.test(action))
    return undefined;
  const app = parts.slice(0, views);
  const source = dirs.at(-1).endsWith("_mailer")
    ? [...app, "mailers", ...dirs].join(sep) + ".rb"
    : [...app, "controllers", ...dirs].join(sep) + "_controller.rb";
  return { file: source, action };
}

/** 0-based line of `def action` in Ruby `source`, or -1. */
function actionLine(source, action) {
  const def = new RegExp(`^\\s*def\\s+${action}\\s*(?:$|[(;#])`);
  return source.split("\n").findIndex((line) => def.test(line));
}

/**
 * A ViewComponent's previews ↔ the component: `app/components/admin/card_component.rb` (or its template, sidecar
 * included) → `{test,spec}/components/previews/admin/card_component_preview.rb`, and a preview back to its class.
 * Candidates only; the caller keeps the ones that exist.
 */
function previewPartners(file, sep = "/") {
  const parts = file.split(sep);
  const app = parts.lastIndexOf("app");
  if (app !== -1 && parts[app + 1] === "components") {
    const dirs = parts.slice(app + 2, -1);
    const stem = stemOf(parts.at(-1));
    if (!stem.endsWith("_component")) return [];
    const rel = [...(dirs.at(-1) === stem ? dirs.slice(0, -1) : dirs), stem];
    return ["test", "spec"].map((tests) =>
      [...parts.slice(0, app), tests, "components", "previews", ...rel]
        .join(sep)
        .concat("_preview.rb"),
    );
  }
  const previews = parts.lastIndexOf("previews");
  const tests = parts[previews - 2];
  if (
    previews < 2 ||
    parts[previews - 1] !== "components" ||
    !["test", "spec"].includes(tests)
  )
    return [];
  const name = parts.at(-1).match(/^(\w+_component)_preview\.rb$/)?.[1];
  if (!name) return [];
  return [
    [
      ...parts.slice(0, previews - 2),
      "app",
      "components",
      ...parts.slice(previews + 1, -1),
      `${name}.rb`,
    ].join(sep),
  ];
}

export { relatedFiles, relatedDirs, actionSource, actionLine, previewPartners };
