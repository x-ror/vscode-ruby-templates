# Compiles the Slim template on stdin with Slim 5.2.2's defaults and prints the outcome as JSON.
# --rails compiles as Rails does (lib/slim/railtie.rb), for comparing the code before and after formatting.
require "json"
require "slim"

# Only generates source, so ActionView isn't needed; use_html_safe is what Slim defaults to once String#html_safe? exists.
RAILS_OPTIONS = { generator: Temple::Generators::RailsOutputBuffer, disable_capture: true, streaming: true, use_html_safe: true }.freeze
rails = ARGV.include?("--rails")
options = rails ? RAILS_OPTIONS : {}
if rails
  # ActiveSupport isn't loaded; compiling only needs its answer for plain strings.
  class String
    def html_safe? = false
    def html_safe = self
  end
end

# Slim counts characters; the language server counts UTF-16 code units.
def utf16_column(template, line, column)
  prefix = template.lines[line - 1].to_s[0, column].to_s
  prefix.encode("UTF-16LE").bytesize / 2
end

def error(message, line: nil, column: nil, slim_column: nil) = { error: { message:, line:, column:, slim_column: } }

template = $stdin.read
result =
  begin
    { source: Slim::Engine.new(file: "(__TEMPLATE__)", **options).call(template) }
  rescue Slim::Parser::SyntaxError => e
    error(e.error, line: e.lineno, column: utf16_column(template, e.lineno, e.column), slim_column: e.column)
  rescue LoadError => e
    { dependency_missing: e.path || e.message[/-- (\S+)/, 1] || e.message }
  rescue StandardError => e
    error(e.message)
  end
puts JSON.generate(result)
