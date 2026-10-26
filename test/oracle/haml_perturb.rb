# Reads a Haml template on stdin and prints, as JSON, what a perturbation must not rescale:
# {"unit": the indentation string Haml found, "flat": {line index => length of the structural prefix}}.
# Lines in a filter or `-#` comment body belong to `flat`: the whitespace past the prefix is content.
require "json"
gem "prism", "1.9.0" # see haml.rb
require "haml"

source = $stdin.read.force_encoding(Encoding::UTF_8)
lines = source.lines.map(&:chomp)
parser = Haml::Parser.new({})
root = parser.call(source)
unit = parser.instance_variable_get(:@indentation) || ""

def openers(node, found = [])
  node.children.each do |child|
    found << child.line - 1 if %i[filter haml_comment].include?(child.type)
    openers(child, found)
  end
  found
end

indent = ->(line) { line[/\A[ \t]*/] }
flat = {}
openers(root).each do |index|
  prefix = indent.call(lines[index]).size + unit.size
  # The body runs while lines are blank or indented deeper than the opener, as Parser#closes_flat? decides.
  lines[(index + 1)..].each_with_index do |line, offset|
    break unless line.strip.empty? || indent.call(line).size >= prefix

    flat[index + 1 + offset] = prefix
  end
end
puts JSON.generate(unit: unit, flat: flat)
