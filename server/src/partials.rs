//! Resolve `render "partial"` and friends to files; list partials for completion.

use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

static RENDER_STR: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"render\s*\(?\s*(?:(partial|template|layout)\s*(?::|=>)\s*)?["']([\w./-]+)["']"#).unwrap()
});
static RENDER_COMPONENT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"render\s*\(?\s*((?:[A-Z]\w*::)*[A-Z]\w*Component)\b").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderKind { Partial, Template, Layout }

#[derive(Debug, Clone)]
pub struct RenderRef {
    pub kind: RenderKind,
    pub name: String,
    /// Byte range of the string literal (without quotes) inside the line.
    pub start: usize,
    pub end: usize,
}

/// Every `render "..."` reference on a line.
pub fn render_refs(line: &str) -> Vec<RenderRef> {
    RENDER_STR.captures_iter(line).filter_map(|cap| {
        let m = cap.get(2)?;
        let kind = match cap.get(1).map(|k| k.as_str()) {
            Some("template") => RenderKind::Template,
            Some("layout") => RenderKind::Layout,
            _ => RenderKind::Partial,
        };
        Some(RenderRef { kind, name: m.as_str().to_string(), start: m.start(), end: m.end() })
    }).collect()
}

/// Find a render reference whose string spans `col` (byte column) on `line`.
pub fn render_ref_at(line: &str, col: usize) -> Option<RenderRef> {
    render_refs(line).into_iter().find(|r| col >= r.start.saturating_sub(1) && col <= r.end + 1)
}

pub fn component_ref_at(line: &str, col: usize) -> Option<String> {
    for cap in RENDER_COMPONENT.captures_iter(line) {
        let m = cap.get(1)?;
        if col >= m.start() && col <= m.end() {
            return Some(m.as_str().to_string());
        }
    }
    None
}

/// Walk up from `file` to find the Rails `app/views` directory.
pub fn views_root(file: &Path) -> Option<PathBuf> {
    let mut dir = file.parent()?;
    loop {
        if dir.file_name().map(|n| n == "views").unwrap_or(false)
            && dir.parent().and_then(|p| p.file_name()).map(|n| n == "app").unwrap_or(false)
        {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

pub fn app_root(file: &Path) -> Option<PathBuf> {
    views_root(file).and_then(|v| v.parent().map(|p| p.to_path_buf()))
}

/// All template files matching `dir/<prefix>name.*`.
fn matching_templates(dir: &Path, basename: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let fname = p.file_name().and_then(|f| f.to_str()).unwrap_or("");
            fname == basename || fname.starts_with(&format!("{}.", basename))
        })
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found
}

/// Resolve a render reference to candidate files.
pub fn resolve(current: &Path, r: &RenderRef) -> Vec<PathBuf> {
    let views = views_root(current);
    let (dir, base) = match r.name.rsplit_once('/') {
        Some((d, b)) => (Some(d.to_string()), b.to_string()),
        None => (None, r.name.clone()),
    };
    let basename = match r.kind {
        RenderKind::Partial => format!("_{}", base),
        _ => base.clone(),
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    match (&dir, &views, r.kind) {
        (Some(d), Some(v), RenderKind::Layout) => dirs.push(v.join("layouts").join(d)),
        (None, Some(v), RenderKind::Layout) => dirs.push(v.join("layouts")),
        (Some(d), Some(v), _) => dirs.push(v.join(d)),
        (Some(d), None, _) => {
            if let Some(p) = current.parent() { dirs.push(p.join(d)); }
        }
        (None, _, _) => {
            if let Some(p) = current.parent() { dirs.push(p.to_path_buf()); }
            if let Some(v) = &views { dirs.push(v.join("application")); }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        out.extend(matching_templates(&d, &basename));
    }
    out
}

/// `Users::CardComponent` → app/components/users/card_component.rb (+ sidecar templates).
pub fn resolve_component(current: &Path, name: &str) -> Vec<PathBuf> {
    let Some(app) = app_root(current) else { return vec![] };
    let rel: PathBuf = name.split("::").map(snake_case).collect();
    let base = app.join("components").join(&rel);
    let mut out = Vec::new();
    let rb = base.with_extension("rb");
    if rb.exists() { out.push(rb); }
    if let (Some(dir), Some(stem)) = (base.parent(), base.file_name().and_then(|s| s.to_str())) {
        out.extend(matching_templates(dir, stem).into_iter().filter(|p| p.extension().map(|e| e != "rb").unwrap_or(true)));
    }
    out
}

fn snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 { out.push('_'); }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Partial names available for completion: same-directory partials first (bare name),
/// then every partial under app/views as `dir/name`.
pub fn partial_candidates(current: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    if let Some(dir) = current.parent() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if let Some(name) = partial_name(&p) {
                    out.push((name, p));
                }
            }
        }
    }
    if let Some(views) = views_root(current) {
        for e in WalkDir::new(&views).min_depth(1).max_depth(6).into_iter().flatten() {
            let p = e.path();
            if !p.is_file() { continue; }
            let Some(name) = partial_name(p) else { continue };
            let rel_dir = p.parent().and_then(|d| d.strip_prefix(&views).ok()).map(|d| d.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            if rel_dir.is_empty() { continue; }
            out.push((format!("{}/{}", rel_dir, name), p.to_path_buf()));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn partial_name(p: &Path) -> Option<String> {
    if !p.is_file() { return None; }
    let fname = p.file_name()?.to_str()?;
    let stem = fname.strip_prefix('_')?;
    let name = stem.split('.').next()?;
    if name.is_empty() { None } else { Some(name.to_string()) }
}

/// Is the cursor inside the string of a `render "..."` call? Returns the typed prefix.
pub fn render_string_prefix(line: &str, col: usize) -> Option<String> {
    static OPEN: Lazy<Regex> = Lazy::new(|| Regex::new(r#"render\s*\(?\s*(?:(?:partial|template|layout)\s*(?::|=>)\s*)?["']([\w./-]*)$"#).unwrap());
    let before = line.get(..col.min(line.len()))?;
    OPEN.captures(before).map(|c| c[1].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refs() {
        let l = r#"<%= render "users/card", user: u %>"#;
        let r = render_ref_at(l, 14).unwrap();
        assert_eq!(r.name, "users/card");
        assert_eq!(r.kind, RenderKind::Partial);
        let r = render_ref_at("= render partial: 'row', collection: @x", 20).unwrap();
        assert_eq!(r.name, "row");
        assert_eq!(render_ref_at("= render template: 'a/b'", 22).unwrap().kind, RenderKind::Template);
        assert_eq!(component_ref_at("= render Users::CardComponent.new(user: u)", 12).unwrap(), "Users::CardComponent");
        assert_eq!(render_string_prefix(r#"<%= render "us"#, 14).unwrap(), "us");
        assert_eq!(snake_case("CardComponent"), "card_component");
    }
}
