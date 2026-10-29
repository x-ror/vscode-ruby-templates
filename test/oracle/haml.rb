# Compiles the Haml template on stdin with Haml 7.5.1's defaults and prints the outcome as JSON.
# --rails compiles with the options of Haml::RailsTemplate (lib/haml/rails_template.rb), for comparing the code
# before and after formatting.
# Without it, the Ruby Haml generates as Rails compiles it is also parsed with Prism 2.0.0 (prism_errors.rb, in a process
# of its own, since this one runs on Prism 1.9.0): its errors come back as {"errors": [{message, line, column: null}]}.
# --ruby-version X.Y|latest picks Prism's grammar (3.3, 3.4, 3.5, 4.0, 4.1 or latest); without it Prism uses the running Ruby's.
require "json"
require "open3"
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
version_flag = ARGV.index("--ruby-version")
PRISM_ERRORS = File.join(__dir__, "prism_errors.rb")
# ActiveSupport isn't loaded; compiling only needs its answer for plain strings.
class String
  def html_safe? = false
  def html_safe = self
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

# Prism 2.0.0's errors in the Ruby Rails would compile for `template`, at the template's lines.
def ruby_errors(template, version_flag)
  source = Haml::Engine.new(RAILS_OPTIONS).call(template)
  last_line = [template.lines.size, 1].max
  arguments = ["--last-line", last_line.to_s, *(version_flag ? ["--ruby-version", ARGV[version_flag + 1]] : [])]
  output, status = Open3.capture2("ruby", PRISM_ERRORS, *arguments, stdin_data: source)
  raise "prism_errors.rb failed" unless status.success?

  JSON.parse(output, symbolize_names: true)
end

def error(message, line = nil, haml_line = nil) = { error: { message:, line:, column: nil, haml_line: } }

template = $stdin.read.force_encoding(Encoding::UTF_8)
result =
  begin
    source = Haml::Engine.new(rails ? RAILS_OPTIONS : {}).call(template)
    if $template_error
      error($template_error.message, one_based_line($template_error), $template_error.line)
    elsif rails
      # Haml can generate broken Ruby from a template it accepts; checked inside a method, where templates may `yield`.
      RubyVM::InstructionSequence.compile("def template\n#{source}\nend")
      # Haml numbers its temporaries per process; the numbers aren't output.
      { source: source.gsub(/_(module|haml_compiler)\d+/, "_x") }
    else
      errors = ruby_errors(template, version_flag)
      errors.empty? ? { source: source.gsub(/_(module|haml_compiler)\d+/, "_x") } : { errors: }
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
