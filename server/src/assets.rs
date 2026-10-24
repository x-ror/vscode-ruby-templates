//! Literal asset helper paths (`image_tag "logo.png"`, `stylesheet_link_tag "application"`) → files.

use crate::partials::app_root;
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};

// `#` keeps interpolated strings out.
static ASSET_CALL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"\b(image_tag|image_path|asset_path|stylesheet_link_tag|javascript_include_tag|favicon_link_tag)\s*\(?\s*["']([^"'#]+)["']"#).unwrap()
});
const ASSET_DIRS: [&str; 6] =
    // Sources before app/assets/builds, so `javascript_include_tag "application"` opens what was written, not the bundle.
    ["app/assets/images", "app/assets/stylesheets", "app/assets/javascripts", "app/javascript", "app/assets/builds", "public"];

pub struct AssetRef<'a> {
    pub helper: &'a str,
    pub path: &'a str,
    /// Byte range of the string literal (without quotes) inside the line.
    pub start: usize,
    pub end: usize,
}

// ponytail: only the first argument; `stylesheet_link_tag "a", "b"` links `a`.
pub fn asset_refs(line: &str) -> Vec<AssetRef<'_>> {
    ASSET_CALL.captures_iter(line).filter_map(|cap| {
        let (helper, path) = (cap.get(1)?.as_str(), cap.get(2)?);
        Some(AssetRef { helper, path: path.as_str(), start: path.start(), end: path.end() })
    }).collect()
}

/// Asset directories for relative paths, public/ for `/absolute` ones; nothing for URLs or paths leaving those dirs.
pub fn resolve(current: &Path, r: &AssetRef) -> Vec<PathBuf> {
    if r.path.starts_with("//") || r.path.contains(':') || r.path.split(['/', '\\']).any(|s| s == "..") { return vec![] }
    let Some(rails) = app_root(current).and_then(|app| app.parent().map(Path::to_path_buf)) else { return vec![] };
    let (dirs, rel) = match r.path.strip_prefix('/') {
        Some(rel) => (vec![rails.join("public")], rel),
        None => (ASSET_DIRS.iter().map(|d| rails.join(d)).collect(), r.path),
    };
    // Rails appends the helper's extension unless the path already has it; Sprockets/dartsass compile the sources.
    let names: Vec<String> = match r.helper {
        "stylesheet_link_tag" => {
            let base = rel.strip_suffix(".css").unwrap_or(rel);
            ["css", "scss", "sass"].iter().map(|ext| format!("{base}.{ext}")).collect()
        }
        "javascript_include_tag" => vec![format!("{}.js", rel.strip_suffix(".js").unwrap_or(rel))],
        _ => vec![rel.to_string()],
    };
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).filter(|p| p.is_file()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_asset_helper_paths() {
        let root = std::env::temp_dir().join(format!("rt-assets-{}", std::process::id()));
        for f in ["app/assets/images/logo.png", "app/assets/stylesheets/application.scss", "app/assets/builds/application.css",
                  "app/javascript/app.js", "app/javascript/application.js", "app/assets/builds/application.js", "public/favicon.ico",
                  "config/master.key", "public/robots.txt", "app/views/home/index.html.erb"] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let current = root.join("app/views/home/index.html.erb");
        let line = concat!(
            r#"<%= image_tag "logo.png" %> <%= stylesheet_link_tag "application", "data-turbo-track": "reload" %> "#,
            r#"<%= javascript_include_tag("app") %> <%= favicon_link_tag "favicon.ico" %> <%= asset_path "/robots.txt" %> "#,
            r##"<%= image_tag "https://x.io/a.png" %> <%= image_tag "//cdn/logo.png" %> <%= image_tag "/logo.png" %> <%= image_tag "#{x}.png" %>"##,
        );
        let refs = asset_refs(line);
        assert_eq!(refs.iter().map(|r| r.path).collect::<Vec<_>>(),
                   ["logo.png", "application", "app", "favicon.ico", "/robots.txt", "https://x.io/a.png", "//cdn/logo.png", "/logo.png"]);
        assert_eq!(&line[refs[0].start..refs[0].end], "logo.png");
        let found: Vec<Vec<PathBuf>> = refs.iter().map(|r| resolve(&current, r)).collect();
        assert_eq!(found[0], [root.join("app/assets/images/logo.png")]);
        assert_eq!(found[1], [root.join("app/assets/stylesheets/application.scss"), root.join("app/assets/builds/application.css")]);
        assert_eq!(found[2], [root.join("app/javascript/app.js")]);
        assert_eq!(found[3], [root.join("public/favicon.ico")]);
        assert_eq!(found[4], [root.join("public/robots.txt")]);
        assert!(found[5..].iter().all(Vec::is_empty));
        let at = |helper, path| resolve(&current, &AssetRef { helper, path, start: 0, end: 0 });
        assert_eq!(at("javascript_include_tag", "application"),
                   [root.join("app/javascript/application.js"), root.join("app/assets/builds/application.js")]);
        for escape in ["../../config/master.key", "/../config/master.key", "..\\..\\config\\master.key", "x/../../../config/master.key"] {
            assert!(at("asset_path", escape).is_empty(), "{escape}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
