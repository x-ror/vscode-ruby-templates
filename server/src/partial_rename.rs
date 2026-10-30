//! `workspace/willRenameFiles`: rewrite literal `render` references to partials that are renamed or moved,
//! one at a time or with their folder. `textDocument/references`: the same references to given files, and the
//! `t("…")` calls of an i18n key.

use crate::code_context::render_filter;
use crate::lang::TemplateLang;
use crate::partials::{
    call_text, first_folder_only, model_renders, passed_locals, render_refs, resolve, resolve_model, RenderKind, RenderRef,
};
use crate::project::Project;
use crate::rails_paths::{app_root, template_stem, views_root};
use crate::util::{lines_with_offsets, LineIndex};
use crate::{i18n, ruby, slim_outline};
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tower_lsp::lsp_types::{Location, Range, TextEdit, Url};
use walkdir::WalkDir;

static PARTIAL_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"partial\s*(?::|=>)\s*["']$"#).unwrap());

pub struct Source {
    pub path: PathBuf,
    /// `None` for `.rb` files, where every string is code.
    pub lang: Option<TemplateLang>,
    pub text: String,
}

/// `app/views/users/_card.html.slim` → `users/card`; `None` for non-partials and files outside a views dir.
fn qualified_name(project: &Project, views: &Path, file: &Path) -> Option<String> {
    let dir = file.parent()?.strip_prefix(views).ok()?.to_string_lossy().replace('\\', "/");
    let name = template_stem(|f| project.is_format(f), file.file_name()?.to_str()?.strip_prefix('_')?);
    (!dir.is_empty() && !name.is_empty()).then(|| format!("{dir}/{name}"))
}

/// Where `path` ends up after `renames`, whether it or one of its folders is renamed.
fn moved(path: &Path, renames: &[(PathBuf, PathBuf)]) -> Option<PathBuf> {
    renames.iter().find_map(|(old, new)| match path.strip_prefix(old) {
        Ok(rel) if rel.as_os_str().is_empty() => Some(new.clone()),
        Ok(rel) => Some(new.join(rel)),
        Err(_) => None,
    })
}

/// A bare name stays bare only while it is looked up first in the referencing file's own folder.
fn new_ref_name(project: &Project, written: &str, source: &Path, target: &Path, renames: &[(PathBuf, PathBuf)]) -> Option<String> {
    let qualified = qualified_name(project, &views_root(target)?, target)?;
    let source_after = moved(source, renames).unwrap_or_else(|| source.to_path_buf());
    if written.contains('/') || source_after.parent() != target.parent() {
        return Some(qualified);
    }
    qualified.rsplit('/').next().map(str::to_string)
}

/// The references `find` sees on each line that are inside code, as (byte start, byte end, reference); only those
/// `want` keeps, so the code filter is built only for sources with a candidate.
fn code_refs(
    source: &Source,
    extra_engines: &[String],
    find: impl Fn(&str) -> Vec<RenderRef>,
    want: impl Fn(&RenderRef) -> bool,
) -> Vec<(usize, usize, RenderRef)> {
    let shift = |offset: usize| move |r: RenderRef| (offset + r.start, offset + r.end, r);
    if matches!(source.lang, Some(TemplateLang::Slim | TemplateLang::Skim)) {
        let spans = slim_outline::code_spans(&source.text, extra_engines);
        return spans.into_iter().flat_map(|(s, e)| find(&source.text[s..e]).into_iter().filter(&want).map(shift(s))).collect();
    }
    let candidates: Vec<(usize, usize, &str, RenderRef)> = lines_with_offsets(&source.text)
        .enumerate()
        .flat_map(|(i, (offset, line))| find(line).into_iter().filter(&want).map(move |r| (i, offset, line, r)))
        .collect();
    if candidates.is_empty() {
        return vec![];
    }
    let template_code = source.lang.map(|lang| render_filter(lang, &source.text, extra_engines));
    candidates
        .into_iter()
        .filter(|(i, offset, line, r)| match &template_code {
            Some(in_code) => in_code(*i, *offset, r),
            None => ruby::code_end(&line[..r.call]) == ruby::CodeEnd::Code,
        })
        .map(|(_, offset, _, r)| shift(offset)(r))
        .collect()
}

/// The files a reference points at. Views look partials up from their own folder (and `layout:` there is a
/// partial too); code outside app/views is checked only for qualified names, since its controller prefix is
/// unknown, and controllers' plain `render "x"` is a template, so only `partial:` counts there.
fn targets(project: &Project, source: &Source, r: &RenderRef, explicit_partial: bool) -> Vec<PathBuf> {
    let as_partial = RenderRef { kind: RenderKind::Partial, ..r.clone() };
    if views_root(&source.path).is_some() {
        return match r.kind {
            RenderKind::Template => vec![],
            RenderKind::Partial => resolve(project, &source.path, &as_partial),
        };
    }
    let controller = source.path.components().any(|c| c.as_os_str() == "controllers");
    let partial = if controller { explicit_partial } else { r.kind == RenderKind::Partial };
    match app_root(&source.path).map(|app| app.join("views")) {
        Some(views) if partial && r.name.contains('/') => resolve(project, &views.join("_"), &as_partial),
        _ => vec![],
    }
}

/// Whether `resolve` could look in `dir` for `r` from `source`: a cheap test before its directory listing.
fn may_look_in(source: &Path, r: &RenderRef, dir: &Path) -> bool {
    let views = views_root(source).or_else(|| Some(app_root(source)?.join("views")));
    match (r.name.rsplit_once('/'), views) {
        // The folders `resolve` lists, compared as paths: `./users` is `users` there too.
        (Some((d, _)), Some(v)) => v.join(d) == dir,
        (Some(_), None) => true,
        (None, v) => source.parent() == Some(dir) || v.is_some_and(|v| v.join("application") == dir),
    }
}

/// `targets` of the reference whose string starts at byte `start` of `source`.
/// Only the first folder's matches: the files Rails would pick.
fn ref_targets(project: &Project, source: &Source, start: usize, r: &RenderRef) -> Vec<PathBuf> {
    let explicit_partial = source.lang.is_none() && PARTIAL_KEY.is_match(source.text[..start].rsplit('\n').next().unwrap_or(""));
    first_folder_only(targets(project, source, r, explicit_partial))
}

/// `render @users` as a reference named after the model (`users`).
fn model_refs(line: &str) -> Vec<RenderRef> {
    model_renders(line)
        .into_iter()
        .map(|(start, end, name)| RenderRef {
            kind: RenderKind::Partial,
            name: name.to_string(),
            start,
            end,
            call: line[..start].rfind("render").unwrap_or(start),
            from_layout: false,
        })
        .collect()
}

/// `_card.html.slim` → `card` for each partial among `files`.
fn partial_stems<'a>(project: &Project, files: impl IntoIterator<Item = &'a PathBuf>) -> Vec<&'a str> {
    files
        .into_iter()
        .filter_map(|f| f.file_name()?.to_str()?.strip_prefix('_').map(|stem| template_stem(|f| project.is_format(f), stem)))
        .collect()
}

/// Whether a reference to `name` can name a partial with one of `stems`; lets callers skip `resolve`, a directory listing.
fn may_name(stems: &[&str], name: &str) -> bool {
    stems.contains(&name.rsplit('/').next().unwrap_or(name))
}

/// Every reference in `sources` that resolves to one of `files`, `render @users` included.
pub fn references(project: &Project, files: &[PathBuf], sources: &[Source], extra_engines: &[String]) -> Vec<Location> {
    let stems = partial_stems(project, files);
    let mut out = Vec::new();
    for source in sources {
        let Ok(uri) = Url::from_file_path(&source.path) else { continue };
        let index = LineIndex::new(&source.text);
        let named = code_refs(source, extra_engines, render_refs, |r| may_name(&stems, &r.name))
            .into_iter()
            .filter(|(_, _, r)| files.iter().any(|f| f.parent().is_some_and(|d| may_look_in(&source.path, r, d))))
            .filter(|(start, _, r)| ref_targets(project, source, *start, r).iter().any(|t| files.contains(t)));
        let models = code_refs(source, extra_engines, model_refs, |r| may_name(&stems, &crate::partials::singularize(&r.name)))
            .into_iter()
            .filter(|(_, _, r)| resolve_model(project, &source.path, &r.name).iter().any(|t| files.contains(t)));
        for (start, end, _) in named.chain(models) {
            let range = Range::new(index.position(start), index.position(end));
            out.push(Location::new(uri.clone(), range));
        }
    }
    out
}

/// The `render` calls of `partial` that pass `local` (`render "card", user: u`), at the local's key.
pub fn local_passes(project: &Project, partial: &Path, local: &str, sources: &[Source], extra_engines: &[String]) -> Vec<Location> {
    let files = [partial.to_path_buf()];
    let stems = partial_stems(project, &files);
    // The key itself, `user:` after the partial name; the call's start when it's spelled another way.
    let key = Regex::new(&format!(r"(?:^|[^\w]){}:", regex::escape(local))).ok();
    let mut out = Vec::new();
    for source in sources {
        let Ok(uri) = Url::from_file_path(&source.path) else { continue };
        let index = LineIndex::new(&source.text);
        for (start, _, r) in code_refs(source, extra_engines, render_refs, |r| may_name(&stems, &r.name)) {
            if !files[0].parent().is_some_and(|d| may_look_in(&source.path, &r, d))
                || !ref_targets(project, source, start, &r).contains(&files[0])
            {
                continue;
            }
            let line_start = start - r.start;
            let call = call_text(&source.text[line_start..], &r);
            if !passed_locals(call, &r).is_some_and(|names| names.contains(&local)) {
                continue;
            }
            let at = key.as_ref().and_then(|k| k.find(&call[r.end..])).map_or(r.call, |m| r.end + m.end() - local.len() - 1);
            let position = index.position(line_start + at);
            out.push(Location::new(uri.clone(), Range::new(position, position)));
        }
    }
    out
}

/// The `t("…")` calls in code whose full key is `key`; lazy `.x` keys count only in views, where they have a scope.
pub fn i18n_references(project: &Project, key: &str, sources: &[Source], extra_engines: &[String]) -> Vec<Location> {
    let last = key.rsplit('.').next().unwrap_or(key);
    let find = |line: &str| if line.contains(last) { i18n_refs(line) } else { vec![] };
    let mut out = Vec::new();
    for source in sources {
        let Ok(uri) = Url::from_file_path(&source.path) else { continue };
        let index = LineIndex::new(&source.text);
        for (start, end, _) in code_refs(source, extra_engines, find, |r| key.ends_with(r.name.as_str()))
            .into_iter()
            .filter(|(_, _, r)| i18n::full_key(project, &source.path, &r.name).is_some_and(|full| full == key))
        {
            let range = Range::new(index.position(start), index.position(end));
            out.push(Location::new(uri.clone(), range));
        }
    }
    out
}

/// i18n calls as references, so `code_refs` keeps those in code.
fn i18n_refs(line: &str) -> Vec<RenderRef> {
    i18n::key_calls(line)
        .into_iter()
        .map(|(call, start, end)| RenderRef {
            kind: RenderKind::Partial,
            name: line[start..end].to_string(),
            start,
            end,
            call,
            from_layout: false,
        })
        .collect()
}

/// Folders outside app/views whose code renders partials.
const CODE_DIRS: &[&str] = &["controllers", "helpers", "components"];

/// Templates under `views`, plus Ruby files and templates in app/{controllers,helpers,components}, that mention
/// `render` or `partial`, as every reference does. Text of `open` documents wins over the disk copy, which may lack
/// unsaved edits.
pub fn sources(views: &Path, open: &HashMap<PathBuf, String>) -> Vec<Source> {
    let code_dirs = views.parent().into_iter().flat_map(|app| CODE_DIRS.iter().map(move |d| app.join(d)));
    let may_render = |text: &str| text.contains("render") || text.contains("partial");
    sources_in(std::iter::once(views.to_path_buf()).chain(code_dirs), open, may_render)
}

/// Templates and Ruby files anywhere in `app` and the app's `lib`: translations are looked up from models,
/// mailers and jobs too.
pub fn i18n_sources(app: &Path, open: &HashMap<PathBuf, String>) -> Vec<Source> {
    sources_in(std::iter::once(app.to_path_buf()).chain(app.parent().map(|rails| rails.join("lib"))), open, |_| true)
}

fn sources_in(roots: impl Iterator<Item = PathBuf>, open: &HashMap<PathBuf, String>, keep: fn(&str) -> bool) -> Vec<Source> {
    roots
        .flat_map(|root| WalkDir::new(root).follow_links(true).into_iter().flatten())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let path = e.into_path();
            let lang = match path.extension() {
                Some(ext) if ext == "rb" => None,
                _ => Some(TemplateLang::from_uri_or_id(&Url::from_file_path(&path).ok()?, None)?),
            };
            let text = match open.get(&path) {
                Some(text) => text.clone(),
                None => std::fs::read_to_string(&path).ok()?,
            };
            keep(&text).then_some(Source { path, lang, text })
        })
        .collect()
}

/// Edits for every partial reference that resolves only to renamed files that all agree on the new name;
/// a reference also matching a file left in place (another format or variant) is not touched.
pub fn render_edits(
    project: &Project,
    renames: &[(PathBuf, PathBuf)],
    sources: &[Source],
    extra_engines: &[String],
) -> HashMap<Url, Vec<TextEdit>> {
    let folder_renamed = renames.iter().any(|(old, _)| old.is_dir());
    let stems = partial_stems(project, renames.iter().map(|(old, _)| old));

    let mut out = HashMap::new();
    for source in sources {
        let index = LineIndex::new(&source.text);
        let mut edits = Vec::new();
        for (start, end, r) in code_refs(source, extra_engines, render_refs, |r| folder_renamed || may_name(&stems, &r.name)) {
            let names: Option<Vec<String>> = ref_targets(project, source, start, &r)
                .iter()
                .map(|t| new_ref_name(project, &r.name, &source.path, &moved(t, renames)?, renames))
                .collect();
            let Some(names) = names else { continue };
            let Some(name) = names.first().filter(|n| **n != r.name && names.iter().all(|m| m == *n)) else {
                continue;
            };
            let range = Range::new(index.position(start), index.position(end));
            edits.push(TextEdit::new(range, name.clone()));
        }
        if let (false, Ok(uri)) = (edits.is_empty(), Url::from_file_path(&source.path)) {
            out.insert(uri, edits);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Position;

    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str, files: &[&str]) -> Self {
            let root = std::env::temp_dir().join(format!("rt-rename-{name}-{}", std::process::id()));
            for f in files {
                let path = root.join(f);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, "").unwrap();
            }
            Tree(root)
        }
        fn path(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
        /// `path` relative to the tree, with `/` on every platform.
        fn rel(&self, path: &Path) -> String {
            path.strip_prefix(&self.0).unwrap().to_string_lossy().replace('\\', "/")
        }
        fn source(&self, rel: &str, lang: Option<TemplateLang>, text: &str) -> Source {
            Source { path: self.path(rel), lang, text: text.into() }
        }
        fn edits(&self, renames: &[(&str, &str)], sources: &[Source]) -> Vec<(String, u32, u32, String)> {
            let renames: Vec<_> = renames.iter().map(|(a, b)| (self.path(a), self.path(b))).collect();
            let mut got: Vec<_> = render_edits(&Project::unknown(), &renames, sources, &[])
                .iter()
                .flat_map(|(uri, edits)| {
                    let file = self.rel(&uri.to_file_path().unwrap());
                    edits.iter().map(move |e| (file.clone(), e.range.start.line, e.range.start.character, e.new_text.clone()))
                })
                .collect();
            got.sort();
            got
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const SLIM: Option<TemplateLang> = Some(TemplateLang::Slim);
    const ERB: Option<TemplateLang> = Some(TemplateLang::Erb);

    #[test]
    fn renames_and_moves_rewrite_matching_renders() {
        let t = Tree::new(
            "basic",
            &["app/views/users/_card.html.slim", "app/views/users/_row.html.erb", "app/views/users/_row.html+phone.erb"],
        );
        let sources = [
            t.source(
                "app/views/users/index.html.slim",
                SLIM,
                "= render \"card\"\np = render partial: 'users/card', locals: {}\np we render \"card\"\n= render \"row\"\n",
            ),
            t.source("app/views/shared/page.html.erb", ERB, "é<%= render \"users/card\" %> <p>render \"users/card\"</p>\n"),
        ];
        let card = "app/views/users/_card.html.slim";
        assert_eq!(
            t.edits(&[(card, "app/views/users/_tile.html.slim")], &sources),
            [
                ("app/views/shared/page.html.erb".into(), 0, 13, "users/tile".into()),
                ("app/views/users/index.html.slim".into(), 0, 10, "tile".into()),
                ("app/views/users/index.html.slim".into(), 1, 21, "users/tile".into()),
            ]
        );
        let moved = t.edits(&[(card, "app/views/shared/_card.html.slim")], &sources);
        assert_eq!(moved.iter().map(|e| e.3.as_str()).collect::<Vec<_>>(), ["shared/card", "shared/card", "shared/card"]);
        assert!(t.edits(&[(card, "app/views/users/_card.html.erb")], &sources).is_empty());
        assert!(t.edits(&[(card, "app/views/users/card.html.slim")], &sources).is_empty());
        assert!(t.edits(&[("app/views/users/_row.html.erb", "app/views/users/_line.html.erb")], &sources).is_empty());
        let both = [
            ("app/views/users/_row.html.erb", "app/views/users/_line.html.erb"),
            ("app/views/users/_row.html+phone.erb", "app/views/users/_line.html+phone.erb"),
        ];
        assert_eq!(t.edits(&both, &sources), [("app/views/users/index.html.slim".into(), 3, 10, "line".into())]);
    }

    #[test]
    fn application_fallback_is_qualified_after_rename() {
        let t = Tree::new("app", &["app/views/application/_card.html.slim", "app/views/users/_tile.html.slim"]);
        let sources = [t.source("app/views/users/index.html.slim", SLIM, "= render \"card\"\n")];
        let got = t.edits(&[("app/views/application/_card.html.slim", "app/views/application/_tile.html.slim")], &sources);
        assert_eq!(got, [("app/views/users/index.html.slim".into(), 0, 10, "application/tile".into())]);
    }

    #[test]
    fn slim_text_comments_and_engines_are_left_alone() {
        let t = Tree::new("slim", &["app/views/users/_card.html.slim"]);
        let text = concat!(
            "= render \"card\"\n",
            "/ = render \"card\"\n",
            "p See (docs) render \"card\"\n",
            "javascript:\n",
            "  fetch(render \"card\")\n",
            "p\n",
            "  | Use render(\"card\") here\n",
            "= render layout: \"card\" do\n",
            "= render \"x\",\n",
            "    other: render(\"card\")\n",
        );
        let sources = [t.source("app/views/users/index.html.slim", SLIM, text)];
        let got = t.edits(&[("app/views/users/_card.html.slim", "app/views/users/_tile.html.slim")], &sources);
        let at: Vec<(u32, u32)> = got.iter().map(|e| (e.1, e.2)).collect();
        assert_eq!(at, [(0, 10), (7, 18), (9, 19)]);
    }

    #[test]
    fn folder_rename_requalifies_refs() {
        let t = Tree::new("folder", &["app/views/users/_card.html.slim", "app/views/users/index.html.slim"]);
        let sources = [
            t.source("app/views/users/index.html.slim", SLIM, "= render \"card\"\n= render \"users/card\"\n"),
            t.source("app/views/home/show.html.erb", ERB, "<%= render \"users/card\" %>\n"),
        ];
        assert_eq!(
            t.edits(&[("app/views/users", "app/views/people")], &sources),
            [
                ("app/views/home/show.html.erb".into(), 0, 12, "people/card".into()),
                ("app/views/users/index.html.slim".into(), 1, 10, "people/card".into()),
            ]
        );
    }

    #[test]
    fn ruby_callers_use_qualified_partials() {
        let t = Tree::new("ruby", &["app/views/users/_card.html.slim"]);
        let sources = [
            t.source(
                "app/controllers/users_controller.rb",
                None,
                "render partial: \"users/card\"\nrender \"users/card\"\n# render partial: \"users/card\"\nnote = \"render partial: 'users/card'\"\nx = 1 # render partial: \"users/card\"\n",
            ),
            t.source("app/helpers/cards_helper.rb", None, "render(\"users/card\")\nrender \"card\"\n"),
        ];
        assert_eq!(
            t.edits(&[("app/views/users/_card.html.slim", "app/views/users/_tile.html.slim")], &sources),
            [
                ("app/controllers/users_controller.rb".into(), 0, 17, "users/tile".into()),
                ("app/helpers/cards_helper.rb".into(), 0, 8, "users/tile".into()),
            ]
        );
    }

    #[test]
    fn local_passes_find_callers() {
        let t = Tree::new("locals", &["app/views/users/_card.html.erb"]);
        let sources = [
            t.source(
                "app/views/users/index.html.erb",
                ERB,
                "<%= render \"card\", current_user: cu, user: u, size: 1 %>\n<%= render \"card\", size: 2 %>\n",
            ),
            t.source("app/views/home/show.html.slim", SLIM, "= render \"users/card\", user: current_user\n"),
            t.source("app/views/home/other.html.erb", ERB, "<%= render \"other\", user: u %>\n"),
            t.source("app/views/home/list.html.slim", SLIM, "= render \"users/card\",\n    size: 1,\n    user: u\n"),
        ];
        let card = t.path("app/views/users/_card.html.erb");
        let mut got: Vec<(String, u32, u32)> = local_passes(&Project::unknown(), &card, "user", &sources, &[])
            .into_iter()
            .map(|l| (t.rel(&l.uri.to_file_path().unwrap()), l.range.start.line, l.range.start.character))
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                ("app/views/home/list.html.slim".into(), 2, 4),
                ("app/views/home/show.html.slim".into(), 0, 23),
                ("app/views/users/index.html.erb".into(), 0, 37)
            ]
        );
        assert!(local_passes(&Project::unknown(), &card, "missing", &sources, &[]).is_empty());
    }

    #[test]
    fn i18n_references_by_full_key() {
        let t = Tree::new("i18n", &[]);
        let sources = [
            t.source(
                "app/views/users/index.html.erb",
                ERB,
                "<%= t(\".title\") %>\n<p>t(\"users.index.title\")</p>\n<%= t \"users.index.title\" %>\n",
            ),
            t.source("app/views/users/show.html.slim", SLIM, "h1 = t(\".title\")\n= t(\"users.index.title\")\n"),
            t.source("app/helpers/users_helper.rb", None, "def x = I18n.t(\"users.index.title\") + t(\".title\")\n"),
        ];
        let mut got: Vec<(String, u32, u32)> = i18n_references(&Project::unknown(), "users.index.title", &sources, &[])
            .into_iter()
            .map(|l| (t.rel(&l.uri.to_file_path().unwrap()), l.range.start.line, l.range.start.character))
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                ("app/helpers/users_helper.rb".into(), 0, 16),
                ("app/views/users/index.html.erb".into(), 0, 7),
                ("app/views/users/index.html.erb".into(), 2, 7),
                ("app/views/users/show.html.slim".into(), 1, 5),
            ]
        );
    }

    #[test]
    fn references_find_renders_of_a_partial() {
        let t = Tree::new("refs", &["app/views/users/_card.html.slim", "app/views/shared/_card.html.erb"]);
        let sources = [
            t.source("app/views/users/index.html.erb", ERB, "<%= render \"card\" %>\n<p>render \"card\"</p>\n"),
            t.source("app/views/home/show.html.slim", SLIM, "= render \"users/card\"\n= render \"shared/card\"\n"),
            t.source("app/views/users/index.json.jbuilder", Some(TemplateLang::Ruby), "json.partial! \"users/card\"\n"),
            t.source("app/controllers/users_controller.rb", None, "render partial: \"users/card\"\n"),
            t.source("app/views/home/dot.html.erb", ERB, "<%= render \"./users/card\" %>\n"),
        ];
        let card = t.path("app/views/users/_card.html.slim");
        let mut got: Vec<(String, u32, u32)> = references(&Project::unknown(), &[card], &sources, &[])
            .into_iter()
            .map(|l| {
                let file = t.rel(&l.uri.to_file_path().unwrap());
                (file, l.range.start.line, l.range.start.character)
            })
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                ("app/controllers/users_controller.rb".into(), 0, 17),
                ("app/views/home/dot.html.erb".into(), 0, 12),
                ("app/views/home/show.html.slim".into(), 0, 10),
                ("app/views/users/index.html.erb".into(), 0, 12),
            ]
        );
    }

    #[test]
    fn references_skip_shadowed_partials_and_count_model_renders() {
        let t = Tree::new(
            "shadow",
            &["app/views/users/_card.html.erb", "app/views/application/_card.html.erb", "app/views/users/_user.html.erb"],
        );
        let sources = [
            t.source("app/views/users/index.html.erb", ERB, "<%= render \"card\" %>\n<%= render @users %>\n"),
            t.source("app/views/posts/index.html.erb", ERB, "<%= render \"card\" %>\n<%# render @users %>\n"),
        ];
        let found = |rel: &str| -> Vec<(String, u32)> {
            let mut got: Vec<_> = references(&Project::unknown(), &[t.path(rel)], &sources, &[])
                .into_iter()
                .map(|l| (t.rel(&l.uri.to_file_path().unwrap()), l.range.start.line))
                .collect();
            got.sort();
            got
        };
        assert_eq!(found("app/views/users/_card.html.erb"), [("app/views/users/index.html.erb".into(), 0)]);
        assert_eq!(found("app/views/application/_card.html.erb"), [("app/views/posts/index.html.erb".into(), 0)]);
        assert_eq!(found("app/views/users/_user.html.erb"), [("app/views/users/index.html.erb".into(), 1)]);
    }

    #[test]
    fn variant_named_neighbour_does_not_block_rename() {
        let t = Tree::new("v2", &["app/views/users/_user.html.erb", "app/views/users/_user.v2.html.erb"]);
        let sources = [t.source("app/views/users/index.html.erb", ERB, "<%= render \"user\" %>\n")];
        let got = t.edits(&[("app/views/users/_user.html.erb", "app/views/users/_member.html.erb")], &sources);
        assert_eq!(got, [("app/views/users/index.html.erb".into(), 0, 12, "member".into())]);
    }

    #[test]
    fn edit_ranges_cover_the_name_on_later_lines() {
        let t = Tree::new("ranges", &["app/views/users/_card.html.slim"]);
        let rename = [(t.path("app/views/users/_card.html.slim"), t.path("app/views/users/_tile.html.slim"))];
        let range_of = |source: Source| {
            let edits = render_edits(&Project::unknown(), &rename, &[source], &[]);
            let edit = edits.into_values().next().unwrap().remove(0);
            (edit.range.start, edit.range.end)
        };
        let at = |line, start, end| (Position::new(line, start), Position::new(line, end));
        let erb = t.source("app/views/home/show.html.erb", ERB, "é<p></p>\n<%= render \"users/card\" %>\n");
        assert_eq!(range_of(erb), at(1, 12, 22));
        let slim = t.source("app/views/users/index.html.slim", SLIM, "p é\n= render \"card\"\n");
        assert_eq!(range_of(slim), at(1, 10, 14));
    }

    #[test]
    fn sources_keep_files_that_render_and_prefer_open_text() {
        let t = Tree::new(
            "sources",
            &[
                "app/views/users/index.html.erb",
                "app/views/users/notes.txt",
                "app/controllers/users_controller.rb",
                "app/helpers/h.rb",
                "app/models/user.rb",
            ],
        );
        let controller = "render partial: 'users/card'";
        std::fs::write(t.path("app/controllers/users_controller.rb"), controller).unwrap();
        std::fs::write(t.path("app/helpers/h.rb"), "def h = 1").unwrap();
        let open = HashMap::from([(t.path("app/views/users/index.html.erb"), "<%= render 'unsaved' %>".to_string())]);
        let mut found: Vec<_> =
            sources(&t.path("app/views"), &open).into_iter().map(|s| (t.rel(&s.path), s.lang.is_none(), s.text)).collect();
        found.sort();
        assert_eq!(
            found,
            [
                ("app/controllers/users_controller.rb".to_string(), true, controller.to_string()),
                ("app/views/users/index.html.erb".to_string(), false, "<%= render 'unsaved' %>".to_string()),
            ]
        );
    }
}
