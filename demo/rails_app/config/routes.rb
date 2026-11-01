Rails.application.routes.draw do
  root "users#index"

  resources :users

  namespace :admin do
    resources :reports, only: :index
  end
end
