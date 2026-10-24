// Minimal LSP smoke test against a fake Rails tree: definition, hover, completion, diagnostics.
const cp = require('child_process'), fs = require('fs'), path = require('path'), os = require('os');
const bin = path.join(__dirname, '..', 'server', 'target', 'release', 'ruby-templates-ls');
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rt-e2e-'));
const views = path.join(root, 'app', 'views');
fs.mkdirSync(path.join(views, 'users'), { recursive: true });
fs.mkdirSync(path.join(views, 'shared'), { recursive: true });
fs.mkdirSync(path.join(root, 'app', 'components'), { recursive: true });
fs.writeFileSync(path.join(views, 'users', '_card.html.slim'), '.card = user.name\n');
fs.writeFileSync(path.join(views, 'shared', '_footer.html.erb'), '<footer></footer>\n');
fs.writeFileSync(path.join(root, 'app', 'components', 'badge_component.rb'), 'class BadgeComponent; end\n');
fs.writeFileSync(path.join(views, 'users', '_user.html.erb'), '<p></p>\n');
fs.mkdirSync(path.join(root, 'app', 'javascript', 'controllers'), { recursive: true });
fs.writeFileSync(path.join(root, 'app', 'javascript', 'controllers', 'hello_controller.js'), '');
fs.mkdirSync(path.join(root, 'app', 'assets', 'images'), { recursive: true });
fs.writeFileSync(path.join(root, 'app', 'assets', 'images', 'logo.png'), '');
const file = path.join(views, 'users', 'index.html.erb');
const text = '<%= render "card", user: u %>\n<%= render "shared/footer" %>\n<%= render BadgeComponent.new %>\n<%= render "sh\n<%= f.text_\n<%= link_to\n<%= render "shared/missing" %>\n<% content_for :aside do %>\n<nav>\n<%= render "card" %>\n</nav>\n<% end %>\n<%= render template: "us\n<%= link_to "x",\n  link_ %>\n<%= render @users %>\n<div data-controller="hello"></div>\n<%= image_tag "logo.png" %>\n';
fs.writeFileSync(file, text);
const uri = 'file://' + file;

const proc = cp.spawn(bin);
let id = 0, buf = Buffer.alloc(0); const pending = new Map(); const notes = [];
proc.stdout.on('data', d => {
  buf = Buffer.concat([buf, d]);
  for (;;) {
    const m = buf.toString('utf8').match(/^Content-Length: (\d+)\r\n\r\n/);
    if (!m) return;
    const len = +m[1], start = m[0].length;
    if (buf.length < start + len) return;
    const msg = JSON.parse(buf.slice(start, start + len).toString('utf8'));
    buf = buf.slice(start + len);
    if (msg.id !== undefined && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
    else notes.push(msg);
  }
});
const send = (method, params, isNote) => new Promise(res => {
  const msg = { jsonrpc: '2.0', method, params }; if (!isNote) { msg.id = ++id; pending.set(msg.id, res); }
  const s = JSON.stringify(msg); proc.stdin.write(`Content-Length: ${Buffer.byteLength(s)}\r\n\r\n${s}`); if (isNote) res();
});
const rel = p => p.replace(root, '');
(async () => {
  await send('initialize', { processId: null, rootUri: 'file://' + root, capabilities: {} });
  await send('initialized', {}, true);
  await send('textDocument/didOpen', { textDocument: { uri, languageId: 'erb', version: 1, text } }, true);
  const def1 = await send('textDocument/definition', { textDocument: { uri }, position: { line: 0, character: 14 } });
  console.log('definition "card"      →', (def1.result || []).map(l => rel(l.uri)));
  const def2 = await send('textDocument/definition', { textDocument: { uri }, position: { line: 1, character: 16 } });
  console.log('definition shared/footer →', (def2.result || []).map(l => rel(l.uri)));
  const def3 = await send('textDocument/definition', { textDocument: { uri }, position: { line: 2, character: 14 } });
  console.log('definition component   →', (def3.result || []).map(l => rel(l.uri)));
  for (const [label, line, character] of [['render @users', 15, 12], ['data-controller', 16, 24], ['image_tag', 17, 18]]) {
    const def = await send('textDocument/definition', { textDocument: { uri }, position: { line, character } });
    console.log(`definition ${label}`.padEnd(23), '→', (def.result || []).map(l => rel(l.uri)));
  }
  const hov = await send('textDocument/hover', { textDocument: { uri }, position: { line: 0, character: 14 } });
  console.log('hover                  →', hov.result && hov.result.contents.value.split('\n')[0]);
  const c1 = await send('textDocument/completion', { textDocument: { uri }, position: { line: 3, character: 14 } });
  console.log('completion render "sh  →', (c1.result || []).map(i => i.label));
  const c2 = await send('textDocument/completion', { textDocument: { uri }, position: { line: 4, character: 11 } });
  console.log('completion f.text_     →', (c2.result || []).map(i => i.label));
  const c3 = await send('textDocument/completion', { textDocument: { uri }, position: { line: 5, character: 11 } });
  console.log('completion link_to     →', (c3.result || []).map(i => i.label));
  const c4 = await send('textDocument/completion', { textDocument: { uri }, position: { line: 12, character: 24 } });
  console.log('completion template: "us →', (c4.result || []).map(i => i.label));
  const c5 = await send('textDocument/completion', { textDocument: { uri }, position: { line: 14, character: 7 } });
  console.log('completion 2nd tag line →', (c5.result || []).map(i => i.label));
  const hov2 = await send('textDocument/hover', { textDocument: { uri }, position: { line: 0, character: 2 } });
  console.log('hover <%=              →', hov2.result && hov2.result.contents.value);
  const hov3 = await send('textDocument/hover', { textDocument: { uri }, position: { line: 5, character: 6 } });
  console.log('hover link_to          →', hov3.result && hov3.result.contents.value.split('\n').pop());
  const links = await send('textDocument/documentLink', { textDocument: { uri } });
  console.log('documentLinks          →', (links.result || []).map(l => rel(l.target)));
  const folds = await send('textDocument/foldingRange', { textDocument: { uri } });
  console.log('foldingRanges          →', (folds.result || []).map(f => `${f.startLine}-${f.endLine}`));
  const syms = await send('textDocument/documentSymbol', { textDocument: { uri } });
  const tree = s => s.children ? `${s.name} [${s.children.map(tree).join(', ')}]` : s.name;
  console.log('documentSymbols        →', (syms.result || []).map(tree));
  await new Promise(r => setTimeout(r, 200));
  const diag = notes.find(n => n.method === 'textDocument/publishDiagnostics');
  console.log('diagnostics            →', diag ? diag.params.diagnostics.map(d => `${d.range.start.line + 1}: ${d.message}`) : 'none');
  await send('shutdown', {}); await send('exit', {}, true);
  fs.rmSync(root, { recursive: true, force: true });
  process.exit(0);
})();
setTimeout(() => { console.error('timeout'); process.exit(1); }, 8000);
