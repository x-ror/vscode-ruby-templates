# Compiles the ERB template on stdin the way Rails 8.1.4 does and parses the Ruby it would eval with Prism 2.0.0.
# Usage: erb.rb [--file name.html.erb] [--normalize] [--ruby-version X.Y|latest]; the name picks the format and appears in Rails' messages.
# --ruby-version picks Prism's grammar (3.3, 3.4, 3.5, 4.0, 4.1 or latest); without it Prism uses the running Ruby's.
# --normalize compiles the template with the leading and trailing whitespace of its text lines removed (see below).
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
version_flag = ARGV.index("--ruby-version")
PRISM_OPTIONS = version_flag ? { version: ARGV[version_flag + 1] }.freeze : {}.freeze
format = File.basename(name, ".erb")[/\.([^.]+)\z/, 1]&.to_sym
# A name without a known format (`mailer/x.erb`) renders as html.
format = :html unless ActionView::Template::Types.symbols.include?(format)
# The source line of ActionView's method definition: the template's first line follows `@virtual_path = …;` on the next.
VIRTUAL_PATH_ASSIGNMENT = "@virtual_path = "

# ERB copies the template's text, indentation included, into the HTML, so re-indenting changes
# the rendered whitespace by design. What formatting may change is therefore compared with the leading and trailing
# whitespace of text lines removed (and that of lines inside a start tag, between attributes), except in `pre`,
# `textarea`, `script` and `style` and inside ERB tags, whose Ruby stays byte-identical. Erubi's own regexp finds the
# tags, so Rails' view of what is text and what is code is the one used; the whitespace Erubi trims around a `<% %>`
# line is text too.
#
# "Text" is what the HTML tokenizer calls data: not inside a start or end tag (attribute values keep their
# whitespace), a comment, a bogus comment, or the body of a `script`, `style` or `textarea` (raw text), nor inside a
# `pre` (nested ones counted). ERB tags are invisible to this scan, so an ERB tag inside an attribute value keeps the
# attribute's state.
class TextWhitespace
  RAW_TEXT_ELEMENTS = %w[script style textarea].freeze

  def initialize
    @mode = :data
    @pre_depth = 0
  end

  # The text without the leading whitespace of the lines that start in HTML text; `at_line_start` says whether the
  # text itself starts a line (it follows a newline, not an ERB tag).
  def strip(text, at_line_start)
    text.lines.each_with_index.map do |line, index|
      stripped = line
      stripped = stripped.sub(/\A[ \t]+/, "") if (index.positive? || at_line_start) && insignificant_whitespace?
      advance(line)
      stripped = stripped.sub(/[ \t]+(\r?\n)\z/, '\1') if insignificant_whitespace?
      stripped
    end.join
  end

  private

  # Text outside pre, and the space between attributes of a start tag; not inside a quoted value or a comment.
  def insignificant_whitespace?
    (@mode == :data && @pre_depth.zero?) || @mode == :tag
  end

  def advance(line)
    position = 0
    position = step(line, position) while position < line.size
  end

  # Consumes the next token of `line` from `position` and returns the position after it (the line's end if the state
  # continues on the next line).
  def step(line, position)
    case @mode
    when :data then data(line, position)
    when :comment then leave(line, position, /--!?>/, :data)
    when :bogus then leave(line, position, />/, :data)
    when :quote then leave(line, position, @quote, :tag)
    when :raw then leave(line, position, /<\/#{@raw}(?=[\s\/>])/i, :raw_end)
    else tag(line, position)
    end
  end

  def data(line, position)
    open = line.index("<", position) or return line.size
    rest = line[open..]
    if rest.start_with?("<!--")
      @mode = :comment
      open + 4
    elsif (name = rest.match(%r{\A<(/?)([A-Za-z][^\s/>]*)}))
      @closing = !name[1].empty?
      @name = name[2].downcase
      @mode = :tag
      @after_equals = false
      open + name[0].size
    elsif rest.match?(/\A<[!?]/)
      @mode = :bogus
      open + 2
    else
      open + 1
    end
  end

  def leave(line, position, terminator, next_mode)
    found = line.index(terminator, position)
    return line.size unless found

    length = terminator.is_a?(Regexp) ? line[found..].match(terminator)[0].size : terminator.size
    if next_mode == :raw_end
      @closing = true
      @name = @raw
      @mode = :tag
      @after_equals = false
    else
      @mode = next_mode
      @after_equals = false
    end
    found + length
  end

  def tag(line, position)
    char = line[position]
    case char
    when ">" then finish_tag
    when '"', "'"
      if @after_equals
        @quote = char
        @mode = :quote
      end
      @after_equals = false
    when /\s/ then nil
    else @after_equals = char == "="
    end
    position + 1
  end

  def finish_tag
    @mode = :data
    if @closing
      @pre_depth -= 1 if @name == "pre" && @pre_depth.positive?
    elsif @name == "pre"
      @pre_depth += 1
    elsif RAW_TEXT_ELEMENTS.include?(@name)
      @mode = :raw
      @raw = @name
    end
  end
end

def normalize_text_whitespace(source)
  text_whitespace = TextWhitespace.new
  at_line_start = true
  position = 0
  normalized = +""
  source.to_enum(:scan, Erubi::Engine::DEFAULT_REGEXP).map { Regexp.last_match }.each do |tag|
    normalized << text_whitespace.strip(source[position...tag.begin(0)], at_line_start)
    # The blanks up to the newline after `%>` are text as well.
    line_end = tag[4].to_s
    normalized << tag[0].delete_suffix(line_end)
    normalized << text_whitespace.strip(line_end, false)
    at_line_start = line_end.end_with?("\n")
    position = tag.end(0)
  end
  normalized << text_whitespace.strip(source[position..], at_line_start)
end

# Elements whose whitespace shows, built by Ruby: the normalization can't see into them, so the file isn't compared.
RUBY_BUILT_SENSITIVE_ELEMENT = /\b(?:content_tag|tag)\s*\(?\s*[:"'](?:pre|textarea|script|style)\b|\btag\.(?:pre|textarea|script|style)\b|\bjavascript_tag\b/
RAW_HTML_OUTPUT = /\braw\b|\bhtml_safe\b/
LITERAL_SENSITIVE_ELEMENT = /<(?:pre|textarea|script|style)\b/i

def ruby_built_sensitive_element?(source)
  source.to_enum(:scan, Erubi::Engine::DEFAULT_REGEXP).map { Regexp.last_match }.any? do |tag|
    indicator, code = tag[1], tag[2]
    next false if indicator == "#"

    code.match?(RUBY_BUILT_SENSITIVE_ELEMENT) ||
      ((indicator == "==" || code.match?(RAW_HTML_OUTPUT)) && code.match?(LITERAL_SENSITIVE_ELEMENT))
  end
end

def error(message, line = nil) = { message:, line:, column: nil }

# Prism's column is in the generated line, which doesn't map back to the template, so only lines are reported.
def prism_errors(compiled, template_lines, signature_line)
  code_line = compiled.lines.index { |line| line.include?(VIRTUAL_PATH_ASSIGNMENT) } + 1
  Prism.parse(compiled, **PRISM_OPTIONS).errors.filter_map do |e|
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
if ARGV.include?("--normalize")
  if ruby_built_sensitive_element?(source)
    puts JSON.generate(not_compared: "whitespace-sensitive element built by Ruby")
    exit
  end
  source = normalize_text_whitespace(source)
end
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
        # Ruby parses with its own grammar: a rejection Prism shares under that grammar is a version difference, and
        # for the requested version Prism's list is the answer.
        if Prism.parse(compiled, version: "current").errors.empty?
          found << error("Ruby rejected what Prism accepted: #{e.message.lines.first.to_s.chomp}")
        end
      end
    end
    found
  rescue StandardError => e
    [error(e.message.lines.first.to_s.chomp)]
  end
# The method name holds the process-wide object id, which differs on every run.
puts JSON.generate(errors.empty? ? { source: compiled.sub(template.send(:method_name), "_template") } : { errors: })
