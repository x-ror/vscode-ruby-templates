//! Model attributes from `db/schema.rb`, for `f.text_field :` completion inside a `form_with model:` block.

use crate::rails::inflect::{pluralize, singularize, snake_case};
use crate::rails::paths::rails_root;
use crate::util::leading_ws;
use dashmap::DashMap;
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;

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
static COLUMN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*t\.(\w+)\s+"(\w+)"(.*)"#).unwrap());

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

/// A column line of `db/schema.rb`; `t.references "user"` is the column `user_id`.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub kind: String,
    /// What follows the name: `, null: false, default: "x"`.
    pub options: String,
    /// 0-based line in the schema.
    pub line: usize,
}

type SchemaEntry = (Option<SystemTime>, u64, Arc<HashMap<String, Vec<Column>>>);
static SCHEMA_CACHE: LazyLock<DashMap<PathBuf, SchemaEntry>> = LazyLock::new(DashMap::new);

fn schema_file(current: &Path) -> Option<PathBuf> {
    Some(rails_root(current)?.join("db/schema.rb"))
}

fn parse_all_tables(schema: &str) -> HashMap<String, Vec<Column>> {
    let mut tables: HashMap<String, Vec<Column>> = HashMap::new();
    let mut current: Option<String> = None;
    for (line, text) in schema.lines().enumerate() {
        if let Some(table) = &current {
            if text.trim() == "end" {
                current = None;
                continue;
            }
            let Some(cap) = COLUMN.captures(text) else { continue };
            let (kind, name, options) = (&cap[1], &cap[2], cap[3].to_string());
            let col = match kind {
                "references" | "belongs_to" => Column { name: format!("{name}_id"), kind: "references".into(), options, line },
                _ => Column { name: name.to_string(), kind: kind.to_string(), options, line },
            };
            tables.entry(table.clone()).or_default().push(col);
        } else if let Some(cap) = CREATE_TABLE.captures(text) {
            let name = cap[1].to_string();
            if !tables.contains_key(&name) {
                tables.insert(name.clone(), Vec::new());
                current = Some(name);
            }
        }
    }
    tables
}

fn cached_schema(current: &Path) -> Option<(PathBuf, Arc<HashMap<String, Vec<Column>>>)> {
    let file = schema_file(current)?;
    let meta = std::fs::metadata(&file).ok();
    let (mtime, len) = meta.map_or((None, 0), |m| (m.modified().ok(), m.len()));
    if let Some(entry) = SCHEMA_CACHE.get(&file) {
        if mtime.is_some() && entry.0 == mtime && entry.1 == len {
            return Some((file, Arc::clone(&entry.2)));
        }
    }
    let schema = std::fs::read_to_string(&file).ok()?;
    let tables = Arc::new(parse_all_tables(&schema));
    if mtime.is_some() {
        SCHEMA_CACHE.insert(file.clone(), (mtime, len, Arc::clone(&tables)));
    }
    Some((file, tables))
}

/// `(column, type)` of `table` in the app's `db/schema.rb`.
pub fn columns(current: &Path, table: &str) -> Vec<(String, String)> {
    let Some((_, tables)) = cached_schema(current) else { return vec![] };
    tables.get(table).into_iter().flatten().map(|c| (c.name.clone(), c.kind.clone())).collect()
}

/// The column `name` in the first of `tables` that has it, with the schema file and the table.
pub fn column(current: &Path, tables: &[String], name: &str) -> Option<(PathBuf, String, Column)> {
    let (file, parsed) = cached_schema(current)?;
    tables.iter().find_map(|table| {
        let column = parsed.get(table)?.iter().find(|c| c.name == name)?.clone();
        Some((file.clone(), table.clone(), column))
    })
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
        // Updating db/schema.rb invalidates the cached parsed tables.
        let updated =
            "ActiveRecord::Schema[7.1].define(version: 2) do\n  create_table \"users\" do |t|\n    t.string \"email\"\n  end\nend\n";
        std::fs::write(root.join("db/schema.rb"), updated).unwrap();
        let names_after: Vec<String> = columns(&view, "users").into_iter().map(|(n, _)| n).collect();
        assert_eq!(names_after, ["email"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
