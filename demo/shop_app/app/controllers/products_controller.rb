class ProductsController < ApplicationController
  include Themeable

  def index
    @products = Product.order(:name)
  end
end
