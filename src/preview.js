// The rendered-HTML preview and the compiled Ruby source of a template.
import * as vscode from "vscode";
import path from "path";

import { settings, commandPath, workspaceFolderFor } from "./workspace.js";
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

// The preview command and its webview's type.
const PREVIEW = "rubyTemplates.preview";
const STATUS_BAR_PRIORITY = 90;

/** @type {vscode.WebviewPanel | undefined} */
let previewPanel;
/** @type {string | undefined} */
let previewUri;

/** Slim's flags: pretty HTML for a render, `--rails` or plain Ruby for a compile. */
function slimArgs(mode) {
  if (mode === "render") return ["--pretty"];
  return settings().get("slim.compileForRails", false)
    ? ["--rails"]
    : ["--compile"];
}

/** Runs the template engine's CLI on the document, `mode` being "render" (HTML) or "compile" (Ruby source). */
function runEngine(doc, mode) {
  const lang = engineOf(doc.languageId);
  const cwd = workspaceFolderFor(doc);
  const src = doc.getText();
  if (lang === "slim")
    return run(commandPath("slimrb"), slimArgs(mode), { cwd }, src);
  if (lang === "haml") {
    const legacyArgs = mode === "compile" ? ["--debug"] : [];
    return runHaml(commandPath("haml"), mode, legacyArgs, cwd, src);
  }
  if (lang === "erb")
    return run(
      commandPath("ruby"),
      erbRubyArgs(mode, doc.languageId === "erb-percent"),
      { cwd },
      src,
    );
  return Promise.resolve({
    code: 1,
    stdout: "",
    stderr: `Unsupported language: ${lang}`,
  });
}

/** Render a template document to HTML, inside the Rails app when the setting asks for it. */
async function renderToHtml(doc) {
  const railsRoot =
    settings().get("preview.useRails", false) && doc.uri.scheme === "file"
      ? findUp(path.dirname(doc.uri.fsPath), path.join("bin", "rails"))
      : undefined;
  if (!railsRoot) return runEngine(doc, "render");
  return renderInRails(
    railsRoot,
    viewDir(railsRoot, doc.uri.fsPath),
    engineOf(doc.languageId),
    doc.getText(),
  );
}

/** Render through the app's `bin/rails runner`, so helpers, routes and partials resolve. */
async function renderInRails(root, dir, lang, src) {
  const r = await run(
    commandPath("ruby"),
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

async function openPreview() {
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
      PREVIEW,
      "Template Preview",
      vscode.ViewColumn.Beside,
      { enableScripts: false },
    );
    previewPanel.onDidDispose(() => {
      previewPanel = undefined;
      previewUri = undefined;
    });
  }
  previewPanel.title = `Preview: ${path.basename(ed.document.fileName)}`;
  await renderPreview(ed.document);
}

/** Re-renders the preview when the document it shows is saved. */
function refreshPreviewOnSave(doc) {
  if (!previewPanel || !previewUri) return;
  if (!settings().get("preview.refreshOnSave", true)) return;
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
  const r = await runEngine(ed.document, "compile");
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

/** The preview and compiled-source commands, and a status bar item showing the active template's engine. */
function registerPreview(context) {
  const status = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Right,
    STATUS_BAR_PRIORITY,
  );
  status.command = PREVIEW;
  status.tooltip = "Ruby Templates: preview rendered HTML";
  const updateStatus = () => {
    const ed = vscode.window.activeTextEditor;
    if (ed && TEMPLATE_LANGS.has(engineOf(ed.document.languageId))) {
      status.text = `$(preview) ${ed.document.languageId.toUpperCase()}`;
      status.show();
    } else status.hide();
  };
  context.subscriptions.push(
    vscode.commands.registerCommand(PREVIEW, openPreview),
    vscode.commands.registerCommand(
      "rubyTemplates.showRenderedSource",
      showCompiledSource,
    ),
    vscode.workspace.onDidSaveTextDocument(refreshPreviewOnSave),
    status,
    vscode.window.onDidChangeActiveTextEditor(updateStatus),
    { dispose: closePreview },
  );
  updateStatus();
}

export { registerPreview };
