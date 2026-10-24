//! `workspace/willRenameFiles`: rewrite literal `render` references to partials being renamed or moved.

use crate::partials::{render_refs, resolve, views_root, RenderKind};
use crate::{in_ruby_context, util, TemplateLang};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{TextEdit, Url};

pub struct Source {
    pub path: PathBuf,
    pub lang: TemplateLang,
    pub text: String,
}

/// `app/views/users/_card.html.slim` → `users/card`; `None` for non-partials and files outside a views dir.
fn qualified_name(views: &Path, file: &Path) -> Option<String> {
    let dir = file
        .parent()?
        .strip_prefix(views)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    let name = file
        .file_name()?
        .to_str()?
        .strip_prefix('_')?
        .split('.')
        .next()?;
    (!dir.is_empty() && !name.is_empty()).then(|| format!("{dir}/{name}"))
}

/// The name a reference written as `current` should use once `old` becomes `new`.
fn renamed_ref(current: &str, old: &Path, new: &Path) -> Option<String> {
    let views = views_root(old)?;
    let qualified = qualified_name(&views, new)?;
    if current.contains('/') || old.parent() != new.parent() {
        return Some(qualified);
    }
    qualified.rsplit('/').next().map(str::to_string)
}

/// Edits for every partial reference that resolves only to renamed files that all agree on the new name;
/// a reference also matching a file left in place (another format or variant) is not touched.
pub fn render_edits(
    renames: &[(PathBuf, PathBuf)],
    sources: &[Source],
) -> HashMap<Url, Vec<TextEdit>> {
    let mut out = HashMap::new();
    for Source { path, lang, text } in sources {
        let mut edits = Vec::new();
        for (i, line) in text.lines().enumerate() {
            for r in render_refs(line) {
                if r.kind != RenderKind::Partial || !in_ruby_context(*lang, line, r.start) {
                    continue;
                }
                let names: Option<Vec<String>> = resolve(path, &r)
                    .iter()
                    .map(|target| {
                        renames
                            .iter()
                            .find(|(old, _)| old == target)
                            .and_then(|(old, new)| renamed_ref(&r.name, old, new))
                    })
                    .collect();
                let Some(names) = names else { continue };
                let Some(name) = names
                    .first()
                    .filter(|n| **n != r.name && names.iter().all(|m| m == *n))
                else {
                    continue;
                };
                edits.push(TextEdit::new(
                    util::line_range(text, i, r.start, r.end),
                    name.clone(),
                ));
            }
        }
        if let (false, Ok(uri)) = (edits.is_empty(), Url::from_file_path(path)) {
            out.insert(uri, edits);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_and_moves_rewrite_matching_renders() {
        let root = std::env::temp_dir().join(format!("rt-rename-{}", std::process::id()));
        let views = root.join("app/views");
        for dir in ["users", "shared"] {
            std::fs::create_dir_all(views.join(dir)).unwrap();
        }
        let card = views.join("users/_card.html.slim");
        std::fs::write(&card, "").unwrap();
        std::fs::write(views.join("users/_row.html.erb"), "").unwrap();
        std::fs::write(views.join("users/_row.html+phone.erb"), "").unwrap();
        let slim = Source {
            path: views.join("users/index.html.slim"),
            lang: TemplateLang::Slim,
            text: "= render \"card\"\np = render partial: 'users/card', locals: {}\np we render \"card\"\n= render \"row\"\n".into(),
        };
        let erb = Source {
            path: views.join("shared/page.html.erb"),
            lang: TemplateLang::Erb,
            text: "é<%= render \"users/card\" %> <p>render \"users/card\"</p>\n".into(),
        };
        let sources = [slim, erb];
        let edits = |renames: &[(PathBuf, PathBuf)]| {
            let out = render_edits(renames, &sources);
            let mut got: Vec<(String, u32, u32, String)> = out
                .iter()
                .flat_map(|(uri, edits)| {
                    edits.iter().map(move |e| {
                        (
                            uri.path().rsplit('/').next().unwrap().to_string(),
                            e.range.start.line,
                            e.range.start.character,
                            e.new_text.clone(),
                        )
                    })
                })
                .collect();
            got.sort();
            got
        };
        let renamed = edits(&[(card.clone(), views.join("users/_tile.html.slim"))]);
        assert_eq!(
            renamed,
            [
                ("index.html.slim".into(), 0, 10, "tile".into()),
                ("index.html.slim".into(), 1, 21, "users/tile".into()),
                ("page.html.erb".into(), 0, 13, "users/tile".into()),
            ]
        );
        let moved = edits(&[(card.clone(), views.join("shared/_card.html.slim"))]);
        assert_eq!(
            moved.iter().map(|e| e.3.as_str()).collect::<Vec<_>>(),
            ["shared/card", "shared/card", "shared/card"]
        );
        assert!(edits(&[(card.clone(), views.join("users/_card.html.erb"))]).is_empty());
        assert!(edits(&[(card.clone(), views.join("users/card.html.slim"))]).is_empty());
        let row = views.join("users/_row.html.erb");
        assert!(edits(&[(row.clone(), views.join("users/_line.html.erb"))]).is_empty());
        let phone = views.join("users/_row.html+phone.erb");
        let both = edits(&[
            (row, views.join("users/_line.html.erb")),
            (phone, views.join("users/_line.html+phone.erb")),
        ]);
        assert_eq!(both, [("index.html.slim".into(), 3, 10, "line".into())]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
