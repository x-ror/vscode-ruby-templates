class CardComponentPreview < ViewComponent::Preview
  def default
    render CardComponent.new(title: "Preview")
  end
end
