module UsersHelper
  def format_price(amount, currency: "USD")
    number_to_currency(amount, unit: currency)
  end
end
