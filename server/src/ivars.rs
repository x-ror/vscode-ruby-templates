//! Instance variables a view gets from its controller: those assigned in the action and in the `before_action`
//! callbacks that run for it. A partial has no action of its own, so it gets every action's.

use crate::partials::snake_case;
use crate::project::Project;
use crate::rails_paths::{app_root, rails_relative, template_stem, views_root};
use crate::ruby::logical_lines;
use crate::util::{inline_code, word_at};
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tower_lsp::lsp_types::Url;

static DEF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)def\s+(?:self\.)?([a-z_]\w*[?!]?)").unwrap());
// Not `@@class_var`, not `==` / `=~` / `=>`.
static ASSIGN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[^@\w])@([a-z_]\w*)\s*(?:\|\||\+|-|\*)?=(?:[^=~>]|$)").unwrap());
static CALLBACK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:prepend_)?before_action\s*\(?\s*(.*)").unwrap());
// `only: [:a]`, `only: %i[a b]`, `only: %w(a)`, `only: :a`, `only: "a"`, `:only => [...]`.
static FILTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:\b(only|except):|:(only|except)\s*=>)\s*(\[[^\]]*\]|%[iw][\[(][^\])]*[\])]|:\w+[?!]?|["']\w+["'])"#).unwrap()
});
static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z_]\w*[?!]?").unwrap());
static PARENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*class\s+[\w:]+\s*<\s*((?:[A-Z]\w*::)*[A-Z]\w*)").unwrap());
static MODULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*module\s+((?:[A-Z]\w*::)*[A-Z]\w*)").unwrap());
/// Deep enough for `UsersController < Admin::BaseController < ApplicationController < ActionController::Base`.
const MAX_ANCESTORS: usize = 4;

/// An instance variable assignment: the variable without `@`, its file, 0-based line and the line's code.
#[derive(Debug, PartialEq)]
pub struct Assignment {
    pub name: String,
    pub file: PathBuf,
    pub line: usize,
    pub code: String,
}

/// The controller (or mailer) of the view folder `views/<dir>`, and the action the view renders (`None` for a
/// partial).
fn source_of(project: &Project, view: &Path) -> Option<(PathBuf, Option<String>)> {
    let rel = view.strip_prefix(views_root(view)?).ok()?;
    let dir = rel.parent()?.to_str()?.replace('\\', "/");
    let name = template_stem(|f| project.is_format(f), rel.file_name()?.to_str()?);
    if dir.is_empty() || dir.starts_with("layouts") {
        return None;
    }
    let app = app_root(view)?;
    let file = if dir.ends_with("_mailer") {
        app.join("mailers").join(format!("{dir}.rb"))
    } else {
        app.join("controllers").join(format!("{dir}_controller.rb"))
    };
    Some((file, (!name.starts_with('_')).then(|| name.to_string())))
}

/// The methods of a Ruby class: name and line range (`def` line up to its `end`, by indentation).
fn methods(text: &str) -> Vec<(String, std::ops::Range<usize>)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(cap) = DEF.captures(line) else { continue };
        let indent = cap[1].len();
        let end = lines[i + 1..]
            .iter()
            .position(|l| {
                let trimmed = l.trim_start();
                let at = l.len() - trimmed.len();
                at <= indent && !trimmed.is_empty() && (trimmed == "end" || trimmed.starts_with("end ") || DEF.is_match(l))
            })
            .map_or(lines.len(), |n| i + 1 + n);
        out.push((cap[2].to_string(), i..end));
    }
    out
}

/// Callback methods that run before `action`: `before_action :a, :b, only: [:show]`, over continuation lines.
fn callbacks_for(text: &str, action: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    for call in logical_lines(text) {
        let Some(cap) = CALLBACK.captures(&call) else { continue };
        let args = &cap[1];
        let applies = FILTER.captures_iter(args).all(|f| {
            let list = f[3].trim_start_matches("%i").trim_start_matches("%w");
            let listed = NAME.find_iter(list).any(|n| Some(n.as_str()) == action);
            match (f.get(1).or(f.get(2)).map_or("", |k| k.as_str()), action) {
                (_, None) => true,
                ("only", _) => listed,
                _ => !listed,
            }
        });
        if !applies {
            continue;
        }
        // The callback names come first; the first `key:` or `:key =>` ends them.
        let names = args.split(',').map(str::trim).take_while(|a| a.starts_with(':') && !a.contains("=>") && !a[1..].contains(':'));
        out.extend(names.filter_map(|a| NAME.find(a).map(|n| n.as_str().to_string())));
    }
    out
}

/// The controller (or mailer) file and its parents in this app, by their `class X < Parent` lines.
fn ancestry(app: &Path, first: PathBuf) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    let mut next = Some(first);
    while let Some(file) = next.take().filter(|f| out.len() < MAX_ANCESTORS && !out.iter().any(|(seen, _)| seen == f)) {
        let Ok(text) = std::fs::read_to_string(&file) else { break };
        next = PARENT.captures(&text).and_then(|cap| {
            let candidates = parent_names(&text[..cap.get(0)?.start()], &cap[1]);
            candidates.iter().find_map(|name| {
                let rel: PathBuf = name.split("::").map(snake_case).collect();
                ["controllers", "mailers"].iter().map(|dir| app.join(dir).join(&rel).with_extension("rb")).find(|f| f.is_file())
            })
        });
        out.push((file, text));
    }
    out
}

/// The constants Ruby tries for `parent` in a class inside the `module` blocks of `before`, innermost first: in
/// `module Admin`, `BaseController` is `Admin::BaseController` before `BaseController`.
fn parent_names(before: &str, parent: &str) -> Vec<String> {
    let mut scopes: Vec<String> = Vec::new();
    for cap in MODULE.captures_iter(before) {
        let scope = scopes.last().map_or_else(|| cap[1].to_string(), |outer| format!("{outer}::{}", &cap[1]));
        scopes.push(scope);
    }
    scopes.iter().rev().map(|scope| format!("{scope}::{parent}")).chain([parent.to_string()]).collect()
}

fn assignments_in(file: &Path, text: &str, lines: std::ops::Range<usize>) -> Vec<Assignment> {
    let all: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for i in lines.filter(|&i| i < all.len()) {
        let names: Vec<String> = ASSIGN.captures_iter(all[i]).map(|cap| cap[1].to_string()).collect();
        if names.is_empty() {
            continue;
        }
        let code = statement_at(&all, i);
        out.extend(names.into_iter().map(|name| Assignment { name, file: file.to_path_buf(), line: i, code: code.clone() }));
    }
    out
}

/// How many lines one assignment shown in a hover may take.
const MAX_STATEMENT_LINES: usize = 6;

/// Line `i` and the lines that continue it (after a trailing `,` `(` `.` `\` or before a leading `.`), joined.
fn statement_at(lines: &[&str], i: usize) -> String {
    let mut code = lines[i].trim().to_string();
    for next in lines.iter().skip(i + 1).take(MAX_STATEMENT_LINES - 1).map(|l| l.trim()) {
        let continues = code.ends_with([',', '(', '.', '\\']) || next.starts_with('.') || next.starts_with("&.");
        if !continues {
            break;
        }
        let glue = if code.ends_with(['(', '.']) || next.starts_with('.') || next.starts_with("&.") { "" } else { " " };
        code = format!("{}{glue}{next}", code.trim_end_matches('\\').trim_end());
    }
    code
}

/// The name, without `@`, of the instance variable under `col`; on the `@` itself too.
pub fn name_at(line: &str, col: usize) -> Option<&str> {
    let col = if line.get(col..).is_some_and(|rest| rest.starts_with('@')) { col + 1 } else { col };
    let before = line[..col.min(line.len())].trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
    let word = word_at(line, col);
    (!word.is_empty() && before.ends_with('@') && !before.ends_with("@@")).then_some(word)
}

/// Markdown for `@name` in `view`: each line assigning it as a link, named from the view's Rails root, with the line's
/// code.
pub fn hover(assignments: &[Assignment], name: &str, view: &Path) -> Option<String> {
    let mut seen = HashSet::new();
    let lines: Vec<String> = assignments
        .iter()
        .filter(|a| a.name == name && seen.insert((&a.file, a.line)))
        .map(|a| {
            let shown = format!("{}:{}", rails_relative(&a.file, view), a.line + 1);
            let link = Url::from_file_path(&a.file).map_or_else(|()| shown.clone(), |uri| format!("[{shown}]({uri}#L{})", a.line + 1));
            format!("- {link}: {}", inline_code(&a.code))
        })
        .collect();
    (!lines.is_empty()).then(|| format!("`@{name}`\n\n{}", lines.join("\n")))
}

/// Assignments the view at `view` can see: from the action and the callbacks its controller and the controller's
/// parents in the app run for it, wherever along that chain the methods are defined.
pub fn for_view(project: &Project, view: &Path) -> Vec<Assignment> {
    let (Some((controller, action)), Some(app)) = (source_of(project, view), app_root(view)) else { return vec![] };
    let chain = ancestry(&app, controller);
    let Some((_, own_text)) = chain.first() else { return vec![] };
    let mut wanted: Vec<String> = chain.iter().flat_map(|(_, text)| callbacks_for(text, action.as_deref())).collect();
    match &action {
        Some(action) => wanted.push(action.clone()),
        None => wanted.extend(methods(own_text).into_iter().map(|(name, _)| name)),
    }
    let mut out = Vec::new();
    for (file, text) in &chain {
        for (name, lines) in methods(text) {
            if wanted.contains(&name) {
                out.extend(assignments_in(file, text, lines));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const USERS: &str = "class UsersController < ApplicationController
  before_action :set_user, only: %i[show edit]
  before_action :load_menu, except: :index

  def index
    @users = User.all
    @count ||= @users.size
    if @users == []
    end
  end

  def show
    @tab = params[:tab]
  end

  private

  def set_user
    @user = User.find(params[:id])
  end

  def load_menu = @menu = Menu.first
end
";

    #[test]
    fn methods_and_callbacks() {
        let names: Vec<(String, std::ops::Range<usize>)> = methods(USERS);
        assert_eq!(
            names.iter().map(|(n, r)| (n.as_str(), r.start, r.end)).collect::<Vec<_>>(),
            [("index", 4, 9), ("show", 11, 13), ("set_user", 17, 19), ("load_menu", 21, 22)]
        );
        assert_eq!(callbacks_for(USERS, Some("show")), ["set_user", "load_menu"]);
        assert_eq!(callbacks_for(USERS, Some("index")), Vec::<String>::new());
        assert_eq!(callbacks_for(USERS, None), ["set_user", "load_menu"]);
        let multi = "  before_action :set_user,\n    only: %i(show edit)\n  before_action :a, :only => [:show, :edit]\n  before_action :b, only: \"edit\"\n";
        assert!(callbacks_for(multi, Some("index")).is_empty());
        assert_eq!(callbacks_for(multi, Some("edit")), ["set_user", "a", "b"]);
        assert!(ASSIGN.captures("@@count = 0").is_none());
    }

    #[test]
    fn view_assignments() {
        let root = std::env::temp_dir().join(format!("rt-ivars-{}", std::process::id()));
        let write = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write("app/controllers/users_controller.rb", USERS);
        write(
            "app/controllers/admin/users_controller.rb",
            "class Admin::UsersController < Admin::BaseController\n  def show\n    @user = 1\n  end\nend\n",
        );
        write(
            "app/controllers/admin/base_controller.rb",
            "class Admin::BaseController < ApplicationController\n  before_action :require_admin\n\n  def require_admin\n    @admin = true\n  end\nend\n",
        );
        write(
            "app/controllers/application_controller.rb",
            "class ApplicationController\n  before_action :set_locale\n\n  def set_locale\n    @locale = :en\n  end\nend\n",
        );
        write(
            "app/controllers/admin/reports_controller.rb",
            "module Admin\n  class ReportsController < BaseController\n    def index\n      @reports = 1\n    end\n  end\nend\n",
        );
        std::fs::create_dir_all(root.join("app/views/users")).unwrap();
        let names = |view: &str| -> Vec<String> {
            let mut names: Vec<String> = for_view(&Project::unknown(), &root.join(view)).into_iter().map(|a| a.name).collect();
            names.sort();
            names.dedup();
            names
        };
        assert_eq!(names("app/views/users/show.html.erb"), ["locale", "menu", "tab", "user"]);
        assert_eq!(names("app/views/users/index.html.erb"), ["count", "locale", "users"]);
        assert_eq!(names("app/views/users/_card.html.erb"), ["count", "locale", "menu", "tab", "user", "users"]);
        assert!(names("app/views/layouts/application.html.erb").is_empty());
        assert_eq!(names("app/views/admin/users/show.html.erb"), ["admin", "locale", "user"]);
        assert_eq!(names("app/views/admin/reports/index.html.erb"), ["admin", "locale", "reports"], "a parent named inside its module");
        let user =
            for_view(&Project::unknown(), &root.join("app/views/users/show.html.erb")).into_iter().find(|a| a.name == "user").unwrap();
        assert_eq!((user.file.file_name().unwrap().to_str().unwrap(), user.line), ("users_controller.rb", 18));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn parent_names_follow_the_module_nesting() {
        assert_eq!(parent_names("module Admin\n  module Reports\n", "Base"), ["Admin::Reports::Base", "Admin::Base", "Base"]);
        assert_eq!(parent_names("module Admin::Reports\n", "Base"), ["Admin::Reports::Base", "Base"]);
        assert_eq!(parent_names("", "Admin::Base"), ["Admin::Base"]);
    }

    #[test]
    fn ivar_names_under_the_cursor() {
        let line = "<%= @user.name, @@count, user %>";
        assert_eq!(name_at(line, 4), Some("user"), "on the @");
        assert_eq!(name_at(line, 7), Some("user"));
        assert_eq!(name_at(line, 11), None, "a method");
        assert_eq!(name_at(line, 19), None, "a class variable");
        assert_eq!(name_at(line, 27), None);
    }

    #[test]
    fn hover_lists_each_assignment() {
        let file = std::env::temp_dir().join("app/controllers/admin/users_controller.rb");
        let view = std::env::temp_dir().join("app/views/admin/users/show.html.erb");
        let at = |line, code: &str| Assignment { name: "user".into(), file: file.clone(), line, code: code.into() };
        let found =
            [at(3, "@user = User.find(params[:id])"), at(9, "@user ||= nil; @user = `whoami`"), at(9, "@user ||= nil; @user = `whoami`")];
        let md = hover(&found, "user", &view).unwrap();
        let uri = Url::from_file_path(&file).unwrap();
        let shown = "app/controllers/admin/users_controller.rb";
        assert_eq!(
            md,
            format!(
                "`@user`\n\n- [{shown}:4]({uri}#L4): `@user = User.find(params[:id])`\n- [{shown}:10]({uri}#L10): `` @user ||= nil; @user = `whoami` ``"
            )
        );
        assert_eq!(hover(&found, "menu", &view), None);
    }

    #[test]
    fn statements_span_their_continued_lines() {
        let lines = ["@users = User", "  .where(active: true)", "  .order(:name)", "@other = 1", "@user = find(", "  id,", "  scope)", "x"];
        assert_eq!(statement_at(&lines, 0), "@users = User.where(active: true).order(:name)");
        assert_eq!(statement_at(&lines, 3), "@other = 1");
        assert_eq!(statement_at(&lines, 4), "@user = find(id, scope)");
    }
}
