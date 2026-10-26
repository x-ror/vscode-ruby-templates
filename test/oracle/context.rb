# Renders one template of an app the way Rails does and prints the error it raises, if any.
# Usage: context.rb <actionview version> <app dir> <template, e.g. users/case_1>
# Prints {"error": {message, line}} (message: first line, app dir removed; line: the template's line the render is on) or {"rendered": true}.
require "json"
version, app, template = ARGV
gem "minitest", "< 6" # minitest 6 wants prism ~> 1.5, which an ActionView install doesn't pin
gem "actionview", version
require "logger"
require "uri" # Rails 7.x needs it before action_view
require "action_view"
require "action_view/template/error" # defines StrictLocalsError, which Rails loads with its railtie
require "active_support/core_ext/array/conversions"
require "active_support/core_ext/string/inflections"

abort "actionview #{ActionView::VERSION::STRING} loaded, #{version} asked" unless ActionView::VERSION::STRING == version

views = File.join(app, "app/views")
lookup = ActionView::LookupContext.new([views])
view = ActionView::Base.with_empty_template_cache.new(lookup, {}, nil)

result =
  begin
    view.render(template: template)
    { rendered: true }
  rescue StandardError => e
    cause = e.is_a?(ActionView::Template::Error) ? (e.cause || e) : e
    path = File.join(views, "#{template}.html.erb")
    line = ([e, cause].flat_map { |x| x.backtrace.to_a }.find { |entry| entry.start_with?("#{path}:") } || "")[/:(\d+)/, 1]
    message = cause.message.lines.first.to_s.chomp.gsub("#{app}/", "")
    { error: { message: message, line: line&.to_i, class: cause.class.name } }
  end
puts JSON.generate(result)
