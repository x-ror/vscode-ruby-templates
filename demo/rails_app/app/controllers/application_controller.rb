class ApplicationController < ActionController::Base
  before_action :set_current_user
  helper_method :current_user_name

  private

  def set_current_user
    @current_user = User.first
  end

  def current_user_name
    @current_user&.name
  end
end
