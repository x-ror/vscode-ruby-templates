class CardComponent < ViewComponent::Base
  renders_one :header
  renders_many :items

  def initialize(title:)
    @title = title
  end
end
