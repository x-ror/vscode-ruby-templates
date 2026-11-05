//! Controller and mailer classes as Rails finds a view's: the class of its folder, the parents and modules it gets
//! view folders and callbacks from, and the callbacks that run before an action.

use crate::inflect::{constant_path, snake_case};
use crate::project::Project;
use crate::rails_paths::{app_root, ruby_files_under, template_stem, view_relative};
use crate::ruby::logical_lines;
use crate::util::slash_path;
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

static DEF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)def\s+(?:self\.)?([a-z_]\w*[?!]?)").unwrap());
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

/// Parents that end a controller chain: ActionController's abstract classes add no view folder.
const ABSTRACT_CONTROLLERS: [&str; 2] = ["ActionController::Base", "ActionController::API"];

/// Names that change the folders Rails searches for a controller's views, which the class chain can't tell.
const LOOKUP_CHANGES: [&str; 4] = ["view_path", "controller_path", "_prefixes", "inherit_views"];

/// View folders of controllers Rails ships, which inherit from ActionController::Base and not ApplicationController.
const RAILS_FOLDERS: [&str; 2] = ["pwa", "rails"];

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
    let scan = controller_scan(project, &app);
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

/// `app/controllers/admin/users_controller.rb` → `admin/users`.
pub fn controller_path(file: &Path) -> Option<String> {
    let rel = file.strip_prefix(app_root(file)?.join("controllers")).ok()?;
    let rel = slash_path(rel);
    rel.strip_suffix("_controller.rb").filter(|p| !p.is_empty()).map(str::to_string)
}

/// The controller file of the view folder `dir` (`admin/users` → `app/controllers/admin/users_controller.rb`) in `app`.
pub fn controller_file(app: &Path, dir: &str) -> PathBuf {
    app.join("controllers").join(format!("{dir}_controller.rb"))
}

/// The folder of `view` below `views/` and its template name without formats.
pub(crate) fn folder_and_name(project: &Project, view: &Path) -> Option<(String, String)> {
    let rel = view_relative(view)?;
    let dir = slash_path(rel.parent()?);
    Some((dir, template_stem(|f| project.is_format(f), rel.file_name()?.to_str()?).to_string()))
}

/// The controller (or mailer) of the view folder `views/<dir>`, and the action the view renders (`None` for a
/// partial).
pub(crate) fn source_of(project: &Project, view: &Path) -> Option<(PathBuf, Option<String>)> {
    let (dir, name) = folder_and_name(project, view)?;
    if dir.is_empty() || dir.starts_with("layouts") {
        return None;
    }
    let app = app_root(view)?;
    let file = if dir.ends_with("_mailer") { app.join("mailers").join(format!("{dir}.rb")) } else { controller_file(&app, &dir) };
    Some((file, (!name.starts_with('_')).then_some(name)))
}

/// The base class whose unconditional callbacks reach a view of a folder without a controller: mailer layouts and
/// mailer folders get `ApplicationMailer`, the rest `ApplicationController`.
pub(crate) fn fallback_base(project: &Project, view: &Path) -> Option<PathBuf> {
    let (dir, name) = folder_and_name(project, view)?;
    if RAILS_FOLDERS.contains(&dir.split('/').next()?) {
        return None;
    }
    let app = app_root(view)?;
    let mailer = dir.ends_with("_mailer") || (dir.starts_with("layouts") && name.contains("mailer"));
    Some(if mailer { app.join("mailers/application_mailer.rb") } else { app.join("controllers/application_controller.rb") })
}

/// The controller (or mailer) file and its parents in this app, by their `class X < Parent` lines.
pub(crate) fn ancestry(app: &Path, first: PathBuf) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    let mut next = Some(first);
    while let Some(file) = next.take().filter(|f| out.len() < MAX_ANCESTORS && !out.iter().any(|(seen, _)| seen == f)) {
        let Ok(text) = std::fs::read_to_string(&file) else { break };
        next = PARENT.captures(&text).and_then(|cap| {
            let candidates = parent_names(&text[..cap.get(0)?.start()], &cap[1]);
            candidates.iter().find_map(|name| {
                let rel = constant_path(name);
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
    let relative = constant_path(constant);
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

impl ControllerScan {
    pub(crate) fn skips(&self, callback: &str) -> bool {
        self.skipped.contains(callback)
    }
}

fn scan_controllers(app: &Path) -> ControllerScan {
    let controllers = app.join("controllers");
    let concerns = controllers.join("concerns");
    let mut scan = ControllerScan::default();
    for file in ruby_files_under(&controllers) {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        scan.subclassed.extend(PARENT.captures_iter(&text).map(|cap| snake_case(cap[1].rsplit("::").next().unwrap_or(&cap[1]))));
        scan.skipped.extend(skipped_callbacks(&text));
        scan.concerns_change_lookup |= file.starts_with(&concerns) && changes_lookup(&text);
    }
    scan
}

/// The scan of `app`'s controllers, read once per Project.
pub(crate) fn controller_scan(project: &Project, app: &Path) -> Arc<ControllerScan> {
    project.controller_scan(app, || scan_controllers(app))
}

/// The methods of a Ruby class: name and line range (`def` line up to its `end`, by indentation).
pub(crate) fn methods(text: &str) -> Vec<(String, std::ops::Range<usize>)> {
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
pub(crate) fn callbacks_for(text: &str, action: Option<&str>) -> Vec<String> {
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
pub(crate) fn unconditional_callbacks(text: &str) -> Vec<String> {
    let body = class_body_indent(text);
    let calls = logical_lines(text).into_iter().filter(|call| indent_of(call) <= body);
    let calls = calls.filter_map(|call| CALLBACK.captures(&call).map(|cap| cap[1].to_string()));
    calls.filter(|args| !CONDITION.is_match(args)).flat_map(|args| callback_names(&args)).collect()
}

/// Callback methods a `skip_before_action` in `text` removes for some action.
pub(crate) fn skipped_callbacks(text: &str) -> Vec<String> {
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
}
