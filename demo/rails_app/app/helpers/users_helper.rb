module UsersHelper
  # Formats an amount in the given currency, e.g. "$42.00".
  def format_price(amount, currency: "USD")
    number_to_currency(amount, unit: currency)
  end

  # The name with the nickname after it, when the user has one.
  def user_label(user,
                 separator: " / ")
    [user.name, user.nickname].compact_blank.join(separator)
  end
end
