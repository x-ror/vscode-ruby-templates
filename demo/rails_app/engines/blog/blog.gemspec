Gem::Specification.new do |spec|
  spec.name = "blog"
  spec.version = "0.1.0"
  spec.summary = "In-repo engine with the blog's views"
  spec.authors = ["Demo"]
  spec.files = Dir["app/**/*", "lib/**/*"]
  spec.add_dependency "rails", ">= 8.0"
end
