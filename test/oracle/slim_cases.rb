# Usage: ruby slim_cases.rb <slim checkout>
# Runs the test methods of <checkout>/test/core/*.rb against a recording TestSlim and prints the templates they
# render as JSON: [{file, test, source, expected_error?, skipped?}].
require "json"
require "slim"
require "slim/erb_converter"

checkout = ARGV.fetch(0)
cases = []

module Kernel
  alias_method :require_without_helper, :require
  def require(name)
    name == "helper" ? false : require_without_helper(name)
  end
end

module Minitest
  class Test
    def initialize(name)
      @name = name
    end
  end
end
Env = Class.new

def deep_copy(value)
  case value
  when Hash then value.to_h { |key, item| [key, deep_copy(item)] }
  when Array then value.map { |item| deep_copy(item) }
  when String then value.dup
  else value
  end
end

# Every Hash behind the option maps of Slim's classes (Temple::OptionMap keeps them in @map, parents included).
def option_hashes
  maps = ObjectSpace.each_object(Class).select { |klass| klass.name.to_s.start_with?("Slim") && klass.respond_to?(:options) }
  maps.flat_map { |klass| klass.options.instance_variable_get(:@map) }.grep(Hash).uniq(&:object_id)
end

DEFAULT_OPTIONS = option_hashes.map { |hash| [hash, deep_copy(hash)] }.freeze

def restore_default_options
  DEFAULT_OPTIONS.each { |hash, copy| hash.replace(deep_copy(copy)) }
end

# with_options only sets thread-local options, which the option hashes don't show.
$in_with_options = false
[Slim::Engine, Slim::Embedded].each do |klass|
  klass.singleton_class.prepend(Module.new do
    def with_options(*args, &block)
      $in_with_options = true
      super
    end
  end)
end

def options_changed?
  $in_with_options || DEFAULT_OPTIONS.any? { |hash, copy| hash != copy }
end

# scope and locals are render-time only and file only names the template in messages; every other option reaches Slim::Engine.
RENDER_ONLY_OPTIONS = %i[scope locals file].freeze

class TestSlim < Minitest::Test
  class << self
    attr_accessor :recorder
  end

  def setup
    @env = Env.new
  end

  def render(source, options = {}, expected_error = nil)
    self.class.recorder.call(source, options, expected_error)
    ""
  end

  def with_html_safe = yield

  def assert_html(_expected, source, options = {}, &_block) = render(source, options)

  def assert_syntax_error(message, source, options = {}) = render(source, options, message)

  def assert_ruby_error(_error, _from, source, options = {}) = render(source, options)

  def assert_ruby_syntax_error(_from, source, options = {}) = render(source, options, :ruby_syntax_error)

  def assert_runtime_error(_message, source, options = {}) = render(source, options)

  # The block may render, so it runs; what it raises is what assert_raises expects.
  def assert_raises(*expected)
    yield
  rescue *(expected.grep(Module).empty? ? [StandardError] : expected.grep(Module))
    nil
  end

  def method_missing(name, *)
    return super unless stubbed?(name)

    yield if block_given? && !name.start_with?("skip")
  end

  def respond_to_missing?(name, include_private = false) = stubbed?(name) || super

  private

  def stubbed?(name) = name.start_with?("assert", "refute", "skip")
end

def skipped_case(file, test, reason) = { file: file, test: test, source: nil, skipped: reason }

def recorded_case(file, test, source, options, expected)
  entry = { file: file, test: test, source: source }
  if expected == :ruby_syntax_error
    entry[:skipped] = "expects a Ruby syntax error, outside Slim's parser"
  elsif expected
    entry[:expected_error] = expected
  end
  unless source.encoding == Encoding::UTF_8 && source.valid_encoding?
    entry[:source] = source.dup.force_encoding(Encoding::UTF_8).scrub
    entry[:skipped] = "source is not valid UTF-8"
  end
  options = options.except(*RENDER_ONLY_OPTIONS)
  if !options.empty? || options_changed?
    changed = options.keys.join(", ")
    entry[:skipped] = "non-default options: #{changed.empty? ? "changed Slim defaults" : changed}"
  end
  entry
end

# Runs one test method with Slim's defaults restored; returns why it couldn't run, or nil.
def run_test(klass, name)
  $in_with_options = false
  restore_default_options
  test = klass.new(name.to_s)
  test.setup
  test.public_send(name)
  nil
rescue LoadError => e
  "dependency missing (#{e.path || e.message[/-- (\S+)/, 1] || e.message})"
rescue StandardError, ScriptError => e
  "test raised #{e.class}"
ensure
  test.teardown if test.respond_to?(:teardown)
end

Dir[File.join(checkout, "test/core/test_*.rb")].sort.each do |path|
  file = File.basename(path)
  slim_tests_before = TestSlim.subclasses
  tests_before = Minitest::Test.subclasses
  begin
    load path
  rescue StandardError, LoadError, ScriptError => e
    cases << skipped_case(file, nil, "cannot load: #{e.class}")
    next
  end
  (TestSlim.subclasses - slim_tests_before).each do |klass|
    klass.public_instance_methods(false).grep(/\Atest_/).sort.each do |name|
      test = name.to_s
      klass.recorder = ->(source, options, expected) { cases << recorded_case(file, test, source, options, expected) }
      recorded = cases.size
      reason = run_test(klass, name) || ("no render calls" if cases.size == recorded)
      cases << skipped_case(file, test, reason) if reason
    end
  end
  (Minitest::Test.subclasses - TestSlim.subclasses - tests_before).each do |klass|
    cases << skipped_case(file, klass.name, "not a TestSlim test")
  end
end

puts JSON.generate(cases)
