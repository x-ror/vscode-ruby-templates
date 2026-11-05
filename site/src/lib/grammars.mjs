// The extension's own TextMate grammars, so every example on the site looks as it does in VS Code.
import fs from "node:fs";
import path from "node:path";
import { bundledLanguages } from "shiki";

// Builds run from site/; this module is bundled elsewhere, so its own path says nothing.
const repo = path.resolve(process.cwd(), "..");
const manifest = JSON.parse(
  fs.readFileSync(path.join(repo, "package.json"), "utf8"),
);

// Shiki's own names for the scopes the grammars embed.
const EMBEDDED = {
  "source.ruby": "ruby",
  "text.html.basic": "html",
  "source.js": "javascript",
  "source.css": "css",
  "source.css.scss": "scss",
  "source.sass": "sass",
  "source.css.less": "less",
  "source.coffee": "coffee",
  "source.json": "json",
  "source.yaml": "yaml",
  "text.xml": "xml",
  "text.html.markdown": "markdown",
};

function grammarOf({ language, path: file }) {
  const grammar = JSON.parse(fs.readFileSync(path.join(repo, file), "utf8"));
  const scopes = [
    ...JSON.stringify(grammar).matchAll(
      /"include":\s*"((?:source|text)\.[\w.-]+)/g,
    ),
  ].map((m) => m[1]);
  const embeddedLangs = [
    ...new Set(scopes.map((scope) => EMBEDDED[scope]).filter(Boolean)),
  ];
  return { ...grammar, name: language, embeddedLangs };
}

// Embedded languages come first: a grammar's embeddedLangs must already be registered.
export const LANGS = [
  ...Object.values(EMBEDDED).map((name) => bundledLanguages[name]),
  bundledLanguages.shellscript,
  ...manifest.contributes.grammars.filter((g) => g.language).map(grammarOf),
];
