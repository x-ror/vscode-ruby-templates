//! Stimulus identifiers in `data-*` attributes → app/javascript/controllers files.

use crate::partials::app_root;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

// `data-controller="a b"`, `"data-action": "…"`, `"data-controller" => "…"`.
static ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"data-(controller|action)["']?\s*(?:=>|=|:)\s*["']([^"']*)["']"#).unwrap());
static TARGET_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"data-([\w-]+)-target\b").unwrap());
// `data: { controller: "x", action: "…", x_target: "…" }` in helpers and Haml.
static DATA_HASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bdata:\s*\{([^}]*)\}").unwrap());
static HASH_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\b(?:(controller|action):\s*["']([^"']*)["']|(\w+)_target:)"#).unwrap());

// An unfinished `data-controller="…`, `data-action="…`, or `controller:`/`action:` value in a `data:` hash.
static OPEN_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:data-(controller|action)["']?\s*(?:=>|=|:)|\bdata:\s*\{[^}]*\b(controller|action):)\s*["']([^"']*)$"#).unwrap()
});

/// The identifier being typed at byte `col`: its start and the typed part. In an action value only the part
/// after `->` and before `#` names a controller.
pub fn completion_prefix(line: &str, col: usize) -> Option<(usize, &str)> {
    let before = line.get(..col)?;
    let cap = OPEN_VALUE.captures(before)?;
    let value = cap.get(3)?;
    let token_start = value.as_str().rfind(char::is_whitespace).map_or(0, |i| i + 1);
    let token = &value.as_str()[token_start..];
    let is_action = cap.get(1).or(cap.get(2))?.as_str() == "action";
    let skip = if is_action { token.rfind("->").map_or(0, |i| i + 2) } else { 0 };
    let typed = &token[skip..];
    if is_action && typed.contains(['#', ':']) || !typed.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return None;
    }
    Some((value.start() + token_start + skip, typed))
}

/// The identifiers stimulus-loading registers for the files under app/javascript/controllers.
pub fn controllers(current: &Path) -> Vec<String> {
    let Some(app) = app_root(current) else { return vec![] };
    let dir = app.join("javascript/controllers");
    let mut out: Vec<String> = WalkDir::new(&dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = e.path().strip_prefix(&dir).ok()?.to_str()?.replace('\\', "/");
            let stem = ["_controller.js", "_controller.ts", "-controller.js", "-controller.ts"].iter().find_map(|s| rel.strip_suffix(s))?;
            Some(stem.replace('/', "--").replace('_', "-"))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Every controller identifier on `line`: its byte range and the identifier Stimulus would register.
pub fn identifiers(line: &str) -> Vec<(usize, usize, String)> {
    let named = |(s, e): (usize, usize)| (s, e, line[s..e].to_string());
    let mut out: Vec<_> = ATTR
        .captures_iter(line)
        .flat_map(|cap| cap.get(2).map(|value| value_identifiers(&cap[1], value)).unwrap_or_default())
        .chain(TARGET_ATTR.captures_iter(line).filter_map(|cap| cap.get(1)).map(|m| (m.start(), m.end())))
        .map(named)
        .collect();
    for hash in DATA_HASH.captures_iter(line).filter_map(|cap| cap.get(1)) {
        let at = |(s, e): (usize, usize)| (hash.start() + s, hash.start() + e);
        for cap in HASH_KEY.captures_iter(hash.as_str()) {
            match (cap.get(1), cap.get(2), cap.get(3)) {
                (Some(key), Some(value), _) => out.extend(value_identifiers(key.as_str(), value).into_iter().map(at).map(named)),
                // Rails dasherizes the key: `date_picker_target:` → `data-date-picker-target`.
                (_, _, Some(target)) => {
                    let (s, e) = at((target.start(), target.end()));
                    out.push((s, e, target.as_str().replace('_', "-")));
                }
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// Identifiers in a `controller` value (space-separated) or an `action` value (`event->id#method` descriptors).
fn value_identifiers(key: &str, value: regex::Match) -> Vec<(usize, usize)> {
    let (offset, value) = (value.start(), value.as_str());
    let mut out = Vec::new();
    let mut pos = 0;
    for token in value.split_whitespace() {
        let start = pos + value[pos..].find(token).unwrap_or(0);
        pos = start + token.len();
        let (s, e) = if key == "controller" {
            (0, token.len())
        } else {
            let Some(hash) = token.find('#') else { continue };
            (token[..hash].rfind("->").map_or(0, |i| i + 2), hash)
        };
        if s < e {
            out.push((offset + start + s, offset + start + e));
        }
    }
    out
}

/// `date-picker` → `date{_,-}picker{_,-}controller.{js,ts}`; `admin--users` → `admin/users_…`.
/// Mirrors stimulus-loading, which registers `a/b_c_controller.js` as `a--b-c`, so `_` never appears in an identifier.
pub fn resolve(current: &Path, identifier: &str) -> Vec<PathBuf> {
    if identifier.is_empty() || !identifier.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return vec![];
    }
    let Some(app) = app_root(current) else { return vec![] };
    let dir = app.join("javascript/controllers");
    let rel = identifier.replace("--", "/");
    let mut out = Vec::new();
    for name in [rel.replace('-', "_"), rel] {
        for file in
            ["_controller.js", "_controller.ts", "-controller.js", "-controller.ts"].map(|suffix| dir.join(format!("{name}{suffix}")))
        {
            if file.is_file() && !out.contains(&file) {
                out.push(file);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_in_attributes_and_hashes() {
        let ids = |line: &str| identifiers(line).into_iter().map(|(_, _, id)| id).collect::<Vec<_>>();
        assert_eq!(
            ids(r#"<div data-controller="admin--users hello" data-action="click->hello#greet keydown.enter->x#y:prevent z#w">"#),
            ["admin--users", "hello", "hello", "x", "z"]
        );
        assert_eq!(ids(r#"<input data-date-picker-target="name">"#), ["date-picker"]);
        assert_eq!(
            ids(r#"<%= tag.div data: { controller: "hello", action: "hello#go", date_picker_target: "x" } do %>"#),
            ["hello", "hello", "date-picker"]
        );
        let line = r#"<p data: { date_picker_target: "x" }>"#;
        let (s, e, _) = identifiers(line)[0];
        assert_eq!(&line[s..e], "date_picker");
        assert_eq!(ids(r#"%div{ "data-controller" => "hello" }"#), ["hello"]);
        assert_eq!(ids(r#"div data-controller="hello""#), ["hello"]);
        assert!(ids(r#"<%= link_to "x", controller: "users", action: "index" %>"#).is_empty());
        assert!(ids(r#"<div data-action="click->nope">"#).is_empty());
    }

    #[test]
    fn completion_prefixes() {
        let at_end = |line: &str| completion_prefix(line, line.len()).map(|(s, typed)| (s, typed.to_string()));
        assert_eq!(at_end(r#"<div data-controller="he"#), Some((22, "he".into())));
        assert_eq!(at_end(r#"<div data-controller="hello da"#), Some((28, "da".into())));
        assert_eq!(at_end(r#"<div data-controller=""#), Some((22, "".into())));
        assert_eq!(at_end(r#"<div data-action="click->he"#), Some((25, "he".into())));
        assert_eq!(at_end(r#"<div data-action="he"#), Some((18, "he".into())));
        assert_eq!(at_end(r#"%div{data: {controller: "x", action: "click->y"#), Some((45, "y".into())));
        assert_eq!(at_end(r#"div data-controller='a--"#), Some((21, "a--".into())));
        for line in [r#"<div data-action="click->hello#gr"#, r#"<div data-controller="x">"#, r#"<%= link_to "x", controller: "us"#] {
            assert_eq!(at_end(line), None, "{line}");
        }
    }

    #[test]
    fn resolves_controller_files() {
        let root = std::env::temp_dir().join(format!("rt-stimulus-{}", std::process::id()));
        let controllers = root.join("app/javascript/controllers");
        std::fs::create_dir_all(controllers.join("admin")).unwrap();
        std::fs::create_dir_all(root.join("app/views/users")).unwrap();
        for f in [
            "hello_controller.js",
            "admin/users_controller.ts",
            "date_picker_controller.js",
            "color-picker_controller.js",
            "tab-list-controller.ts",
        ] {
            std::fs::write(controllers.join(f), "").unwrap();
        }
        let current = root.join("app/views/users/index.html.erb");
        assert_eq!(resolve(&current, "hello"), [controllers.join("hello_controller.js")]);
        assert_eq!(resolve(&current, "admin--users"), [controllers.join("admin/users_controller.ts")]);
        assert_eq!(resolve(&current, "date-picker"), [controllers.join("date_picker_controller.js")]);
        assert_eq!(resolve(&current, "color-picker"), [controllers.join("color-picker_controller.js")]);
        assert_eq!(resolve(&current, "tab-list"), [controllers.join("tab-list-controller.ts")]);
        assert_eq!(super::controllers(&current), ["admin--users", "color-picker", "date-picker", "hello", "tab-list"]);
        std::fs::create_dir_all(controllers.join("a")).unwrap();
        std::fs::write(controllers.join("a/b_controller.js"), "").unwrap();
        std::fs::write(root.join("app/x_controller.js"), "").unwrap();
        assert_eq!(resolve(&current, "a--b"), [controllers.join("a/b_controller.js")]);
        // Stimulus registers these files under other identifiers (`date-picker`, `a--b`); paths must stay inside controllers/.
        for id in ["missing", "date_picker", "a/b", "../x", "../../app/x", "Hello", ""] {
            assert!(resolve(&current, id).is_empty(), "{id}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
