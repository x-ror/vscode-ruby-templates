//! A helper called in a view → its `def` in app/helpers, or in a controller that exposes it with `helper_method`.

use crate::rails_paths::app_root;
use crate::ruby::logical_lines;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

static HELPER_METHOD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*helper_method\b(.*)").unwrap());

/// The helper name a view calls at byte `col`: an identifier not after `.`/`@`/`:`, so `user.name`, `@name` and
/// `:name` are not helpers.
pub fn name_at(line: &str, col: usize) -> Option<&str> {
    let col = col.min(line.len());
    let before = line[..col].trim_end_matches(|c: char| c.is_alphanumeric() || matches!(c, '_' | '?' | '!')).chars().next_back();
    let word = crate::util::word_at(line, col);
    let is_name = word.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        && word.trim_end_matches(['?', '!']).chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    (is_name && !matches!(before, Some('.' | '@' | ':' | '$'))).then_some(word)
}

/// Files and 0-based lines defining helper `name`.
pub fn definitions(current: &Path, name: &str) -> Vec<(PathBuf, usize)> {
    let Some(app) = app_root(current) else { return vec![] };
    // `helper_method def current_user` defines and exposes it in one line.
    let def = Regex::new(&format!(r"^\s*(?:helper_method\s+)?def\s+(?:self\.)?{}\s*(?:$|[(;=#])", regex::escape(name))).unwrap();
    let exposed = |text: &str| {
        logical_lines(text).iter().filter_map(|l| HELPER_METHOD.captures(l)).any(|cap| {
            let args = &cap[1];
            args.trim_start().starts_with("def ") || args.split([',', ' ', '(', ')']).any(|a| a == format!(":{name}"))
        })
    };
    let mut out = Vec::new();
    for (dir, needs_helper_method) in [("helpers", false), ("controllers", true)] {
        for entry in WalkDir::new(app.join(dir)).into_iter().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "rb")) {
            let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
            if needs_helper_method && !exposed(&text) {
                continue;
            }
            out.extend(text.lines().enumerate().filter(|(_, l)| def.is_match(l)).map(|(i, _)| (entry.path().to_path_buf(), i)));
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_names() {
        let at = |line: &'static str, needle: &str| name_at(line, line.find(needle).unwrap() + 1);
        assert_eq!(at("<%= format_price(@order.total) %>", "format_price"), Some("format_price"));
        assert_eq!(at("<%= admin? && x %>", "admin?"), Some("admin?"));
        assert_eq!(at("<%= @order.total %>", "total"), None);
        assert_eq!(at("<%= @order %>", "order"), None);
        assert_eq!(at("<%= link_to :back %>", "back"), None);
        assert_eq!(at("<%= User.count %>", "User"), None);
    }

    #[test]
    fn helper_definitions() {
        let root = std::env::temp_dir().join(format!("rt-helpers-{}", std::process::id()));
        let write = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write("app/helpers/orders_helper.rb", "module OrdersHelper\n  def format_price(x)\n  end\n  def format_price_short = 1\nend\n");
        write(
            "app/controllers/application_controller.rb",
            "class ApplicationController\n  helper_method :current_user, :admin?\n\n  def current_user\n  end\nend\n",
        );
        write("app/controllers/users_controller.rb", "class UsersController\n  def current_user; end\nend\n");
        write("app/controllers/sessions_controller.rb", "class SessionsController\n  helper_method :a,\n    :signed_in?\n\n  def signed_in?\n  end\n\n  helper_method def theme\n  end\nend\n");
        let view = root.join("app/views/orders/index.html.erb");
        let rel = |name: &str| -> Vec<(String, usize)> {
            definitions(&view, name)
                .into_iter()
                .map(|(f, line)| (f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"), line))
                .collect()
        };
        assert_eq!(rel("format_price"), [("app/helpers/orders_helper.rb".to_string(), 1)]);
        assert_eq!(rel("current_user"), [("app/controllers/application_controller.rb".to_string(), 3)]);
        assert!(rel("missing").is_empty());
        assert_eq!(rel("signed_in?"), [("app/controllers/sessions_controller.rb".to_string(), 4)]);
        assert_eq!(rel("theme"), [("app/controllers/sessions_controller.rb".to_string(), 7)]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
