// The rendered-HTML preview and the compiled Ruby source of a template.
import * as vscode from "vscode";
import path from "path";

import { log } from "./client.js";
import {
  settings,
  commandPath,
  useBundler,
  workspaceFolderFor,
} from "./workspace.js";
import {
  RAILS_BIN,
  findUp,
  run,
  runHaml,
  railsRunner,
  firstLine,
} from "./run.js";
import {
  ERB_PERCENT,
  engineOf,
  isTemplateLanguage,
  isErb,
} from "./languages.js";
import {
  erbRubyArgs,
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
  const options = { cwd: workspaceFolderFor(doc), useBundler: useBundler() };
  const src = doc.getText();
  if (lang === "slim")
    return run(commandPath("slimrb"), slimArgs(mode), options, src);
  if (lang === "haml") {
    const legacyArgs = mode === "compile" ? ["--debug"] : [];
    return runHaml(commandPath("haml"), mode, legacyArgs, options, src);
  }
  if (lang === "erb")
    return run(
      commandPath("ruby"),
      erbRubyArgs(mode, doc.languageId === ERB_PERCENT),
      options,
      src,
    );
  return Promise.resolve({
    code: 1,
    stdout: "",
    stderr: `Unsupported language: ${lang}`,
  });
}

/** Render a template document to HTML, through the app's `bin/rails runner` when the setting asks for it. */
async function renderToHtml(doc) {
  const railsRoot =
    settings().get("preview.useRails", false) && doc.uri.scheme === "file"
      ? findUp(path.dirname(doc.uri.fsPath), RAILS_BIN)
      : undefined;
  if (!railsRoot) return runEngine(doc, "render");
  const script = railsRenderScript(
    viewDir(railsRoot, doc.uri.fsPath),
    engineOf(doc.languageId),
  );
  return railsRunner(railsRoot, script, {
    ruby: commandPath("ruby"),
    useBundler: useBundler(),
    input: doc.getText(),
  });
}

async function openPreview() {
  const ed = vscode.window.activeTextEditor;
  if (!ed || !isTemplateLanguage(ed.document.languageId)) {
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
  if (doc.uri.toString() === previewUri)
    renderPreview(doc).catch((e) => log(`preview: ${e?.message ?? e}`));
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
      isErb(doc.languageId)
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
  if (!ed || !isTemplateLanguage(ed.document.languageId)) return;
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
    if (ed && isTemplateLanguage(ed.document.languageId)) {
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
