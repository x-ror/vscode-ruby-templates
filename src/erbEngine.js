// Pure part of ERB preview/compile (no vscode dependency; tested in test/clientPure.js).

// Erubi with `escape: true` is how Rails compiles ERB: `<%=` escapes, `<%==` doesn't. Without the gem,
// stdlib ERB still renders plain templates.
const SCRIPT = `
src = $stdin.read
code = begin
  require "erubi"
  Erubi::Engine.new(src, escape: true).src
rescue LoadError
  require "erb"
  ERB.new(src, trim_mode: "-").src
end
print(ARGV[0] == "compile" ? code : eval(code, TOPLEVEL_BINDING, "(template)"))
`;

/** `ruby` arguments that compile (`"compile"`) or render (`"render"`) the ERB on stdin. */
function erbRubyArgs(mode) {
  return ["-e", SCRIPT, mode];
}

export { erbRubyArgs };
