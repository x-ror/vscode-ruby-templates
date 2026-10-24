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
} from "./client.js";
import { ROUTES_SCRIPT, routesFromOutput } from "./routes.js";
import { toggleErbTagAt } from "./erbTags.js";
import { toggleHamlLines, reindentHaml, hamlLintOffenses } from "./haml.js";
import { extractBlock, partialFileName } from "./partial.js";
import { convertedFileName } from "./convert.js";
import {
  relatedFiles,
  relatedDirs,
  actionSource,
  actionLine,
  previewPartners,
} from "./related.js";
import { ERB_FORMATTERS, erbFormatterArgs } from "./erbFormat.js";
import { ERB_LINT_OK_CODES, erbLintOffenses } from "./erbLint.js";
import { tagToClose, isHtmlView, doubledErbClose } from "./autoClose.js";
import { erbRubyArgs } from "./erbEngine.js";
import { herbLintReport } from "./herbLint.js";
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
  registerHamlFormatting(context, output);
  registerErbFormatting(context, output);
  registerErbAutoClose(context);
  startHamlLint(context, output);
  sub("rubyTemplates.restartServer", () => restartClient(context, output));
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (SERVER_SETTINGS.some((s) => e.affectsConfiguration(s)))
        void restartClient(context, output);
    }),
  );
  void startClient(context, output);
  registerSlimLint(context, output);
  registerErbLint(context, output);
  registerHerbLint(context, output);
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

/** Edits re-indenting Haml lines `from`..`to`; nesting is computed over the whole document. */
function reindentEdits(doc, tabSize, from = 0, to = doc.lineCount - 1) {
  const lines = documentLines(doc);
  const fixed = reindentHaml(lines, tabSize);
  const edits = [];
  for (let i = from; i <= to; i++) {
    if (fixed[i] !== lines[i])
      edits.push(vscode.TextEdit.replace(doc.lineAt(i).range, fixed[i]));
  }
  return edits;
}

async function reindentActiveHaml() {
  const ed = vscode.window.activeTextEditor;
  if (ed?.document.languageId !== "haml") return;
  const { selection, document: doc } = ed;
  const lines = selection.isEmpty
    ? [0, doc.lineCount - 1]
    : selectedLines(selection);
  const edits = reindentEdits(
    doc,
    Number(ed.options.tabSize),
    lines[0],
    lines.at(-1),
  );
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

// ---------------------------------------------------------------- erb_lint

/** Lint ERB views on open and save with `erb_lint --stdin`, so the editor's text is what gets linted. */
function registerErbLint(context, output) {
  const lints = vscode.languages.createDiagnosticCollection("erb_lint");
  // Saves can overlap; only the latest run per document may publish.
  const latestRun = new Map();
  let reportedMissing = false;

  const lint = async (doc) => {
    if (!/^erb/.test(doc.languageId) || doc.uri.scheme !== "file") return;
    if (doc.uri.path.toLowerCase().endsWith(".tt")) return;
    if (!cfg().get("erbLint.enabled", false)) return;
    const key = doc.uri.toString();
    const runId = (latestRun.get(key) ?? 0) + 1;
    latestRun.set(key, runId);
    const bin = cfg().get("commands.erbLint", "erb_lint");
    const r = await run(
      bin,
      ["--format", "json", "--stdin", doc.uri.fsPath],
      { cwd: workspaceFolderFor(doc) },
      doc.getText(),
    );
    // A run that was in flight when the linter was turned off must not bring its offenses back.
    if (latestRun.get(key) !== runId || doc.isClosed) return;
    if (!cfg().get("erbLint.enabled", false)) return;
    if (toolMissing(r)) {
      if (!reportedMissing)
        vscode.window.showWarningMessage(
          `"${bin}" was not found. Install it with: gem install erb_lint — or set rubyTemplates.commands.erbLint.`,
        );
      reportedMissing = true;
      return;
    }
    if (!ERB_LINT_OK_CODES.has(r.code)) {
      output.appendLine(`[erb_lint] ${doc.uri.fsPath}: ${firstLine(r)}`);
      return;
    }
    try {
      const diagnostics = erbLintOffenses(r.stdout).map(
        ({ line, column, endLine, endColumn, linter, message }) => {
          const d = new vscode.Diagnostic(
            new vscode.Range(
              line,
              utf16Column(doc, line, column),
              endLine,
              utf16Column(doc, endLine, endColumn),
            ),
            message,
            vscode.DiagnosticSeverity.Warning,
          );
          d.source = "erb_lint";
          d.code = linter;
          return d;
        },
      );
      lints.set(doc.uri, diagnostics);
    } catch (e) {
      output.appendLine(
        `[erb_lint] unreadable JSON for ${doc.uri.fsPath}: ${e.message}`,
      );
    }
  };

  context.subscriptions.push(
    lints,
    vscode.workspace.onDidOpenTextDocument(lint),
    vscode.workspace.onDidSaveTextDocument(lint),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      lints.delete(doc.uri);
      latestRun.delete(doc.uri.toString());
    }),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (
        !e.affectsConfiguration("rubyTemplates.erbLint") &&
        !e.affectsConfiguration("rubyTemplates.commands.erbLint")
      )
        return;
      reportedMissing = false;
      if (cfg().get("erbLint.enabled", false))
        vscode.workspace.textDocuments.forEach(lint);
      else lints.clear();
    }),
  );
  vscode.workspace.textDocuments.forEach(lint);
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

// ---------------------------------------------------------------- herb-lint

const HERB_SEVERITY = {
  error: vscode.DiagnosticSeverity.Error,
  warning: vscode.DiagnosticSeverity.Warning,
  info: vscode.DiagnosticSeverity.Information,
  hint: vscode.DiagnosticSeverity.Hint,
};

/** The project's own `node_modules/.bin/herb-lint` while the setting is left at its default. */
function herbLintBin(doc) {
  const bin = cfg().get("commands.herbLint", "herb-lint");
  if (bin !== "herb-lint") return bin;
  const rel = path.join("node_modules", ".bin", "herb-lint");
  const project = findUp(path.dirname(doc.uri.fsPath), rel);
  return project ? path.join(project, rel) : bin;
}

/** Lint saved ERB files with `herb-lint --json`; it has no stdin mode, so unsaved text isn't linted. */
function registerHerbLint(context, output) {
  const lints = vscode.languages.createDiagnosticCollection("herb-lint");
  // Saves can overlap; only the latest run per document may publish.
  const latestRun = new Map();
  let reportedMissing = false;

  const lint = async (doc) => {
    if (!/^erb/.test(doc.languageId) || doc.uri.scheme !== "file") return;
    if (doc.uri.path.toLowerCase().endsWith(".tt") || doc.isDirty) return;
    if (!cfg().get("herbLint.enabled", false)) return;
    const key = doc.uri.toString();
    const runId = (latestRun.get(key) ?? 0) + 1;
    latestRun.set(key, runId);
    const bin = herbLintBin(doc);
    const cwd =
      findUp(path.dirname(doc.uri.fsPath), ".herb.yml") ??
      workspaceFolderFor(doc);
    const r = await run(
      bin,
      ["--json", "--no-timing", "--no-color", doc.uri.fsPath],
      { cwd, useBundler: false },
    );
    if (latestRun.get(key) !== runId || doc.isClosed) return;
    if (!cfg().get("herbLint.enabled", false)) return;
    if (toolMissing(r)) {
      if (!reportedMissing)
        vscode.window.showWarningMessage(
          `"${bin}" was not found. Install it with: npm install --save-dev @herb-tools/linter — or set rubyTemplates.commands.herbLint.`,
        );
      reportedMissing = true;
      return;
    }
    let report;
    try {
      report = herbLintReport(r.stdout);
    } catch {
      output.appendLine(`[herb-lint] ${doc.uri.fsPath}: ${firstLine(r)}`);
      return;
    }
    if (report.error) {
      output.appendLine(`[herb-lint] ${doc.uri.fsPath}: ${report.error}`);
      return;
    }
    lints.set(
      doc.uri,
      report.offenses.map(
        ({ line, column, endLine, endColumn, code, message, severity }) => {
          const d = new vscode.Diagnostic(
            new vscode.Range(
              line,
              utf16Column(doc, line, column),
              endLine,
              utf16Column(doc, endLine, endColumn),
            ),
            message,
            HERB_SEVERITY[severity] ?? vscode.DiagnosticSeverity.Warning,
          );
          d.source = "herb-lint";
          d.code = code;
          return d;
        },
      ),
    );
  };

  context.subscriptions.push(
    lints,
    vscode.workspace.onDidOpenTextDocument(lint),
    vscode.workspace.onDidSaveTextDocument(lint),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      lints.delete(doc.uri);
      latestRun.delete(doc.uri.toString());
    }),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (
        !e.affectsConfiguration("rubyTemplates.herbLint") &&
        !e.affectsConfiguration("rubyTemplates.commands.herbLint")
      )
        return;
      reportedMissing = false;
      if (cfg().get("herbLint.enabled", false))
        vscode.workspace.textDocuments.forEach(lint);
      else lints.clear();
    }),
  );
  vscode.workspace.textDocuments.forEach(lint);
}

/** erb_lint and herb-lint count columns in characters; VS Code in UTF-16 units, which differ after an emoji. */
function utf16Column(doc, line, column) {
  if (line >= doc.lineCount) return column;
  return [...doc.lineAt(line).text].slice(0, column).join("").length;
}

// ---------------------------------------------------------------- slim-lint

function registerSlimLint(context, output) {
  const lints = vscode.languages.createDiagnosticCollection("slim-lint");
  let warnedMissing = false;
  // Saves can overlap; only the latest run per document may publish.
  const latestRun = new Map();

  const warnMissing = (message) => {
    if (!warnedMissing) vscode.window.showWarningMessage(message);
    warnedMissing = true;
  };

  const lint = async (doc) => {
    if (doc.languageId !== "slim" || doc.uri.scheme !== "file") return;
    // Generator templates (`*.slim.tt`) hold ERB tags that slim-lint would misreport.
    if (doc.uri.path.toLowerCase().endsWith(".tt")) return;
    if (!cfg().get("slimLint.enabled", false)) return;
    const key = doc.uri.toString();
    const runId = (latestRun.get(key) ?? 0) + 1;
    latestRun.set(key, runId);
    const bin = cfg().get("commands.slimLint", "slim-lint");
    const r = await run(bin, ["--reporter", "json", doc.uri.fsPath], {
      cwd: workspaceFolderFor(doc),
    });
    if (latestRun.get(key) !== runId || doc.isClosed) return;
    if (r.err?.code === "ENOENT") {
      const missing = r.err.path ?? bin;
      warnMissing(
        missing === bin
          ? `"${bin}" was not found. Install it with: gem install slim_lint — or set rubyTemplates.commands.slimLint.`
          : `"${missing}" was not found, so slim-lint can't run through Bundler. Install Bundler or turn off rubyTemplates.preview.useBundler.`,
      );
      return;
    }
    // `bundle exec` exits 127 when the bundle doesn't include the command.
    if (r.code === 127 || /command not found/.test(r.stderr)) {
      warnMissing(
        `"${bin}" is not in the bundle. Add slim_lint to the Gemfile, or turn off rubyTemplates.preview.useBundler.`,
      );
      return;
    }
    try {
      lints.set(doc.uri, slimLintDiagnostics(JSON.parse(r.stdout), doc));
    } catch {
      output.appendLine(
        `[slim-lint] ${bin} exited ${r.code}: ${r.stderr || r.stdout}`,
      );
    }
  };

  context.subscriptions.push(
    lints,
    vscode.workspace.onDidSaveTextDocument((doc) => lint(doc)),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      lints.delete(doc.uri);
      latestRun.delete(doc.uri.toString());
    }),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (
        e.affectsConfiguration("rubyTemplates.slimLint") &&
        !cfg().get("slimLint.enabled", false)
      )
        lints.clear();
    }),
  );
}

/** Map a `slim-lint --reporter json` report to diagnostics on the offending lines of `doc`. */
function slimLintDiagnostics(report, doc) {
  return (report.files ?? [])
    .flatMap((f) => f.offenses ?? [])
    .map(({ severity, message, linter, location }) => {
      const line = doc.lineAt(
        Math.min(Math.max((location?.line ?? 1) - 1, 0), doc.lineCount - 1),
      );
      const range = new vscode.Range(
        line.lineNumber,
        line.firstNonWhitespaceCharacterIndex,
        line.lineNumber,
        line.text.length,
      );
      const level =
        severity === "error"
          ? vscode.DiagnosticSeverity.Error
          : vscode.DiagnosticSeverity.Warning;
      const d = new vscode.Diagnostic(range, message, level);
      d.source = "slim-lint";
      d.code = linter;
      return d;
    });
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

// ---------------------------------------------------------------- haml-lint

const LINT_DEBOUNCE_MS = 300;
// haml-lint exits 65 (EX_DATAERR) when it found lints; anything else non-zero is a failure.
const HAML_LINT_OK_CODES = new Set([0, 65]);
const LINT_SEVERITY = {
  error: vscode.DiagnosticSeverity.Error,
  warning: vscode.DiagnosticSeverity.Warning,
};

/** `haml-lint <args> --stdin <file>` fed the document's text. */
function runHamlLint(doc, args, signal) {
  const bin = cfg().get("commands.hamlLint", "haml-lint");
  const useBundler = cfg().get("hamlLint.useBundler", false);
  return run(
    bin,
    [...args, "--stdin", doc.uri.fsPath],
    { cwd: workspaceFolderFor(doc), useBundler, signal },
    doc.getText(),
  );
}

// 127: `bundle exec` couldn't find the executable.
const toolMissing = (r) => r.err?.code === "ENOENT" || r.code === 127;
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

/** Registered only while a formatter is chosen, so another extension's ERB formatter isn't shadowed. */
function registerErbFormatting(context, output) {
  let registration;
  const update = () => {
    registration?.dispose();
    registration = undefined;
    const tool = cfg().get("erb.formatter", "none");
    if (!ERB_FORMATTERS[tool]) return;
    registration = vscode.languages.registerDocumentFormattingEditProvider(
      "erb",
      {
        provideDocumentFormattingEdits: (doc, options) =>
          formatErb(doc, options, tool, output),
      },
    );
  };
  update();
  context.subscriptions.push(
    { dispose: () => registration?.dispose() },
    vscode.workspace.onDidChangeConfiguration(
      (e) => e.affectsConfiguration("rubyTemplates.erb.formatter") && update(),
    ),
  );
}

async function formatErb(doc, options, tool, output) {
  const { command, gem } = ERB_FORMATTERS[tool];
  const bin = cfg().get(`commands.${command}`, tool);
  const { version } = doc;
  const r = await run(
    bin,
    erbFormatterArgs(tool, options),
    { cwd: workspaceFolderFor(doc) },
    doc.getText(),
  );
  if (toolMissing(r)) {
    vscode.window.showErrorMessage(
      `${bin} was not found. Install it with: gem install ${gem} — or set rubyTemplates.commands.${command}.`,
    );
    return [];
  }
  if (r.code !== 0) {
    output.appendLine(
      `[${tool}] format ${doc.uri.fsPath} failed:\n${r.stderr || r.stdout}`,
    );
    vscode.window.showErrorMessage(`${tool} failed: ${firstLine(r)}`);
    return [];
  }
  if (!r.stdout || r.stdout === doc.getText() || doc.version !== version)
    return [];
  return [
    vscode.TextEdit.replace(
      new vscode.Range(doc.positionAt(0), doc.positionAt(doc.getText().length)),
      r.stdout,
    ),
  ];
}

function registerHamlFormatting(context, output) {
  context.subscriptions.push(
    vscode.languages.registerDocumentFormattingEditProvider("haml", {
      provideDocumentFormattingEdits: (doc, { tabSize }) =>
        cfg().get("haml.formatter", "reindent") === "haml-lint"
          ? formatWithHamlLint(doc, output)
          : reindentEdits(doc, tabSize),
    }),
    vscode.languages.registerDocumentRangeFormattingEditProvider("haml", {
      provideDocumentRangeFormattingEdits: (doc, range, { tabSize }) =>
        reindentEdits(doc, tabSize, range.start.line, range.end.line),
    }),
  );
}

async function formatWithHamlLint(doc, output) {
  const { version } = doc;
  const r = await runHamlLint(doc, ["--auto-correct-only", "--stderr"]);
  if (toolMissing(r)) {
    vscode.window.showErrorMessage(
      "haml-lint was not found. Install it with: gem install haml_lint — or set rubyTemplates.commands.hamlLint.",
    );
    return [];
  }
  if (!HAML_LINT_OK_CODES.has(r.code)) {
    output.appendLine(
      `[haml-lint] format ${doc.uri.fsPath} failed:\n${r.stderr || r.stdout}`,
    );
    vscode.window.showErrorMessage(`haml-lint failed: ${firstLine(r)}`);
    return [];
  }
  // haml-lint prints nothing when there was nothing to correct.
  if (!r.stdout || doc.version !== version) return [];
  return [
    vscode.TextEdit.replace(
      new vscode.Range(doc.positionAt(0), doc.positionAt(doc.getText().length)),
      r.stdout,
    ),
  ];
}

function lintDiagnostic(doc, { line, severity, linter, message }) {
  const { lineNumber, firstNonWhitespaceCharacterIndex, text } = doc.lineAt(
    Math.min(line, doc.lineCount - 1),
  );
  const d = new vscode.Diagnostic(
    new vscode.Range(
      lineNumber,
      firstNonWhitespaceCharacterIndex,
      lineNumber,
      text.length,
    ),
    message,
    LINT_SEVERITY[severity] ?? vscode.DiagnosticSeverity.Information,
  );
  d.source = "haml-lint";
  d.code = linter;
  return d;
}

/** Run haml-lint on open and save; a newer run for the same file cancels the older one. */
function startHamlLint(context, output) {
  const diagnostics = vscode.languages.createDiagnosticCollection("haml-lint");
  const pending = new Map(); // uri → { timer, controller }
  let reportedMissing = false;

  const cancel = (key) => {
    const p = pending.get(key);
    if (!p) return;
    clearTimeout(p.timer);
    p.controller.abort();
    pending.delete(key);
  };

  const lint = async (doc, controller) => {
    const r = await runHamlLint(doc, ["--reporter", "json"], controller.signal);
    if (controller.signal.aborted) return;
    pending.delete(doc.uri.toString());
    if (toolMissing(r)) {
      if (!reportedMissing)
        output.appendLine(
          "[haml-lint] not found; Haml lint diagnostics are off. Install it with: gem install haml_lint",
        );
      reportedMissing = true;
      return;
    }
    if (!HAML_LINT_OK_CODES.has(r.code)) {
      output.appendLine(`[haml-lint] ${doc.uri.fsPath}: ${firstLine(r)}`);
      return;
    }
    try {
      diagnostics.set(
        doc.uri,
        hamlLintOffenses(r.stdout).map((o) => lintDiagnostic(doc, o)),
      );
    } catch (e) {
      output.appendLine(
        `[haml-lint] unreadable JSON for ${doc.uri.fsPath}: ${e.message}`,
      );
    }
  };

  const schedule = (doc) => {
    if (
      doc.languageId !== "haml" ||
      doc.uri.scheme !== "file" ||
      !cfg().get("hamlLint.enabled", true)
    )
      return;
    // Generator templates (`*.haml.tt`) hold ERB tags that haml-lint would misreport.
    if (doc.uri.path.toLowerCase().endsWith(".tt")) return;
    const key = doc.uri.toString();
    cancel(key);
    const controller = new AbortController();
    const timer = setTimeout(
      () =>
        lint(doc, controller).catch((e) =>
          output.appendLine(`[haml-lint] ${e.message}`),
        ),
      LINT_DEBOUNCE_MS,
    );
    pending.set(key, { timer, controller });
  };

  const relintAll = () => {
    [...pending.keys()].forEach(cancel);
    diagnostics.clear();
    vscode.workspace.textDocuments.forEach(schedule);
  };

  context.subscriptions.push(
    diagnostics,
    { dispose: () => [...pending.keys()].forEach(cancel) },
    vscode.workspace.onDidOpenTextDocument(schedule),
    vscode.workspace.onDidSaveTextDocument(schedule),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      cancel(doc.uri.toString());
      diagnostics.delete(doc.uri);
    }),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (
        e.affectsConfiguration("rubyTemplates.hamlLint") ||
        e.affectsConfiguration("rubyTemplates.commands.hamlLint")
      )
        relintAll();
    }),
  );
  vscode.workspace.textDocuments.forEach(schedule);
}

function deactivate() {
  previewPanel?.dispose();
  return stopClient(output);
}

export { activate, deactivate };
