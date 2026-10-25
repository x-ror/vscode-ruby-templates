# Compiles the Slim template on stdin with Slim 5.2.2's defaults and prints the outcome as JSON.
require "json"
require "slim"

# Slim counts characters; the language server counts UTF-16 code units.
def utf16_column(template, line, column)
  prefix = template.lines[line - 1].to_s[0, column].to_s
  prefix.encode("UTF-16LE").bytesize / 2
end

def error(message, line: nil, column: nil, slim_column: nil) = { error: { message:, line:, column:, slim_column: } }

template = $stdin.read
result =
  begin
    { source: Slim::Engine.new(file: "(__TEMPLATE__)").call(template) }
  rescue Slim::Parser::SyntaxError => e
    error(e.error, line: e.lineno, column: utf16_column(template, e.lineno, e.column), slim_column: e.column)
  rescue LoadError => e
    { dependency_missing: e.path || e.message[/-- (\S+)/, 1] || e.message }
  rescue StandardError => e
    error(e.message)
  end
puts JSON.generate(result)
