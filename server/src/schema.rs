//! Model attributes from `db/schema.rb`, for `f.text_field :` completion inside a `form_with model:` block.

use crate::partials::{app_root, pluralize, singularize};
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

// `f.text_field :na`, `form.label(:na`: the symbol being typed.
static ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:f|form|ff|builder|fields)\.\w+\s*\(?\s*:(\w*)$").unwrap());
// `form_with model: @user`, `form_with(model: [@post, comment])`, `form_for @user`: the (last) model's name.
static FORM_MODEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:form_with\s*\(?\s*model:|form_for\s*\(?)\s*\[?\s*(?:@?\w+\s*,\s*)*@?(\w+)").unwrap());
static CREATE_TABLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*create_table\s+"(\w+)""#).unwrap());
static COLUMN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*t\.(\w+)\s+"(\w+)""#).unwrap());

/// The attribute name typed after a form builder field method's `:`.
pub fn attribute_prefix(before: &str) -> Option<&str> {
    Some(ATTRIBUTE.captures(before)?.get(1)?.as_str())
}

/// The table of the innermost `form_with model:` / `form_for` opened in `text_before`, by Rails' naming.
pub fn form_table(text_before: &str) -> Option<String> {
    let model = FORM_MODEL.captures_iter(text_before).last()?.get(1)?.as_str().to_string();
    Some(pluralize(&singularize(&model)))
}

/// `(column, type)` of `table` in the app's `db/schema.rb`; `t.references "user"` gives `user_id`.
pub fn columns(current: &Path, table: &str) -> Vec<(String, String)> {
    let Some(schema) = app_root(current).and_then(|app| std::fs::read_to_string(app.parent()?.join("db/schema.rb")).ok()) else {
        return vec![];
    };
    let mut lines = schema.lines().skip_while(|l| CREATE_TABLE.captures(l).is_none_or(|c| &c[1] != table)).skip(1);
    let mut out = Vec::new();
    for line in lines.by_ref() {
        if line.trim() == "end" {
            break;
        }
        let Some(cap) = COLUMN.captures(line) else { continue };
        let (kind, name) = (&cap[1], &cap[2]);
        match kind {
            "references" | "belongs_to" => out.push((format!("{name}_id"), "references".to_string())),
            _ => out.push((name.to_string(), kind.to_string())),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_context() {
        assert_eq!(attribute_prefix("<%= f.text_field :na"), Some("na"));
        assert_eq!(attribute_prefix("= form.label(:"), Some(""));
        assert_eq!(attribute_prefix("<%= f.text_field :name, class: :a"), None);
        assert_eq!(attribute_prefix("<%= link_to :x"), None);
        assert_eq!(form_table("<%= form_with model: @user do |f| %>").as_deref(), Some("users"));
        assert_eq!(form_table("= form_with(model: [@post, comment]) do |f|").as_deref(), Some("comments"));
        assert_eq!(form_table("form_for @person do |f|").as_deref(), Some("people"));
        assert_eq!(form_table("<%= form_with url: x do |f| %>"), None);
    }

    #[test]
    fn schema_columns() {
        let root = std::env::temp_dir().join(format!("rt-schema-{}", std::process::id()));
        std::fs::create_dir_all(root.join("db")).unwrap();
        std::fs::create_dir_all(root.join("app/views/users")).unwrap();
        let schema = concat!(
            "ActiveRecord::Schema[7.1].define(version: 1) do\n",
            "  create_table \"posts\", force: :cascade do |t|\n",
            "    t.string \"title\"\n",
            "  end\n\n",
            "  create_table \"users\", force: :cascade do |t|\n",
            "    t.string \"name\", null: false\n",
            "    t.references \"team\", null: false\n",
            "    t.datetime \"created_at\", null: false\n",
            "    t.index [\"name\"], name: \"index_users_on_name\"\n",
            "  end\n",
            "end\n",
        );
        std::fs::write(root.join("db/schema.rb"), schema).unwrap();
        let view = root.join("app/views/users/_form.html.erb");
        let names: Vec<String> = columns(&view, "users").into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["name", "team_id", "created_at"]);
        assert!(columns(&view, "nope").is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
