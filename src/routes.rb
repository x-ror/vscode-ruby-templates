# Routes for the extension's route helpers: `bin/rails runner routes.rb` prints a marker line, then JSON.
# The marker separates the JSON from whatever the app's initializers print.
require "json"

routes = Rails.application.routes.routes.reject(&:internal).map do |route|
  {
    name: route.name.to_s,
    verb: route.verb.to_s,
    path: route.path.spec.to_s,
    action: route.requirements.values_at(:controller, :action).compact.join("#"),
    source: (route.source_location if route.respond_to?(:source_location))
  }
end
puts "", "__RUBY_TEMPLATES_ROUTES__", JSON.generate(routes)
