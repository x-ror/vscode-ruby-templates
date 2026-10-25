# Compiles the ERB template on stdin the way Rails 8.1.4 does and parses the Ruby it would eval with Prism 2.0.0.
# Usage: erb.rb [--file name.html.erb]; the name picks the format and appears in Rails' messages.
# Prints {"errors": [{message, line, column}]} for every error, else {"source": the method source Rails evals}.
require "json"
gem "minitest", "< 6" # minitest 6 (an ActiveSupport dependency) wants prism ~> 1.5
gem "prism", "2.0.0"
gem "actionview", "8.1.4"
gem "erubi", "1.13.1"
require "prism"
require "action_view"
# Rails loads these; Template#compile's messages use them.
require "active_support/core_ext/array/conversions"
require "active_support/core_ext/string/inflections"

file_flag = ARGV.index("--file")
name = file_flag ? ARGV[file_flag + 1] : "template.html.erb"
format = File.basename(name, ".erb")[/\.([^.]+)\z/, 1]&.to_sym
# A name without a known format (`mailer/x.erb`) renders as html.
format = :html unless ActionView::Template::Types.symbols.include?(format)
# The source line of ActionView's method definition: the template's first line follows `@virtual_path = …;` on the next.
VIRTUAL_PATH_ASSIGNMENT = "@virtual_path = "

def error(message, line = nil) = { message:, line:, column: nil }

# Prism's column is in the generated line, which doesn't map back to the template, so only lines are reported.
def prism_errors(compiled, template_lines, signature_line)
  code_line = compiled.lines.index { |line| line.include?(VIRTUAL_PATH_ASSIGNMENT) } + 1
  Prism.parse(compiled).errors.filter_map do |e|
    line = e.location.start_line
    if line < code_line
      # The `def` line: its own "unclosed def" is the wrapper's, but a bad `locals:` signature is the template's.
      next if e.type == :def_term || signature_line.nil?

      next error(e.message, signature_line)
    end
    # Past the template are the lines Erubi and ActionView add after it (the buffer and the method's `end`).
    error(e.message, [line - code_line + 1, template_lines].min)
  end
end

source = $stdin.read.force_encoding(Encoding::UTF_8)
comment = source.match(ActionView::Template::STRICT_LOCALS_REGEX)
signature_line = comment && source[0...comment.begin(0)].count("\n") + 1
template_lines = [source.lines.size, 1].max
errors =
  begin
    template = ActionView::Template.new(source.dup, name, ActionView::Template::Handlers::ERB.new, locals: [], format:, virtual_path: "template")
    compiled = template.send(:compiled_source)
    found = prism_errors(compiled, template_lines, signature_line)
    if found.empty?
      begin
        # Rails' own compile also rejects strict locals that aren't keywords (ArgumentError), after the syntax check.
        template.send(:compile, Module.new)
      rescue ArgumentError => e
        found << error(e.message, signature_line)
      rescue ::SyntaxError, ActionView::Template::Error => e
        found << error("Ruby rejected what Prism accepted: #{e.message.lines.first.to_s.chomp}")
      end
    end
    found
  rescue StandardError => e
    [error(e.message.lines.first.to_s.chomp)]
  end
puts JSON.generate(errors.empty? ? { source: compiled } : { errors: })
