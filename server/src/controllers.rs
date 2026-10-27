//! Controllers → views: `layout "admin"`, an action's `def show`, and `render :edit` / `render "users/show"`.
//! Controllers reach the server only for navigation; Ruby LSP owns everything else in them.

use crate::partials::matching_templates;
use crate::project::Project;
use crate::rails_paths::app_root;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static LAYOUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*layout\s*\(?\s*["']([\w/]+)["']"#).unwrap());
static ACTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*def\s+([a-z_]\w*)\s*(?:$|[(;#])").unwrap());
// `render :edit`, `render "users/show"`, `render action: :edit`, `render :template => "x"`, `render partial: "form"`.
static RENDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\brender\s*\(?\s*(?::?(action|template|partial)\s*(?::|=>)\s*)?(?::(\w+)|["']([\w/]+)["'])"#).unwrap());

/// `app/controllers/admin/users_controller.rb` → `admin/users`.
pub fn controller_path(file: &Path) -> Option<String> {
    let rel = file.strip_prefix(app_root(file)?.join("controllers")).ok()?;
    let rel = rel.to_str()?.replace('\\', "/");
    rel.strip_suffix("_controller.rb").filter(|p| !p.is_empty()).map(str::to_string)
}

/// Byte ranges on `line` of a controller that name views, with the files they name. `col` keeps only the
/// reference under the cursor; links (`col` = None) leave out action names, which every `def` would carry.
pub fn targets(project: &Project, file: &Path, line: &str, col: Option<usize>) -> Vec<(usize, usize, Vec<PathBuf>)> {
    let (Some(app), Some(controller)) = (app_root(file), controller_path(file)) else { return vec![] };
    let views = app.join("views");
    let at = |s: usize, e: usize| col.is_none_or(|c| (s..=e).contains(&c));
    let in_views = |name: &str, partial: bool| {
        let (dir, base) = match name.rsplit_once('/') {
            Some((dir, base)) => (views.join(dir), base),
            None => (views.join(&controller), name),
        };
        matching_templates(project, &dir, &if partial { format!("_{base}") } else { base.to_string() })
    };
    let mut out = Vec::new();
    if let Some(m) = LAYOUT.captures(line).and_then(|cap| cap.get(1)).filter(|m| at(m.start(), m.end())) {
        out.push((m.start(), m.end(), matching_templates(project, &views.join("layouts"), m.as_str())));
    }
    if let Some(m) = ACTION.captures(line).and_then(|cap| cap.get(1)).filter(|m| col.is_some() && at(m.start(), m.end())) {
        out.push((m.start(), m.end(), in_views(m.as_str(), false)));
    }
    for cap in RENDER.captures_iter(line) {
        let Some(m) = cap.get(2).or(cap.get(3)).filter(|m| at(m.start(), m.end())) else { continue };
        out.push((m.start(), m.end(), in_views(m.as_str(), cap.get(1).is_some_and(|k| k.as_str() == "partial"))));
    }
    out.retain(|(_, _, found)| !found.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_views() {
        let root = std::env::temp_dir().join(format!("rt-controllers-{}", std::process::id()));
        for f in [
            "app/views/layouts/admin.html.erb",
            "app/views/admin/users/show.html.erb",
            "app/views/admin/users/show.json.jbuilder",
            "app/views/admin/users/edit.html.slim",
            "app/views/admin/users/_form.html.erb",
            "app/views/shared/missing.html.haml",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let file = root.join("app/controllers/admin/users_controller.rb");
        assert_eq!(controller_path(&file).as_deref(), Some("admin/users"));
        let names = |line: &str, col: Option<usize>| -> Vec<(String, Vec<String>)> {
            targets(&Project::unknown(), &file, line, col)
                .into_iter()
                .map(|(s, e, found)| {
                    let rel = found.iter().map(|f| f.strip_prefix(root.join("app/views")).unwrap().to_string_lossy().replace('\\', "/"));
                    (line[s..e].to_string(), rel.collect())
                })
                .collect()
        };
        assert_eq!(names(r#"  layout "admin""#, None), [("admin".into(), vec!["layouts/admin.html.erb".to_string()])]);
        assert_eq!(
            names("  def show", Some(8)),
            [("show".into(), vec!["admin/users/show.html.erb".into(), "admin/users/show.json.jbuilder".into()])]
        );
        assert!(names("  def show", None).is_empty());
        assert!(names("  def show_all", Some(8)).is_empty());
        assert_eq!(
            names("    render :edit, status: :unprocessable_entity", None),
            [("edit".into(), vec!["admin/users/edit.html.slim".into()])]
        );
        assert_eq!(names(r#"render action: :edit"#, None)[0].0, "edit");
        assert_eq!(names(r#"render :template => "shared/missing""#, None)[0].0, "shared/missing");
        assert_eq!(names(r#"render partial: "form""#, None)[0].1, ["admin/users/_form.html.erb"]);
        for line in ["render json: @user", "render plain: \"ok\"", "layout :choose_layout", "render :nope"] {
            assert!(names(line, None).is_empty(), "{line}");
        }
        assert!(controller_path(&root.join("app/models/user.rb")).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
