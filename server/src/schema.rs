//! Model attributes from `db/schema.rb`, for `f.text_field :` completion inside a `form_with model:` block.

use crate::partials::{pluralize, singularize, snake_case};
use crate::rails_paths::app_root;
use crate::util::leading_ws;
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

// `f.text_field :na`, `form.label(:na`: the symbol being typed.
static ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:f|form|ff|builder|fields)\.\w+\s*\(?\s*:(\w*)$").unwrap());
// `form_with model: @user`, `form_with(model: [:admin, @post, comment])`, `form_for User.new`, `f.fields_for :address`:
// the (last) model expression.
static FORM_MODEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:form_with\s*\(?\s*model:|form_for\s*\(?|fields_for\s*\(?)\s*\[?\s*(?::?@?\w+\s*,\s*)*:?@?([\w.]+)").unwrap()
});
/// Calls that build or fetch a record of the collection before them (`@post.comments.build`).
const RECORD_CALLS: &[&str] = &["new", "build", "create", "first", "last", "find"];
static CREATE_TABLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*create_table\s+"(\w+)""#).unwrap());
static COLUMN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*t\.(\w+)\s+"(\w+)""#).unwrap());

/// The attribute name typed after a form builder field method's `:`.
pub fn attribute_prefix(before: &str) -> Option<&str> {
    Some(ATTRIBUTE.captures(before)?.get(1)?.as_str())
}

/// The table of the innermost form block in `text_before` that the cursor (at its end) is still inside, by
/// Rails' naming. Inside means every line after the form's is indented deeper, which holds in all three languages:
/// ERB's `<% end %>` and a Haml/Slim dedent both come back to the form's level.
pub fn form_table(text_before: &str) -> Option<String> {
    let enclosing = |at: usize| {
        let line_start = text_before[..at].rfind('\n').map_or(0, |i| i + 1);
        let indent = leading_ws(&text_before[line_start..]).len();
        let rest = text_before[at..].split_once('\n').map_or("", |(_, rest)| rest);
        rest.split('\n').filter(|l| !l.trim().is_empty()).all(|l| leading_ws(l).len() > indent)
    };
    let cap = FORM_MODEL.captures_iter(text_before).filter(|c| c.get(0).is_some_and(|m| enclosing(m.start()))).last()?;
    let model = cap.get(1)?.as_str().split('.').rfind(|part| !RECORD_CALLS.contains(part))?;
    Some(pluralize(&singularize(&snake_case(model))))
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
        assert_eq!(form_table("<%= form_with model: [:admin, @user] do |f| %>").as_deref(), Some("users"));
        assert_eq!(form_table("<%= form_with model: User.new do |f| %>").as_deref(), Some("users"));
        assert_eq!(form_table("<%= form_with model: @post.comments.build do |f| %>").as_deref(), Some("comments"));
        let nested = "<%= form_with model: @user do |f| %>\n  <%= f.fields_for :address do |ff| %>\n    ";
        assert_eq!(form_table(nested).as_deref(), Some("addresses"));
        let closed = "<%= form_with model: @user do |f| %>\n  x\n<% end %>\n<%= form_with url: search_path do |f| %>\n  ";
        assert_eq!(form_table(closed), None);
        let haml = "= form_with model: @user do |f|\n  = f.text_field :name\n= form_with model: @post do |f|\n  ";
        assert_eq!(form_table(haml).as_deref(), Some("posts"));
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
