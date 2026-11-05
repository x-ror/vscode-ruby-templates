class ApplicationMailer < ActionMailer::Base
  before_action :set_company
  default from: "hello@example.com"
  layout "mailer"

  private

  def set_company
    @company = "Demo Inc."
  end
end
