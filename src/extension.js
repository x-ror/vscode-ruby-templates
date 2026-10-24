'use strict';
const vscode = require('vscode');
const cp = require('child_process');
const path = require('path');
const fs = require('fs');

const { startClient, stopClient, restartClient } = require('./client');
const { toggleSlimMarker, wrapSlim, dedent, partialPath } = require('./slim');

const TEMPLATE_LANGS = new Set(['erb', 'haml', 'slim']);
// Host-specific ERB ids (erb-js, erb-css, ...) render and compile like plain ERB.
const engineOf = languageId => languageId.startsWith('erb-') ? 'erb' : languageId;

// Rails boots slowly on big apps, but a hung runner shouldn't leave the preview waiting forever.
const RAILS_RENDER_TIMEOUT_MS = 60_000;
// How far up from a template to look for a Gemfile or bin/rails.
const MAX_PARENT_DIRS = 6;

// The server reads these at startup, so a change restarts it.
const SERVER_SETTINGS = ['rubyTemplates.server', 'rubyTemplates.slim.extraEngines', 'rubyTemplates.slim.disabledDiagnostics'];

/** @type {vscode.WebviewPanel | undefined} */
let previewPanel;
/** @type {string | undefined} */
let previewUri;

function activate(context) {
  const sub = (cmd, fn) => context.subscriptions.push(vscode.commands.registerCommand(cmd, fn));

  sub('rubyTemplates.toggleErbTag', toggleErbTag);
  sub('rubyTemplates.wrapInErb', wrapInErb);
  sub('rubyTemplates.preview', () => openPreview(context));
  sub('rubyTemplates.showRenderedSource', showCompiledSource);
  sub('rubyTemplates.convertToHaml', () => convert('haml'));
  sub('rubyTemplates.convertToSlim', () => convert('slim'));
  sub('rubyTemplates.convertToErb', () => convert('erb'));
  sub('rubyTemplates.toggleSlimTag', toggleSlimTag);
  sub('rubyTemplates.wrapInSlim', wrapInSlim);
  sub('rubyTemplates.extractSlimPartial', extractSlimPartial);

  // Language server (Rust): diagnostics, go-to-partial, completions, hover.
  const output = vscode.window.createOutputChannel('Ruby Templates');
  context.subscriptions.push(output);
  sub('rubyTemplates.restartServer', () => restartClient(context, output));
  context.subscriptions.push(vscode.workspace.onDidChangeConfiguration(e => {
    if (SERVER_SETTINGS.some(s => e.affectsConfiguration(s))) restartClient(context, output);
  }));
  startClient(context, output);
  registerSlimLint(context, output);

  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument(doc => {
      if (!previewPanel || !previewUri) return;
      if (!cfg().get('preview.refreshOnSave', true)) return;
      if (doc.uri.toString() === previewUri) renderPreview(doc);
    })
  );

  // Status bar: shows active template engine and offers preview.
  const status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 90);
  status.command = 'rubyTemplates.preview';
  status.tooltip = 'Ruby Templates: preview rendered HTML';
  const updateStatus = () => {
    const ed = vscode.window.activeTextEditor;
    if (ed && TEMPLATE_LANGS.has(engineOf(ed.document.languageId))) {
      status.text = `$(preview) ${ed.document.languageId.toUpperCase()}`;
      status.show();
    } else status.hide();
  };
  context.subscriptions.push(status, vscode.window.onDidChangeActiveTextEditor(updateStatus));
  updateStatus();
}

function cfg() { return vscode.workspace.getConfiguration('rubyTemplates'); }

// ---------------------------------------------------------------- ERB editing

const ERB_RE = /<%(==|=|#|-)?([\s\S]*?)(-?)%>/g;

/** Find the ERB tag containing `offset`, or null. */
function erbTagAt(text, offset) {
  ERB_RE.lastIndex = 0;
  let m;
  while ((m = ERB_RE.exec(text))) {
    if (offset >= m.index && offset <= m.index + m[0].length) {
      return { start: m.index, end: m.index + m[0].length, kind: m[1] || '', body: m[2], trimEnd: m[3] };
    }
    if (m.index > offset) break;
  }
  return null;
}

async function toggleErbTag() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const doc = ed.document;
  const cycle = { '': '=', '=': '#', '#': '', '-': '=', '==': '#' };
  await ed.edit(eb => {
    for (const sel of ed.selections) {
      const tag = erbTagAt(doc.getText(), doc.offsetAt(sel.active));
      if (!tag) continue;
      const next = cycle[tag.kind] ?? '';
      const body = tag.body.trim();
      const replacement = `<%${next} ${body} ${tag.trimEnd}%>`.replace(/\s+-%>$/, ' -%>');
      eb.replace(new vscode.Range(doc.positionAt(tag.start), doc.positionAt(tag.end)), replacement);
    }
  });
}

async function wrapInErb() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  await ed.edit(eb => {
    for (const sel of ed.selections) {
      const text = ed.document.getText(sel);
      eb.replace(sel, text ? `<%= ${text} %>` : '<%=  %>');
    }
  });
  if (ed.selections.length === 1 && ed.selection.isEmpty) {
    const p = ed.selection.active.translate(0, -3);
    ed.selection = new vscode.Selection(p, p);
  }
}

// ---------------------------------------------------------------- Slim editing

async function toggleSlimTag() {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const lines = new Set(ed.selections.flatMap(({ start, end }) =>
    Array.from({ length: end.line - start.line + 1 }, (_, i) => start.line + i)));
  await ed.edit(eb => {
    for (const n of lines) {
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
  await ed.edit(eb => {
    for (const sel of ed.selections.filter(s => s.isSingleLine)) {
      const line = doc.lineAt(sel.start.line);
      const content = line.text.trim();
      if (content && doc.getText(sel).trim() === content) {
        const start = line.firstNonWhitespaceCharacterIndex;
        eb.replace(new vscode.Range(line.lineNumber, start, line.lineNumber, start + content.length), wrapSlim(content, true));
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
  if (!ed || ed.selection.isEmpty || ed.document.uri.scheme !== 'file') return;
  const doc = ed.document;
  const { start, end } = ed.selection;
  // A selection ending at column 0 doesn't take that line along.
  const last = end.character === 0 && end.line > start.line ? end.line - 1 : end.line;
  const range = new vscode.Range(start.line, 0, last, doc.lineAt(last).text.length);

  const name = await vscode.window.showInputBox({
    prompt: 'Partial name, rendered as `= render "name"` (`card`, `shared/card`)',
    validateInput: v => /^([\w-]+\/)*[a-z_]\w*$/.test(v) ? null : 'Use a lowercase Ruby identifier, optionally prefixed by directories.'
  });
  if (!name) return;
  const target = partialPath(doc.uri.fsPath, name);
  if (!target) {
    vscode.window.showErrorMessage(`"${name}" renders from app/views, but this file isn't under app/views.`);
    return;
  }
  if (fs.existsSync(target)) {
    vscode.window.showErrorMessage(`${path.basename(target)} already exists.`);
    return;
  }
  const eol = doc.eol === vscode.EndOfLine.CRLF ? '\r\n' : '\n';
  const { indent, lines } = dedent(doc.getText(range).split(/\r?\n/));
  await vscode.workspace.fs.createDirectory(vscode.Uri.file(path.dirname(target)));
  await vscode.workspace.fs.writeFile(vscode.Uri.file(target), Buffer.from(lines.join(eol) + eol));
  await ed.edit(eb => eb.replace(range, `${indent}= render "${name}"`));
}

// ---------------------------------------------------------------- rendering

function workspaceFolderFor(doc) {
  return vscode.workspace.getWorkspaceFolder(doc.uri)?.uri.fsPath || path.dirname(doc.uri.fsPath);
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

function run(cmd, args, options, input) {
  return new Promise(resolve => {
    const useBundler = cfg().get('preview.useBundler', false) && findUp(options.cwd, 'Gemfile') !== undefined;
    const full = useBundler ? ['bundle', ['exec', cmd, ...args]] : [cmd, args];
    const child = cp.execFile(full[0], full[1], { ...options, maxBuffer: 16 * 1024 * 1024 }, (err, stdout, stderr) => {
      resolve({ code: err ? (err.code ?? 1) : 0, stdout: String(stdout || ''), stderr: String(stderr || ''), err });
    });
    if (input !== undefined && child.stdin) {
      child.stdin.on('error', () => {});
      child.stdin.end(input);
    }
  });
}

/** Render a template document to HTML with the matching CLI. */
async function renderToHtml(doc) {
  const lang = engineOf(doc.languageId);
  const cwd = workspaceFolderFor(doc);
  const src = doc.getText();
  const railsRoot = cfg().get('preview.useRails', false) && doc.uri.scheme === 'file'
    ? findUp(path.dirname(doc.uri.fsPath), path.join('bin', 'rails'))
    : undefined;
  if (railsRoot) return renderInRails(railsRoot, lang, src);
  if (lang === 'slim') {
    const bin = cfg().get('commands.slimrb', 'slimrb');
    return run(bin, ['--pretty'], { cwd }, src);
  }
  if (lang === 'haml') {
    const bin = cfg().get('commands.haml', 'haml');
    return run(bin, ['render', '--'], { cwd }, src).then(async r => {
      if (r.code === 0) return r;
      // Haml < 6 CLI: `haml` reads stdin, no `render` subcommand.
      return run(bin, [], { cwd }, src);
    });
  }
  if (lang === 'erb') {
    const bin = cfg().get('commands.erb', 'erb');
    return run(bin, ['-T', '-'], { cwd }, src);
  }
  return { code: 1, stdout: '', stderr: `Unsupported language: ${lang}` };
}

/** Render through the app's `bin/rails runner`, so helpers, routes and partials resolve. */
async function renderInRails(root, lang, src) {
  const script = `print ApplicationController.render(inline: $stdin.read, type: :${lang})`;
  const r = await run('ruby', [path.join('bin', 'rails'), 'runner', script], { cwd: root, timeout: RAILS_RENDER_TIMEOUT_MS }, src);
  if (r.err?.killed) return { ...r, code: 1, stderr: `bin/rails runner timed out after ${RAILS_RENDER_TIMEOUT_MS / 1000}s.\n${r.stderr}` };
  return r;
}

/** Compile a template to the Ruby source the engine generates. */
async function compileToRuby(doc) {
  const lang = engineOf(doc.languageId);
  const cwd = workspaceFolderFor(doc);
  const src = doc.getText();
  if (lang === 'slim') {
    const args = cfg().get('slim.compileForRails', false) ? ['--rails'] : ['--compile'];
    return run(cfg().get('commands.slimrb', 'slimrb'), args, { cwd }, src);
  }
  if (lang === 'haml') {
    const bin = cfg().get('commands.haml', 'haml');
    const r = await run(bin, ['compile', '--'], { cwd }, src);
    return r.code === 0 ? r : run(bin, ['--debug'], { cwd }, src);
  }
  if (lang === 'erb') return run(cfg().get('commands.erb', 'erb'), ['-x', '-T', '-'], { cwd }, src);
  return { code: 1, stdout: '', stderr: `Unsupported language: ${lang}` };
}

function missingToolMessage(bin, lang) {
  const gem = { slim: 'slim', haml: 'haml', erb: '(ships with Ruby)' }[engineOf(lang)] || bin;
  return `"${bin}" was not found. Install it with: gem install ${gem} — or point rubyTemplates.commands.* at the executable.`;
}

async function openPreview(context) {
  const ed = vscode.window.activeTextEditor;
  if (!ed || !TEMPLATE_LANGS.has(engineOf(ed.document.languageId))) {
    vscode.window.showInformationMessage('Open an ERB, Haml or Slim file first.');
    return;
  }
  previewUri = ed.document.uri.toString();
  if (!previewPanel) {
    previewPanel = vscode.window.createWebviewPanel('rubyTemplates.preview', 'Template Preview', vscode.ViewColumn.Beside, { enableScripts: false });
    previewPanel.onDidDispose(() => { previewPanel = undefined; previewUri = undefined; }, null, context.subscriptions);
  }
  previewPanel.title = `Preview: ${path.basename(ed.document.fileName)}`;
  await renderPreview(ed.document);
}

async function renderPreview(doc) {
  if (!previewPanel) return;
  const r = await renderToHtml(doc);
  const esc = s => s.replace(/[&<>]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));
  if (r.err && r.err.code === 'ENOENT') {
    previewPanel.webview.html = errorPage('Renderer not found', missingToolMessage(r.err.path, doc.languageId));
    return;
  }
  if (r.code !== 0) {
    previewPanel.webview.html = errorPage('Render failed', r.stderr || r.stdout,
      engineOf(doc.languageId) === 'erb'
        ? 'Plain `erb` has no Rails helpers (link_to, render, …). The preview works best for templates that only use Ruby core.'
        : 'Rails helpers (link_to, render, …) are not available in a standalone render; set rubyTemplates.preview.useRails to render inside the app.');
    return;
  }
  previewPanel.webview.html = `<!DOCTYPE html><html><head><meta charset="utf-8">
<style>
  body{font-family:system-ui,sans-serif;margin:0}
  .bar{display:flex;gap:1rem;align-items:center;padding:.4rem .8rem;font-size:12px;background:var(--vscode-editorWidget-background);border-bottom:1px solid var(--vscode-widget-border,#444)}
  .bar b{opacity:.8}
  iframe{border:0;width:100%;height:calc(55vh);background:#fff}
  pre{margin:0;padding:.8rem;font:12px/1.4 var(--vscode-editor-font-family,monospace);white-space:pre-wrap;word-break:break-word;background:var(--vscode-textCodeBlock-background);max-height:40vh;overflow:auto}
</style></head><body>
<div class="bar"><b>Rendered</b> <span>${esc(path.basename(doc.fileName))} · ${esc(doc.languageId)}</span></div>
<iframe sandbox="" srcdoc="${r.stdout.replace(/"/g, '&quot;')}"></iframe>
<div class="bar"><b>HTML source</b></div>
<pre>${esc(r.stdout)}</pre>
</body></html>`;
}

function errorPage(title, detail, hint) {
  const esc = s => String(s).replace(/[&<>]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));
  return `<!DOCTYPE html><html><body style="font-family:system-ui;padding:1rem">
<h3 style="color:var(--vscode-errorForeground)">${esc(title)}</h3>
<pre style="white-space:pre-wrap">${esc(detail)}</pre>
${hint ? `<p style="opacity:.8">${esc(hint)}</p>` : ''}</body></html>`;
}

async function showCompiledSource() {
  const ed = vscode.window.activeTextEditor;
  if (!ed || !TEMPLATE_LANGS.has(engineOf(ed.document.languageId))) return;
  const r = await compileToRuby(ed.document);
  if (r.err && r.err.code === 'ENOENT') {
    vscode.window.showErrorMessage(missingToolMessage(r.err.path, ed.document.languageId));
    return;
  }
  if (r.code !== 0) {
    vscode.window.showErrorMessage(`Compile failed: ${(r.stderr || r.stdout).split('\n')[0]}`);
    return;
  }
  const doc = await vscode.workspace.openTextDocument({ language: 'ruby', content: r.stdout });
  await vscode.window.showTextDocument(doc, { viewColumn: vscode.ViewColumn.Beside, preview: true });
}

// ---------------------------------------------------------------- slim-lint

function registerSlimLint(context, output) {
  const lints = vscode.languages.createDiagnosticCollection('slim-lint');
  let warnedMissing = false;
  // Saves can overlap; only the latest run per document may publish.
  const latestRun = new Map();

  const warnMissing = message => {
    if (!warnedMissing) vscode.window.showWarningMessage(message);
    warnedMissing = true;
  };

  const lint = async doc => {
    if (doc.languageId !== 'slim' || doc.uri.scheme !== 'file') return;
    if (!cfg().get('slimLint.enabled', false)) return;
    const key = doc.uri.toString();
    const runId = (latestRun.get(key) ?? 0) + 1;
    latestRun.set(key, runId);
    const bin = cfg().get('commands.slimLint', 'slim-lint');
    const r = await run(bin, ['--reporter', 'json', doc.uri.fsPath], { cwd: workspaceFolderFor(doc) });
    if (latestRun.get(key) !== runId || doc.isClosed) return;
    if (r.err?.code === 'ENOENT') {
      const missing = r.err.path ?? bin;
      warnMissing(missing === bin
        ? `"${bin}" was not found. Install it with: gem install slim_lint — or set rubyTemplates.commands.slimLint.`
        : `"${missing}" was not found, so slim-lint can't run through Bundler. Install Bundler or turn off rubyTemplates.preview.useBundler.`);
      return;
    }
    // `bundle exec` exits 127 when the bundle doesn't include the command.
    if (r.code === 127 || /command not found/.test(r.stderr)) {
      warnMissing(`"${bin}" is not in the bundle. Add slim_lint to the Gemfile, or turn off rubyTemplates.preview.useBundler.`);
      return;
    }
    try {
      lints.set(doc.uri, slimLintDiagnostics(JSON.parse(r.stdout), doc));
    } catch {
      output.appendLine(`[slim-lint] ${bin} exited ${r.code}: ${r.stderr || r.stdout}`);
    }
  };

  context.subscriptions.push(
    lints,
    vscode.workspace.onDidSaveTextDocument(doc => lint(doc)),
    vscode.workspace.onDidCloseTextDocument(doc => {
      lints.delete(doc.uri);
      latestRun.delete(doc.uri.toString());
    }),
    vscode.workspace.onDidChangeConfiguration(e => {
      if (e.affectsConfiguration('rubyTemplates.slimLint') && !cfg().get('slimLint.enabled', false)) lints.clear();
    })
  );
}

/** Map a `slim-lint --reporter json` report to diagnostics on the offending lines of `doc`. */
function slimLintDiagnostics(report, doc) {
  return (report.files ?? []).flatMap(f => f.offenses ?? []).map(({ severity, message, linter, location }) => {
    const line = doc.lineAt(Math.min(Math.max((location?.line ?? 1) - 1, 0), doc.lineCount - 1));
    const range = new vscode.Range(line.lineNumber, line.firstNonWhitespaceCharacterIndex, line.lineNumber, line.text.length);
    const level = severity === 'error' ? vscode.DiagnosticSeverity.Error : vscode.DiagnosticSeverity.Warning;
    const d = new vscode.Diagnostic(range, message, level);
    d.source = 'slim-lint';
    d.code = linter;
    return d;
  });
}

// ---------------------------------------------------------------- converters

async function convert(target) {
  const ed = vscode.window.activeTextEditor;
  if (!ed) return;
  const from = ed.document.languageId;
  const cwd = workspaceFolderFor(ed.document);
  const src = ed.document.getText();
  let bin, args, gem;
  if (from === 'erb' && target === 'haml') { bin = cfg().get('commands.html2haml', 'html2haml'); args = ['--erb']; gem = 'html2haml'; }
  else if (from === 'erb' && target === 'slim') { bin = cfg().get('commands.erb2slim', 'erb2slim'); args = []; gem = 'html2slim'; }
  else if (from === 'haml' && target === 'slim') { bin = cfg().get('commands.haml2slim', 'haml2slim'); args = []; gem = 'haml2slim'; }
  else if (from === 'slim' && target === 'erb') { bin = cfg().get('commands.slimrb', 'slimrb'); args = ['--erb']; gem = 'slim'; }
  else { vscode.window.showInformationMessage(`No converter for ${from} → ${target}.`); return; }

  const r = await run(bin, args, { cwd }, src);
  if (r.err && r.err.code === 'ENOENT') {
    const pick = await vscode.window.showErrorMessage(`"${bin}" not found. Install with: gem install ${gem}`, 'Copy command');
    if (pick) vscode.env.clipboard.writeText(`gem install ${gem}`);
    return;
  }
  if (r.code !== 0) { vscode.window.showErrorMessage(`Conversion failed: ${(r.stderr || r.stdout).split('\n')[0]}`); return; }

  const newName = ed.document.fileName.replace(/\.(erb|haml|rhtml|slim)$/, `.${target}`);
  const choice = await vscode.window.showQuickPick(
    [
      { label: `Save as ${path.basename(newName)} next to the original`, id: 'save' },
      { label: 'Open as untitled document', id: 'untitled' },
      { label: 'Replace contents of the current file', id: 'replace' }
    ],
    { placeHolder: `Converted ${from} → ${target}. What to do with the result?` });
  if (!choice) return;
  if (choice.id === 'untitled') {
    const d = await vscode.workspace.openTextDocument({ language: target, content: r.stdout });
    await vscode.window.showTextDocument(d, { viewColumn: vscode.ViewColumn.Beside });
  } else if (choice.id === 'save') {
    if (fs.existsSync(newName)) {
      const ok = await vscode.window.showWarningMessage(`${path.basename(newName)} exists. Overwrite?`, { modal: true }, 'Overwrite');
      if (ok !== 'Overwrite') return;
    }
    fs.writeFileSync(newName, r.stdout);
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(newName), { viewColumn: vscode.ViewColumn.Beside });
  } else {
    const full = new vscode.Range(ed.document.positionAt(0), ed.document.positionAt(src.length));
    await ed.edit(eb => eb.replace(full, r.stdout));
    await vscode.languages.setTextDocumentLanguage(ed.document, target);
  }
}

function deactivate() { previewPanel?.dispose(); return stopClient(); }

module.exports = { activate, deactivate };
