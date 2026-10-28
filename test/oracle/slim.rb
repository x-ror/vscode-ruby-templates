# Compiles the Slim template on stdin with Slim 5.2.2's defaults and prints the outcome as JSON.
# --rails compiles as Rails does (lib/slim/railtie.rb), for comparing the code before and after formatting.
# Without it, the compiled Ruby is also parsed with Prism 2.0.0: its errors come back as
# {"errors": [{message, line, column: null}]}, at the template's line (Slim keeps one compiled line per template line).
# --ruby-version X.Y|latest picks Prism's grammar (3.3, 3.4, 3.5, 4.0, 4.1 or latest); without it Prism uses the running Ruby's.
require "json"
require "slim"

# Only generates source, so ActionView isn't needed; use_html_safe is what Slim defaults to once String#html_safe? exists.
RAILS_OPTIONS = { generator: Temple::Generators::RailsOutputBuffer, disable_capture: true, streaming: true, use_html_safe: true }.freeze
rails = ARGV.include?("--rails")
version_flag = ARGV.index("--ruby-version")
PRISM_OPTIONS = version_flag ? { version: ARGV[version_flag + 1] }.freeze : {}.freeze
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

# Prism's column is in the compiled line, which doesn't map back to the template, so only lines are reported. Slim
# adds a last line of its own, so lines past the template are clamped to its last line.
# Tilt evals the source inside a method, where `yield` is valid; parsed on its own Prism calls it invalid.
IGNORED_RUBY_ERRORS = %i[invalid_yield].freeze

def ruby_errors(compiled, template)
  gem "prism", "2.0.0"
  require "prism"
  last_line = [template.lines.size, 1].max
  Prism.parse(compiled, **PRISM_OPTIONS).errors.reject { |e| IGNORED_RUBY_ERRORS.include?(e.type) }.map { |e| { message: e.message, line: [e.location.start_line, last_line].min, column: nil } }
end

def error(message, line: nil, column: nil, slim_column: nil) = { error: { message:, line:, column:, slim_column: } }

template = $stdin.read
result =
  begin
    compiled = Slim::Engine.new(file: "(__TEMPLATE__)", **options).call(template)
    ruby = rails ? [] : ruby_errors(compiled, template)
    ruby.empty? ? { source: compiled } : { errors: ruby }
  rescue Slim::Parser::SyntaxError => e
    error(e.error, line: e.lineno, column: utf16_column(template, e.lineno, e.column), slim_column: e.column)
  rescue LoadError => e
    { dependency_missing: e.path || e.message[/-- (\S+)/, 1] || e.message }
  rescue StandardError => e
    error(e.message)
  end
puts JSON.generate(result)
