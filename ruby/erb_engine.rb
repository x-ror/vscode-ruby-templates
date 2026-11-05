# ERB preview/compile for the extension: `ruby erb_engine.rb render|compile` < template.
# Erubi with `escape: true` is how Rails compiles ERB: `<%=` escapes, `<%==` doesn't. Without the gem,
# stdlib ERB renders plain templates, unescaped.
# `percent` (the erb-percent language id) is stdlib ERB with `trim_mode: "%-"`: Erubi has no `%` lines.
# A method's binding has no locals, so the template can't see `src` or `code`.
def empty_binding
  binding
end

$stdin.set_encoding(Encoding::UTF_8)
$stdout.set_encoding(Encoding::UTF_8)
src = $stdin.read
percent = ARGV[1] == "percent"
code = begin
  raise LoadError if percent
  require "erubi"
  Erubi::Engine.new(src, escape: true).src
rescue LoadError
  require "erb"
  ERB.new(src.gsub("<%==", "<%="), trim_mode: percent ? "%-" : "-").src
end
print(ARGV[0] == "compile" ? code : eval(code, empty_binding, "(template)")) # rubocop:disable Security/Eval -- rendering the template is the point
