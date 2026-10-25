# Compiles the Haml template on stdin with Haml 7.5.1's defaults and prints the outcome as JSON.
# --rails compiles with the options of Haml::RailsTemplate (lib/haml/rails_template.rb), for comparing the code
# before and after formatting.
require "json"
# Haml 7.5.1 mis-lexes nested hashes with prism 2.x (BRACE_LEFT_HASH isn't balanced), so it runs on 1.9.0.
gem "prism", "1.9.0"
require "haml"

# That class needs ActionView, so its option hash is repeated here; it only generates source.
RAILS_OPTIONS = {
  generator: Temple::Generators::RailsOutputBuffer,
  use_html_safe: true,
  streaming: true,
  buffer_class: "ActionView::OutputBuffer",
  disable_capture: true
}.freeze
rails = ARGV.include?("--rails")
if rails
  # ActiveSupport isn't loaded; compiling only needs its answer for plain strings.
  class String
    def html_safe? = false
    def html_safe = self
  end
end

# Haml::Engine turns a template error into code that raises it when run, so the error is taken on its way there.
module CaptureTemplateError
  def runtime_error(error)
    $template_error ||= error
    super
  end
end
Haml::Compiler.prepend(CaptureTemplateError)

# Haml::Error#line is a 0-based index. The parser's errors also carry the 1-based line in their first backtrace
# entry, and an error built from that line by Parser#error_with_lineno has no backtrace and holds it directly.
def one_based_line(error)
  from_trace = error.backtrace&.first.to_s[/\A.*:(\d+)\z/, 1]
  return from_trace.to_i if from_trace

  error.backtrace ? error.line&.succ : error.line
end

def error(message, line = nil, haml_line = nil) = { error: { message:, line:, column: nil, haml_line: } }

template = $stdin.read.force_encoding(Encoding::UTF_8)
result =
  begin
    source = Haml::Engine.new(rails ? RAILS_OPTIONS : {}).call(template)
    if $template_error
      error($template_error.message, one_based_line($template_error), $template_error.line)
    else
      # Haml can generate broken Ruby from a template it accepts; checked inside a method, where templates may `yield`.
      RubyVM::InstructionSequence.compile("def template\n#{source}\nend")
      # Haml numbers its temporaries per process; the numbers aren't output.
      { source: source.gsub(/_(module|haml_compiler)\d+/, "_x") }
    end
  rescue Haml::Error => e
    error(e.message, e.line&.succ, e.line)
  rescue ::SyntaxError => e
    # "<compiled>:N: message", where N counts the `def template` line added above.
    _, line, message = e.message.lines.first.match(/\A<compiled>:(\d+): (.*)/).to_a
    { invalid_ruby: message || e.message.lines.first.chomp, line: line && line.to_i - 1 }
  rescue LoadError => e
    { dependency_missing: e.path || e.message[/-- (\S+)/, 1] || e.message }
  rescue StandardError => e
    error(e.message)
  end
puts JSON.generate(result)
