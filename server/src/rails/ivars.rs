//! Instance variables a view gets from its controller: those assigned in the action and in the `before_action`
//! callbacks that run for it. A partial has no action of its own, so it gets every action's.

use crate::rails::controller_classes::{
    ancestry, callbacks_for, controller_scan, fallback_base, methods, skipped_callbacks, source_of, unconditional_callbacks,
};
use crate::rails::paths::{app_root, rails_relative};
use crate::rails::project::Project;
use crate::util::{inline_code, word_at};
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

// Not `@@class_var`, not `==` / `=~` / `=>`.
static ASSIGN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[^@\w])@([a-z_]\w*)\s*(?:\|\||\+|-|\*)?=(?:[^=~>]|$)").unwrap());

/// An instance variable assignment: the variable without `@`, its file, 0-based line and the line's code.
#[derive(Debug, PartialEq)]
pub struct Assignment {
    pub name: String,
    pub file: PathBuf,
    pub line: usize,
    pub code: String,
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
            let link =
                crate::util::file_uri(&a.file).map_or_else(|| shown.clone(), |uri| format!("[{shown}]({}#L{})", uri.as_str(), a.line + 1));
            format!("- {link}: {}", inline_code(&a.code))
        })
        .collect();
    (!lines.is_empty()).then(|| format!("`@{name}`\n\n{}", lines.join("\n")))
}

/// Assignments the view at `view` can see: from the action and the callbacks its controller and the controller's
/// parents in the app run for it, wherever along that chain the methods are defined. A view whose folder has no
/// controller gets the unconditional callbacks of `ApplicationController` (`ApplicationMailer` for mailers).
pub fn for_view(project: &Project, view: &Path) -> Vec<Assignment> {
    let Some(app) = app_root(view) else { return vec![] };
    let own = source_of(project, view).filter(|(file, _)| file.is_file() && Some(file) != fallback_base(project, view).as_ref());
    let Some((controller, action)) = own else {
        return fallback_assignments(project, view, &app);
    };
    let chain = ancestry(&app, controller);
    let Some((_, own_text)) = chain.first() else { return vec![] };
    let mut wanted: Vec<String> = chain.iter().flat_map(|(_, text)| callbacks_for(text, action.as_deref())).collect();
    match &action {
        Some(action) => wanted.push(action.clone()),
        None => wanted.extend(methods(own_text).into_iter().map(|(name, _)| name)),
    }
    assignments_of(&chain, &wanted)
}

/// Assignments in the methods of `chain` named in `wanted`.
fn assignments_of(chain: &[(PathBuf, String)], wanted: &[String]) -> Vec<Assignment> {
    let mut out = Vec::new();
    for (file, text) in chain {
        for (name, lines) in methods(text) {
            if wanted.contains(&name) {
                out.extend(assignments_in(file, text, lines));
            }
        }
    }
    out
}

fn fallback_assignments(project: &Project, view: &Path, app: &Path) -> Vec<Assignment> {
    let Some(base) = fallback_base(project, view) else { return vec![] };
    let chain = ancestry(app, base);
    let scan = controller_scan(project, app);
    let skipped = |name: &String| scan.skips(name) || chain.iter().any(|(_, text)| skipped_callbacks(text).contains(name));
    let wanted: Vec<String> = chain.iter().flat_map(|(_, text)| unconditional_callbacks(text)).filter(|name| !skipped(name)).collect();
    assignments_of(&chain, &wanted)
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
    fn class_variables_are_not_assignments() {
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
        assert_eq!(names("app/views/layouts/application.html.erb"), ["locale"], "no controller: ApplicationController's");
        assert_eq!(names("app/views/admin/users/show.html.erb"), ["admin", "locale", "user"]);
        assert_eq!(names("app/views/admin/reports/index.html.erb"), ["admin", "locale", "reports"], "a parent named inside its module");
        let user =
            for_view(&Project::unknown(), &root.join("app/views/users/show.html.erb")).into_iter().find(|a| a.name == "user").unwrap();
        assert_eq!((user.file.file_name().unwrap().to_str().unwrap(), user.line), ("users_controller.rb", 18));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn folders_without_a_controller_get_the_application_base_callbacks() {
        let root = std::env::temp_dir().join(format!("rt-ivars-base-{}", std::process::id()));
        let write = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write(
            "app/controllers/application_controller.rb",
            "class ApplicationController < ActionController::Base
  before_action :set_current_user, :set_second
  prepend_before_action :set_early
  before_action :set_admin, only: :show
  before_action :set_beta, if: :beta?
  before_action :set_old, :if => :old?
  before_action :set_flag,
    unless: -> { false }
  before_action :set_skipped
  with_options only: :index do
    before_action :set_nested
  end

  def set_current_user = @current_user = User.first
  def set_second = @second = 1
  def set_early = @early = 1
  def set_admin = @admin = true
  def set_beta = @beta = true
  def set_old = @old = true
  def set_flag = @flag = true
  def set_skipped = @skipped = true
  def set_nested = @nested = true
  def unused = @unused = 1
end
",
        );
        write(
            "app/controllers/users_controller.rb",
            "class UsersController < ApplicationController\n  def show\n    @user = 1\n  end\nend\n",
        );
        write(
            "app/controllers/health_controller.rb",
            "class HealthController < ApplicationController\n  skip_before_action :set_skipped, only: :show\nend\n",
        );
        write(
            "app/mailers/application_mailer.rb",
            "class ApplicationMailer < ActionMailer::Base\n  before_action :set_brand\n\n  def set_brand\n    @brand = 1\n  end\nend\n",
        );
        let names = |view: &str| -> Vec<String> {
            let mut names: Vec<String> = for_view(&Project::unknown(), &root.join(view)).into_iter().map(|a| a.name).collect();
            names.sort();
            names
        };
        let base = ["current_user", "early", "second"];
        assert_eq!(names("app/views/shared/_nav.html.haml"), base);
        assert_eq!(names("app/views/layouts/application.html.erb"), base);
        assert_eq!(names("app/views/application/_x.html.erb"), base, "the base's own folder is a fallback too");
        assert_eq!(names("app/views/layouts/mailer.html.erb"), ["brand"]);
        assert!(names("app/views/pwa/manifest.json.erb").is_empty(), "Rails' own controllers don't inherit from it");
        assert_eq!(
            names("app/views/users/show.html.erb"),
            ["admin", "beta", "current_user", "early", "flag", "nested", "old", "second", "skipped", "user"],
            "a folder with a controller keeps its own lookup"
        );
        std::fs::remove_dir_all(&root).unwrap();
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
        let uri = crate::util::file_uri(&file).unwrap();
        let shown = "app/controllers/admin/users_controller.rb";
        assert_eq!(
            md,
            format!(
                "`@user`\n\n- [{shown}:4]({}#L4): `@user = User.find(params[:id])`\n- [{shown}:10]({}#L10): `` @user ||= nil; @user = `whoami` ``",
                uri.as_str(),
                uri.as_str()
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
