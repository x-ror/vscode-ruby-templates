// The template language ids and the engine each one runs on (no vscode dependency; tested in test/clientPure.js).

const TEMPLATE_LANGS = new Set(["erb", "haml", "slim"]);
// ERB with `trim_mode: "%"`: lines starting with `%` are Ruby.
const ERB_PERCENT = "erb-percent";

// Host-specific ERB ids (erb-js, erb-css, ...) render and compile like plain ERB.
const engineOf = (languageId) =>
  languageId.startsWith("erb-")
    ? "erb"
    : languageId === "skim"
      ? "slim"
      : languageId;

const isTemplateLanguage = (languageId) =>
  TEMPLATE_LANGS.has(engineOf(languageId));

const isErb = (languageId) => engineOf(languageId) === "erb";

export { ERB_PERCENT, engineOf, isTemplateLanguage, isErb };
