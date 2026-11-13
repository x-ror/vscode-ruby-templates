// Pure part of the preview and compiled-source commands (no vscode dependency; tested in test/clientPure.js).
import path from "path";

import { engineOf, isErb } from "./languages.js";

// A script file, not `-e`: Windows runs commands through cmd.exe, which can't pass its quotes and newlines.
const ERB_SCRIPT = path.join(
  import.meta.dirname,
  "..",
  "ruby",
  "erb_engine.rb",
);

/** `ruby` arguments that compile (`"compile"`) or render (`"render"`) the ERB on stdin; `percentLines` for `trim_mode: "%"`. */
function erbRubyArgs(mode, percentLines = false) {
  return percentLines ? [ERB_SCRIPT, mode, "percent"] : [ERB_SCRIPT, mode];
}

/** `users` for `<root>/app/views/users/show.html.slim`; undefined outside app/views. */
function viewDir(root, file) {
  const rel = path.relative(
    path.join(root, "app", "views"),
    path.dirname(file),
  );
  return rel && !rel.startsWith("..") && !path.isAbsolute(rel)
    ? rel.split(path.sep).join("/")
    : undefined;
}

const rubyString = (s) => `'${s.replace(/[\\']/g, "\\$&")}'`;

/**
 * The `bin/rails runner` script rendering the template on stdin. A controller whose path is the template's
 * directory makes a bare `render "card"` find `<dir>/_card`, as in a request.
 */
function railsRenderScript(dir, lang) {
  const controller = dir
    ? `Class.new(ApplicationController) { define_singleton_method(:controller_path) { ${rubyString(dir)} } }`
    : "ApplicationController";
  return `print ${controller}.render(inline: $stdin.read, type: :${lang})`;
}

function missingToolMessage(bin, lang) {
  if (isErb(lang))
    return `"${bin}" was not found. Install Ruby (and gem install erubi) — or point rubyTemplates.commands.ruby at it.`;
  const gem = { slim: "slim", haml: "haml" }[engineOf(lang)] || bin;
  return `"${bin}" was not found. Install it with: gem install ${gem} — or point rubyTemplates.commands.* at the executable.`;
}

const escapeHtml = (s) =>
  String(s).replace(
    /[&<>]/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c],
  );

/** The preview webview: `html` in a sandboxed frame above its source. */
function previewPage(name, languageId, html) {
  return `<!DOCTYPE html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline';">
<style>
  body{font-family:system-ui,sans-serif;margin:0}
  .bar{display:flex;gap:1rem;align-items:center;padding:.4rem .8rem;font-size:12px;background:var(--vscode-editorWidget-background);border-bottom:1px solid var(--vscode-widget-border,#444)}
  .bar b{opacity:.8}
  iframe{border:0;width:100%;height:calc(55vh);background:#fff}
  pre{margin:0;padding:.8rem;font:12px/1.4 var(--vscode-editor-font-family,monospace);white-space:pre-wrap;word-break:break-word;background:var(--vscode-textCodeBlock-background);max-height:40vh;overflow:auto}
</style></head><body>
<div class="bar"><b>Rendered</b> <span>${escapeHtml(name)} · ${escapeHtml(languageId)}</span></div>
<iframe sandbox="" srcdoc="${escapeHtml(html).replace(/"/g, "&quot;")}"></iframe>
<div class="bar"><b>HTML source</b></div>
<pre>${escapeHtml(html)}</pre>
</body></html>`;
}

function errorPage(title, detail, hint) {
  return `<!DOCTYPE html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline';"></head><body style="font-family:system-ui;padding:1rem">
<h3 style="color:var(--vscode-errorForeground)">${escapeHtml(title)}</h3>
<pre style="white-space:pre-wrap">${escapeHtml(detail)}</pre>
${hint ? `<p style="opacity:.8">${escapeHtml(hint)}</p>` : ""}</body></html>`;
}

export {
  erbRubyArgs,
  viewDir,
  railsRenderScript,
  missingToolMessage,
  previewPage,
  errorPage,
};
