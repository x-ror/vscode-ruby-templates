# Parses the Ruby on stdin with Prism 2.0.0, inside the method Rails compiles a template into, and prints its errors as
# JSON: [{message, line, column: null}], at the lines of the source on stdin.
# Haml 7.5.1 mis-lexes nested hashes with Prism 2.x, so haml.rb (on 1.9.0) runs this in a process of its own.
# --last-line N clamps the lines Haml's own last line adds to the template's; --ruby-version X.Y|latest picks the grammar.
require "json"
gem "prism", "2.0.0"
require "prism"

def option(name) = (ARGV[ARGV.index(name) + 1] if ARGV.include?(name))

WRAPPER = "def _template(local_assigns, output_buffer, &_)\n"
UNCLOSED_DEF = "expected an `end` to close the `def` statement"
last_line = Integer(option("--last-line"))
version = option("--ruby-version")
options = version ? { version: } : {}
errors = Prism.parse("#{WRAPPER}#{$stdin.read}\nend\n", **options).errors
# The wrapper's own errors are not the template's; `yield` is valid in the method.
errors = errors.reject { |e| e.type == :invalid_yield || e.location.start_line == 1 || e.message == UNCLOSED_DEF }
puts JSON.generate(errors.map { |e| { message: e.message, line: [e.location.start_line - 1, last_line].min, column: nil } })
