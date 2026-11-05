Rails.application.routes.draw do
  root "users#index"

  resources :users do
    get :engines, on: :member
    get :formats, on: :member
  end

  namespace :admin do
    resources :reports, only: :index
  end
end
