'use strict';
const vscode = require('vscode');
const cp = require('child_process');
const path = require('path');
const fs = require('fs');

const { startClient, stopClient, restartClient } = require('./client');

const TEMPLATE_LANGS = new Set(['erb', 'haml', 'slim']);
// Host-specific ERB ids (erb-js, erb-css, ...) render and compile like plain ERB.
const engineOf = languageId => languageId.startsWith('erb-') ? 'erb' : languageId;

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

  // Language server (Rust): diagnostics, go-to-partial, completions, hover.
  const output = vscode.window.createOutputChannel('Ruby Templates');
  context.subscriptions.push(output);
  sub('rubyTemplates.restartServer', () => restartClient(context, output));
  context.subscriptions.push(vscode.workspace.onDidChangeConfiguration(e => {
    if (e.affectsConfiguration('rubyTemplates.server')) restartClient(context, output);
  }));
  startClient(context, output);

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

// ---------------------------------------------------------------- rendering

function workspaceFolderFor(doc) {
  return vscode.workspace.getWorkspaceFolder(doc.uri)?.uri.fsPath || path.dirname(doc.uri.fsPath);
}

function hasGemfile(dir) {
  let d = dir;
  for (let i = 0; i < 6; i++) {
    if (fs.existsSync(path.join(d, 'Gemfile'))) return true;
    const parent = path.dirname(d);
    if (parent === d) break;
    d = parent;
  }
  return false;
}

function run(cmd, args, options, input) {
  return new Promise(resolve => {
    const useBundler = cfg().get('preview.useBundler', false) && hasGemfile(options.cwd);
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

/** Compile a template to the Ruby source the engine generates. */
async function compileToRuby(doc) {
  const lang = engineOf(doc.languageId);
  const cwd = workspaceFolderFor(doc);
  const src = doc.getText();
  if (lang === 'slim') return run(cfg().get('commands.slimrb', 'slimrb'), ['--compile'], { cwd }, src);
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
        : 'Rails helpers (link_to, render, …) are not available in a standalone render.');
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
  else { vscode.window.showInformationMessage(`No converter for ${from} → ${target}.`); return; }

  const r = await run(bin, args, { cwd }, src);
  if (r.err && r.err.code === 'ENOENT') {
    const pick = await vscode.window.showErrorMessage(`"${bin}" not found. Install with: gem install ${gem}`, 'Copy command');
    if (pick) vscode.env.clipboard.writeText(`gem install ${gem}`);
    return;
  }
  if (r.code !== 0) { vscode.window.showErrorMessage(`Conversion failed: ${(r.stderr || r.stdout).split('\n')[0]}`); return; }

  const newName = ed.document.fileName.replace(/\.(erb|haml|rhtml)$/, `.${target}`);
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
