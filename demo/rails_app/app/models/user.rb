class User < ApplicationRecord
  has_many :roles
  has_many :tags
  has_many :posts, class_name: "Blog::Post"
end
