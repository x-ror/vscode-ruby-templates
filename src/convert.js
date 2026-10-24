// Pure part of the template converters (no vscode dependency).

/**
 * Where a converted template is saved: index.html → index.html.haml, show.html.erb → show.html.haml,
 * show.html.haml.tt → show.html.slim.tt, show.html.erubi → show.html.haml. `from` is the source language id.
 */
function convertedFileName(fileName, from, target) {
  const renamed = fileName.replace(
    /\.(erb|erubis?|haml|rhtml|slim)(\.tt)?$/i,
    `.${target}$2`,
  );
  return from !== "html" && renamed !== fileName
    ? renamed
    : `${fileName}.${target}`;
}

export { convertedFileName };
