//! The model column behind `record.column` in a view, when the record is simple to infer: an `@ivar` the controller
//! assigns from `Model.find(…)`, or the block variable of `@relation.each do |record|` where `@relation` is
//! `Model.where(…)`. Anything less plain answers nothing.

use crate::inflect::{pluralize, snake_case};
use crate::rails::ivars;
use crate::rails::ivars::Assignment;
use crate::rails::paths;
use crate::rails::paths::app_root;
use crate::rails::project::Project;
use crate::rails::schema;
use crate::rails::schema::Column;
use crate::util;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

// `report.title`, `@report.title`: the receiver and the member.
static MEMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(@?[a-z_]\w*)\.([a-z_]\w*)").unwrap());
static ASSIGNED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^@(\w+)\s*(?:\|\|)?=\s*(.+)$").unwrap());
static ARGUMENTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\([^()]*\)").unwrap());
static MODEL_CHAIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^((?:::)?[A-Z]\w*(?:::[A-Z]\w*)*)((?:\.[a-z_]\w*[?!]?)+)$").unwrap());
// What precedes the first variable of `@reports.each do |report|` or `@reports.map { |report, i|`: the collection and the iterator.
static BLOCK_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(@[a-z_]\w*)\.([a-z_]\w*[?!]?)\s*(?:do|\{)\s*\|\s*$").unwrap());
static TABLE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^\s*self\.table_name\s*=\s*["':]?(\w+)"#).unwrap());

/// Class-level calls that return one record.
const RECORD_CALLS: &[&str] = &[
    "find",
    "find_by",
    "find_by!",
    "new",
    "build",
    "create",
    "create!",
    "first",
    "first!",
    "last",
    "last!",
    "take",
    "take!",
    "sole",
    "find_or_create_by",
    "find_or_initialize_by",
];
/// Calls that return an array instead of one record when given a count or a list.
const LIST_CALLS: &[&str] = &["first", "first!", "last", "last!", "take", "take!"];
/// Calls that return a relation, on the model or on another relation.
const RELATION_CALLS: &[&str] = &[
    "all",
    "where",
    "order",
    "reorder",
    "includes",
    "preload",
    "eager_load",
    "joins",
    "left_joins",
    "limit",
    "offset",
    "select",
    "distinct",
    "group",
    "having",
    "references",
    "unscoped",
    "none",
    "reverse_order",
    "page",
    "per",
];
/// Relation methods that yield each record as the block's first variable.
const ITERATORS: &[&str] = &[
    "each",
    "each_with_index",
    "map",
    "collect",
    "flat_map",
    "filter_map",
    "select",
    "filter",
    "reject",
    "find",
    "detect",
    "find_each",
    "sort_by",
    "group_by",
    "partition",
    "min_by",
    "max_by",
    "sum",
    "count",
    "any?",
    "all?",
    "none?",
];

#[derive(Debug, Clone, PartialEq)]
enum Source {
    Record(String),
    Relation(String),
}

/// A column of the model behind a `record.column` member in a view.
#[derive(Debug)]
pub struct ModelColumn {
    pub table: String,
    pub schema: PathBuf,
    pub column: Column,
}

impl ModelColumn {
    pub fn hover(&self) -> String {
        let Column { name, kind, options, .. } = &self.column;
        let options = options.trim_start_matches([',', ' ']);
        let details = if options.is_empty() { kind.clone() } else { format!("{kind}, {options}") };
        format!("`{}.{name}` — {details}", self.table)
    }
}

/// The column under `col` in `line` (starting at `line_start` of `text`) when it is `receiver.column` of a record
/// inferred from the controller. `declaration` gives the offset where the template declares the block variable
/// at an offset.
pub fn column_at(
    project: &Project,
    view: &Path,
    text: &str,
    line_start: usize,
    col: usize,
    declaration: impl FnOnce(usize) -> Option<usize>,
) -> Option<ModelColumn> {
    let line = util::lines_with_offsets(&text[line_start..]).next().map_or("", |(_, line)| line);
    let (receiver_at, receiver, member) = member_at(line, col)?;
    let source = if let Some(ivar) = receiver.strip_prefix('@') {
        (ivar, true)
    } else {
        let before = &text[..line_start + receiver_at];
        if !before.contains(&format!("|{receiver}")) && !before.contains(&format!("| {receiver}")) {
            return None;
        }
        let declared = declaration(line_start + receiver_at)?;
        (block_collection(text, declared, receiver)?, false)
    };
    let assignments = ivars::for_view(project, view);
    let model = match (ivar_source(&assignments, source.0)?, source.1) {
        (Source::Record(model), true) | (Source::Relation(model), false) => model,
        _ => return None,
    };
    let tables = tables(view, &model);
    let (schema, table, column) = schema::column(view, &tables, member)?;
    Some(ModelColumn { table, schema, column })
}

/// The offset in `line`, receiver and member of the `receiver.member` the cursor is on.
fn member_at(line: &str, col: usize) -> Option<(usize, &str, &str)> {
    MEMBER.captures_iter(line).find_map(|cap| {
        let (receiver, member) = (cap.get(1)?, cap.get(2)?);
        let before = line[..receiver.start()].chars().next_back();
        let after = line[member.end()..].chars().next();
        let inside = (member.start()..=member.end()).contains(&col);
        let plain_receiver = !before.is_some_and(|b| b.is_alphanumeric() || matches!(b, '_' | '.' | '@' | ':'));
        let plain_member = !matches!(after, Some('(' | '?' | '!'));
        (inside && plain_receiver && plain_member).then_some((receiver.start(), receiver.as_str(), member.as_str()))
    })
}

/// What every assignment of `@name` in the controller makes it, when they all agree.
fn ivar_source(assignments: &[Assignment], name: &str) -> Option<Source> {
    let mut sources = assignments.iter().filter(|a| a.name == name).map(|a| source_of(&a.code, name));
    let first = sources.next()??;
    sources.all(|source| source.as_ref() == Some(&first)).then_some(first)
}

/// `Report.find(id)` is a record, `Report.where(…).order(:title)` a relation; a call to anything else is neither.
fn source_of(code: &str, name: &str) -> Option<Source> {
    let value = ASSIGNED.captures(code).filter(|cap| &cap[1] == name)?.get(2)?.as_str().trim_end_matches(';').trim();
    let mut plain = value.to_string();
    while ARGUMENTS.is_match(&plain) {
        plain = ARGUMENTS.replace_all(&plain, "").into_owned();
    }
    let chain = MODEL_CHAIN.captures(&plain)?;
    let model = chain[1].trim_start_matches("::").to_string();
    let calls: Vec<&str> = chain[2].split('.').skip(1).collect();
    let (last, through) = calls.split_last()?;
    if !through.iter().all(|call| RELATION_CALLS.contains(call)) {
        return None;
    }
    if RELATION_CALLS.contains(last) {
        return Some(Source::Relation(model));
    }
    let args = trailing_arguments(value);
    let returns_array = match *last {
        _ if LIST_CALLS.contains(last) => !args.is_empty(),
        "find" => args.contains(',') || args.starts_with('['),
        _ => args.starts_with('['),
    };
    (RECORD_CALLS.contains(last) && !returns_array).then_some(Source::Record(model))
}

/// The text inside the parentheses that end `call`, trimmed; empty when it has none.
fn trailing_arguments(call: &str) -> &str {
    let Some(inner) = call.strip_suffix(')') else { return "" };
    let mut depth = 0;
    for (i, ch) in inner.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' if depth == 0 => return inner[i + 1..].trim(),
            '(' => depth -= 1,
            _ => {}
        }
    }
    ""
}

/// The `@collection` of the block whose first variable `variable` is declared at `declared`, when the text right
/// before it is `@collection.each do |`.
fn block_collection<'a>(text: &'a str, declared: usize, variable: &str) -> Option<&'a str> {
    let before = &text[..declared];
    let head = BLOCK_HEAD.captures(before.rsplit('\n').next()?)?;
    let is_variable =
        text[declared..].strip_prefix(variable).is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_'));
    (is_variable && ITERATORS.contains(&&head[2])).then(|| head.get(1).map_or("", |m| m.as_str()).trim_start_matches('@'))
}

/// The tables `model` is stored in, once its file in `app/models` is known: its `self.table_name`, else Rails' name,
/// prefixed by the namespace only when the namespace module sets `table_name_prefix`. Empty when the model isn't in
/// this app, or when an unqualified name in a namespaced controller's view may be a model of that namespace.
fn tables(view: &Path, model: &str) -> Vec<String> {
    let parts: Vec<String> = model.split("::").map(snake_case).collect();
    let Some((last, namespace)) = parts.split_last() else { return vec![] };
    let Some(models) = app_root(view).map(|app| app.join("models")) else { return vec![] };
    let Ok(text) = std::fs::read_to_string(models.join(parts.join("/")).with_extension("rb")) else { return vec![] };
    if let Some(cap) = TABLE_NAME.captures(&text) {
        return vec![cap[1].to_string()];
    }
    if namespace.is_empty() && namespace_shadows(view, &models, last) {
        return vec![];
    }
    let prefixed = std::fs::read_to_string(models.join(namespace.join("/")).with_extension("rb"))
        .is_ok_and(|module| module.contains("table_name_prefix"));
    let plural = pluralize(last);
    vec![if prefixed { namespace.iter().chain([&plural]).cloned().collect::<Vec<_>>().join("_") } else { plural }]
}

/// Does the view's controller namespace (`views/admin/reports/` → `Admin`) have its own `last`, which Ruby's lexical lookup
/// would pick over the top-level one?
fn namespace_shadows(view: &Path, models: &Path, last: &str) -> bool {
    let Some(folder) = paths::view_relative(view).and_then(Path::parent) else { return false };
    let mut scope = PathBuf::new();
    let mut parents: Vec<PathBuf> = Vec::new();
    for part in folder.components().take(folder.components().count().saturating_sub(1)) {
        scope.push(part);
        parents.push(scope.clone());
    }
    parents.iter().any(|namespace| models.join(namespace).join(last).with_extension("rb").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assigned(name: &str, code: &str) -> Assignment {
        Assignment { name: name.into(), file: PathBuf::new(), line: 0, code: code.into() }
    }

    #[test]
    fn members_under_the_cursor() {
        let line = "<%= report.title %> <%= @report.name %> <%= a.b.c %> <%= x.size() %> <%= user.admin? %>";
        assert_eq!(member_at(line, 12), Some((4, "report", "title")));
        assert_eq!(member_at(line, 8), None, "on the receiver");
        assert_eq!(member_at(line, 36), Some((24, "@report", "name")));
        assert_eq!(member_at(line, 48), None, "a.b is the member of a, not c");
        assert_eq!(member_at(line, 62), None, "a call");
        assert_eq!(member_at(line, 80), None, "a query method");
    }

    #[test]
    fn sources_of_assignments() {
        let record = |model: &str| Some(Source::Record(model.into()));
        let relation = |model: &str| Some(Source::Relation(model.into()));
        assert_eq!(source_of("@report = Report.find(params[:id])", "report"), record("Report"));
        assert_eq!(source_of("@report ||= Admin::Report.new(params.permit(:a));", "report"), record("Admin::Report"));
        assert_eq!(source_of("@report = Report.where(active: true).order(:title).first", "report"), record("Report"));
        assert_eq!(source_of("@report = Report.first", "report"), record("Report"));
        assert_eq!(source_of("@reports = Report.order(:title)", "reports"), relation("Report"));
        assert_eq!(source_of("@reports = ::Report.includes(:author).where.not(a: 1)", "reports"), None, "`not` is not known");
        assert_eq!(source_of("@reports = Report.all.page(params[:page]).per(10)", "reports"), relation("Report"));
        assert_eq!(source_of("@reports = Report.all.to_a", "reports"), None);
        assert_eq!(source_of("@report = Report.find(1).comments", "report"), None);
        assert_eq!(source_of("@report = Report", "report"), None);
        assert_eq!(source_of("@report = find_report", "report"), None);
        assert_eq!(source_of("@report = Report.find(1) if x", "report"), None);
        assert_eq!(source_of("@a = 1; @report = Report.first", "report"), None);
        for array in [
            "Report.first(5)",
            "Report.last(3)",
            "Report.take(2)",
            "Report.find([1, 2])",
            "Report.find(1, 2)",
            "Report.create([{ a: 1 }])",
            "Report.where(a: 1).first(2)",
        ] {
            assert_eq!(source_of(&format!("@report = {array}"), "report"), None, "{array}");
        }
    }

    #[test]
    fn assignments_must_agree() {
        let same = [assigned("r", "@r = Report.first"), assigned("r", "@r = Report.find(1)"), assigned("x", "@x = 1")];
        assert_eq!(ivar_source(&same, "r"), Some(Source::Record("Report".into())));
        let differ = [assigned("r", "@r = Report.first"), assigned("r", "@r = Report.all")];
        assert_eq!(ivar_source(&differ, "r"), None);
        let unknown = [assigned("r", "@r = Report.first"), assigned("r", "@r = load")];
        assert_eq!(ivar_source(&unknown, "r"), None);
        assert_eq!(ivar_source(&same, "missing"), None);
    }

    #[test]
    fn block_variables_name_their_collection() {
        fn collection<'a>(text: &'a str, variable: &str, nth: usize) -> Option<&'a str> {
            let declared = text
                .match_indices(&format!("|{variable}"))
                .nth(nth)
                .map(|(i, _)| i + 1)
                .or_else(|| text.match_indices(variable).nth(nth).map(|(i, _)| i))?;
            block_collection(text, declared, variable)
        }
        assert_eq!(collection("<% @reports.each do |report| %>\n  <%= report.title %>", "report", 0), Some("reports"));
        assert_eq!(collection("- @reports.each_with_index do |r, i|\n  = r.title", "r", 0), Some("reports"));
        assert_eq!(collection("= @a.map { |x| x.b }", "x", 0), Some("a"));
        assert_eq!(collection("- @a.times do |x|", "x", 0), None, "not an iterator");
        assert_eq!(collection("- @a.each_slice(2) do |x|", "x", 0), None, "not an iterator");
        assert_eq!(collection("- @a.each do |x|\n- report.children.each do |report|", "report", 0), None, "shadowed by a block of a local");
        assert_eq!(collection("- @a.each do |x|\n- current_user.drafts.each do |x|", "x", 1), None, "another collection");
        assert_eq!(collection("- @a.each_with_index do |i, report|", "report", 0), None, "second variable");
        assert_eq!(collection("- @a.each do |report_id|", "report", 0), None, "longer name");
    }

    #[test]
    fn hover_shows_the_schema_options() {
        let column = |options: &str| ModelColumn {
            table: "reports".into(),
            schema: PathBuf::new(),
            column: Column { name: "tags".into(), kind: "string".into(), options: options.into(), line: 0 },
        };
        assert_eq!(
            column(", default: [\"a\", \"b\"], comment: \"null: false\"").hover(),
            "`reports.tags` — string, default: [\"a\", \"b\"], comment: \"null: false\""
        );
        assert_eq!(column("").hover(), "`reports.tags` — string");
    }

    #[test]
    fn column_of_an_inferred_record() {
        let root = std::env::temp_dir().join(format!("rt-model-columns-{}", std::process::id()));
        let write = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write(
            "app/controllers/admin/reports_controller.rb",
            "class Admin::ReportsController < ApplicationController\n  def index\n    @reports = Report.order(:title)\n    @latest = Admin::Widget.last\n    @odd = Legacy.first\n    @plain = Result.new\n    @sub = Item.first\n  end\nend\n",
        );
        write("app/models/report.rb", "class Report\nend\n");
        write("app/models/admin/widget.rb", "class Admin::Widget\nend\n");
        write("app/models/legacy.rb", "class Legacy\n  self.table_name = \"old_things\"\nend\n");
        write("app/models/item.rb", "class Item\nend\n");
        write("app/models/admin/item.rb", "class Admin::Item\nend\n");
        write(
            "db/schema.rb",
            "  create_table \"reports\" do |t|\n    t.string \"title\", null: false\n  end\n  create_table \"widgets\" do |t|\n    t.integer \"views\", default: 0\n  end\n  create_table \"old_things\" do |t|\n    t.text \"note\"\n  end\n  create_table \"results\" do |t|\n    t.text \"note\"\n  end\n  create_table \"items\" do |t|\n    t.text \"note\"\n  end\n",
        );
        let view = root.join("app/views/admin/reports/index.html.erb");
        let project = Project::unknown();
        let text = "<% @reports.each do |report| %>\n  <%= report.title %> <%= @latest.views %> <%= @odd.note %> <%= @reports.title %> <%= @plain.note %> <%= @sub.note %>";
        let at = |needle: &str, declared: Option<usize>| {
            let end = text.find(needle).unwrap() + needle.len();
            let line_start = text[..end].rfind('\n').map_or(0, |i| i + 1);
            column_at(&project, &view, text, line_start, end - line_start - 1, |_| declared)
        };
        let declared = text.find("|report").map(|i| i + 1);
        let title = at("report.title", declared).unwrap();
        assert_eq!(title.hover(), "`reports.title` — string, null: false");
        assert_eq!((title.schema, title.column.line), (root.join("db/schema.rb"), 1));
        assert!(at("report.title", None).is_none(), "not a block variable here");
        assert_eq!(at("@latest.views", None).unwrap().hover(), "`widgets.views` — integer, default: 0");
        assert_eq!(at("@odd.note", None).unwrap().hover(), "`old_things.note` — text");
        assert!(at("@reports.title", None).is_none(), "a relation has no columns");
        assert!(at("@plain.note", None).is_none(), "not a model in app/models");
        assert!(at("@sub.note", None).is_none(), "Admin::Item may be the one meant");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn namespace_prefixes_tables_only_when_declared() {
        let root = std::env::temp_dir().join(format!("rt-model-tables-{}", std::process::id()));
        std::fs::create_dir_all(root.join("app/models/billing")).unwrap();
        std::fs::create_dir_all(root.join("app/models/admin")).unwrap();
        std::fs::write(root.join("app/models/billing.rb"), "module Billing\n  def self.table_name_prefix = \"billing_\"\nend\n").unwrap();
        std::fs::write(root.join("app/models/billing/invoice.rb"), "").unwrap();
        std::fs::write(root.join("app/models/admin.rb"), "module Admin\nend\n").unwrap();
        std::fs::write(root.join("app/models/admin/report.rb"), "").unwrap();
        let view = root.join("app/views/users/show.html.erb");
        assert_eq!(tables(&view, "Billing::Invoice"), ["billing_invoices"]);
        assert_eq!(tables(&view, "Admin::Report"), ["reports"]);
        assert!(tables(&view, "Missing").is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
