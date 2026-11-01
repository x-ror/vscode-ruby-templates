class UsersController < ApplicationController
  before_action :set_user, only: %i[show edit]

  def index
    @users = User.all
    @total = @users.count
  end

  def show
  end

  def new
    @user = User.new
    render :new
  end

  def edit
  end

  private

  def set_user
    @user = User.find(params[:id])
  end
end
