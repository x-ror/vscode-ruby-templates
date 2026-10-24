// Pure part of the ERB formatter (no vscode dependency; tested in test/clientPure.js).

/** The external ERB formatters, keyed by `rubyTemplates.erb.formatter`. */
const ERB_FORMATTERS = {
  htmlbeautifier: { command: "htmlbeautifier", gem: "htmlbeautifier" },
  "erb-format": { command: "erbFormat", gem: "erb-formatter" },
};

/** Arguments that make `tool` read stdin and indent like the editor. */
function erbFormatterArgs(tool, { tabSize, insertSpaces }) {
  if (tool === "htmlbeautifier") {
    return insertSpaces ? ["--tab-stops", String(tabSize)] : ["--tab"];
  }
  return ["--stdin"];
}

export { ERB_FORMATTERS, erbFormatterArgs };
