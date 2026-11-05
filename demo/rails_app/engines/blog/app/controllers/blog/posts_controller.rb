module Blog
  class PostsController < ApplicationController
    def index
      @posts = Post.order(:title)
    end
  end
end
