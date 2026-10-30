//! Resolve `render "partial"` and friends to files; list partials for completion.

use crate::project::Project;
use crate::rails_paths::{app_root, template_format, template_stem, template_suffix, views_root};
use crate::strict_locals::{self, Close};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

static RENDER_STR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:render|json\.partial!)\s*\(?\s*(?::?(partial|template|layout)\s*(?::|=>)\s*)?["']([\w./-]+)["']"#).unwrap()
});
// Jbuilder's `json.array! @posts, partial: "posts/post", as: :post` and `json.comments @c, partial: "…"`.
// ponytail: stops at any `#`, so a `partial:` after `"#x"` or an interpolation is missed (no ref, no warning).
static JSON_PARTIAL_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"json\.\w+!?[\s(][^#]*?\bpartial\s*(?::|=>)\s*["']([\w./-]+)["']"#).unwrap());
// `render partial:|template: "y", layout: "x"`; group 2 must be balanced so `layout:` is a top-level key.
// After a bare `render "y", layout: …` it's just a local.
static RENDER_LAYOUT_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"render\s*\(?\s*:?(partial|template)\s*(?::|=>)([^%]*?),\s*:?layout\s*(?::|=>)\s*["']([\w./-]+)["']"#).unwrap()
});
// `[^:]` keeps `Card::DEFAULTS` from reading as a `Card:` keyword.
static KEYWORD_ARG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)^(\w+):(|[^:].*)$").unwrap());
static RENDER_COMPONENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"render\s*\(?\s*((?:[A-Z]\w*::)*[A-Z]\w*Component)\b").unwrap());
static RENDER_MODEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"render\s*\(?\s*(@\w+(?:\.\w+)?)").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderKind {
    Partial,
    Template,
}

impl RenderKind {
    pub fn name(self) -> &'static str {
        match self {
            RenderKind::Partial => "Partial",
            RenderKind::Template => "Template",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RenderRef {
    pub kind: RenderKind,
    pub name: String,
    /// Byte range of the string literal (without quotes) inside the line.
    pub start: usize,
    pub end: usize,
    /// Byte start of the call (`render`, `json.partial!`, `json.array!`) the string belongs to.
    pub call: usize,
    /// The `layout:` of a `render partial:|template:` call; it receives that call's locals.
    pub from_layout: bool,
}

/// Every `render "..."` reference on a line, in order.
/// ActionView 7.0 to 8.1: a view's `render layout: "x"` (with a block, or next to `partial:`) renders the `_x` partial;
/// next to `template:` the layout is a template path.
pub fn render_refs(line: &str) -> Vec<RenderRef> {
    let render_ref = |call: regex::Match, kind: Option<regex::Match>, m: regex::Match, from_layout| {
        let kind = if kind.is_some_and(|k| k.as_str() == "template") { RenderKind::Template } else { RenderKind::Partial };
        RenderRef { kind, name: m.as_str().to_string(), start: m.start(), end: m.end(), call: call.start(), from_layout }
    };
    let calls = RENDER_STR.captures_iter(line).filter_map(|cap| Some(render_ref(cap.get(0)?, cap.get(1), cap.get(2)?, false)));
    // ponytail: a rejected `layout:` inside `{…}` hides a later top-level one on the same line.
    let layouts = RENDER_LAYOUT_KEY
        .captures_iter(line)
        .filter(|cap| split_args(&cap[2]).is_some())
        .filter_map(|cap| Some(render_ref(cap.get(0)?, cap.get(1), cap.get(3)?, true)));
    let json = JSON_PARTIAL_KEY.captures_iter(line).filter_map(|cap| Some(render_ref(cap.get(0)?, None, cap.get(1)?, false)));
    let mut refs: Vec<RenderRef> = calls.chain(layouts).chain(json).collect();
    refs.sort_by_key(|r| r.start);
    // `json.partial! partial: "x"` matches both RENDER_STR and JSON_PARTIAL_KEY.
    refs.dedup_by_key(|r| r.start);
    refs
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

/// `render @user` / `@users` / `@post.comments`: byte range of the expression and the name its partial follows.
/// Longer chains and calls (`@posts.page(1)`, `@post.comments.order(:id)`) name a query, not a model.
pub fn model_renders(line: &str) -> Vec<(usize, usize, &str)> {
    RENDER_MODEL
        .captures_iter(line)
        .filter_map(|cap| {
            let m = cap.get(1)?;
            if line[m.end()..].starts_with(['.', '(']) {
                return None;
            }
            Some((m.start(), m.end(), m.as_str().rsplit(['.', '@']).next()?))
        })
        .collect()
}

/// `user`/`users` → app/views/users/_user.* (`to_partial_path`); nothing when the inflection finds no file.
// ponytail: namespaced models (`Admin::User` → admin/users/_user) aren't knowable from the variable.
pub fn resolve_model(project: &Project, current: &Path, name: &str) -> Vec<PathBuf> {
    let Some(views) = views_root(current) else { return vec![] };
    let singular = singularize(name);
    matching_templates(project, &views.join(pluralize(&singular)), &format!("_{singular}"))
        .into_iter()
        .filter(|p| renders_format(project, current, p))
        .collect()
}

/// Where a missing `dir/name` reference would live: `app/views/dir/_name.<format>.<ext>` in the view's format
/// (variant dropped) and template language, so the view finds it.
pub fn new_template_path(project: &Project, current: &Path, r: &RenderRef) -> Option<PathBuf> {
    let (dir, base) = r.name.rsplit_once('/')?;
    if base.is_empty() || dir.split('/').any(|part| part == "..") {
        return None;
    }
    let ext = current.extension()?.to_str()?;
    let prefix = if r.kind == RenderKind::Partial { "_" } else { "" };
    let file = match template_format(|f| project.is_format(f), current) {
        Some(format) => format!("{prefix}{base}.{format}.{ext}"),
        None => format!("{prefix}{base}.{ext}"),
    };
    Some(views_root(current)?.join(dir).join(file))
}

/// The `resolve` results Rails would actually pick from: it stops at the first folder with a match, so later
/// ones (`application/`) are shadowed.
pub fn first_folder_only(mut found: Vec<PathBuf>) -> Vec<PathBuf> {
    let first = found.first().and_then(|f| f.parent()).map(Path::to_path_buf);
    found.retain(|f| f.parent() == first.as_deref());
    found
}

/// Rails looks a partial up in the view's format only, plus HTML from JS (`LookupContext#formats=`);
/// format-less files match any.
// ponytail: turbo_stream gets the JS fallback too, so its HTML partials aren't flagged unresolved.
fn renders_format(project: &Project, current: &Path, candidate: &Path) -> bool {
    let is_format = |f: &str| project.is_format(f);
    match (template_format(is_format, current), template_format(is_format, candidate)) {
        (Some(view), Some(partial)) => view == partial || (partial == "html" && matches!(view, "js" | "turbo_stream")),
        _ => true,
    }
}

// Suffix-matched like ActiveSupport's irregulars, so `sales_person` → `sales_people`.
const IRREGULAR: [(&str, &str); 10] = [
    ("person", "people"),
    ("child", "children"),
    ("movie", "movies"),
    ("status", "statuses"),
    ("alias", "aliases"),
    ("cookie", "cookies"),
    ("hero", "heroes"),
    ("quiz", "quizzes"),
    ("cache", "caches"),
    ("analysis", "analyses"),
];
const UNCOUNTABLE: [&str; 9] = ["equipment", "information", "rice", "money", "species", "series", "fish", "sheep", "news"];

/// Matched on the last `_` segment so `prices` isn't read as `rice`.
fn uncountable(word: &str) -> bool {
    UNCOUNTABLE.contains(&word.rsplit('_').next().unwrap_or(word))
}

fn ends_with_consonant_y(word: &str) -> bool {
    word.strip_suffix('y').and_then(|w| w.chars().last()).is_some_and(|c| !"aeiou".contains(c))
}

pub fn pluralize(word: &str) -> String {
    if uncountable(word) {
        return word.to_string();
    }
    if let Some((stem, plural)) = IRREGULAR.iter().find_map(|(s, p)| Some((word.strip_suffix(s)?, p))) {
        return format!("{stem}{plural}");
    }
    if ends_with_consonant_y(word) {
        return format!("{}ies", &word[..word.len() - 1]);
    }
    if ["s", "x", "z", "ch", "sh"].iter().any(|e| word.ends_with(e)) {
        return format!("{word}es");
    }
    format!("{word}s")
}

pub fn singularize(word: &str) -> String {
    if uncountable(word) {
        return word.to_string();
    }
    for (singular, plural) in IRREGULAR {
        if let Some(stem) = word.strip_suffix(plural) {
            return format!("{stem}{singular}");
        }
        if word.ends_with(singular) {
            return word.to_string();
        }
    }
    if let Some(stem) = word.strip_suffix("ies").filter(|s| ends_with_consonant_y(&format!("{s}y"))) {
        return format!("{stem}y");
    }
    if let Some(stem) = word.strip_suffix("es").filter(|s| ["x", "ch", "ss", "sh", "zz"].iter().any(|e| s.ends_with(e))) {
        return stem.to_string();
    }
    // `ss|us|is` end singulars (`address`, `bus`, `axis`); their plurals took `es` above or just miss.
    match word.strip_suffix('s') {
        Some(stem) if !["ss", "us", "is"].iter().any(|e| word.ends_with(e)) => stem.to_string(),
        _ => word.to_string(),
    }
}

/// All template files matching `dir/<prefix>name.*`.
pub fn matching_templates(project: &Project, dir: &Path, basename: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let fname = p.file_name().and_then(|f| f.to_str()).unwrap_or("");
            template_suffix(|f| project.is_format(f), fname, basename).is_some()
        })
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found
}

/// Resolve a render reference to candidate files.
pub fn resolve(project: &Project, current: &Path, r: &RenderRef) -> Vec<PathBuf> {
    let views = views_root(current);
    let (dir, base) = match r.name.rsplit_once('/') {
        Some((d, b)) => (Some(d.to_string()), b.to_string()),
        None => (None, r.name.clone()),
    };
    let basename = match r.kind {
        RenderKind::Partial => format!("_{}", base),
        RenderKind::Template => base.clone(),
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    match (&dir, &views, r.kind) {
        (Some(d), Some(v), _) => dirs.push(v.join(d)),
        (Some(d), None, _) => {
            if let Some(p) = current.parent() {
                dirs.push(p.join(d));
            }
        }
        (None, _, _) => {
            if let Some(p) = current.parent() {
                dirs.push(p.to_path_buf());
            }
            if let Some(v) = &views {
                dirs.push(v.join("application"));
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        out.extend(matching_templates(project, &d, &basename).into_iter().filter(|p| renders_format(project, current, p)));
    }
    out
}

/// `Users::CardComponent` → app/components/users/card_component.rb (+ sidecar templates).
pub fn resolve_component(project: &Project, current: &Path, name: &str) -> Vec<PathBuf> {
    let Some(app) = app_root(current) else { return vec![] };
    let rel: PathBuf = name.split("::").map(snake_case).collect();
    let base = app.join("components").join(&rel);
    let mut out = Vec::new();
    let rb = base.with_extension("rb");
    if rb.exists() {
        out.push(rb);
    }
    if let (Some(dir), Some(stem)) = (base.parent(), base.file_name().and_then(|s| s.to_str())) {
        out.extend(matching_templates(project, dir, stem).into_iter().filter(|p| p.extension().map(|e| e != "rb").unwrap_or(true)));
    }
    out
}

/// ViewComponent classes under app/components, by the constant `render` takes (`admin/user_card_component.rb` →
/// `Admin::UserCardComponent`), with their file.
pub fn component_classes(current: &Path) -> Vec<(String, PathBuf)> {
    let Some(app) = app_root(current) else { return vec![] };
    let dir = app.join("components");
    let mut out: Vec<(String, PathBuf)> = WalkDir::new(&dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = e.path().strip_prefix(&dir).ok()?.to_str()?.replace('\\', "/");
            let name = rel.strip_suffix("_component.rb")?;
            let constant: Vec<String> = format!("{name}_component").split('/').map(camel_case).collect();
            Some((constant.join("::"), e.into_path()))
        })
        .collect();
    out.sort();
    out
}

/// `user_card` → `UserCard`.
fn camel_case(s: &str) -> String {
    s.split('_')
        .flat_map(|w| {
            let mut chars = w.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>())
        })
        .collect()
}

pub fn snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Names `render` can complete for `kind`: partials (same-dir bare names, then `dir/name` under app/views)
/// or non-partial templates as `dir/name`. `files` lists app/views, or the current directory outside one.
pub fn render_candidates(project: &Project, current: &Path, kind: RenderKind, files: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let views = views_root(current);
    let partial = kind == RenderKind::Partial;
    let same_dir = current
        .parent()
        .filter(|_| partial)
        .into_iter()
        .flat_map(|d| templates_in(project, d, files, true))
        .filter(|(name, _)| !name.contains('/'));
    let nested = views.iter().flat_map(|v| templates_in(project, v, files, partial)).filter(|(name, _)| name.contains('/'));
    // Same filter as `resolve`: a name it would not find isn't worth offering.
    let mut out: Vec<_> = same_dir.chain(nested).filter(|(_, p)| renders_format(project, current, p)).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

/// Every partial and template among `files` under `views`, by the name `render` uses for it.
pub fn named_templates(project: &Project, views: &Path, files: &[PathBuf]) -> Vec<(String, RenderKind, PathBuf)> {
    let partials = templates_in(project, views, files, true).map(|(name, p)| (name, RenderKind::Partial, p));
    let templates = templates_in(project, views, files, false).map(|(name, p)| (name, RenderKind::Template, p));
    partials.chain(templates).collect()
}

/// Partials (`_x.*`) or non-partial templates among `files` under `root`, named `dir/x` relative to `root`.
fn templates_in<'a>(
    project: &'a Project,
    root: &'a Path,
    files: &'a [PathBuf],
    partial: bool,
) -> impl Iterator<Item = (String, PathBuf)> + 'a {
    files.iter().filter_map(move |p| {
        let name = template_name(project, p, partial)?;
        let rel_dir = p.parent()?.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
        let full = if rel_dir.is_empty() { name } else { format!("{rel_dir}/{name}") };
        Some((full, p.clone()))
    })
}

fn template_name(project: &Project, p: &Path, partial: bool) -> Option<String> {
    let fname = p.file_name()?.to_str()?;
    let stem = match fname.strip_prefix('_') {
        Some(stem) if partial => stem,
        None if !partial => fname,
        _ => return None,
    };
    let name = template_stem(|f| project.is_format(f), stem);
    (!name.is_empty()).then(|| name.to_string())
}

/// Is the cursor inside the string of a `render "..."` call?
/// Returns what the string names (see `render_refs` for `layout:`) and the typed prefix.
pub fn render_string_prefix(line: &str, col: usize) -> Option<(RenderKind, String)> {
    static OPEN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(concat!(
            r#"(?:(?:render|json\.partial!)\s*\(?\s*(?::?(partial|template|layout)\s*(?::|=>)\s*)?"#,
            r#"|render\s*\(?\s*:?(partial|template)\s*(?::|=>)([^%]*?),\s*:?layout\s*(?::|=>)\s*)["']([\w./-]*)$"#,
        ))
        .unwrap()
    });
    let before = line.get(..col.min(line.len()))?;
    let cap = OPEN.captures(before)?;
    if cap.get(3).is_some_and(|between| split_args(between.as_str()).is_none()) {
        return None;
    }
    let kind = if cap.get(1).or(cap.get(2)).is_some_and(|k| k.as_str() == "template") { RenderKind::Template } else { RenderKind::Partial };
    Some((kind, cap[4].to_string()))
}

/// A template's `locals: (…)` magic comment.
#[derive(Debug, PartialEq)]
pub struct StrictLocals {
    /// The parameter list as written, without the parentheses.
    pub params: String,
    /// `(name, default)`; no default means required.
    pub locals: Vec<(String, Option<String>)>,
    /// `**rest` accepts any other local.
    pub rest: bool,
}

pub fn strict_locals(text: &str, close: Close) -> Option<StrictLocals> {
    let comment = strict_locals::find(text, close)?;
    let params = if comment.blank() { "" } else { comment.parameters.trim() };
    let mut sig = StrictLocals { params: params.to_string(), locals: vec![], rest: false };
    for arg in split_args(params)? {
        // `**nil` is how Rails spells "no other locals".
        if arg.starts_with("**") && arg != "**nil" {
            sig.rest = true;
        } else if let Some((name, default)) = keyword_arg(arg) {
            sig.locals.push((name.to_string(), Some(default.to_string()).filter(|d| !d.is_empty())));
        }
    }
    Some(sig)
}

pub fn file_strict_locals(file: &Path, close: Close) -> Option<StrictLocals> {
    strict_locals(&std::fs::read_to_string(file).ok()?, close)
}

/// `name: value` → `(name, value)`; `value` is empty for `name:`.
fn keyword_arg(arg: &str) -> Option<(&str, &str)> {
    let cap = KEYWORD_ARG.captures(arg)?;
    Some((cap.get(1)?.as_str(), cap.get(2)?.as_str().trim()))
}

/// Split a Ruby argument list at top-level commas; `None` when brackets or quotes don't balance.
fn split_args(s: &str) -> Option<Vec<&str>> {
    if s.trim().is_empty() {
        return Some(vec![]);
    }
    let (mut depth, mut quote, mut start, mut out) = (0u32, None, 0, vec![]);
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match (quote, c) {
            (Some(_), '\\') => {
                chars.next();
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')' | ']' | '}') => depth = depth.checked_sub(1)?,
            (None, ',') if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 || quote.is_some() {
        return None;
    }
    out.push(s[start..].trim());
    Some(out)
}

/// Local names passed by a one-line literal `render "x", a: 1` or `render partial: "x", locals: { a: 1 }`;
/// `None` for anything less simple (splats, `collection:`, variables, continued lines).
pub fn passed_locals<'a>(line: &'a str, r: &RenderRef) -> Option<Vec<&'a str>> {
    if r.kind != RenderKind::Partial {
        return None;
    }
    let rest = line.get(r.end + 1..)?.split("%>").next()?.trim_end().trim_end_matches(['-', '=']).trim();
    // `render("x", a: 1)`: the call's closing paren is the one that doesn't balance.
    let args = split_args(rest).or_else(|| split_args(rest.strip_suffix(')')?))?;
    let args = match args.as_slice() {
        [] => &[][..],
        ["", args @ ..] => args,
        _ => return None,
    };
    let options_form = line[..r.start.saturating_sub(1)].trim_end().ends_with([':', '>']);
    let jbuilder = line[r.call..].starts_with("json.");
    let pairs = match (options_form, args) {
        // Jbuilder reads a lone `locals:` as the locals hash, like the options form.
        (false, [locals]) if jbuilder && keyword_arg(locals).is_some_and(|(name, _)| name == "locals") => {
            split_args(keyword_arg(locals)?.1.strip_prefix('{')?.strip_suffix('}')?)?
        }
        (false, args) => args.to_vec(),
        (true, []) => vec![],
        (true, [locals]) => match keyword_arg(locals)? {
            ("locals", hash) => split_args(hash.strip_prefix('{')?.strip_suffix('}')?)?,
            _ => return None,
        },
        (true, _) => return None,
    };
    let names: Vec<&str> = pairs.into_iter().map(|p| keyword_arg(p).map(|(name, _)| name)).collect::<Option<_>>()?;
    // Jbuilder takes these out of the locals; `as:` names the local the collection items arrive in.
    if jbuilder && names.iter().any(|n| JBUILDER_OPTIONS.contains(n)) {
        return None;
    }
    Some(names)
}

const JBUILDER_OPTIONS: [&str; 3] = ["collection", "as", "cached"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rails_paths::files_under;

    #[test]
    fn new_template_paths() {
        let views = Path::new("/r/app/views");
        let path = |current: &str, line: &str| new_template_path(&Project::unknown(), &views.join(current), &render_refs(line)[0]);
        assert_eq!(path("users/show.html+phone.slim", r#"= render "shared/card""#), Some(views.join("shared/_card.html.slim")));
        assert_eq!(path("users/index.json.jbuilder", r#"json.partial! "users/user""#), Some(views.join("users/_user.json.jbuilder")));
        assert_eq!(path("users/show.de.erb", r#"render template: "a/b""#), Some(views.join("a/b.erb")));
        assert_eq!(path("users/show.html.erb", r#"render template: "a/b", layout: "layouts/lt""#), Some(views.join("a/b.html.erb")));
        assert_eq!(
            new_template_path(
                &Project::unknown(),
                &views.join("users/show.html.erb"),
                &render_refs(r#"render template: "a/b", layout: "layouts/lt""#)[1]
            ),
            Some(views.join("layouts/lt.html.erb"))
        );
        assert_eq!(path("users/show.html.erb", r#"render "card""#), None);
        assert_eq!(path("users/show.html.erb", r#"render "../../config/x""#), None);
        assert_eq!(path("users/show.html.erb", r#"render "shared/""#), None);
        assert_eq!(new_template_path(&Project::unknown(), Path::new("/tmp/x.html.erb"), &render_refs(r#"render "a/b""#)[0]), None);
    }

    #[test]
    fn jbuilder_refs() {
        let names = |line: &str| render_refs(line).into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(names(r#"json.partial! "users/user", user: u"#), ["users/user"]);
        assert_eq!(names(r#"json.partial!("users/user")"#), ["users/user"]);
        assert_eq!(names(r#"json.partial! partial: 'users/user', locals: { user: u }"#), ["users/user"]);
        assert_eq!(names(r#"json.array! @users, partial: "users/user", as: :user"#), ["users/user"]);
        assert_eq!(names(r#"json.array!(@users, :partial => "users/user", :as => :user)"#), ["users/user"]);
        assert_eq!(names(r#"json.comments @post.comments, partial: "comments/comment", as: :comment"#), ["comments/comment"]);
        assert_eq!(names(r#"xml << render(partial: "items/item", locals: { i: i })"#), ["items/item"]);
        assert_eq!(names(r#"<%= render partial: "x" %>"#), ["x"]);
    }

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
        assert_eq!(render_string_prefix(r#"<%= render "us"#, 14).unwrap(), (RenderKind::Partial, "us".to_string()));
        assert_eq!(render_string_prefix(r#"render template: "a/"#, 20).unwrap(), (RenderKind::Template, "a/".to_string()));
        assert_eq!(render_string_prefix(r#"render partial: "x", layout: 'ad"#, 32).unwrap(), (RenderKind::Partial, "ad".to_string()));
        assert_eq!(render_string_prefix(r#"render layout: "ad"#, 18).unwrap(), (RenderKind::Partial, "ad".to_string()));
        assert_eq!(render_string_prefix(r#"render template: "a", layout: "la"#, 33).unwrap(), (RenderKind::Template, "la".to_string()));
        assert!(render_string_prefix(r#"render partial: "x", locals: { layout: "ad"#, 41).is_none());
        assert!(render_string_prefix(r#"render "x", layout: "ad"#, 23).is_none());
        assert!(render_string_prefix(r#"page_layout "ad"#, 15).is_none());
        assert_eq!(render_string_prefix(r#"json.partial! "po"#, 17).unwrap(), (RenderKind::Partial, "po".to_string()));
        assert_eq!(render_string_prefix(r#"json.partial! partial: 'po"#, 26).unwrap(), (RenderKind::Partial, "po".to_string()));
        assert_eq!(snake_case("CardComponent"), "card_component");
        let refs = |l: &str| render_refs(l).into_iter().map(|r| (r.kind, r.name)).collect::<Vec<_>>();
        let p = |n: &str| (RenderKind::Partial, n.to_string());
        assert_eq!(refs(r#"<%= render layout: "box" do %>"#), [p("box")]);
        assert_eq!(refs(r#"<%= render partial: "row", layout: "box", locals: { a: 1 } %>"#), [p("row"), p("box")]);
        assert_eq!(
            refs(r#"<%= render template: "a/b", layout: "layouts/x" %>"#),
            [(RenderKind::Template, "a/b".to_string()), (RenderKind::Template, "layouts/x".to_string())]
        );
        assert_eq!(refs(r#"<%= render "row", layout: "box" %>"#), [p("row")]);
        assert_eq!(refs(r#"<%= render partial: "row", locals: { a: 1, layout: "gone" } %>"#), [p("row")]);
        assert_eq!(refs(r#"<%= render partial: "row", locals: [{ a: 1 }], layout: "box" %>"#), [p("row"), p("box")]);
        assert!(render_refs(r#"render partial: "row", layout: "box""#)[1].from_layout);
        // Pre-Ruby-1.9 hash rockets, common in old views.
        assert_eq!(refs(r#"<%= render :partial => "row", :locals => { :a => 1 } %>"#), [p("row")]);
        assert_eq!(refs(r#"render(:template => "a/b")"#), [(RenderKind::Template, "a/b".to_string())]);
        assert_eq!(refs(r#"render(:partial => "row", :layout => "box")"#), [p("row"), p("box")]);
        assert_eq!(render_string_prefix(r#"render :partial => "ro"#, 22).unwrap(), (RenderKind::Partial, "ro".to_string()));
        assert_eq!(render_string_prefix(r#"render :partial => "x", :layout => "a"#, 37).unwrap(), (RenderKind::Partial, "a".to_string()));
    }

    #[test]
    fn inflection_round_trips() {
        for (singular, plural) in [
            ("user", "users"),
            ("category", "categories"),
            ("day", "days"),
            ("address", "addresses"),
            ("box", "boxes"),
            ("match", "matches"),
            ("person", "people"),
            ("sales_person", "sales_people"),
            ("child", "children"),
            ("movie", "movies"),
            ("status", "statuses"),
            ("news", "news"),
            ("line_item", "line_items"),
            ("price", "prices"),
            ("cookie", "cookies"),
            ("hero", "heroes"),
            ("quiz", "quizzes"),
            ("cache", "caches"),
            ("analysis", "analyses"),
            ("daily_news", "daily_news"),
        ] {
            assert_eq!(pluralize(singular), plural);
            assert_eq!(singularize(plural), singular);
            assert_eq!(singularize(singular), singular);
        }
        for singular in ["bus", "virus", "axis"] {
            assert_eq!(singularize(singular), singular);
        }
    }

    #[test]
    fn model_render_partials() {
        let root = std::env::temp_dir().join(format!("rt-models-{}", std::process::id()));
        let views = root.join("app/views");
        for f in ["users/_user.html.erb", "comments/_comment.html.erb", "people/_person.html.haml", "categories/_category.html.erb"] {
            let p = views.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let current = views.join("posts/show.html.erb");
        let line = "<%= render @user %> <%= render(@users) %> <%= render @post.comments, cached: true %> <%= render @people %> <%= render @categories %> <%= render @post.author %> <%= render @user.name %>";
        let refs = model_renders(line);
        assert_eq!(refs.iter().map(|r| r.2).collect::<Vec<_>>(), ["user", "users", "comments", "people", "categories", "author", "name"]);
        assert_eq!(&line[refs[2].0..refs[2].1], "@post.comments");
        let found: Vec<Vec<PathBuf>> = refs.iter().map(|r| resolve_model(&Project::unknown(), &current, r.2)).collect();
        assert_eq!(found[0], [views.join("users/_user.html.erb")]);
        assert_eq!(found[1], found[0]);
        assert_eq!(found[2], [views.join("comments/_comment.html.erb")]);
        assert_eq!(found[3], [views.join("people/_person.html.haml")]);
        assert_eq!(found[4], [views.join("categories/_category.html.erb")]);
        assert!(found[5..].iter().all(Vec::is_empty));
        for query in [
            "render @products.page(params[:page])",
            "render @post.comments.order(:id)",
            "render @users.map(&:x)",
            "render @users.first.posts",
        ] {
            assert!(model_renders(query).is_empty(), "{query}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_by_view_format() {
        let root = std::env::temp_dir().join(format!("rt-formats-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        for f in ["_user.html.erb", "_user.html+phone.erb", "_user.json.jbuilder", "_user.erb"] {
            std::fs::write(users.join(f), "").unwrap();
        }
        let user = render_refs(r#"render "users/user""#).remove(0);
        let found = |view: &str| -> Vec<String> {
            resolve(&Project::unknown(), &users.join(view), &user)
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(found("index.html.erb"), ["_user.erb", "_user.html+phone.erb", "_user.html.erb"]);
        assert_eq!(found("show.html+phone.erb"), found("index.html.erb"));
        assert_eq!(found("index.json.jbuilder"), ["_user.erb", "_user.json.jbuilder"]);
        assert_eq!(found("create.js.erb"), found("index.html.erb"));
        assert_eq!(found("index.erb").len(), 4);
        assert_eq!(resolve_model(&Project::unknown(), &users.join("index.json.jbuilder"), "users").len(), 2);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn app_registered_format_is_not_part_of_the_name() {
        let root = std::env::temp_dir().join(format!("rt-registered-format-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        for f in ["_card.html.erb", "_card.mobile.erb"] {
            std::fs::write(users.join(f), "").unwrap();
        }
        let card = render_refs(r#"render "users/card""#).remove(0);
        let found = |view: &str| -> Vec<String> {
            let view = users.join(view);
            resolve(&Project::for_template(&view), &view, &card)
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(found("show.mobile.erb"), ["_card.html.erb"]);
        assert_eq!(found("show.html.erb"), ["_card.html.erb"]);

        let initializers = root.join("config/initializers");
        std::fs::create_dir_all(&initializers).unwrap();
        std::fs::write(initializers.join("mime_types.rb"), "Mime::Type.register_alias \"text/html\", :mobile\n").unwrap();
        assert_eq!(found("show.mobile.erb"), ["_card.mobile.erb"]);
        assert_eq!(found("show.html.erb"), ["_card.html.erb"]);
        assert_eq!(found("show.erb"), ["_card.html.erb", "_card.mobile.erb"]);
        let project = Project::for_template(&users.join("show.mobile.erb"));
        assert_eq!(template_stem(|f| project.is_format(f), "_card.mobile.erb"), "_card");
        assert_eq!(template_format(|f| project.is_format(f), &users.join("_card.mobile.erb")), Some("mobile"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn candidates_per_render_kind() {
        let root = std::env::temp_dir().join(format!("rt-candidates-{}", std::process::id()));
        let views = root.join("app/views");
        for f in [
            "users/index.html.erb",
            "users/index.json.jbuilder",
            "users/_card.html.erb",
            "users/_row.json.jbuilder",
            "layouts/application.html.erb",
            "layouts/admin/base.html.erb",
            "layouts/_nav.html.erb",
            "layouts/.keep",
        ] {
            let p = views.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let current = views.join("users/show.html.erb");
        let files = files_under(&views, usize::MAX);
        let names = |kind| render_candidates(&Project::unknown(), &current, kind, &files).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names(RenderKind::Partial), ["card", "layouts/nav", "users/card"]);
        assert_eq!(names(RenderKind::Template), ["layouts/admin/base", "layouts/application", "users/index"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn strict_locals_signature() {
        let sig = strict_locals("<%# locals: (title:, items: [1, 2], label: t(\"a, b\"), **options) -%>\n<p>", Close::default()).unwrap();
        assert_eq!(sig.params, "title:, items: [1, 2], label: t(\"a, b\"), **options");
        assert_eq!(
            sig.locals,
            vec![
                ("title".to_string(), None),
                ("items".to_string(), Some("[1, 2]".to_string())),
                ("label".to_string(), Some("t(\"a, b\")".to_string())),
            ]
        );
        assert!(sig.rest);
        let none = strict_locals("<%# locals: () -%>", Close::default()).unwrap();
        assert!(none.locals.is_empty() && !none.rest);
        assert_eq!(
            strict_locals("-# locals: (user: nil)", Close::default()).unwrap().locals,
            vec![("user".to_string(), Some("nil".to_string()))]
        );
        assert_eq!(
            strict_locals("<%# locals: (a: ')') %>", Close::default()).unwrap().locals,
            vec![("a".to_string(), Some("')'".to_string()))]
        );
        let no_rest = strict_locals("<%# locals: (a:, **nil) %>", Close::default()).unwrap();
        assert_eq!((no_rest.locals.len(), no_rest.rest), (1, false));
        // Lazy: stops at the `)` that closes the tag, not the last one on the line.
        assert_eq!(strict_locals("<%# locals: (a:) %> <%= f(1) %>", Close::default()).unwrap().params, "a:");
        let multi = strict_locals("<%# locals: (\n  a:,\n  b: [1,\n 2]\n) -%>", Close::default()).unwrap();
        assert_eq!(multi.locals, vec![("a".to_string(), None), ("b".to_string(), Some("[1,\n 2]".to_string()))]);
        // ActionView needs whitespace before the parenthesis.
        assert!(strict_locals("<%# locals:(a:) %>", Close::default()).is_none());
        assert!(strict_locals("<%# just a comment %>", Close::default()).is_none());
    }

    #[test]
    fn literal_render_locals() {
        fn passed(line: &str) -> Option<Vec<&str>> {
            passed_locals(line, &render_refs(line)[0])
        }
        assert_eq!(passed(r#"<%= render "card", title: "a, b", n: f(1, 2) %>"#), Some(vec!["title", "n"]));
        assert_eq!(passed(r#"<%= render("card", user:) -%>"#), Some(vec!["user"]));
        assert_eq!(passed(r#"<%= render "card" %>"#), Some(vec![]));
        assert_eq!(passed(r#"<%= render partial: "card", locals: { a: 1, b: [2] } %>"#), Some(vec!["a", "b"]));
        assert_eq!(passed(r#"= render(partial: "card")"#), Some(vec![]));
        assert_eq!(passed(r#"json.partial! "posts/post", post: p"#), Some(vec!["post"]));
        assert_eq!(passed(r#"json.partial! partial: "posts/post", locals: { post: p }"#), Some(vec!["post"]));
        assert_eq!(passed(r#"json.partial! "posts/post", collection: @posts, as: :post"#), None);
        assert_eq!(passed(r#"json.array! @posts, partial: "posts/post", as: :post"#), None);
        assert_eq!(passed(r#"<%= render "card", collection: @cards %>"#), Some(vec!["collection"]));
        assert_eq!(passed(r#"json.partial! "posts/post", locals: { post: @post }"#), Some(vec!["post"]));
        assert_eq!(passed(r#"<%= @x.to_json.html_safe %><%= render "card", as: 1 %>"#), Some(vec!["as"]));
        for skip in [
            r#"<%= render partial: "card", collection: @cards %>"#,
            r#"<%= render "card", **opts %>"#,
            r#"<%= render "card", locals %>"#,
            r#"<%= render "card", Card::DEFAULTS %>"#,
            r#"<%= render "card", "a" => 1 %>"#,
            r#"<%= render "card", a: 1,"#,
            r#"<%= render "card" do %>"#,
            r#"<%= render template: "a/b" %>"#,
        ] {
            assert_eq!(passed(skip), None, "{skip}");
        }
    }

    #[test]
    fn component_classes_by_constant() {
        let root = std::env::temp_dir().join(format!("rt-components-{}", std::process::id()));
        for f in [
            "app/components/admin/user_card_component.rb",
            "app/components/button_component.rb",
            "app/components/button_component.html.erb",
            "app/components/helpers.rb",
            "app/components/über_component.rb",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        std::fs::create_dir_all(root.join("app/views/home")).unwrap();
        let names: Vec<String> = component_classes(&root.join("app/views/home/index.html.erb")).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["Admin::UserCardComponent", "ButtonComponent", "ÜberComponent"]);
        let from_component: Vec<String> =
            component_classes(&root.join("app/components/button_component.html.erb")).into_iter().map(|(n, _)| n).collect();
        assert_eq!(from_component, names);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn component_haml_sidecar() {
        let root = std::env::temp_dir().join(format!("rt-component-{}", std::process::id()));
        let components = root.join("app/components/users");
        std::fs::create_dir_all(&components).unwrap();
        std::fs::create_dir_all(root.join("app/views/users")).unwrap();
        for f in ["card_component.rb", "card_component.html.haml"] {
            std::fs::write(components.join(f), "").unwrap();
        }
        let found = resolve_component(&Project::unknown(), &root.join("app/views/users/show.html.haml"), "Users::CardComponent");
        assert_eq!(found, [components.join("card_component.rb"), components.join("card_component.html.haml")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Files of `dir` under a fresh temp `app/views`, and a resolver from `view` (path under app/views) to file names.
    fn partial_tree(tag: &str, files: &[&str]) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("rt-{tag}-{}", std::process::id()));
        let views = root.join("app/views");
        for f in files {
            let p = views.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        (root, views)
    }

    fn resolved_names(views: &Path, view: &str, render: &str) -> Vec<String> {
        let r = render_refs(render).remove(0);
        resolve(&Project::unknown(), &views.join(view), &r).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn locale_only_partial_resolves_from_any_format() {
        let (root, views) = partial_tree("locale-only", &["shared/_greeting.de.erb"]);
        for view in ["users/show.html.erb", "users/index.json.jbuilder", "users/show.erb"] {
            assert_eq!(resolved_names(&views, view, r#"render "shared/greeting""#), ["_greeting.de.erb"], "{view}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn locale_and_format_partial_follows_view_format() {
        let (root, views) = partial_tree("locale-format", &["users/_card.en.html.erb", "users/_clip.js.erb", "users/_event.ics.erb"]);
        assert_eq!(resolved_names(&views, "users/show.html.erb", r#"render "card""#), ["_card.en.html.erb"]);
        assert!(resolved_names(&views, "users/index.json.jbuilder", r#"render "card""#).is_empty());
        // `js` has locale shape but is a format: it must not read as format-less and match JSON views.
        assert!(resolved_names(&views, "users/index.json.jbuilder", r#"render "clip""#).is_empty());
        assert_eq!(resolved_names(&views, "users/index.js.erb", r#"render "clip""#), ["_clip.js.erb"]);
        assert!(resolved_names(&views, "users/index.json.jbuilder", r#"render "event""#).is_empty());
        assert_eq!(resolved_names(&views, "users/index.ics.erb", r#"render "event""#), ["_event.ics.erb"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn variant_named_partial_is_not_the_bare_name() {
        let (root, views) = partial_tree("variant-name", &["users/_user.v2.html.erb", "users/_user.html.erb"]);
        assert_eq!(resolved_names(&views, "users/show.html.erb", r#"render "user""#), ["_user.html.erb"]);
        assert_eq!(resolved_names(&views, "users/show.html.erb", r#"render "user.v2""#), ["_user.v2.html.erb"]);
        let only_v2 = partial_tree("variant-only", &["users/_user.v2.html.erb"]);
        assert!(resolved_names(&only_v2.1, "users/show.html.erb", r#"render "user""#).is_empty());
        assert!(resolve_model(&Project::unknown(), &only_v2.1.join("users/show.html.erb"), "user").is_empty());
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&only_v2.0).unwrap();
    }

    #[test]
    fn completion_names_drop_locale_format_and_keep_variant_looking_stems() {
        let (root, views) = partial_tree(
            "names",
            &["users/_user.v2.html.erb", "users/_card.en.html.erb", "users/_row.html+phone.erb", "users/index.de.html.erb"],
        );
        let current = views.join("users/show.html.erb");
        let files = files_under(&views, usize::MAX);
        let names = |kind| render_candidates(&Project::unknown(), &current, kind, &files).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names(RenderKind::Partial), ["card", "row", "user.v2", "users/card", "users/row", "users/user.v2"]);
        assert_eq!(names(RenderKind::Template), ["users/index"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn render_ref_at_covers_quotes_but_not_beyond() {
        let line = r#"<%= render "users/card" %>"#;
        let open_quote = line.find('"').unwrap();
        let end = open_quote + 1 + "users/card".len();
        assert!(render_ref_at(line, open_quote - 1).is_none());
        assert!(render_ref_at(line, open_quote).is_some());
        assert!(render_ref_at(line, end + 1).is_some());
        assert!(render_ref_at(line, end + 2).is_none());
    }

    #[test]
    fn component_ref_at_covers_exactly_the_class_name() {
        let line = "= render Users::CardComponent.new";
        let start = line.find("Users").unwrap();
        let end = start + "Users::CardComponent".len();
        assert!(component_ref_at(line, start - 1).is_none());
        assert!(component_ref_at(line, start).is_some());
        assert!(component_ref_at(line, end).is_some());
        assert!(component_ref_at(line, end + 1).is_none());
    }

    #[test]
    fn split_args_honors_escapes_and_nested_quotes() {
        assert_eq!(split_args(r#"a: "x\",y", b: 1"#), Some(vec![r#"a: "x\",y""#, "b: 1"]));
        assert_eq!(split_args(r#"a: "x\\", b: 1"#), Some(vec![r#"a: "x\\""#, "b: 1"]));
        assert_eq!(split_args(r#"a: "it's, ok", b: 1"#), Some(vec![r#"a: "it's, ok""#, "b: 1"]));
        assert_eq!(split_args(r#"a: 'say "hi", ok', b: 1"#), Some(vec![r#"a: 'say "hi", ok'"#, "b: 1"]));
        assert_eq!(split_args(r#"a: "x\""#), None);
        assert_eq!(split_args("a: f(1, 2), b: [3, 4]"), Some(vec!["a: f(1, 2)", "b: [3, 4]"]));
        assert_eq!(split_args("a: )"), None);
    }
}
