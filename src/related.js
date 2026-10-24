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

export { relatedFiles, relatedDirs };
