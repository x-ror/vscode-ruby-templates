module Admin
  class ReportsController < BaseController
    def index
      @reports = Report.order(:title)
    end
  end
end
