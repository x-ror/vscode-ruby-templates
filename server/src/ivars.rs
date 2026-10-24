//! Instance variables a view gets from its controller: those assigned in the action and in the `before_action`
//! callbacks that run for it. A partial has no action of its own, so it gets every action's.

use crate::partials::{app_root, template_stem, views_root};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static DEF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)def\s+(?:self\.)?([a-z_]\w*[?!]?)").unwrap());
static ASSIGN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@([a-z_]\w*)\s*(?:\|\||\+|-|\*)?=(?:[^=~>]|$)").unwrap());
static CALLBACK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:prepend_)?before_action\s*\(?\s*(.*)").unwrap());
static FILTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(only|except):\s*(\[[^\]]*\]|%i\[[^\]]*\]|:\w+[?!]?)").unwrap());
static SYMBOL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r":?([a-z_]\w*[?!]?)").unwrap());

/// An instance variable assignment: the variable without `@`, its file and 0-based line.
#[derive(Debug, PartialEq)]
pub struct Assignment {
    pub name: String,
    pub file: PathBuf,
    pub line: usize,
}

/// The controller (or mailer) of the view folder `views/<dir>`, and the action the view renders (`None` for a
/// partial).
fn source_of(view: &Path) -> Option<(PathBuf, Option<String>)> {
    let rel = view.strip_prefix(views_root(view)?).ok()?;
    let dir = rel.parent()?.to_str()?.replace('\\', "/");
    let name = template_stem(rel.file_name()?.to_str()?);
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

/// Callback methods that run before `action`: `before_action :a, :b, only: [:show]`.
fn callbacks_for(text: &str, action: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    for cap in text.lines().filter_map(|l| CALLBACK.captures(l)) {
        let args = &cap[1];
        let mut filters = FILTER.captures_iter(args);
        let applies = filters.all(|f| {
            let listed = SYMBOL.captures_iter(&f[2]).any(|s| Some(&s[1]) == action);
            match (&f[1], action) {
                (_, None) => true,
                ("only", _) => listed,
                _ => !listed,
            }
        });
        if !applies {
            continue;
        }
        out.extend(
            args.split(',').map(str::trim).take_while(|a| a.starts_with(':')).filter_map(|a| SYMBOL.captures(a).map(|s| s[1].to_string())),
        );
    }
    out
}

fn assignments_in(file: &Path, text: &str, lines: std::ops::Range<usize>) -> Vec<Assignment> {
    text.lines()
        .enumerate()
        .skip(lines.start)
        .take(lines.len())
        .flat_map(|(i, line)| {
            ASSIGN.captures_iter(line).map(move |cap| Assignment { name: cap[1].to_string(), file: file.to_path_buf(), line: i })
        })
        .collect()
}

/// Assignments the view at `view` can see, from its controller and ApplicationController's callbacks.
pub fn for_view(view: &Path) -> Vec<Assignment> {
    let Some((controller, action)) = source_of(view) else { return vec![] };
    let Ok(text) = std::fs::read_to_string(&controller) else { return vec![] };
    let own = methods(&text);
    let mut out = Vec::new();
    let mut add = |file: &Path, text: &str, methods: &[(String, std::ops::Range<usize>)], wanted: &[String]| {
        for (name, lines) in methods {
            if wanted.contains(name) {
                out.extend(assignments_in(file, text, lines.clone()));
            }
        }
    };
    let mut wanted = callbacks_for(&text, action.as_deref());
    match &action {
        Some(action) => wanted.push(action.clone()),
        None => wanted.extend(own.iter().map(|(name, _)| name.clone())),
    }
    add(&controller, &text, &own, &wanted);
    let application = controller.with_file_name("application_controller.rb");
    if let Some(app_text) =
        controller.parent().filter(|_| application != controller).and_then(|_| std::fs::read_to_string(&application).ok())
    {
        let inherited = callbacks_for(&app_text, action.as_deref());
        add(&application, &app_text, &methods(&app_text), &inherited);
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
            "app/controllers/application_controller.rb",
            "class ApplicationController\n  before_action :set_locale\n\n  def set_locale\n    @locale = :en\n  end\nend\n",
        );
        std::fs::create_dir_all(root.join("app/views/users")).unwrap();
        let names = |view: &str| -> Vec<String> {
            let mut names: Vec<String> = for_view(&root.join(view)).into_iter().map(|a| a.name).collect();
            names.sort();
            names.dedup();
            names
        };
        assert_eq!(names("app/views/users/show.html.erb"), ["locale", "menu", "tab", "user"]);
        assert_eq!(names("app/views/users/index.html.erb"), ["count", "locale", "users"]);
        assert_eq!(names("app/views/users/_card.html.erb"), ["count", "locale", "menu", "tab", "user", "users"]);
        assert!(names("app/views/layouts/application.html.erb").is_empty());
        let user = for_view(&root.join("app/views/users/show.html.erb")).into_iter().find(|a| a.name == "user").unwrap();
        assert_eq!((user.file.file_name().unwrap().to_str().unwrap(), user.line), ("users_controller.rb", 18));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
