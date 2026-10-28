import * as vscode from "vscode";
import cp from "child_process";
import path from "path";
import fs from "fs";

import {
  startClient,
  stopClient,
  restartClient,
  onServerStarted,
  notifyServer,
  serverFormatEdits,
} from "./client.js";
import { ROUTES_SCRIPT, routesFromOutput } from "./routes.js";
import { toggleErbTagAt } from "./erbTags.js";
import { toggleHamlLines } from "./haml.js";
import { extractBlock, partialFileName } from "./partial.js";
import { convertedFileName } from "./convert.js";
import {
  relatedFiles,
  relatedDirs,
  actionSource,
  actionLine,
  previewPartners,
} from "./related.js";
import { tagToClose, isHtmlView, doubledErbClose } from "./autoClose.js";
import { erbRubyArgs } from "./erbEngine.js";
import { WINDOWS_COMMAND_NOT_FOUND, windowsShellCommand } from "./shell.js";
import {
  toggleSlimMarker,
  inVerbatimBlock,
  wrapSlim,
  dedent,
  partialPath,
} from "./slim.js";

const TEMPLATE_LANGS = new Set(["erb", "haml", "slim"]);
// Host-specific ERB ids (erb-js, erb-css, ...) render and compile like plain ERB.
const engineOf = (languageId) =>
  languageId.startsWith("erb-") ? "erb" : languageId;

// Rails boots slowly on big apps, but a hung runner shouldn't leave the preview waiting forever.
const RAILS_RENDER_TIMEOUT_MS = 60_000;
// How far up from a template to look for a Gemfile or bin/rails.
const MAX_PARENT_DIRS = 6;

// The server reads these at startup, so a change restarts it.
const SERVER_SETTINGS = [
  "rubyTemplates.server",
  "rubyTemplates.slim.extraEngines",
  "rubyTemplates.slim.disabledDiagnostics",
  "rubyTemplates.erb.disabledDiagnostics",
  "rubyTemplates.haml.disabledDiagnostics",
  "rubyTemplates.jbuilder.disabledDiagnostics",
  ...["erb", "haml", "slim"].flatMap((lang) => [
    `rubyTemplates.${lang}.diagnostics.enabled`,
    `rubyTemplates.${lang}.format.enabled`,
  ]),
];

/** @type {vscode.WebviewPanel | undefined} */
let previewPanel;
/** @type {string | undefined} */
let previewUri;
/** @type {vscode.LogOutputChannel | undefined} */
let output;

function activate(context) {
  const sub = (cmd, fn) =>
    context.subscriptions.push(vscode.commands.registerCommand(cmd, fn));

  sub("rubyTemplates.toggleErbTag", toggleErbTag);
  sub("rubyTemplates.wrapInErb", wrapInErb);
  sub("rubyTemplates.preview", () => openPreview(context));
  sub("rubyTemplates.showRenderedSource", showCompiledSource);
  sub("rubyTemplates.convertToHaml", () => convert("haml"));
  sub("rubyTemplates.convertToSlim", () => convert("slim"));
  sub("rubyTemplates.convertToErb", () => convert("erb"));
  sub("rubyTemplates.extractPartial", extractPartial);
  sub("rubyTemplates.switchRelated", switchRelated);
  sub("rubyTemplates.toggleHamlPrefix", toggleActiveHamlLines);
  sub("rubyTemplates.reindentHaml", reindentActiveHaml);
  sub("rubyTemplates.toggleSlimTag", toggleSlimTag);
  sub("rubyTemplates.wrapInSlim", wrapInSlim);
  sub("rubyTemplates.extractSlimPartial", extractSlimPartial);

  // Language server (Rust): diagnostics, go-to-partial, completions, hover.
  output = vscode.window.createOutputChannel("Ruby Templates", {
    log: true,
  });
  context.subscriptions.push(output);
  registerServerFormatting(context);
  registerErbAutoClose(context);
  sub("rubyTemplates.restartServer", () => restartClient(context, output));
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (SERVER_SETTINGS.some((s) => e.affectsConfiguration(s)))
        void restartClient(context, output);
    }),
  );
  void startClient(context, output);
  registerRoutes(context, output);

  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument((doc) => {
      if (!previewPanel || !previewUri) return;
      if (!cfg().get("preview.refreshOnSave", true)) return;
      if (doc.uri.toString() === previewUri) renderPreview(doc);
    }),
  );

  // Status bar: shows active template engine and offers preview.
  const status = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Right,
    90,
  );
  status.command = "rubyTemplates.preview";
  status.tooltip = "Ruby Templates: preview rendered HTML";
  const updateStatus = () => {
    const ed = vscode.window.activeTextEditor;
    if (ed && TEMPLATE_LANGS.has(engineOf(ed.document.languageId))) {
      status.text = `$(preview) ${ed.document.languageId.toUpperCase()}`;
      status.show();
    } else status.hide();
  };
  context.subscriptions.push(
    status,
    vscode.window.onDidChangeActiveTextEditor(updateStatus),
  );
  updateStatus();
}

function cfg() {
  return vscode.workspace.getConfiguration("rubyTemplates");
}

// ---------------------------------------------------------------- ERB editing

async function toggleErbTag() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const doc = ed.document;
  const text = doc.getText();
  // Several cursors in one tag must yield one edit; overlapping edits make ed.edit throw.
  const done = new Set();
  await ed.edit((eb) => {
    for (const sel of ed.selections) {
      const tag = toggleErbTagAt(text, doc.offsetAt(sel.active));
      if (!tag || done.has(tag.start)) continue;
      done.add(tag.start);
      eb.replace(
        new vscode.Range(doc.positionAt(tag.start), doc.positionAt(tag.end)),
        tag.replacement,
      );
    }
  });
}

async function wrapInErb() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  await ed.edit((eb) => {
    for (const sel of ed.selections) {
      const text = ed.document.getText(sel);
      eb.replace(sel, text ? `<%= ${text} %>` : "<%=  %>");
    }
  });
  if (ed.selections.length === 1 && ed.selection.isEmpty) {
    const p = ed.selection.active.translate(0, -3);
    ed.selection = new vscode.Selection(p, p);
  }
}

// ---------------------------------------------------------------- Haml editing

/** Line numbers a selection covers, leaving out a last line it only touches at column 0. */
function selectedLines(sel) {
  const last =
    sel.end.character === 0 && sel.end.line > sel.start.line
      ? sel.end.line - 1
      : sel.end.line;
  return Array.from(
    { length: last - sel.start.line + 1 },
    (_, i) => sel.start.line + i,
  );
}

const documentLines = (doc) =>
  Array.from({ length: doc.lineCount }, (_, i) => doc.lineAt(i).text);

async function toggleActiveHamlLines() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const edits = toggleHamlLines(
    documentLines(ed.document),
    new Set(ed.selections.flatMap(selectedLines)),
  );
  await ed.edit((eb) =>
    edits.forEach(([n, text]) => eb.replace(ed.document.lineAt(n).range, text)),
  );
}

/** Re-indents the selected lines, or the whole document, through the server's formatter. */
async function reindentActiveHaml() {
  const ed = vscode.window.activeTextEditor;
  if (ed?.document.languageId !== "haml") return;
  const { selection, document: doc } = ed;
  const lines = selectedLines(selection);
  const range = selection.isEmpty
    ? undefined
    : new vscode.Range(lines[0], 0, lines.at(-1), 0);
  const { tabSize, insertSpaces } = ed.options;
  const options = {
    tabSize: Number(tabSize),
    insertSpaces: insertSpaces === true,
  };
  const edits = await serverFormatEdits(doc, options, range);
  if (!edits) {
    vscode.window.showInformationMessage(
      "Re-indent Haml: the language server is not running, or left this file unchanged (the reason is in the Ruby Templates output).",
    );
    return;
  }
  await ed.edit((eb) =>
    edits.forEach(({ range, newText }) => eb.replace(range, newText)),
  );
}

// ---------------------------------------------------------------- partials

const PARTIAL_NAME_RE = /^\w+$/;
const RENDER_CALL = {
  erb: (p) => `<%= render "${p}" %>`,
  haml: (p) => `= render "${p}"`,
};

/** Move the selection into `_name.<ext>` next to the template and render it in its place. */
/**
 * Open the view's other formats (`.html.erb` ↔ `.text.erb`), its controller action, or a component's class ↔
 * template ↔ preview.
 */
async function switchRelated() {
  const doc = vscode.window.activeTextEditor?.document;
  if (doc?.uri.scheme !== "file") return;
  const dir = path.dirname(doc.uri.fsPath);
  const name = path.basename(doc.uri.fsPath);
  const candidates = [];
  for (const folder of [dir, ...relatedDirs(dir, name, path.sep)]) {
    let entries;
    try {
      entries = await vscode.workspace.fs.readDirectory(
        vscode.Uri.file(folder),
      );
    } catch {
      continue; // most components have no sidecar folder
    }
    const files = entries
      .filter(([, type]) => type === vscode.FileType.File)
      .map(([n]) => n);
    candidates.push(
      ...relatedFiles(name, files).map((n) => path.join(folder, n)),
    );
  }
  const action = await controllerAction(doc.uri.fsPath);
  if (action) candidates.push(action);
  for (const partner of previewPartners(doc.uri.fsPath, path.sep))
    if (await exists(vscode.Uri.file(partner))) candidates.push(partner);
  if (!candidates.length) {
    vscode.window.showInformationMessage(`No related file for ${name}.`);
    return;
  }
  const picked =
    candidates.length === 1
      ? candidates[0]
      : (
          await vscode.window.showQuickPick(
            candidates.map((file) => ({
              label:
                file === action
                  ? `${path.basename(file)} #${actionName(doc.uri.fsPath)}`
                  : path.basename(file),
              description: vscode.workspace.asRelativePath(path.dirname(file)),
              file,
            })),
            { placeHolder: "Related file" },
          )
        )?.file;
  if (!picked) return;
  const editor = await vscode.window.showTextDocument(vscode.Uri.file(picked));
  if (picked !== action) return;
  const line = actionLine(
    editor.document.getText(),
    actionName(doc.uri.fsPath),
  );
  if (line < 0) return;
  const at = new vscode.Position(line, 0);
  editor.selection = new vscode.Selection(at, at);
  editor.revealRange(
    new vscode.Range(at, at),
    vscode.TextEditorRevealType.InCenter,
  );
}

const actionName = (view) => actionSource(view, path.sep)?.action;

/** The controller or mailer that renders `view`, when it exists. */
async function controllerAction(view) {
  const source = actionSource(view, path.sep);
  return source && (await exists(vscode.Uri.file(source.file)))
    ? source.file
    : undefined;
}

async function extractPartial() {
  const ed = vscode.window.activeTextEditor;
  if (!ed || ed.selection.isEmpty) return;
  const doc = ed.document;
  const lang = engineOf(doc.languageId);
  // Slim has its own extract command.
  if (!RENDER_CALL[lang]) return;
  if (doc.isUntitled) {
    vscode.window.showInformationMessage(
      "Save the template before extracting a partial.",
    );
    return;
  }
  // Haml nests by indentation, so only whole lines can move.
  const block = extractBlock(documentLines(doc), ed.selection, lang === "haml");
  if (!block) return;
  const name = await vscode.window.showInputBox({
    prompt: "Partial name (without the leading _ and the extension)",
    validateInput: (v) =>
      PARTIAL_NAME_RE.test(v) ? null : "Use letters, digits and _ only.",
  });
  if (!name) return;

  const { start, end } = block.range;
  const range = new vscode.Range(
    start.line,
    start.character,
    end.line,
    end.character,
  );
  const partial = vscode.Uri.joinPath(
    doc.uri,
    "..",
    partialFileName(path.basename(doc.uri.path), name),
  );
  const viewDir = /\/app\/views\/(.+)\/[^/]+$/.exec(doc.uri.path)?.[1];
  const renderPath = viewDir ? `${viewDir}/${name}` : name;

  if (await exists(partial)) {
    const ok = await vscode.window.showWarningMessage(
      `${path.basename(partial.path)} exists. Overwrite?`,
      { modal: true },
      "Overwrite",
    );
    if (ok !== "Overwrite") return;
  }
  await vscode.workspace.fs.writeFile(
    partial,
    new TextEncoder().encode(block.text),
  );
  await ed.edit((eb) =>
    eb.replace(range, `${block.indent}${RENDER_CALL[lang](renderPath)}`),
  );
  await vscode.window.showTextDocument(partial, {
    viewColumn: vscode.ViewColumn.Beside,
    preview: true,
  });
}

// ---------------------------------------------------------------- Slim editing

/** Last line a selection covers; one ending at column 0 of a later line doesn't take that line along. */
const lastSelectedLine = ({ start, end }) =>
  end.character === 0 && end.line > start.line ? end.line - 1 : end.line;

async function toggleSlimTag() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const text = ed.document.getText().split(/\r?\n/);
  const lines = new Set(
    ed.selections.flatMap((sel) =>
      Array.from(
        { length: lastSelectedLine(sel) - sel.start.line + 1 },
        (_, i) => sel.start.line + i,
      ),
    ),
  );
  await ed.edit((eb) => {
    for (const n of lines) {
      if (inVerbatimBlock(text, n)) continue;
      const line = ed.document.lineAt(n);
      const next = toggleSlimMarker(line.text);
      if (next !== null) eb.replace(line.range, next);
    }
  });
}

async function wrapInSlim() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const doc = ed.document;
  if (ed.selections.some((s) => !s.isSingleLine)) {
    vscode.window.showInformationMessage(
      "Wrap in Slim Output skips selections that span several lines.",
    );
  }
  await ed.edit((eb) => {
    for (const sel of ed.selections.filter((s) => s.isSingleLine)) {
      const line = doc.lineAt(sel.start.line);
      const content = line.text.trim();
      if (content && doc.getText(sel).trim() === content) {
        const start = line.firstNonWhitespaceCharacterIndex;
        eb.replace(
          new vscode.Range(
            line.lineNumber,
            start,
            line.lineNumber,
            start + content.length,
          ),
          wrapSlim(content, true),
        );
      } else {
        eb.replace(sel, wrapSlim(doc.getText(sel), false));
      }
    }
  });
  if (ed.selections.length === 1 && ed.selection.isEmpty) {
    const p = ed.selection.active.translate(0, -1);
    ed.selection = new vscode.Selection(p, p);
  }
}

async function extractSlimPartial() {
  const ed = vscode.window.activeTextEditor;
  if (!ed || ed.selection.isEmpty || ed.document.uri.scheme !== "file") return;
  const doc = ed.document;
  const last = lastSelectedLine(ed.selection);
  const range = new vscode.Range(
    ed.selection.start.line,
    0,
    last,
    doc.lineAt(last).text.length,
  );

  const name = await vscode.window.showInputBox({
    prompt:
      'Partial name, rendered as `= render "name"` (`card`, `shared/card`)',
    validateInput: (v) =>
      /^([\w-]+\/)*[a-z_]\w*$/.test(v)
        ? null
        : "Use a lowercase Ruby identifier, optionally prefixed by directories.",
  });
  if (!name) return;
  const target = partialPath(doc.uri.fsPath, name);
  if (!target) {
    vscode.window.showErrorMessage(
      `"${name}" renders from app/views, but this file isn't under app/views.`,
    );
    return;
  }
  const targetUri = vscode.Uri.file(target);
  if (await exists(targetUri)) {
    vscode.window.showErrorMessage(`${path.basename(target)} already exists.`);
    return;
  }
  const eol = doc.eol === vscode.EndOfLine.CRLF ? "\r\n" : "\n";
  const { indent, lines } = dedent(doc.getText(range).split(/\r?\n/));
  // One edit, so a single undo restores the selection and a failed create leaves the template alone.
  const edit = new vscode.WorkspaceEdit();
  edit.createFile(targetUri, { overwrite: false, ignoreIfExists: false });
  // A new document takes files.eol (CRLF on Windows) and would convert the inserted text to it.
  edit.set(targetUri, [
    vscode.TextEdit.setEndOfLine(doc.eol),
    vscode.TextEdit.insert(new vscode.Position(0, 0), lines.join(eol) + eol),
  ]);
  edit.replace(doc.uri, range, `${indent}= render "${name}"`);
  if (!(await vscode.workspace.applyEdit(edit))) {
    vscode.window.showErrorMessage(`Couldn't create ${path.basename(target)}.`);
    return;
  }
  // Text edits leave the new partial open and unsaved.
  await (await vscode.workspace.openTextDocument(targetUri)).save();
}

async function exists(uri) {
  try {
    await vscode.workspace.fs.stat(uri);
    return true;
  } catch (e) {
    if (e instanceof vscode.FileSystemError && e.code === "FileNotFound")
      return false;
    throw e;
  }
}

// ---------------------------------------------------------------- rendering

function workspaceFolderFor(doc) {
  return (
    vscode.workspace.getWorkspaceFolder(doc.uri)?.uri.fsPath ||
    path.dirname(doc.uri.fsPath)
  );
}

/** The nearest of `dir` and its parents that contains `rel`, or undefined. */
function findUp(dir, rel) {
  let d = dir;
  for (let i = 0; i < MAX_PARENT_DIRS; i++) {
    if (fs.existsSync(path.join(d, rel))) return d;
    const parent = path.dirname(d);
    if (parent === d) break;
    d = parent;
  }
  return undefined;
}

function run(
  cmd,
  args,
  { useBundler = cfg().get("preview.useBundler", false), ...options },
  input,
) {
  return new Promise((resolve) => {
    const [bin, binArgs] =
      useBundler && findUp(options.cwd, "Gemfile") !== undefined
        ? ["bundle", ["exec", cmd, ...args]]
        : [cmd, args];
    const windows = process.platform === "win32";
    try {
      const [file, fileArgs, spawnOptions] = windows
        ? windowsShellCommand(bin, binArgs)
        : [bin, binArgs, {}];
      const child = cp.execFile(
        file,
        fileArgs,
        { ...options, ...spawnOptions, maxBuffer: 16 * 1024 * 1024 },
        (err, stdout, stderr) => {
          // Callers detect a missing tool by ENOENT, which a shell can't report.
          if (windows && err?.code === WINDOWS_COMMAND_NOT_FOUND) {
            err.code = "ENOENT";
            err.path = bin;
          }
          resolve({
            code: err ? (err.code ?? 1) : 0,
            stdout: String(stdout || ""),
            stderr: String(stderr || ""),
            err,
          });
        },
      );
      if (input !== undefined && child.stdin) {
        child.stdin.on("error", () => {});
        child.stdin.end(input);
      }
    } catch (err) {
      resolve({ code: err.code ?? 1, stdout: "", stderr: err.message, err });
    }
  });
}

/** `haml <subcommand> -` (Haml 6+), falling back to the Haml < 6 CLI, which has no subcommands and reads stdin. */
async function runHaml(bin, subcommand, legacyArgs, cwd, src) {
  const r = await run(bin, [subcommand, "-"], { cwd }, src);
  // Haml < 6 opens the subcommand name as the input file.
  if (
    r.code !== 0 &&
    r.stderr.includes("ENOENT") &&
    r.stderr.includes(` - ${subcommand}`)
  )
    return run(bin, legacyArgs, { cwd }, src);
  // Thor reports usage errors on stderr but exits 0.
  if (r.code === 0 && !r.stdout && r.stderr) return { ...r, code: 1 };
  return r;
}

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
      erbRubyArgs("render"),
      { cwd },
      src,
    );
  return { code: 1, stdout: "", stderr: `Unsupported language: ${lang}` };
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
 * Render through the app's `bin/rails runner`, so helpers, routes and partials resolve. A controller
 * whose path is the template's directory makes a bare `render "card"` find `<dir>/_card`, as in a request.
 */
async function renderInRails(root, dir, lang, src) {
  const controller = dir
    ? `Class.new(ApplicationController) { define_singleton_method(:controller_path) { ${rubyString(dir)} } }`
    : "ApplicationController";
  const script = `print ${controller}.render(inline: $stdin.read, type: :${lang})`;
  const r = await run(
    "ruby",
    [path.join("bin", "rails"), "runner", script],
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
      erbRubyArgs("compile"),
      { cwd },
      src,
    );
  return { code: 1, stdout: "", stderr: `Unsupported language: ${lang}` };
}

function missingToolMessage(bin, lang) {
  if (engineOf(lang) === "erb")
    return `"${bin}" was not found. Install Ruby (and gem install erubi) — or point rubyTemplates.commands.ruby at it.`;
  const gem = { slim: "slim", haml: "haml" }[engineOf(lang)] || bin;
  return `"${bin}" was not found. Install it with: gem install ${gem} — or point rubyTemplates.commands.* at the executable.`;
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

const escapeHtml = (s) =>
  String(s).replace(
    /[&<>]/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c],
  );

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
  panel.webview.html = `<!DOCTYPE html><html><head><meta charset="utf-8">
<style>
  body{font-family:system-ui,sans-serif;margin:0}
  .bar{display:flex;gap:1rem;align-items:center;padding:.4rem .8rem;font-size:12px;background:var(--vscode-editorWidget-background);border-bottom:1px solid var(--vscode-widget-border,#444)}
  .bar b{opacity:.8}
  iframe{border:0;width:100%;height:calc(55vh);background:#fff}
  pre{margin:0;padding:.8rem;font:12px/1.4 var(--vscode-editor-font-family,monospace);white-space:pre-wrap;word-break:break-word;background:var(--vscode-textCodeBlock-background);max-height:40vh;overflow:auto}
</style></head><body>
<div class="bar"><b>Rendered</b> <span>${escapeHtml(path.basename(doc.fileName))} · ${escapeHtml(doc.languageId)}</span></div>
<iframe sandbox="" srcdoc="${escapeHtml(r.stdout).replace(/"/g, "&quot;")}"></iframe>
<div class="bar"><b>HTML source</b></div>
<pre>${escapeHtml(r.stdout)}</pre>
</body></html>`;
}

function errorPage(title, detail, hint) {
  return `<!DOCTYPE html><html><body style="font-family:system-ui;padding:1rem">
<h3 style="color:var(--vscode-errorForeground)">${escapeHtml(title)}</h3>
<pre style="white-space:pre-wrap">${escapeHtml(detail)}</pre>
${hint ? `<p style="opacity:.8">${escapeHtml(hint)}</p>` : ""}</body></html>`;
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
    vscode.window.showErrorMessage(
      `Compile failed: ${(r.stderr || r.stdout).split("\n")[0]}`,
    );
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

// ---------------------------------------------------------------- routes

/**
 * Read each Rails app's routes with `bin/rails runner` once one of its views is open, again when its routes
 * files change, and hand them to the server for `*_path` / `*_url` completion, hover and definition.
 */
function registerRoutes(context, output) {
  const cache = new Map(); // Rails root → routes
  // Roots whose boot failed: retried on a routes file change or a settings change, not on every view opened.
  const failed = new Set();
  const busy = new Set();
  const again = new Set();
  const enabled = () => cfg().get("routes.enabled", true);
  const send = async (root, routes) => {
    try {
      await notifyServer("rubyTemplates/routes", { root, routes });
    } catch (e) {
      output.appendLine(
        `[routes] sending routes for ${root}: ${e?.message ?? e}`,
      );
    }
  };

  const load = async (root) => {
    const r = await run(
      cfg().get("commands.ruby", "ruby"),
      [path.join("bin", "rails"), "runner", ROUTES_SCRIPT],
      { cwd: root, timeout: RAILS_RENDER_TIMEOUT_MS, useBundler: false },
    );
    if (!enabled()) return;
    let routes;
    try {
      routes = r.code === 0 ? routesFromOutput(r.stdout) : undefined;
    } catch {
      routes = undefined;
    }
    if (!routes) {
      output.appendLine(
        `[routes] bin/rails runner failed in ${root}: ${firstLine(r) || `exit ${r.code}`}`,
      );
      failed.add(root);
      return;
    }
    failed.delete(root);
    cache.set(root, routes);
    await send(root, routes);
  };

  // A change while a load runs reloads once it finishes, so the last edit to routes.rb always counts.
  const refresh = async (root) => {
    if (busy.has(root)) {
      again.add(root);
      return;
    }
    busy.add(root);
    try {
      do {
        again.delete(root);
        await load(root);
      } while (again.has(root));
    } finally {
      busy.delete(root);
    }
  };

  // Booting runs the app's own code: only for an app inside a trusted workspace folder, never for a file
  // opened on its own (an empty window is trusted by default) or a bin/rails found above the folder.
  const railsRoot = (file) => {
    if (!enabled() || !vscode.workspace.isTrusted) return undefined;
    const root = findUp(path.dirname(file), path.join("bin", "rails"));
    const folder =
      root && vscode.workspace.getWorkspaceFolder(vscode.Uri.file(root));
    return folder && !path.relative(folder.uri.fsPath, root).startsWith("..")
      ? root
      : undefined;
  };
  const opened = (doc) => {
    if (doc.uri.scheme !== "file") return;
    if (!TEMPLATE_LANGS.has(engineOf(doc.languageId))) return;
    const root = railsRoot(doc.uri.fsPath);
    if (root && !cache.has(root) && !failed.has(root) && !busy.has(root))
      void refresh(root);
  };
  const routesChanged = (uri) => {
    const root = railsRoot(uri.fsPath);
    if (root && (cache.has(root) || failed.has(root))) void refresh(root);
  };

  const watcher = vscode.workspace.createFileSystemWatcher(
    "**/config/{routes.rb,routes/**/*.rb}",
  );
  context.subscriptions.push(
    watcher,
    watcher.onDidChange(routesChanged),
    watcher.onDidCreate(routesChanged),
    watcher.onDidDelete(routesChanged),
    vscode.workspace.onDidOpenTextDocument(opened),
    vscode.workspace.onDidGrantWorkspaceTrust(() =>
      vscode.workspace.textDocuments.forEach(opened),
    ),
    onServerStarted(() => {
      for (const [root, routes] of cache) void send(root, routes);
    }),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (
        !e.affectsConfiguration("rubyTemplates.routes") &&
        !e.affectsConfiguration("rubyTemplates.commands.ruby")
      )
        return;
      failed.clear();
      if (enabled()) vscode.workspace.textDocuments.forEach(opened);
      else {
        for (const root of cache.keys()) void send(root, []);
        cache.clear();
      }
    }),
  );
  vscode.workspace.textDocuments.forEach(opened);
}

// ---------------------------------------------------------------- converters

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

async function convert(target) {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const from = ed.document.languageId;
  const cwd = workspaceFolderFor(ed.document);
  const src = ed.document.getText();
  const converter = CONVERTERS[`${from}>${target}`];
  if (!converter) {
    vscode.window.showInformationMessage(
      `No converter for ${from} → ${target}.`,
    );
    return;
  }
  const { command, args, gem } = converter;
  const bin = cfg().get(`commands.${command}`, command);

  const r = await run(bin, args, { cwd }, src);
  const missingGem = r.stderr.includes(`cannot load such file -- ${gem}`);
  if (r.err?.code === "ENOENT" || missingGem) {
    const what = missingGem ? `The ${gem} gem was` : `"${bin}" was`;
    const pick = await vscode.window.showErrorMessage(
      `${what} not found. Install with: gem install ${gem}`,
      "Copy command",
    );
    if (pick) await vscode.env.clipboard.writeText(`gem install ${gem}`);
    return;
  }
  if (r.code !== 0) {
    vscode.window.showErrorMessage(
      `Conversion failed: ${(r.stderr || r.stdout).split("\n")[0]}`,
    );
    return;
  }

  const newName = convertedFileName(ed.document.fileName, from, target);
  const canSave = ed.document.uri.scheme === "file";
  const choice = await vscode.window.showQuickPick(
    [
      ...(canSave
        ? [
            {
              label: `Save as ${path.basename(newName)} next to the original`,
              id: "save",
            },
          ]
        : []),
      { label: "Open as untitled document", id: "untitled" },
      { label: "Replace contents of the current file", id: "replace" },
    ],
    {
      placeHolder: `Converted ${from} → ${target}. What to do with the result?`,
    },
  );
  if (!choice) return;
  if (choice.id === "untitled") {
    const d = await vscode.workspace.openTextDocument({
      language: target,
      content: r.stdout,
    });
    await vscode.window.showTextDocument(d, {
      viewColumn: vscode.ViewColumn.Beside,
    });
  } else if (choice.id === "save") {
    const newUri = vscode.Uri.file(newName);
    if (await exists(newUri)) {
      const ok = await vscode.window.showWarningMessage(
        `${path.basename(newName)} exists. Overwrite?`,
        { modal: true },
        "Overwrite",
      );
      if (ok !== "Overwrite") return;
    }
    await vscode.workspace.fs.writeFile(
      newUri,
      new TextEncoder().encode(r.stdout),
    );
    await vscode.window.showTextDocument(newUri, {
      viewColumn: vscode.ViewColumn.Beside,
    });
  } else {
    const full = new vscode.Range(
      ed.document.positionAt(0),
      ed.document.positionAt(src.length),
    );
    await ed.edit((eb) => eb.replace(full, r.stdout));
    await vscode.languages.setTextDocumentLanguage(ed.document, target);
  }
}

const firstLine = (r) => (r.stderr || r.stdout).split("\n")[0];

/**
 * Typing `>` in ERB: drops the ` %>` auto-closing added when the user types the tag's own `%>`, and in HTML views
 * adds a start tag's end tag, as VS Code does for plain HTML.
 */
function registerErbAutoClose(context) {
  context.subscriptions.push(
    vscode.workspace.onDidChangeTextDocument(async (e) => {
      if (!/^erb/.test(e.document.languageId) || e.reason !== undefined) return;
      if (e.contentChanges.length !== 1 || e.contentChanges[0].text !== ">")
        return;
      const editor = vscode.window.activeTextEditor;
      if (editor?.document !== e.document || editor.selections.length !== 1)
        return;
      const at = e.contentChanges[0].range.start.translate(0, 1);
      const { text } = e.document.lineAt(at.line);
      const doubled = doubledErbClose(
        text.slice(0, at.character),
        text.slice(at.character),
      );
      if (doubled) {
        await editor.edit(
          (b) => b.delete(new vscode.Range(at, at.translate(0, doubled))),
          {
            undoStopBefore: false,
            undoStopAfter: false,
          },
        );
        return;
      }
      if (!isHtmlView(e.document.fileName)) return;
      if (!cfg().get("erb.autoCloseTags", true)) return;
      const before = e.document.getText(
        new vscode.Range(0, 0, at.line, at.character),
      );
      const name = tagToClose(before);
      if (!name || text.slice(at.character).startsWith(`</${name}`)) return;
      const inserted = await editor.edit((b) => b.insert(at, `</${name}>`), {
        undoStopBefore: false,
        undoStopAfter: false,
      });
      if (inserted) editor.selection = new vscode.Selection(at, at);
    }),
  );
}

const formatWithServer = async (doc, options, range) =>
  (await serverFormatEdits(doc, options, range)) ?? [];

/** Offers the server's formatting for ERB and Haml only while `rubyTemplates.<lang>.format.enabled` is on, so another extension's formatter isn't shadowed. */
function registerServerFormatting(context) {
  const registrations = new Map();
  const update = () => {
    for (const lang of ["erb", "haml"]) {
      const enabled = cfg().get(`${lang}.format.enabled`, true);
      if (enabled === registrations.has(lang)) continue;
      if (!enabled) {
        registrations.get(lang).dispose();
        registrations.delete(lang);
        continue;
      }
      registrations.set(
        lang,
        vscode.Disposable.from(
          vscode.languages.registerDocumentFormattingEditProvider(lang, {
            provideDocumentFormattingEdits: (doc, options) =>
              formatWithServer(doc, options),
          }),
          vscode.languages.registerDocumentRangeFormattingEditProvider(lang, {
            provideDocumentRangeFormattingEdits: (doc, range, options) =>
              formatWithServer(doc, options, range),
          }),
        ),
      );
    }
  };
  update();
  context.subscriptions.push(
    { dispose: () => registrations.forEach((r) => r.dispose()) },
    vscode.workspace.onDidChangeConfiguration(
      (e) =>
        (e.affectsConfiguration("rubyTemplates.erb.format.enabled") ||
          e.affectsConfiguration("rubyTemplates.haml.format.enabled")) &&
        update(),
    ),
  );
}

function deactivate() {
  previewPanel?.dispose();
  return stopClient(output);
}

export { activate, deactivate };
