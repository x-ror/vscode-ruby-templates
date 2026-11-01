module Admin
  class BaseController < ApplicationController
    layout "admin"
    before_action :require_admin

    private

    def require_admin
      @admin_section = true
    end
  end
end
