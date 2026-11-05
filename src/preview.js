// The rendered-HTML preview and the compiled Ruby source of a template.
import * as vscode from "vscode";
import path from "path";

import { cfg, workspaceFolderFor } from "./workspace.js";
import {
  RAILS_RENDER_TIMEOUT_MS,
  findUp,
  run,
  runHaml,
  firstLine,
} from "./run.js";
import { erbRubyArgs } from "./erbEngine.js";
import {
  TEMPLATE_LANGS,
  engineOf,
  viewDir,
  railsRenderScript,
  missingToolMessage,
  previewPage,
  errorPage,
} from "./render.js";

/** @type {vscode.WebviewPanel | undefined} */
let previewPanel;
/** @type {string | undefined} */
let previewUri;

/** Render a template document to HTML with the matching CLI. */
async function renderToHtml(doc) {
  const lang = engineOf(doc.languageId);
  const cwd = workspaceFolderFor(doc);
  const src = doc.getText();
  const railsRoot =
    cfg().get("preview.useRails", false) && doc.uri.scheme === "file"
      ? findUp(path.dirname(doc.uri.fsPath), path.join("bin", "rails"))
      : undefined;
  if (railsRoot)
    return renderInRails(
      railsRoot,
      viewDir(railsRoot, doc.uri.fsPath),
      lang,
      src,
    );
  if (lang === "slim") {
    const bin = cfg().get("commands.slimrb", "slimrb");
    return run(bin, ["--pretty"], { cwd }, src);
  }
  if (lang === "haml") {
    return runHaml(cfg().get("commands.haml", "haml"), "render", [], cwd, src);
  }
  if (lang === "erb")
    return run(
      cfg().get("commands.ruby", "ruby"),
      erbRubyArgs("render", doc.languageId === "erb-percent"),
      { cwd },
      src,
    );
  return { code: 1, stdout: "", stderr: `Unsupported language: ${lang}` };
}

/** Render through the app's `bin/rails runner`, so helpers, routes and partials resolve. */
async function renderInRails(root, dir, lang, src) {
  const r = await run(
    "ruby",
    [path.join("bin", "rails"), "runner", railsRenderScript(dir, lang)],
    { cwd: root, timeout: RAILS_RENDER_TIMEOUT_MS },
    src,
  );
  if (r.err?.killed)
    return {
      ...r,
      code: 1,
      stderr: `bin/rails runner timed out after ${RAILS_RENDER_TIMEOUT_MS / 1000}s.\n${r.stderr}`,
    };
  return r;
}

/** Compile a template to the Ruby source the engine generates. */
async function compileToRuby(doc) {
  const lang = engineOf(doc.languageId);
  const cwd = workspaceFolderFor(doc);
  const src = doc.getText();
  if (lang === "slim") {
    const args = cfg().get("slim.compileForRails", false)
      ? ["--rails"]
      : ["--compile"];
    return run(cfg().get("commands.slimrb", "slimrb"), args, { cwd }, src);
  }
  if (lang === "haml") {
    return runHaml(
      cfg().get("commands.haml", "haml"),
      "compile",
      ["--debug"],
      cwd,
      src,
    );
  }
  if (lang === "erb")
    return run(
      cfg().get("commands.ruby", "ruby"),
      erbRubyArgs("compile", doc.languageId === "erb-percent"),
      { cwd },
      src,
    );
  return { code: 1, stdout: "", stderr: `Unsupported language: ${lang}` };
}

async function openPreview(context) {
  const ed = vscode.window.activeTextEditor;
  if (!ed || !TEMPLATE_LANGS.has(engineOf(ed.document.languageId))) {
    vscode.window.showInformationMessage(
      "Open an ERB, Haml or Slim file first.",
    );
    return;
  }
  previewUri = ed.document.uri.toString();
  if (!previewPanel) {
    previewPanel = vscode.window.createWebviewPanel(
      "rubyTemplates.preview",
      "Template Preview",
      vscode.ViewColumn.Beside,
      { enableScripts: false },
    );
    previewPanel.onDidDispose(
      () => {
        previewPanel = undefined;
        previewUri = undefined;
      },
      null,
      context.subscriptions,
    );
  }
  previewPanel.title = `Preview: ${path.basename(ed.document.fileName)}`;
  await renderPreview(ed.document);
}

/** Re-renders the preview when the document it shows is saved. */
function refreshPreviewOnSave(doc) {
  if (!previewPanel || !previewUri) return;
  if (!cfg().get("preview.refreshOnSave", true)) return;
  if (doc.uri.toString() === previewUri) renderPreview(doc);
}

async function renderPreview(doc) {
  const panel = previewPanel;
  if (!panel) return;
  const r = await renderToHtml(doc);
  // The panel may have closed, or moved to another document, during a slow render.
  if (previewPanel !== panel || previewUri !== doc.uri.toString()) return;
  if (r.err?.code === "ENOENT") {
    panel.webview.html = errorPage(
      "Renderer not found",
      missingToolMessage(r.err.path, doc.languageId),
    );
    return;
  }
  if (r.code !== 0) {
    panel.webview.html = errorPage(
      "Render failed",
      r.stderr || r.stdout,
      engineOf(doc.languageId) === "erb"
        ? "A standalone Erubi render has no Rails helpers (link_to, render, …); set rubyTemplates.preview.useRails to render inside the app."
        : "Rails helpers (link_to, render, …) are not available in a standalone render; set rubyTemplates.preview.useRails to render inside the app.",
    );
    return;
  }
  panel.webview.html = previewPage(
    path.basename(doc.fileName),
    doc.languageId,
    r.stdout,
  );
}

function closePreview() {
  previewPanel?.dispose();
}

async function showCompiledSource() {
  const ed = vscode.window.activeTextEditor;
  if (!ed || !TEMPLATE_LANGS.has(engineOf(ed.document.languageId))) return;
  const r = await compileToRuby(ed.document);
  if (r.err?.code === "ENOENT") {
    vscode.window.showErrorMessage(
      missingToolMessage(r.err.path, ed.document.languageId),
    );
    return;
  }
  if (r.code !== 0) {
    vscode.window.showErrorMessage(`Compile failed: ${firstLine(r)}`);
    return;
  }
  const doc = await vscode.workspace.openTextDocument({
    language: "ruby",
    content: r.stdout,
  });
  await vscode.window.showTextDocument(doc, {
    viewColumn: vscode.ViewColumn.Beside,
    preview: true,
  });
}

export { openPreview, refreshPreviewOnSave, closePreview, showCompiledSource };
