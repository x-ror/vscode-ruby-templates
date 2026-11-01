ActiveRecord::Schema[8.0].define(version: 2026_11_01_000000) do
  create_table "users", force: :cascade do |t|
    t.string "name"
    t.string "nickname"
    t.string "email"
    t.boolean "admin", default: false
    t.datetime "created_at", null: false
  end

  create_table "reports", force: :cascade do |t|
    t.string "title"
    t.text "body"
  end
end
