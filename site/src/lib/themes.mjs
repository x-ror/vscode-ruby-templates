// The code themes of the site: GitHub's, with crimson kept for fault marks and every token at 4.5:1 or more on the
// code ground and on a fault line's tint.
import { bundledThemes } from "shiki";

const REPLACEMENTS = {
  "github-light": {
    "#d73a49": "#6f42c1",
    "#b31d28": "#6f42c1",
    "#22863a": "#1d7332",
    "#6a737d": "#5d656e",
    "#e36209": "#a34706",
    "#f9826c": "#dcdbe1",
  },
  "github-dark-dimmed": {
    "#f47067": "#dcbdfb",
    "#ff938a": "#dcbdfb",
    "#768390": "#949ea8",
  },
};

async function themed(name) {
  let json = JSON.stringify((await bundledThemes[name]()).default);
  for (const [from, to] of Object.entries(REPLACEMENTS[name]))
    json = json.replaceAll(new RegExp(from, "gi"), to);
  return JSON.parse(json);
}

export const THEMES = {
  light: await themed("github-light"),
  dark: await themed("github-dark-dimmed"),
};
