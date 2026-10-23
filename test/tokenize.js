// Usage: node test/tokenize.js <grammar-scope> <file>
const fs = require('fs'), path = require('path');
const vsctm = require('vscode-textmate'), onig = require('vscode-oniguruma');
const root = path.join(__dirname, '..');
const wasm = fs.readFileSync(path.join(root, 'node_modules/vscode-oniguruma/release/onig.wasm')).buffer;
const grammars = {
  'text.html.erb': 'syntaxes/erb.tmLanguage.json',
  'text.haml': 'syntaxes/haml.tmLanguage.json',
  'text.slim': 'syntaxes/slim.tmLanguage.json'
};
const registry = new vsctm.Registry({
  onigLib: onig.loadWASM(wasm).then(() => ({ createOnigScanner: s => new onig.OnigScanner(s), createOnigString: s => new onig.OnigString(s) })),
  loadGrammar: async scope => {
    if (grammars[scope]) return vsctm.parseRawGrammar(fs.readFileSync(path.join(root, grammars[scope]), 'utf8'), grammars[scope]);
    // Stub for grammars VS Code ships (ruby, js, css, html…): mark words so we can see the embedding worked.
    return { scopeName: scope, patterns: [{ name: 'stub.' + scope, match: '[^\\s{}()\\[\\]"\']+|"[^"]*"|\'[^\']*\'' }] };
  }
});
(async () => {
  const g = await registry.loadGrammar(process.argv[2]);
  const lines = fs.readFileSync(process.argv[3], 'utf8').split('\n');
  let state = vsctm.INITIAL;
  for (const line of lines) {
    const r = g.tokenizeLine(line, state);
    console.log(JSON.stringify(line));
    for (const t of r.tokens) {
      const txt = line.slice(t.startIndex, t.endIndex);
      if (!txt.trim()) continue;
      console.log('   ' + JSON.stringify(txt).padEnd(28) + t.scopes.slice(1).map(s => s.replace(/\.(erb|haml|slim)$/, '')).join(' > '));
    }
    state = r.ruleStack;
  }
})().catch(e => { console.error(e); process.exit(1); });
