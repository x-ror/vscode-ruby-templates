# Usage: ruby haml_cases.rb <haml checkout>
# Replays the test files of <checkout>/test/haml against a recording stand-in for test_helper.rb and prints the
# templates Haml::Engine compiles for them, plus haml-spec/tests.yml and templates/*.haml, as JSON:
# [{file, test, source, expected_error?, skipped?}].
require "json"
require "yaml"
gem "prism", "1.9.0" # see haml.rb
require "haml"

checkout = File.realpath(ARGV.fetch(0))
test_dir = File.join(checkout, "test")
cases = []

# The real test_helper.rb needs ActionView, Rails and minitest; its stand-in is defined below.
$LOADED_FEATURES << File.join(test_dir, "test_helper.rb")
$LOAD_PATH.unshift(test_dir)

# Options that change the generated code or the messages' file name, not what the parser accepts.
RENDER_ONLY_OPTIONS = %i[escape_html escape_attrs filename file].freeze
DEFAULT_OPTIONS = Haml::Engine.new.options.to_hash.freeze

# Messages built by Haml::Error.message while the test files load: the expected messages of EXCEPTION_MAP.
$template_messages = []
Haml::Error.singleton_class.prepend(Module.new do
  def message(*)
    super.tap { |text| $template_messages << text }
  end
end)

# Held strongly: Class#subclasses is weak, and a collected class takes its tests with it.
$test_classes = []
$recorder = nil
$compiling = false
record_compiles = Module.new do
  def call(template)
    return super if $compiling || !$recorder

    $compiling = true
    begin
      $recorder.call(template, respond_to?(:options) ? options.to_hash : {})
      super
    ensure
      $compiling = false
    end
  end
end
# Temple::Engine redefines `prepend` for its own DSL.
Module.instance_method(:prepend).bind_call(Haml::Engine, record_compiles)
# The tests also run the parser alone, which raises where the engine would emit code that raises.
Haml::Parser.prepend(record_compiles)

class String
  def unindent
    margin = scan(/^[ \t]*(?=\S)/).min_by(&:size)&.size || 0
    gsub(/^[ \t]{0,#{margin}}/, "")
  end
end

class OfficialSkip < StandardError; end
class AssertionFailed < StandardError; end

module RenderHelper
  def assert_render(_expected, haml, options = {}) = render_haml(haml, options)

  def render_haml(haml, options = {})
    options = options.dup
    locals = options.delete(:locals) || {}
    Haml::Template.new(options) { haml }.render(Object.new, locals)
  end

  def assert_haml(haml, options = {}) = render_haml(haml, options)

  def render_hash(hash) = Haml::Util.escape_html(hash)
end

module Haml
  class TestCase
    include RenderHelper

    class << self
      def test(name, &block) = define_method("test_ #{name}", &block)

      def describe(_name = nil, &block)
        Class.new(self).tap { |klass| klass.class_eval(&block) }
      end

      def inherited(subclass)
        super
        $test_classes << subclass
      end

      def it(name = nil, &block) = define_method("test_#{instance_methods.size}_#{name}", &block)

      def error(*args) = Haml::Error.message(*args)
    end

    def initialize(name)
      @name = name
    end

    def render(text, options = {}, base = nil, &_block)
      options = { escape_html: false }.merge(options)
      %i[attr_wrapper cdata suppress_eval ugly].each { |option| options.delete(option) }
      scope = base ? base.instance_eval { binding } : nil
      eval Haml::Engine.new(options).call(text), scope
    end

    def render_template(text, options = {})
      scope = options.delete(:scope) || Object.new
      locals = options.delete(:locals) || {}
      Haml::Template.new({ escape_html: true }.merge(options)) { text }.render(scope, locals)
    end
    alias_method :assert_haml_ugly, :render_template

    def silence_warnings(&block) = Haml::Util.silence_warnings(&block)

    def skip(*) = raise(OfficialSkip)

    def assert_raises(*classes)
      yield
      raise AssertionFailed, "expected #{classes.join(", ")}"
    rescue *classes => e
      e
    end

    def assert_raises_message(klass, _message, &block) = assert_raises(klass, &block)

    def method_missing(name, *) = name.to_s.start_with?("assert", "refute", "flunk") ? nil : super

    def respond_to_missing?(name, include_private = false) = name.to_s.start_with?("assert", "refute", "flunk") || super

    def self.error_message(*args) = Haml::Error.message(*args)
  end
end

module Kernel
  def describe(name = nil, &block) = Haml::TestCase.describe(name, &block)
end

# The expected message and line of each erroneous document in EngineTest::EXCEPTION_MAP, for Haml's own errors.
def expected_errors
  return {} unless defined?(EngineTest)

  EngineTest::EXCEPTION_MAP.filter_map do |source, value|
    message, line = Array(value)
    next unless message.is_a?(String) && $template_messages.include?(message)

    [source, { message: message, line: line || source.split("\n").length }]
  end.to_h
end

# The documents of EngineTest::EXCEPTION_MAP that Haml compiles to Ruby that doesn't parse.
def ruby_syntax_error_sources
  defined?(EngineTest) ? EngineTest::EXCEPTION_MAP.select { |_, value| value == :compile }.keys : []
end

def entry(file, test, source, expected = nil, options = {})
  item = { file: file, test: test, source: source }
  item[:skipped] = "expects a Ruby syntax error, outside Haml's parser" if ruby_syntax_error_sources.include?(source)
  item[:expected_error] = expected if expected
  unless source.encoding == Encoding::UTF_8 && source.valid_encoding?
    item[:source] = source.dup.force_encoding(Encoding::UTF_8).scrub
    item[:skipped] = "source is not valid UTF-8"
  end
  changed = options.reject { |key, value| RENDER_ONLY_OPTIONS.include?(key) || DEFAULT_OPTIONS[key] == value }
  item[:skipped] = "non-default options: #{changed.keys.join(", ")}" unless changed.empty?
  item
end

# Runs one test method; returns why it failed, or nil.
def run_test(klass, name)
  test = klass.new(name.to_s)
  test.setup if test.respond_to?(:setup)
  test.public_send(name)
  nil
rescue OfficialSkip
  "official test skipped"
rescue LoadError => e
  "dependency missing (#{e.path || e.message[/-- (\S+)/, 1] || e.message})"
rescue AssertionFailed
  "test failed"
rescue StandardError, ScriptError
  "test raised"
ensure
  begin
    test.teardown if test.respond_to?(:teardown)
  rescue StandardError
    nil
  end
end

test_files = Dir[File.join(test_dir, "haml/{,engine/}*_test.rb")].sort
test_files.each do |path|
  file = path.delete_prefix("#{test_dir}/")
  before = $test_classes.size
  begin
    load path
  rescue StandardError, LoadError, ScriptError => e
    cases << { file: file, test: nil, source: nil, skipped: "cannot load: #{e.class}" }
    next
  end
  expected = expected_errors
  $test_classes[before..].each do |klass|
    klass.public_instance_methods(false).grep(/\Atest_/).sort.each do |name|
      test = name.to_s
      recorded = cases.size
      $recorder = ->(source, options) { cases << entry(file, test, source, expected[source], options) }
      reason = run_test(klass, name)
      $recorder = nil
      # A test that raised after compiling still contributed its cases; only a test that gave none is a skip.
      cases << { file: file, test: test, source: nil, skipped: reason || "no compile calls" } if cases.size == recorded
    end
  end
end

spec = YAML.safe_load_file(File.join(test_dir, "haml/haml-spec/tests.yml"))
spec.each do |group, tests|
  tests.each do |name, test|
    options = (test["config"] || {}).transform_keys(&:to_sym)
    cases << entry("haml-spec/tests.yml", "#{group}/#{name}", test["haml"], nil, options)
  end
end

Dir[File.join(test_dir, "haml/templates/*.haml")].sort.each do |path|
  cases << entry("templates/#{File.basename(path)}", nil, File.read(path, encoding: Encoding::UTF_8))
end

puts JSON.generate(cases)
