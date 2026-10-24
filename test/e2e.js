// Minimal LSP smoke test against a fake Rails tree: definition, hover, completion, diagnostics, symbols, rename.
const cp = require('child_process'), fs = require('fs'), path = require('path'), os = require('os');
const { isDeepStrictEqual } = require('util');
const bin = path.join(__dirname, '..', 'server', 'target', 'release', 'ruby-templates-ls');
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rt-e2e-'));
const views = path.join(root, 'app', 'views');
fs.mkdirSync(path.join(views, 'users'), { recursive: true });
fs.mkdirSync(path.join(views, 'shared'), { recursive: true });
fs.mkdirSync(path.join(root, 'app', 'components'), { recursive: true });
fs.writeFileSync(path.join(views, 'users', '_card.html.slim'), '.card = user.name\n');
fs.writeFileSync(path.join(views, 'shared', '_footer.html.erb'), '<footer></footer>\n');
fs.writeFileSync(path.join(root, 'app', 'components', 'badge_component.rb'), 'class BadgeComponent; end\n');
fs.writeFileSync(path.join(root, 'app', 'components', 'badge_component.html.haml'), '%span.badge= content\n');
const file = path.join(views, 'users', 'index.html.erb');
const text = '<%= render "card", user: u %>\n<%= render "shared/footer" %>\n<%= render BadgeComponent.new %>\n<%= render "sh\n<%= f.text_\n<%= link_to\n<%= render "shared/missing" %>\n<% content_for :aside do %>\n<nav>\n<%= render "card" %>\n</nav>\n<% end %>\n';
fs.writeFileSync(file, text);
const uri = 'file://' + file;
const hamlFile = path.join(views, 'users', 'show.html.haml');
const hamlText = [
  '- content_for :aside do',
  '  %nav= render "card"',
  '%section#main',
  '  = render "shared/footer"',
  '  = render BadgeComponent.new',
  '  = link_',
  '  %p see (link_',
  '  %sec',
  '  %a{hr}',
  '  :jav',
  '  &= x',
  '  = render "shared/missing"',
  '  %p text',
  '    %b nested',
  '  :css',
  '    .btn{',
  '  :ruby',
  '    x = link_',
  ''
].join('\n');
fs.writeFileSync(hamlFile, hamlText);
const hamlUri = 'file://' + hamlFile;

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
const rel = p => p.replace('file://' + root, '');
let failures = 0;
const check = (label, actual, expected) => {
  const ok = isDeepStrictEqual(actual, expected);
  if (!ok) failures++;
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${label.padEnd(24)} →`, actual);
  if (!ok) console.log('     expected', expected);
};
(async () => {
  await send('initialize', { processId: null, rootUri: 'file://' + root, capabilities: {} });
  await send('initialized', {}, true);
  await send('textDocument/didOpen', { textDocument: { uri, languageId: 'erb', version: 1, text } }, true);
  const req = (method, u, line, character) => send(method, { textDocument: { uri: u }, position: { line, character } });
  const targets = r => (r.result || []).map(l => rel(l.uri));
  const labels = r => (r.result || []).map(i => i.label);
  const firstLine = r => r.result?.contents.value.split('\n')[0] ?? null;
  const tree = s => s.children ? `${s.name} [${s.children.map(tree).join(', ')}]` : s.name;
  const component = ['/app/components/badge_component.rb', '/app/components/badge_component.html.haml'];

  check('definition "card"', targets(await req('textDocument/definition', uri, 0, 14)), ['/app/views/users/_card.html.slim']);
  check('definition shared/footer', targets(await req('textDocument/definition', uri, 1, 16)), ['/app/views/shared/_footer.html.erb']);
  check('definition component', targets(await req('textDocument/definition', uri, 2, 14)), component);
  check('hover', firstLine(await req('textDocument/hover', uri, 0, 14)), '**Partial** `card`');
  check('completion render "sh', labels(await req('textDocument/completion', uri, 3, 14)), ['shared/footer']);
  check('completion f.text_', labels(await req('textDocument/completion', uri, 4, 11)), ['text_field', 'text_area']);
  check('completion link_to', labels(await req('textDocument/completion', uri, 5, 11)), ['link_to']);
  const links = await send('textDocument/documentLink', { textDocument: { uri } });
  check('documentLinks', (links.result || []).map(l => rel(l.target)),
    ['/app/views/users/_card.html.slim', '/app/views/shared/_footer.html.erb', '/app/views/users/_card.html.slim']);
  const folds = await send('textDocument/foldingRange', { textDocument: { uri } });
  check('foldingRanges', (folds.result || []).map(f => `${f.startLine}-${f.endLine}`), ['3-5', '7-10', '8-9']);
  const syms = await send('textDocument/documentSymbol', { textDocument: { uri } });
  check('documentSymbols', (syms.result || []).map(tree),
    ['render "card"', 'render "shared/footer"', 'render "shared/missing"', 'content_for :aside [nav [render "card"]]']);
  await new Promise(r => setTimeout(r, 200));
  const diagnostics = u => {
    const diag = notes.find(n => n.method === 'textDocument/publishDiagnostics' && n.params.uri === u);
    return diag ? diag.params.diagnostics.map(d => `${d.range.start.line + 1}: ${d.message}`) : 'none';
  };
  check('diagnostics', diagnostics(uri), ['7: Partial `shared/missing` not found']);

  await send('textDocument/didOpen', { textDocument: { uri: hamlUri, languageId: 'haml', version: 1, text: hamlText } }, true);
  const haml = (method, line, character) => req(method, hamlUri, line, character);
  check('haml definition "card"', targets(await haml('textDocument/definition', 1, 17)), ['/app/views/users/_card.html.slim']);
  check('haml definition comp.', targets(await haml('textDocument/definition', 4, 15)), component);
  check('haml hover render', firstLine(await haml('textDocument/hover', 3, 16)), '**Partial** `shared/footer`');
  check('haml hover &=', firstLine(await haml('textDocument/hover', 10, 2)), '**Haml** `&=`');
  check('haml completion link_', labels(await haml('textDocument/completion', 5, 9)), ['link_to']);
  check('haml completion text (', labels(await haml('textDocument/completion', 6, 15)), []);
  check('haml completion tag', labels(await haml('textDocument/completion', 7, 6)), ['section']);
  check('haml completion {hr', labels(await haml('textDocument/completion', 8, 7)), ['href', 'hreflang']);
  check('haml completion :jav', labels(await haml('textDocument/completion', 9, 6)), ['javascript']);
  check('haml completion :css {', labels(await haml('textDocument/completion', 15, 9)), []);
  check('haml completion :ruby', labels(await haml('textDocument/completion', 17, 14)), ['link_to']);
  const hamlSyms = await send('textDocument/documentSymbol', { textDocument: { uri: hamlUri } });
  check('haml documentSymbols', (hamlSyms.result || []).map(tree),
    ['content_for :aside [nav [render "card"]]', 'section#main [render "shared/footer", render "shared/missing"]']);
  const hamlFolds = await send('textDocument/foldingRange', { textDocument: { uri: hamlUri } });
  check('haml foldingRanges', (hamlFolds.result || []).map(f => `${f.startLine}-${f.endLine}`).sort(),
    ['0-1', '12-13', '14-15', '16-17', '2-17']);
  await new Promise(r => setTimeout(r, 200));
  check('haml diagnostics', diagnostics(hamlUri), [
    '10: Unknown Haml filter `:jav`',
    "13: Illegal nesting: content can't be both given on the same line as %p and nested within it.",
    '12: Partial `shared/missing` not found'
  ]);

  const card = path.join(views, 'users', '_card.html.slim');
  const renamed = await send('workspace/willRenameFiles', { files: [{ oldUri: 'file://' + card, newUri: 'file://' + card.replace('_card', '_tile') }] });
  const edits = Object.entries(renamed.result?.changes ?? {})
    .flatMap(([u, es]) => es.map(e => `${rel(u)} ${e.range.start.line}:${e.newText}`)).sort();
  check('willRenameFiles _card', edits,
    ['/app/views/users/index.html.erb 0:tile', '/app/views/users/index.html.erb 9:tile', '/app/views/users/show.html.haml 1:tile']);
  await send('shutdown', {}); await send('exit', {}, true);
  fs.rmSync(root, { recursive: true, force: true });
  console.log(failures ? `${failures} check(s) failed` : 'all checks passed');
  process.exit(failures ? 1 : 0);
})();
setTimeout(() => { console.error('timeout'); process.exit(1); }, 8000);
