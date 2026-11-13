// Pure part of the template converters (no vscode dependency).

// `command` is both the rubyTemplates.commands.* setting name and its default executable.
const CONVERTERS = {
  "erb>haml": { command: "html2haml", args: ["--erb"], gem: "html2haml" },
  "html>haml": { command: "html2haml", args: ["--no-erb"], gem: "html2haml" },
  "erb>slim": { command: "erb2slim", args: [], gem: "html2slim" },
  "haml>slim": { command: "haml2slim", args: [], gem: "haml2slim" },
  // haml_to_erb's own CLI only converts files in place, so call its library through ruby.
  "haml>erb": {
    command: "ruby",
    args: ["-rhaml_to_erb", "-e", "print HamlToErb.convert($stdin.read)"],
    gem: "haml_to_erb",
  },
  "slim>erb": { command: "slimrb", args: ["--erb"], gem: "slim" },
};

/**
 * Where a converted template is saved: index.html → index.html.haml, show.html.erb → show.html.haml,
 * show.html.haml.tt → show.html.slim.tt, show.html.erubi → show.html.haml. `from` is the source language id.
 */
function convertedFileName(fileName, from, target) {
  const renamed = fileName.replace(
    /\.(erb|erubis?|haml|rhtml|slim|skim)(\.tt)?$/i,
    `.${target}$2`,
  );
  return from !== "html" && renamed !== fileName
    ? renamed
    : `${fileName}.${target}`;
}

export { CONVERTERS, convertedFileName };
