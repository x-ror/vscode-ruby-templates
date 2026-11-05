//! A helper called in a view → its `def` in app/helpers, or in a controller that exposes it with `helper_method`.

use crate::rails_paths::{app_root, rails_relative};
use crate::ruby::logical_lines;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

static HELPER_METHOD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*helper_method\b(.*)").unwrap());

const MAX_SIGNATURE_LINES: usize = 10;
/// Comment lines that configure tools rather than document the method.
const COMMENT_DIRECTIVES: [&str; 5] = ["rubocop:", "frozen_string_literal:", ":nodoc:", "typed:", "encoding:"];

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
    let symbol = format!(":{name}");
    let exposed = |text: &str| {
        logical_lines(text).iter().filter_map(|l| HELPER_METHOD.captures(l)).any(|cap| {
            let args = &cap[1];
            args.trim_start().starts_with("def ") || args.split([',', ' ', '(', ')']).any(|a| a == symbol)
        })
    };
    let mut out = Vec::new();
    for (dir, needs_helper_method) in [("helpers", false), ("controllers", true)] {
        for entry in WalkDir::new(app.join(dir)).into_iter().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "rb")) {
            let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
            // Necessary for a match, and far cheaper than the line scans below.
            if !text.contains(name) || needs_helper_method && !(text.contains("helper_method") && exposed(&text)) {
                continue;
            }
            let lines: Vec<&str> = text.lines().collect();
            let stem = entry.path().file_stem().map(|s| s.to_string_lossy().replace('_', "")).unwrap_or_default();
            // A nested class's methods are not the module's or the controller's.
            let in_own_class = |i: usize| {
                enclosing(&lines, i).is_none_or(|class| class.rsplit("::").next().is_some_and(|last| last.eq_ignore_ascii_case(&stem)))
            };
            out.extend((0..lines.len()).filter(|&i| defines(lines[i], name) && in_own_class(i)).map(|i| (entry.path().to_path_buf(), i)));
        }
    }
    out.sort();
    out
}

/// Whether `line` is `def name`, also as `helper_method def name`, which defines and exposes it in one line.
fn defines(line: &str, name: &str) -> bool {
    let rest = line.trim_start();
    let rest = match rest.strip_prefix("helper_method") {
        Some(after) if after.starts_with(char::is_whitespace) => after.trim_start(),
        _ => rest,
    };
    let Some(rest) = rest.strip_prefix("def").filter(|r| r.starts_with(char::is_whitespace)) else { return false };
    let Some(rest) = rest.trim_start().strip_prefix(name) else { return false };
    let rest = rest.trim_start();
    rest.is_empty() || rest.starts_with(['(', ';', '=', '#'])
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The name of the nearest `class` or `module` above `line` that is less indented, so encloses it.
fn enclosing<'a>(lines: &[&'a str], line: usize) -> Option<&'a str> {
    let def_indent = indent(lines[line]);
    lines[..line].iter().rev().filter(|l| indent(l) < def_indent).find_map(|l| {
        let rest = l.trim_start();
        let rest = rest.strip_prefix("class ").or_else(|| rest.strip_prefix("module "))?;
        let end = rest.find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | ':'))).unwrap_or(rest.len());
        Some(&rest[..end]).filter(|name| !name.is_empty())
    })
}

/// Markdown for helper `name` called in the view `current`: the first definition's signature, where it comes from and its comment.
pub fn hover(current: &Path, name: &str) -> Option<String> {
    let (file, line) = definitions(current, name).into_iter().next()?;
    describe(current, &file, line)
}

fn describe(current: &Path, file: &Path, line: usize) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let mut signature = lines.get(line)?.trim().trim_start_matches("helper_method ").to_string();
    // A parameter list that wraps onto the next lines.
    for next in lines.iter().skip(line + 1).take(MAX_SIGNATURE_LINES).map(|l| l.trim()) {
        let first_word = next.split(|c: char| !(c.is_alphanumeric() || c == '_')).next();
        let wraps = signature.ends_with([',', '(']) || next.starts_with(')');
        if !wraps || matches!(first_word, Some("end" | "def")) {
            break;
        }
        signature.push(' ');
        signature.push_str(next);
    }
    let is_directive = |c: &str| COMMENT_DIRECTIVES.iter().any(|d| c.trim_start().starts_with(d));
    let comment: Vec<&str> = lines[..line]
        .iter()
        .rev()
        .map_while(|l| l.trim_start().strip_prefix('#'))
        .map(|c| c.strip_prefix(' ').unwrap_or(c).trim_end())
        .filter(|c| !is_directive(c))
        .collect();

    let shown = format!("{}:{}", rails_relative(file, current), line + 1);
    let link = crate::util::file_uri(file).map_or_else(|| shown.clone(), |uri| format!("[{shown}]({}#L{})", uri.as_str(), line + 1));
    let source = if file.starts_with(app_root(current)?.join("controllers")) {
        format!("`helper_method` in `{}`: {link}", enclosing(&lines, line).unwrap_or("controller"))
    } else {
        link
    };
    let mut out = format!("```ruby\n{signature}\n```\n\n{source}");
    if !comment.is_empty() {
        out.push_str("\n\n");
        out.push_str(&comment.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::slash_path;

    #[test]
    fn hover_shows_signature_source_and_comment() {
        let root = std::env::temp_dir().join(format!("rt-helper-hover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write(
            "app/helpers/orders_helper.rb",
            "module OrdersHelper\n  def other; end\n\n  # rubocop:disable Style/Foo\n  # Formats cents.\n  #\n  #   format_price(5)\n  def format_price(amount,\n                   currency: \"USD\")\n  end\n\n  def label_for(a = \"(\", b)\n    x\n  end\nend\n",
        );
        write(
            "app/controllers/application_controller.rb",
            "class ApplicationController\n  helper_method :current_user\n  def current_user\n  end\nend\n",
        );
        let view = root.join("app/views/orders/index.html.erb");
        let link = |rel: &str, line: usize| format!("[{rel}:{line}]({}#L{line})", crate::util::file_uri(root.join(rel)).unwrap().as_str());
        assert_eq!(
            hover(&view, "format_price").unwrap(),
            format!(
                "```ruby\ndef format_price(amount, currency: \"USD\")\n```\n\n{}\n\nFormats cents.\n\n  format_price(5)",
                link("app/helpers/orders_helper.rb", 8)
            )
        );
        assert_eq!(
            hover(&view, "current_user").unwrap(),
            format!(
                "```ruby\ndef current_user\n```\n\n`helper_method` in `ApplicationController`: {}",
                link("app/controllers/application_controller.rb", 3)
            )
        );
        write(
            "app/controllers/nested_controller.rb",
            "module Admin\n  class NestedController\n    helper_method :shown\n    # Shown.\n    def shown; end\n  end\nend\n",
        );
        let shown = hover(&view, "shown").unwrap();
        assert!(shown.contains("`helper_method` in `NestedController`: ") && shown.ends_with("Shown."), "{shown}");
        let label = hover(&view, "label_for").unwrap();
        assert!(label.starts_with("```ruby\ndef label_for(a = \"(\", b)\n```"), "{label}");
        assert_eq!(hover(&view, "missing"), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

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
        let _ = std::fs::remove_dir_all(&root);
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
        write(
            "app/helpers/nested_helper.rb",
            "module NestedHelper\n  def self.util; end\n  def outer_thing; end\n\n  class Builder\n    def nested_thing; end\n  end\nend\n",
        );
        write(
            "app/controllers/reports_controller.rb",
            "class ReportsController\n  helper_method :sort_column, :nested_thing\n  def sort_column; end\n\n  class Sorter\n    def sort_column; end\n    def nested_thing; end\n  end\nend\n",
        );
        let view = root.join("app/views/orders/index.html.erb");
        let rel = |name: &str| -> Vec<(String, usize)> {
            definitions(&view, name).into_iter().map(|(f, line)| (slash_path(f.strip_prefix(&root).unwrap()), line)).collect()
        };
        assert_eq!(rel("format_price"), [("app/helpers/orders_helper.rb".to_string(), 1)]);
        assert_eq!(rel("current_user"), [("app/controllers/application_controller.rb".to_string(), 3)]);
        assert!(rel("missing").is_empty());
        assert_eq!(rel("signed_in?"), [("app/controllers/sessions_controller.rb".to_string(), 4)]);
        assert_eq!(rel("theme"), [("app/controllers/sessions_controller.rb".to_string(), 7)]);
        assert!(rel("util").is_empty(), "def self. is no helper");
        assert_eq!(rel("outer_thing"), [("app/helpers/nested_helper.rb".to_string(), 2)]);
        assert!(rel("nested_thing").is_empty(), "a nested class's method");
        assert_eq!(rel("sort_column"), [("app/controllers/reports_controller.rb".to_string(), 2)]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
