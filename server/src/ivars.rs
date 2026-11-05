//! Instance variables a view gets from its controller: those assigned in the action and in the `before_action`
//! callbacks that run for it. A partial has no action of its own, so it gets every action's.

use crate::controllers::controller_path;
use crate::partials::snake_case;
use crate::project::Project;
use crate::rails_paths::{app_root, files_under, rails_relative, template_stem, views_root};
use crate::ruby::logical_lines;
use crate::util::{inline_code, slash_path, word_at};
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static DEF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)def\s+(?:self\.)?([a-z_]\w*[?!]?)").unwrap());
// Not `@@class_var`, not `==` / `=~` / `=>`.
static ASSIGN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[^@\w])@([a-z_]\w*)\s*(?:\|\||\+|-|\*)?=(?:[^=~>]|$)").unwrap());
static CALLBACK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:prepend_)?before_action\s*\(?\s*(.*)").unwrap());
// `only: [:a]`, `only: %i[a b]`, `only: %w(a)`, `only: :a`, `only: "a"`, `:only => [...]`.
static FILTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:\b(only|except):|:(only|except)\s*=>)\s*(\[[^\]]*\]|%[iw][\[(][^\])]*[\])]|:\w+[?!]?|["']\w+["'])"#).unwrap()
});
static SKIP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*skip_before_action\s*\(?\s*(.*)").unwrap());
static CLASS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*class\s").unwrap());
// Any condition on a callback: it may not run for the view.
static CONDITION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:only|except|if|unless):|:(?:only|except|if|unless)\s*=>").unwrap());
static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z_]\w*[?!]?").unwrap());
static PARENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*class\s+[\w:]+\s*<\s*(?:::)?((?:[A-Z]\w*::)*[A-Z]\w*)").unwrap());
static INCLUDE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*include\s+\(?\s*((?:::)?[A-Z][\w:, ]*)").unwrap());
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

/// The folder of `view` below `views/` and its template name without formats.
fn folder_and_name(project: &Project, view: &Path) -> Option<(String, String)> {
    let rel = view.strip_prefix(views_root(view)?).ok()?;
    let dir = slash_path(rel.parent()?);
    Some((dir, template_stem(|f| project.is_format(f), rel.file_name()?.to_str()?).to_string()))
}

/// The controller (or mailer) of the view folder `views/<dir>`, and the action the view renders (`None` for a
/// partial).
fn source_of(project: &Project, view: &Path) -> Option<(PathBuf, Option<String>)> {
    let (dir, name) = folder_and_name(project, view)?;
    if dir.is_empty() || dir.starts_with("layouts") {
        return None;
    }
    let app = app_root(view)?;
    let file = if dir.ends_with("_mailer") {
        app.join("mailers").join(format!("{dir}.rb"))
    } else {
        app.join("controllers").join(format!("{dir}_controller.rb"))
    };
    Some((file, (!name.starts_with('_')).then_some(name)))
}

/// View folders of controllers Rails ships, which inherit from ActionController::Base and not ApplicationController.
const RAILS_FOLDERS: [&str; 2] = ["pwa", "rails"];

/// The base class whose unconditional callbacks reach a view of a folder without a controller: mailer layouts and
/// mailer folders get `ApplicationMailer`, the rest `ApplicationController`.
fn fallback_base(project: &Project, view: &Path) -> Option<PathBuf> {
    let (dir, name) = folder_and_name(project, view)?;
    if RAILS_FOLDERS.contains(&dir.split('/').next()?) {
        return None;
    }
    let app = app_root(view)?;
    let mailer = dir.ends_with("_mailer") || (dir.starts_with("layouts") && name.contains("mailer"));
    Some(if mailer { app.join("mailers/application_mailer.rb") } else { app.join("controllers/application_controller.rb") })
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
        out.extend(callback_names(args));
    }
    out
}

/// Callback methods that run for every action: `before_action` calls in the class body with no `only:`, `except:`,
/// `if:` or `unless:`. Calls nested deeper (`with_options only: … do`, `if …`) have a condition of their own.
fn unconditional_callbacks(text: &str) -> Vec<String> {
    let body = class_body_indent(text);
    let calls = logical_lines(text).into_iter().filter(|call| indent_of(call) <= body);
    let calls = calls.filter_map(|call| CALLBACK.captures(&call).map(|cap| cap[1].to_string()));
    calls.filter(|args| !CONDITION.is_match(args)).flat_map(|args| callback_names(&args)).collect()
}

/// Callback methods a `skip_before_action` in `text` removes for some action.
fn skipped_callbacks(text: &str) -> Vec<String> {
    let calls = logical_lines(text).into_iter().filter_map(|call| SKIP.captures(&call).map(|cap| cap[1].to_string()));
    calls.flat_map(|args| callback_names(&args)).collect()
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The indentation of the first code line after the first `class` line.
fn class_body_indent(text: &str) -> usize {
    let mut lines = text.lines().skip_while(|l| !CLASS.is_match(l)).skip(1);
    lines.find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')).map_or(0, indent_of)
}

/// The callback names come first in the arguments; the first `key:` or `:key =>` ends them.
fn callback_names(args: &str) -> Vec<String> {
    let names = args.split(',').map(str::trim).take_while(|a| a.starts_with(':') && !a.contains("=>") && !a[1..].contains(':'));
    names.filter_map(|a| NAME.find(a).map(|n| n.as_str().to_string())).collect()
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

/// Parents that end a controller chain: ActionController's abstract classes add no view folder.
const ABSTRACT_CONTROLLERS: [&str; 2] = ["ActionController::Base", "ActionController::API"];

/// Names that change the folders Rails searches for a controller's views, which the class chain can't tell.
const LOOKUP_CHANGES: [&str; 4] = ["view_path", "controller_path", "_prefixes", "inherit_views"];

/// The view folders Rails searches for a bare `render "x"` in `view` (`_prefixes`): its controller's, then each parent's.
/// `None` unless the controller and every parent up to ActionController::Base are in this app, none of them changes
/// the lookup paths, and no other controller inherits from this one (it renders this folder's views too).
pub fn view_prefixes(project: &Project, view: &Path) -> Option<Vec<String>> {
    project.view_prefixes_of(view.parent()?, || prefixes_of(project, view))
}

fn prefixes_of(project: &Project, view: &Path) -> Option<Vec<String>> {
    let (controller, _) = source_of(project, view)?;
    let app = app_root(view)?;
    let chain = ancestry(&app, controller);
    let (_, last) = chain.last()?;
    let root_parent = PARENT.captures(last)?;
    let ends_at_base = ABSTRACT_CONTROLLERS.iter().any(|&base| base == &root_parent[1]);
    let chain_changes_lookup = chain.iter().any(|(_, text)| changes_lookup(text));
    if !ends_at_base || chain_changes_lookup || project.adds_view_paths {
        return None;
    }
    let scan = project.controller_scan(&app, || scan_controllers(&app));
    if scan.concerns_change_lookup || scan.subclassed.contains(chain[0].0.file_stem()?.to_str()?) {
        return None;
    }
    chain.iter().map(|(file, _)| controller_path(file)).collect()
}

/// Can a bare `render "x"` in `view` be checked: its folders are `view_prefixes`, and no module the controllers include
/// nor installed gem adds folders (a gem's `app/views/<prefix>`, or gems not installed to look in) that Rails searches too.
pub fn bare_lookup_is_certain(project: &Project, view: &Path) -> bool {
    let Some(prefixes) = view_prefixes(project, view) else { return false };
    let (Some((controller, _)), Some(app)) = (source_of(project, view), app_root(view)) else { return false };
    let autoload = autoload_dirs(&app);
    let mut seen = HashSet::new();
    if ancestry(&app, controller).iter().any(|(_, text)| included_modules_change_lookup(&autoload, text, &mut seen)) {
        return false;
    }
    project.view_roots().is_none_or(|roots| roots.gems_known && !prefixes.iter().any(|prefix| roots.gems_have_folder(prefix)))
}

/// The folders Zeitwerk autoloads from: each `app/*` (and its `concerns`), and `lib`.
fn autoload_dirs(app: &Path) -> Vec<PathBuf> {
    let app_folders = std::fs::read_dir(app).into_iter().flatten().flatten().map(|entry| entry.path());
    let mut dirs: Vec<PathBuf> = app_folders
        .filter(|dir| dir.is_dir() && dir.file_name().is_some_and(|name| name != "views"))
        .flat_map(|dir| [dir.join("concerns"), dir])
        .collect();
    dirs.extend(app.parent().map(|root| root.join("lib")));
    dirs
}

/// Does a module that `text` includes, found by Zeitwerk naming, or one of its own includes, change the lookup paths?
fn included_modules_change_lookup(autoload: &[PathBuf], text: &str, seen: &mut HashSet<PathBuf>) -> bool {
    let lines = logical_lines(text);
    lines.iter().enumerate().filter_map(|(i, line)| Some((i, INCLUDE.captures(line)?))).any(|(i, cap)| {
        let before = lines[..i].join("\n");
        let mut names = cap[1].split(',').filter_map(|name| name.split_whitespace().next());
        names.any(|name| {
            let candidates = match name.strip_prefix("::") {
                Some(rooted) => vec![rooted.to_string()],
                None => parent_names(&before, name),
            };
            let Some(file) = candidates.iter().find_map(|candidate| module_file(autoload, candidate)) else { return false };
            if !seen.insert(file.clone()) {
                return false;
            }
            std::fs::read_to_string(&file)
                .is_ok_and(|module| changes_lookup(&module) || included_modules_change_lookup(autoload, &module, seen))
        })
    })
}

fn module_file(autoload: &[PathBuf], constant: &str) -> Option<PathBuf> {
    let relative: PathBuf = constant.split("::").map(snake_case).collect();
    autoload.iter().map(|dir| dir.join(&relative).with_extension("rb")).find(|file| file.is_file())
}

fn changes_lookup(text: &str) -> bool {
    LOOKUP_CHANGES.iter().any(|name| text.contains(name))
}

/// What the controllers of an app say about every controller's view lookup, read once per Project.
#[derive(Debug, Clone, Default)]
pub struct ControllerScan {
    /// Stems (`users_controller`) of the controllers some other controller inherits from, by the last part of the parent's name.
    subclassed: HashSet<String>,
    /// Does a file in `controllers/concerns` change the lookup paths? Any controller may include it.
    concerns_change_lookup: bool,
    /// Callbacks some controller skips with `skip_before_action`, whatever the options.
    skipped: HashSet<String>,
}

fn scan_controllers(app: &Path) -> ControllerScan {
    let controllers = app.join("controllers");
    let concerns = controllers.join("concerns");
    let mut scan = ControllerScan::default();
    for file in files_under(&controllers, usize::MAX).into_iter().filter(|file| file.extension().is_some_and(|ext| ext == "rb")) {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        scan.subclassed.extend(PARENT.captures_iter(&text).map(|cap| snake_case(cap[1].rsplit("::").next().unwrap_or(&cap[1]))));
        scan.skipped.extend(skipped_callbacks(&text));
        scan.concerns_change_lookup |= file.starts_with(&concerns) && changes_lookup(&text);
    }
    scan
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
    let scan = project.controller_scan(app, || scan_controllers(app));
    let skipped = |name: &String| scan.skipped.contains(name) || chain.iter().any(|(_, text)| skipped_callbacks(text).contains(name));
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
    fn included_modules_are_found_by_name_across_lines_and_roots() {
        let root = std::env::temp_dir().join(format!("rt-ivars-includes-{}", std::process::id()));
        let changes = "module Themeable\n  included { prepend_view_path \"x\" }\nend\n";
        crate::util::write_files(
            &root,
            &[("lib/themeable.rb", changes), ("app/models/admin/themeable.rb", "module Admin::Themeable\nend\n")],
        );
        let autoload = autoload_dirs(&root.join("app"));
        let check = |text: &str| included_modules_change_lookup(&autoload, text, &mut HashSet::new());
        assert!(check("class X\n  include Pagy::Backend,\n    Themeable\nend\n"), "a list over two lines");
        assert!(!check("module Admin\n  class X\n    include Themeable\n  end\nend\n"), "Admin::Themeable comes first");
        assert!(check("module Admin\n  class X\n    include ::Themeable\n  end\nend\n"), "rooted names skip the nesting");
        assert!(!check("class X\n  include Pagy::Backend\nend\n"));
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
