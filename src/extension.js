import * as vscode from "vscode";
import path from "path";

import {
  startClient,
  stopClient,
  restartClient,
  serverFormatEdits,
} from "./client.js";
import {
  erbTagBodyLines,
  toggleErbTagAt,
  toggleRubyComment,
} from "./erbTags.js";
import { toggleHamlLines } from "./haml.js";
import { extractBlock, partialFileName } from "./partial.js";
import {
  relatedFiles,
  relatedDirs,
  actionSource,
  actionLine,
  previewPartners,
} from "./related.js";
import {
  toggleSlimMarker,
  inVerbatimBlock,
  wrapSlim,
  dedent,
  partialPath,
} from "./slim.js";
import { TEMPLATE_LANGS, engineOf } from "./render.js";
import { cfg, exists, confirmOverwrite } from "./workspace.js";
import {
  openPreview,
  refreshPreviewOnSave,
  closePreview,
  showCompiledSource,
} from "./preview.js";
import { convert } from "./convertTemplate.js";
import { registerRoutes } from "./railsRoutes.js";
import { registerErbAutoClose } from "./erbAutoClose.js";

// The server reads these at startup, so a change restarts it.
const SERVER_SETTINGS = [
  "rubyTemplates.server",
  "rubyTemplates.slim.extraEngines",
  "rubyTemplates.slim.disabledDiagnostics",
  "rubyTemplates.erb.disabledDiagnostics",
  "rubyTemplates.haml.disabledDiagnostics",
  "rubyTemplates.jbuilder.disabledDiagnostics",
  "rubyTemplates.erb.format.alignComments",
  ...["erb", "haml", "slim"].flatMap((lang) => [
    `rubyTemplates.${lang}.diagnostics.enabled`,
    `rubyTemplates.${lang}.format.enabled`,
  ]),
];

/** @type {vscode.LogOutputChannel | undefined} */
let output;

function activate(context) {
  const sub = (cmd, fn) =>
    context.subscriptions.push(vscode.commands.registerCommand(cmd, fn));

  sub("rubyTemplates.toggleErbTag", toggleErbTag);
  sub("rubyTemplates.toggleErbLineComment", toggleErbLineComment);
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
    vscode.workspace.onDidSaveTextDocument(refreshPreviewOnSave),
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

// The built-in toggle reads the language at column 1, which is the ERB host on an indented line.
async function toggleErbLineComment() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const doc = ed.document;
  const lastLine = Math.max(...ed.selections.map((sel) => sel.end.line));
  const body = erbTagBodyLines(doc.getText(), lastLine);
  // One group per selection, like the built-in: each decides add or remove on its own.
  const claimed = new Set();
  const groups = ed.selections
    .map((sel) =>
      selectedLines(sel).filter(
        (line) => !claimed.has(line) && claimed.add(line),
      ),
    )
    .filter((group) => group.length);
  if (!groups.length || !groups.flat().every((line) => body.has(line))) {
    return vscode.commands.executeCommand("editor.action.commentLine");
  }
  const edits = toggleRubyComment(
    groups.map((group) => group.map((line) => doc.lineAt(line).text)),
  );
  await ed.edit((eb) => {
    groups.forEach((group, g) => {
      for (const { index, start, end, text } of edits[g]) {
        const line = group[index];
        eb.replace(new vscode.Range(line, start, line, end), text);
      }
    });
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

/** Last line a selection covers; one ending at column 0 of a later line doesn't take that line along. */
const lastSelectedLine = ({ start, end }) =>
  end.character === 0 && end.line > start.line ? end.line - 1 : end.line;

/** Line numbers a selection covers, up to `lastSelectedLine`. */
function selectedLines(sel) {
  return Array.from(
    { length: lastSelectedLine(sel) - sel.start.line + 1 },
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

/** Move the selection into `_name.<ext>` next to the template and render it in its place. */
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

  if (!(await confirmOverwrite(partial))) return;
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

async function toggleSlimTag() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const text = ed.document.getText().split(/\r?\n/);
  const lines = new Set(ed.selections.flatMap(selectedLines));
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

// ---------------------------------------------------------------- formatting

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
  closePreview();
  return stopClient(output);
}

export { activate, deactivate };
