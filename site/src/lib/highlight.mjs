import { createHighlighter } from "shiki";
import { LANGS, THEMES } from "./grammars.mjs";

let highlighter;

/** Lines of `code` as tokens carrying both theme colours (`--shiki-light`, `--shiki-dark`). */
export async function tokenLines(code, lang) {
  highlighter ??= createHighlighter({
    themes: Object.values(THEMES),
    langs: LANGS,
  });
  const { tokens } = (await highlighter).codeToTokens(code, {
    lang,
    themes: THEMES,
    defaultColor: false,
  });
  return tokens;
}
