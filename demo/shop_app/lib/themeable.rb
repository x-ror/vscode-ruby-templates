module Themeable
  extend ActiveSupport::Concern

  included do
    prepend_view_path Rails.root.join("themes")
  end
end
