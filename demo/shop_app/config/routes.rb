Rails.application.routes.draw do
  root "products#index"

  resources :products, only: %i[index show]
  get "about", to: "pages#index"
end
