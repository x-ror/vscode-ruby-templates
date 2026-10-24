//! `workspace/willRenameFiles`: rewrite literal `render` references to partials that are renamed or moved,
//! one at a time or with their folder.

use crate::partials::{render_refs, resolve, template_stem, views_root, RenderKind, RenderRef};
use crate::util::{lines_with_offsets, offset_to_position};
use crate::{render_filter, slim_outline, TemplateLang};
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tower_lsp::lsp_types::{Range, TextEdit, Url};
use walkdir::WalkDir;

static PARTIAL_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"partial\s*(?::|=>)\s*["']$"#).unwrap());

pub struct Source {
    pub path: PathBuf,
    /// `None` for `.rb` files, where every string is code.
    pub lang: Option<TemplateLang>,
    pub text: String,
}

/// `app/views/users/_card.html.slim` → `users/card`; `None` for non-partials and files outside a views dir.
fn qualified_name(views: &Path, file: &Path) -> Option<String> {
    let dir = file.parent()?.strip_prefix(views).ok()?.to_string_lossy().replace('\\', "/");
    let name = template_stem(file.file_name()?.to_str()?.strip_prefix('_')?);
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
fn new_ref_name(written: &str, source: &Path, target: &Path, renames: &[(PathBuf, PathBuf)]) -> Option<String> {
    let qualified = qualified_name(&views_root(target)?, target)?;
    let source_after = moved(source, renames).unwrap_or_else(|| source.to_path_buf());
    if written.contains('/') || source_after.parent() != target.parent() {
        return Some(qualified);
    }
    qualified.rsplit('/').next().map(str::to_string)
}

fn app_views(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|a| a.file_name().is_some_and(|n| n == "app")).map(|app| app.join("views"))
}

/// Literal `render` references inside code, as (byte start, byte end, reference).
fn code_refs(source: &Source, extra_engines: &[String]) -> Vec<(usize, usize, RenderRef)> {
    let shift = |offset: usize| move |r: RenderRef| (offset + r.start, offset + r.end, r);
    if matches!(source.lang, Some(TemplateLang::Slim | TemplateLang::Skim)) {
        let spans = slim_outline::code_spans(&source.text, extra_engines);
        return spans.into_iter().flat_map(|(s, e)| render_refs(&source.text[s..e]).into_iter().map(shift(s))).collect();
    }
    let template_code = source.lang.map(|lang| render_filter(lang, &source.text, extra_engines));
    let mut out = Vec::new();
    for (i, (offset, line)) in lines_with_offsets(&source.text).enumerate() {
        let in_code = |r: &RenderRef| match &template_code {
            Some(in_code) => in_code(i, offset, r),
            None => !line.trim_start().starts_with('#'),
        };
        out.extend(render_refs(line).into_iter().filter(in_code).map(shift(offset)));
    }
    out
}

/// The files a reference points at. Views look partials up from their own folder (and `layout:` there is a
/// partial too); code outside app/views is checked only for qualified names, since its controller prefix is
/// unknown, and controllers' plain `render "x"` is a template, so only `partial:` counts there.
fn targets(source: &Source, r: &RenderRef, explicit_partial: bool) -> Vec<PathBuf> {
    let as_partial = RenderRef { kind: RenderKind::Partial, ..r.clone() };
    if views_root(&source.path).is_some() {
        return match r.kind {
            RenderKind::Template => vec![],
            RenderKind::Partial => resolve(&source.path, &as_partial),
        };
    }
    let controller = source.path.components().any(|c| c.as_os_str() == "controllers");
    let partial = if controller { explicit_partial } else { r.kind == RenderKind::Partial };
    match app_views(&source.path) {
        Some(views) if partial && r.name.contains('/') => resolve(&views.join("_"), &as_partial),
        _ => vec![],
    }
}

/// Folders outside app/views whose code renders partials.
const CODE_DIRS: &[&str] = &["controllers", "helpers", "components"];

/// Templates under `views`, plus Ruby files and templates in app/{controllers,helpers,components}. Text of
/// `open` documents wins over the disk copy, which may lack unsaved edits.
pub fn sources(views: &Path, open: &HashMap<PathBuf, String>) -> Vec<Source> {
    let code_dirs = views.parent().into_iter().flat_map(|app| CODE_DIRS.iter().map(move |d| app.join(d)));
    std::iter::once(views.to_path_buf())
        .chain(code_dirs)
        .flat_map(|root| WalkDir::new(root).into_iter().flatten())
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
            Some(Source { path, lang, text })
        })
        .collect()
}

/// Edits for every partial reference that resolves only to renamed files that all agree on the new name;
/// a reference also matching a file left in place (another format or variant) is not touched.
pub fn render_edits(renames: &[(PathBuf, PathBuf)], sources: &[Source], extra_engines: &[String]) -> HashMap<Url, Vec<TextEdit>> {
    // Skip `resolve` (a directory listing) for references that can't name a renamed partial.
    let folder_renamed = renames.iter().any(|(old, _)| old.is_dir());
    let stems: Vec<&str> = renames.iter().filter_map(|(old, _)| old.file_name()?.to_str()?.strip_prefix('_').map(template_stem)).collect();
    let may_match = |name: &str| folder_renamed || stems.contains(&name.rsplit('/').next().unwrap_or(name));

    let mut out = HashMap::new();
    for source in sources {
        let mut edits = Vec::new();
        for (start, end, r) in code_refs(source, extra_engines) {
            if !may_match(&r.name) {
                continue;
            }
            let explicit_partial = source.lang.is_none() && PARTIAL_KEY.is_match(source.text[..start].rsplit('\n').next().unwrap_or(""));
            let found = targets(source, &r, explicit_partial);
            // Rails picks from the first folder that has a match; later ones (`application/`) are shadowed.
            let first_dir = found.first().and_then(|t| t.parent());
            let names: Option<Vec<String>> = found
                .iter()
                .filter(|t| t.parent() == first_dir)
                .map(|t| new_ref_name(&r.name, &source.path, &moved(t, renames)?, renames))
                .collect();
            let Some(names) = names else { continue };
            let Some(name) = names.first().filter(|n| **n != r.name && names.iter().all(|m| m == *n)) else {
                continue;
            };
            let range = Range::new(offset_to_position(&source.text, start), offset_to_position(&source.text, end));
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
        fn source(&self, rel: &str, lang: Option<TemplateLang>, text: &str) -> Source {
            Source { path: self.path(rel), lang, text: text.into() }
        }
        fn edits(&self, renames: &[(&str, &str)], sources: &[Source]) -> Vec<(String, u32, u32, String)> {
            let renames: Vec<_> = renames.iter().map(|(a, b)| (self.path(a), self.path(b))).collect();
            let mut got: Vec<_> = render_edits(&renames, sources, &[])
                .iter()
                .flat_map(|(uri, edits)| {
                    let file = uri.to_file_path().unwrap().strip_prefix(&self.0).unwrap().to_string_lossy().into_owned();
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
                "render partial: \"users/card\"\nrender \"users/card\"\n# render partial: \"users/card\"\n",
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
            let edits = render_edits(&rename, &[source], &[]);
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
    fn sources_include_ruby_files_and_prefer_open_text() {
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
        let open = HashMap::from([(t.path("app/views/users/index.html.erb"), "unsaved".to_string())]);
        let mut found: Vec<_> = sources(&t.path("app/views"), &open)
            .into_iter()
            .map(|s| (s.path.strip_prefix(&t.0).unwrap().to_string_lossy().into_owned(), s.lang.is_none(), s.text))
            .collect();
        found.sort();
        let ruby = |p: &str| (p.to_string(), true, String::new());
        assert_eq!(
            found,
            [
                ruby("app/controllers/users_controller.rb"),
                ruby("app/helpers/h.rb"),
                ("app/views/users/index.html.erb".to_string(), false, "unsaved".to_string()),
            ]
        );
    }
}
