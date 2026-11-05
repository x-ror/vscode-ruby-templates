//! Resolve `render "partial"` and friends to files; list partials for completion.

use crate::rails::controller_classes;
use crate::rails::controller_classes::controller_file;
use crate::rails::inflect;
use crate::rails::inflect::{pluralize, singularize};
use crate::rails::paths::{
    app_root, is_partial_path, template_format, template_stem, template_suffix, template_variant, view_relative, views_root,
};
use crate::rails::project::Project;
use crate::rails::render_refs::{Lookup, RenderKind, RenderRef};
use crate::util::slash_path;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Resolve a render reference to candidate files.
pub fn resolve(project: &Project, current: &Path, r: &RenderRef) -> Vec<PathBuf> {
    let views = views_root(current);
    let (dir, base) = split_render_name(&r.name);
    let basename = match r.kind {
        RenderKind::Partial => format!("_{base}"),
        RenderKind::Template => base.to_string(),
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut searched: Vec<PathBuf> = Vec::new();
    let mut in_app = false;
    match (&dir, &views, r.kind) {
        // Rails searches the app's root before an engine's, so the app can override an engine's view.
        (Some(_), Some(v), _) => {
            let app = project.app_views_of(v);
            in_app = app.is_some();
            searched.extend(app.into_iter().filter(|app| app != v));
            searched.push(v.clone());
        }
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
                let prefixes = controller_classes::view_prefixes(project, current).unwrap_or_else(|| vec!["application".to_string()]);
                for prefix in &prefixes {
                    let dir = v.join(prefix);
                    if !dirs.contains(&dir) {
                        dirs.push(dir);
                    }
                }
            }
        }
    }
    let found_in = |d: &Path| {
        let mut found: Vec<PathBuf> =
            matching_templates(project, d, &basename).into_iter().filter(|p| looked_up(project, current, p, &r.lookup)).collect();
        sort_like_action_view(project, current, &r.lookup, &mut found);
        found
    };
    let Some(d) = dir.filter(|_| !searched.is_empty()) else { return dirs.iter().flat_map(|d| found_in(d)).collect() };
    // Rails takes the first root with a match; a qualified name missing from these may be another engine's or a gem's.
    let mut in_roots = searched.iter().map(|root| found_in(&root.join(d))).filter(|found| !found.is_empty());
    let found = in_roots.next().or_else(|| {
        // Another Rails app (`test/dummy`) renders from its own roots, not from the sibling apps'.
        let others = project.view_roots().filter(|_| in_app)?.roots_after(&searched);
        others.into_iter().map(|root| found_in(&root.join(d))).find(|found| !found.is_empty())
    });
    found.unwrap_or_default()
}

/// `user`/`users` → app/views/users/_user.* (`to_partial_path`); nothing when the inflection finds no file.
/// A model that only exists namespaced (`app/models/admin/user.rb`) looks in `admin/users`. A view under a namespace
/// directory gets it prefixed first, as Rails does with `prefix_partial_path_with_controller_namespace`.
pub fn resolve_model(project: &Project, current: &Path, name: &str) -> Vec<PathBuf> {
    let Some(views) = views_root(current) else { return vec![] };
    let singular = singularize(name);
    let plural = pluralize(&singular);
    let view_dir = view_relative(current).and_then(Path::parent).unwrap_or(Path::new(""));
    let mut namespaces = model_namespaces(project, &views, current, &singular);
    namespaces.push(PathBuf::new());
    for namespace in namespaces {
        let prefixed = views.join(prefixed_with_view_namespace(view_dir, &namespace)).join(&plural);
        // Rails raises MissingTemplate rather than trying the model's own folder, but only for a controller's action views.
        let dirs = if project.prefixes_partial_paths && !is_partial_path(current) && is_controller_folder(&views, view_dir) {
            vec![prefixed]
        } else {
            vec![prefixed, views.join(namespace.join(&plural))]
        };
        for dir in dirs {
            let mut found: Vec<PathBuf> = matching_templates(project, &dir, &format!("_{singular}"))
                .into_iter()
                .filter(|p| renders_format(project, current, p))
                .collect();
            if !found.is_empty() {
                sort_like_action_view(project, current, &Lookup::default(), &mut found);
                return found;
            }
        }
    }
    vec![]
}

/// Is `views/<view_dir>` the folder of an existing controller (`admin/users` for `admin/users_controller.rb`)?
fn is_controller_folder(views: &Path, view_dir: &Path) -> bool {
    let Some(app) = views.parent() else { return false };
    controller_file(app, &view_dir.to_string_lossy()).is_file()
}

/// The model's namespace behind the namespace directories of `view_dir` (all but its last part) that it doesn't start with
/// (ActionView's `merge_prefix_into_object_path`): `admin/orders` + `` → `admin`, `shop/orders` + `admin` → `shop/admin`.
fn prefixed_with_view_namespace(view_dir: &Path, model_namespace: &Path) -> PathBuf {
    let dirs: Vec<_> = view_dir.components().collect();
    let Some((_, view_namespace)) = dirs.split_last() else { return model_namespace.to_path_buf() };
    let mut model_parts = model_namespace.components();
    let prefix: PathBuf = view_namespace.iter().map_while(|dir| (Some(*dir) != model_parts.next()).then_some(dir)).collect();
    prefix.join(model_namespace)
}

/// The namespaces of `app/models/<namespace>/<singular>.rb`, those holding the view `current` first; none when a top-level
/// `app/models/<singular>.rb` exists, since the variable is likely that one. `concerns/` is no namespace.
fn model_namespaces(project: &Project, views: &Path, current: &Path, singular: &str) -> Vec<PathBuf> {
    let Some(models) = views.parent().map(|app| app.join("models")) else { return vec![] };
    let file = format!("{singular}.rb");
    if models.join(&file).is_file() {
        return vec![];
    }
    let mut namespaces: Vec<PathBuf> = project
        .namespaced_models(&models)
        .iter()
        .filter(|path| path.file_name().is_some_and(|name| name == file.as_str()))
        .filter_map(|path| Some(path.parent()?.to_path_buf()))
        .collect();
    let view_dir = view_relative(current).and_then(Path::parent);
    let outside_view = |namespace: &Path| !view_dir.is_some_and(|dir| dir.starts_with(namespace));
    namespaces.sort_by(|a, b| (outside_view(a), a).cmp(&(outside_view(b), b)));
    namespaces
}

/// Where a missing `dir/name` reference would live: `app/views/dir/_name.<format>+<variant>.<ext>` with the first of the
/// call's `formats:`, `variants:` and `handlers:`, else the view's format (variant dropped) and template language.
pub fn new_template_path(project: &Project, current: &Path, r: &RenderRef) -> Option<PathBuf> {
    let (Some(dir), base) = split_render_name(&r.name) else { return None };
    if base.is_empty() || dir.split('/').any(|part| part == "..") {
        return None;
    }
    let first = |names: &Option<Vec<String>>| names.as_ref().and_then(|names| names.first()).cloned();
    let ext = first(&r.lookup.handlers).or_else(|| Some(current.extension()?.to_str()?.to_string()))?;
    let prefix = if r.kind == RenderKind::Partial { "_" } else { "" };
    let format = first(&r.lookup.formats).or_else(|| template_format(|f| project.is_format(f), current).map(str::to_string));
    let format = format.map(|format| format!(".{format}")).unwrap_or_default();
    let variant = first(&r.lookup.variants).map(|variant| format!("+{variant}")).unwrap_or_default();
    let file = format!("{prefix}{base}{format}{variant}.{ext}");
    Some(views_root(current)?.join(dir).join(file))
}

/// The `resolve` results Rails would actually pick from: it stops at the first folder with a match, so later
/// ones (`application/`) are shadowed.
pub fn first_folder_only(mut found: Vec<PathBuf>) -> Vec<PathBuf> {
    let first = found.first().and_then(|f| f.parent()).map(Path::to_path_buf);
    found.retain(|f| f.parent() == first.as_deref());
    found
}

/// ` and 1 other` / ` and 2 others` after a file's name; nothing for none.
pub fn and_others(count: usize) -> String {
    match count {
        0 => String::new(),
        1 => " and 1 other".to_string(),
        n => format!(" and {n} others"),
    }
}

/// The formats Rails looks a partial up in for a view: its own, plus HTML from JS (`LookupContext#formats=`) and
/// from turbo_stream (its request's Accept header is `text/vnd.turbo-stream.html, text/html`).
fn lookup_formats(view: &str) -> impl Iterator<Item = &str> {
    std::iter::once(view).chain(matches!(view, "js" | "turbo_stream").then_some("html"))
}

/// Format-less files match any format.
fn renders_format(project: &Project, current: &Path, candidate: &Path) -> bool {
    let is_format = |f: &str| project.is_format(f);
    match (template_format(is_format, current), template_format(is_format, candidate)) {
        (Some(view), Some(partial)) => lookup_formats(view).any(|format| format == partial),
        _ => true,
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

/// A render name split into its folder and base name; the folder drops a leading `/` (`/users/card` is
/// `users` under app/views).
pub fn split_render_name(name: &str) -> (Option<&str>, &str) {
    match name.rsplit_once('/') {
        Some((dir, base)) => (Some(dir.trim_start_matches('/')), base),
        None => (None, name),
    }
}

/// Orders candidates like ActionView's `TemplateDetails#sort_key_for`: by format, variant, then handler (the locale,
/// unknown here, isn't ordered). With no `variants:` the view's own is wanted, as in Rails.
fn sort_like_action_view(project: &Project, current: &Path, lookup: &Lookup, found: &mut [PathBuf]) {
    let view_format = template_format(|f| project.is_format(f), current);
    let view_variant = template_variant(current);
    found.sort_by_key(|p| {
        let format = template_format(|f| project.is_format(f), p);
        let format_rank = match (&lookup.formats, view_format) {
            (Some(_), _) => rank(listed(&lookup.formats), format),
            (None, Some(view)) => rank(lookup_formats(view), format),
            (None, None) => usize::from(format != Some("html")) + usize::from(format.is_none()),
        };
        let wanted_variants = listed(&lookup.variants).chain(view_variant.filter(|_| lookup.variants.is_none()));
        (format_rank, rank(wanted_variants, template_variant(p)), rank(listed(&lookup.handlers), p.extension().and_then(|e| e.to_str())))
    });
}

fn listed(names: &Option<Vec<String>>) -> impl Iterator<Item = &str> {
    names.iter().flatten().map(String::as_str)
}

/// The place of `value` among the wanted `names`: Rails tries them in order, a template without that detail after
/// them all.
fn rank<'a>(names: impl Iterator<Item = &'a str>, value: Option<&str>) -> usize {
    let mut wanted = 0;
    for name in names {
        if value == Some(name) {
            return wanted;
        }
        wanted += 1;
    }
    wanted + usize::from(value.is_some())
}

/// Whether Rails could pick `candidate` for a render in `current` with the call's `lookup`: its format among
/// `formats:` (else one `current` renders), its variant among `variants:`, its handler among `handlers:`.
fn looked_up(project: &Project, current: &Path, candidate: &Path, lookup: &Lookup) -> bool {
    // A template without that detail (`_card.erb` has no format) is the fallback for every value.
    let among = |names: &Option<Vec<String>>, value: Option<&str>| {
        names.as_ref().is_none_or(|names| value.is_none_or(|value| names.iter().any(|name| name == value)))
    };
    let format_ok = if lookup.formats.is_some() {
        among(&lookup.formats, template_format(|f| project.is_format(f), candidate))
    } else {
        renders_format(project, current, candidate)
    };
    format_ok
        && among(&lookup.variants, template_variant(candidate))
        && among(&lookup.handlers, candidate.extension().and_then(|e| e.to_str()))
}

/// `Users::CardComponent` → app/components/users/card_component.rb (+ sidecar templates).
pub fn resolve_component(project: &Project, current: &Path, name: &str) -> Vec<PathBuf> {
    let Some(app) = app_root(current) else { return vec![] };
    let base = app.join("components").join(inflect::constant_path(name));
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
            let rel = slash_path(e.path().strip_prefix(&dir).ok()?);
            let name = rel.strip_suffix("_component.rb")?;
            let constant: Vec<String> = format!("{name}_component").split('/').map(inflect::camel_case).collect();
            Some((constant.join("::"), e.into_path()))
        })
        .collect();
    out.sort();
    out
}

/// Names `render` can complete for `kind`: partials (same-dir bare names, then `dir/name` under app/views)
/// or non-partial templates as `dir/name`. `files` lists app/views, or the current directory outside one; `other_roots`
/// are the app's other view roots (an engine's) with their files, after it.
pub fn render_candidates(
    project: &Project,
    current: &Path,
    kind: RenderKind,
    files: &[PathBuf],
    other_roots: &[(&Path, &[PathBuf])],
) -> Vec<(String, PathBuf)> {
    let views = views_root(current);
    let partial = kind == RenderKind::Partial;
    let same_dir = current
        .parent()
        .filter(|_| partial)
        .into_iter()
        .flat_map(|d| templates_in(project, d, files, true))
        .filter(|(name, _)| !name.contains('/'));
    let own = views.iter().map(|v| (v.as_path(), files));
    let nested = own
        .chain(other_roots.iter().copied())
        .flat_map(|(root, files)| templates_in(project, root, files, partial))
        .filter(|(name, _)| name.contains('/'));
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
        let rel_dir = slash_path(p.parent()?.strip_prefix(root).ok()?);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rails::paths::files_under;
    use crate::rails::render_refs::*;

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
        assert_eq!(path("home/show.html.erb", r#"render "/users/x""#), Some(views.join("users/_x.html.erb")));
        assert_eq!(
            path("users/show.html.erb", r#"render partial: "users/card", formats: [:json, :html]"#),
            Some(views.join("users/_card.json.erb"))
        );
        assert_eq!(
            path("users/show.html.erb", r#"render partial: "users/card", variants: [:phone], handlers: [:haml]"#),
            Some(views.join("users/_card.html+phone.haml"))
        );
        assert_eq!(path("users/show.html.erb", r#"render partial: "users/card", formats: []"#), Some(views.join("users/_card.html.erb")));
        assert_eq!(path("users/show.html.erb", r#"render "card""#), None);
        assert_eq!(path("users/show.html.erb", r#"render "../../config/x""#), None);
        assert_eq!(path("users/show.html.erb", r#"render "shared/""#), None);
        assert_eq!(new_template_path(&Project::unknown(), Path::new("/tmp/x.html.erb"), &render_refs(r#"render "a/b""#)[0]), None);
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
    fn model_render_partials_of_namespaced_models() {
        let root = std::env::temp_dir().join(format!("rt-ns-models-{}", std::process::id()));
        let views = root.join("app/views");
        for f in [
            "app/models/admin/user.rb",
            "app/models/admin/post.rb",
            "app/models/post.rb",
            "app/models/billing/invoice.rb",
            "app/views/admin/users/_user.html.erb",
            "app/views/users/_user.html.erb",
            "app/views/admin/posts/_post.html.erb",
            "app/views/posts/_post.html.erb",
            "app/views/billing/invoices/_invoice.html.erb",
            "app/views/billing/reports/show.html.erb",
            "app/views/admin/users/index.html.erb",
            "app/views/home/index.html.erb",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let found = |view: &str, name: &str| resolve_model(&Project::unknown(), &views.join(view), name);
        assert_eq!(found("home/index.html.erb", "users"), [views.join("admin/users/_user.html.erb")]);
        assert_eq!(found("admin/users/index.html.erb", "user"), [views.join("admin/users/_user.html.erb")]);
        assert_eq!(found("billing/reports/show.html.erb", "invoices"), [views.join("billing/invoices/_invoice.html.erb")]);
        // Rails prefixes the view's namespace, whether the model is `Post` or `Admin::Post`.
        assert_eq!(found("admin/users/index.html.erb", "posts"), [views.join("admin/posts/_post.html.erb")]);
        assert_eq!(found("home/index.html.erb", "posts"), [views.join("posts/_post.html.erb")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_partial_under_a_controller_folder_falls_back_to_the_model_folder() {
        let root = std::env::temp_dir().join(format!("rt-model-partial-fallback-{}", std::process::id()));
        crate::util::write_files(
            &root,
            &[
                ("app/controllers/admin/users_controller.rb", ""),
                ("app/views/admin/users/index.html.erb", ""),
                ("app/views/admin/users/_row.html.erb", ""),
                ("app/views/posts/_post.html.erb", ""),
            ],
        );
        let views = root.join("app/views");
        let found = |view: &str| resolve_model(&Project::unknown(), &views.join(view), "posts");
        assert!(found("admin/users/index.html.erb").is_empty(), "an action view gets no fallback");
        assert_eq!(found("admin/users/_row.html.erb"), [views.join("posts/_post.html.erb")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn namespaced_models_in_several_namespaces_prefer_the_views_own() {
        let root = std::env::temp_dir().join(format!("rt-ns-two-{}", std::process::id()));
        let views = root.join("app/views");
        for f in [
            "app/models/admin/user.rb",
            "app/models/shop/user.rb",
            "app/views/admin/users/_user.html.erb",
            "app/views/shop/users/_user.html.erb",
            "app/views/shop/orders/show.html.erb",
            "app/views/admin/orders/show.html.erb",
            "app/views/home/index.html.erb",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let found = |view: &str| resolve_model(&Project::unknown(), &views.join(view), "user");
        assert_eq!(found("shop/orders/show.html.erb"), [views.join("shop/users/_user.html.erb")]);
        assert_eq!(found("admin/orders/show.html.erb"), [views.join("admin/users/_user.html.erb")]);
        assert_eq!(found("home/index.html.erb"), [views.join("admin/users/_user.html.erb")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn model_partials_get_the_views_namespace_prefixed_like_rails() {
        let root = std::env::temp_dir().join(format!("rt-ns-prefix-{}", std::process::id()));
        let views = root.join("app/views");
        for f in [
            "app/models/admin/user.rb",
            "app/models/concerns/taggable.rb",
            "app/models/concerns/user.rb",
            "app/views/shop/admin/users/_user.html.erb",
            "app/views/admin/users/_user.html.erb",
            "app/views/shop/orders/show.html.erb",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let found = |view: &str| resolve_model(&Project::unknown(), &views.join(view), "user");
        assert_eq!(found("shop/orders/show.html.erb"), [views.join("shop/admin/users/_user.html.erb")]);
        let prefixed = |view_dir: &str, namespace: &str| prefixed_with_view_namespace(Path::new(view_dir), Path::new(namespace));
        assert_eq!(prefixed("a/b/c", "a/x"), Path::new("a/x"));
        assert_eq!(prefixed("a/b/c", ""), Path::new("a/b"));
        assert_eq!(prefixed("home", "admin"), Path::new("admin"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn model_partials_of_namespaced_views_fall_back_only_when_rails_would() {
        let root = std::env::temp_dir().join(format!("rt-ns-fallback-{}", std::process::id()));
        let views = root.join("app/views");
        let controller = "class Admin::UsersController < ApplicationController\nend\n";
        crate::util::write_files(
            &root,
            &[
                ("app/views/posts/_post.html.erb", ""),
                ("app/views/admin/users/index.html.erb", ""),
                ("app/views/shared/forms/show.html.erb", ""),
                ("app/views/posts/partials/_list.html.erb", ""),
                ("app/controllers/admin/users_controller.rb", controller),
            ],
        );
        let own = [views.join("posts/_post.html.erb")];
        let found = |view: &str| {
            let current = views.join(view);
            resolve_model(&Project::for_template(&current), &current, "posts")
        };
        assert!(found("admin/users/index.html.erb").is_empty(), "Rails raises MissingTemplate for admin/posts/_post");
        assert_eq!(found("shared/forms/show.html.erb"), own, "shared partials have no controller namespace");
        assert_eq!(found("posts/partials/_list.html.erb"), own, "a folder that is no controller's has no namespace");
        std::fs::create_dir_all(root.join("config/initializers")).unwrap();
        let setting = "Rails.application.config.action_view.prefix_partial_path_with_controller_namespace = false\n";
        std::fs::write(root.join("config/initializers/action_view.rb"), format!("# {setting}")).unwrap();
        assert!(found("admin/users/index.html.erb").is_empty(), "a comment doesn't turn it off");
        std::fs::write(root.join("config/initializers/action_view.rb"), setting).unwrap();
        assert_eq!(found("admin/users/index.html.erb"), own);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bare_names_resolve_through_the_controllers_parents() {
        let root = std::env::temp_dir().join(format!("rt-bare-prefixes-{}", std::process::id()));
        crate::util::write_files(
            &root,
            &[
                ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
                ("app/controllers/admin/base_controller.rb", "class Admin::BaseController < ApplicationController\nend\n"),
                ("app/controllers/admin/users_controller.rb", "class Admin::UsersController < Admin::BaseController\nend\n"),
                ("app/views/admin/users/index.html.erb", ""),
                ("app/views/admin/base/_nav.html.erb", ""),
                ("app/views/application/_flash.html.erb", ""),
            ],
        );
        let views = root.join("app/views");
        let current = views.join("admin/users/index.html.erb");
        let found = |name: &str| resolve(&Project::unknown(), &current, &render_refs(&format!("render \"{name}\"")).remove(0));
        assert_eq!(found("nav"), [views.join("admin/base/_nav.html.erb")]);
        assert_eq!(found("flash"), [views.join("application/_flash.html.erb")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bare_names_resolve_when_only_the_checks_are_unsure_of_the_lookup() {
        let root = std::env::temp_dir().join(format!("rt-bare-prefixes-unsure-{}", std::process::id()));
        crate::util::write_files(
            &root,
            &[
                ("Gemfile.lock", "GEM\n  specs:\n    actionview (7.1.0)\n    theme (1.0)\n      railties\n    railties (7.1.0)\n"),
                ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
                ("app/controllers/admin/base_controller.rb", "class Admin::BaseController < ApplicationController\nend\n"),
                (
                    "app/controllers/admin/users_controller.rb",
                    "class Admin::UsersController < Admin::BaseController\n  include Themeable\nend\n",
                ),
                ("lib/themeable.rb", "module Themeable\n  included { prepend_view_path \"x\" }\nend\n"),
                ("app/views/admin/users/index.html.erb", ""),
                ("app/views/admin/base/_nav.html.erb", ""),
            ],
        );
        let views = root.join("app/views");
        let current = views.join("admin/users/index.html.erb");
        let project = Project::for_template(&current);
        assert_eq!(
            controller_classes::view_prefixes(&project, &current),
            Some(vec!["admin/users".to_string(), "admin/base".to_string(), "application".to_string()])
        );
        assert!(!controller_classes::bare_lookup_is_certain(&project, &current));
        assert_eq!(resolve(&project, &current, &render_refs("render \"nav\"").remove(0)), [views.join("admin/base/_nav.html.erb")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_leading_slash_folder_from_views_root() {
        let root = std::env::temp_dir().join(format!("rt-slash-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        std::fs::write(users.join("_card.html.erb"), "").unwrap();
        let card = render_refs(r#"render "/users/card""#).remove(0);
        let found = resolve(&Project::unknown(), &root.join("app/views/home/index.html.erb"), &card);
        assert_eq!(found, [users.join("_card.html.erb")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn qualified_names_fall_back_to_engine_then_gem_roots() {
        let root = std::env::temp_dir().join(format!("rt-other-roots-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let lock = "GEM\n  specs:\n    actionview (7.1.0)\n    devise (4.9.4)\n      railties (>= 4.1)\n    railties (7.1.0)\n";
        let gem_views = "vendor/bundle/ruby/3.3.0/gems/devise-4.9.4/app/views";
        crate::util::write_files(
            &root,
            &[
                ("Gemfile.lock", lock),
                ("app/views/home/index.html.erb", ""),
                ("app/views/blog/_post.html.erb", "app"),
                ("engines/blog/app/views/blog/_post.html.erb", "engine"),
                ("engines/blog/app/views/blog/_comment.html.erb", ""),
                (&format!("{gem_views}/devise/shared/_links.html.erb"), ""),
                (&format!("{gem_views}/blog/_comment.html.erb"), "gem"),
                ("vendor/bundle/ruby/3.3.0/gems/devise-4.9.4/lib/devise/engine.rb", "class Engine < ::Rails::Engine; end"),
                ("test/dummy/app/views/ghost/_only_in_dummy.html.erb", ""),
            ],
        );
        let current = root.join("app/views/home/index.html.erb");
        let project = Project::for_template(&current);
        let engine_comment = root.join("engines/blog/app/views/blog/_comment.html.erb");
        let found = |name: &str| resolve(&project, &current, &render_refs(&format!("render \"{name}\"")).remove(0));
        assert_eq!(found("blog/post"), [root.join("app/views/blog/_post.html.erb")], "the app's own root wins");
        assert_eq!(found("blog/comment"), [root.join("engines/blog/app/views/blog/_comment.html.erb")], "an engine before a gem");
        let links = root.join(gem_views).join("devise/shared/_links.html.erb");
        assert_eq!(found("devise/shared/links"), std::slice::from_ref(&links));
        assert!(found("devise/shared/nope").is_empty());
        assert!(found("ghost/only_in_dummy").is_empty(), "a dummy app's views are fixtures");
        let engine_view = root.join("engines/blog/app/views/blog/_post.html.erb");
        let from_engine = |name: &str| resolve(&project, &engine_view, &render_refs(&format!("render \"{name}\"")).remove(0));
        assert!(from_engine("home/missing").is_empty());
        assert_eq!(from_engine("blog/post"), [root.join("app/views/blog/_post.html.erb")], "the app overrides an engine's view");
        assert_eq!(from_engine("blog/comment"), [engine_comment], "the engine's own view before other roots");
        assert!(project.is_gem_file(&links));
        assert!(!project.is_gem_file(&engine_view));
        let candidates = render_candidates(
            &project,
            &current,
            RenderKind::Partial,
            &[root.join("app/views/home/index.html.erb")],
            &[(&root.join("engines/blog/app/views"), std::slice::from_ref(&engine_view))],
        );
        assert_eq!(candidates, [("blog/post".to_string(), engine_view)]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn another_rails_app_in_the_root_renders_from_its_own_views() {
        let root = std::env::temp_dir().join(format!("rt-other-apps-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        crate::util::write_files(
            &root,
            &[
                ("Gemfile.lock", "GEM\n  specs:\n    actionview (7.1.0)\n"),
                ("app/views/blog/_post.html.erb", "root"),
                ("test/dummy/config/application.rb", ""),
                ("test/dummy/app/views/blog/_post.html.erb", "dummy"),
                ("test/dummy/app/views/home/index.html.erb", ""),
                ("apps/b/config/application.rb", ""),
                ("apps/b/app/views/x/index.html.erb", ""),
            ],
        );
        let ask = |from: &str, name: &str| {
            let current = root.join(from);
            resolve(&Project::for_template(&current), &current, &render_refs(&format!("render \"{name}\"")).remove(0))
        };
        assert_eq!(ask("test/dummy/app/views/home/index.html.erb", "blog/post"), [root.join("test/dummy/app/views/blog/_post.html.erb")]);
        assert!(ask("apps/b/app/views/x/index.html.erb", "blog/post").is_empty(), "no falling into the root app or a sibling");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_by_view_format() {
        let root = std::env::temp_dir().join(format!("rt-formats-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        for f in ["_user.html.erb", "_user.json.jbuilder", "_user.html+phone.erb", "_user.erb"] {
            std::fs::write(users.join(f), "").unwrap();
        }
        let views = root.join("app/views");
        let found = |view: &str| resolved_names(&Project::unknown(), &views, &format!("users/{view}"), r#"render "users/user""#);
        assert_eq!(found("index.html.erb"), ["_user.html.erb", "_user.html+phone.erb", "_user.erb"]);
        assert_eq!(found("show.html+phone.erb"), ["_user.html+phone.erb", "_user.html.erb", "_user.erb"]);
        assert_eq!(found("show.erb"), ["_user.html.erb", "_user.html+phone.erb", "_user.json.jbuilder", "_user.erb"]);
        let render = render_refs(r#"render "users/user""#).remove(0);
        for view in ["index.html.erb", "show.html+phone.erb", "show.erb", "index.json.jbuilder"] {
            let current = users.join(view);
            assert_eq!(resolve_model(&Project::unknown(), &current, "users"), resolve(&Project::unknown(), &current, &render));
        }
        assert_eq!(found("index.json.jbuilder"), ["_user.json.jbuilder", "_user.erb"]);
        assert_eq!(found("create.js.erb"), found("index.html.erb"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn turbo_stream_view_renders_turbo_stream_then_html_partials() {
        let root = std::env::temp_dir().join(format!("rt-turbo-{}", std::process::id()));
        let cards = root.join("app/views/cards");
        std::fs::create_dir_all(&cards).unwrap();
        let views = root.join("app/views");
        let found = |view: &str| resolved_names(&Project::unknown(), &views, &format!("cards/{view}"), r#"render "cards/card""#);
        std::fs::write(cards.join("_card.js.erb"), "").unwrap();
        assert!(found("create.turbo_stream.erb").is_empty());
        assert_eq!(found("create.js.erb"), ["_card.js.erb"]);
        std::fs::write(cards.join("_card.html.erb"), "").unwrap();
        assert_eq!(found("create.turbo_stream.erb"), ["_card.html.erb"]);
        assert_eq!(found("create.js.erb"), ["_card.js.erb", "_card.html.erb"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn partials_are_ordered_by_the_views_formats_then_variants_then_handlers() {
        let root = std::env::temp_dir().join(format!("rt-partial-order-{}", std::process::id()));
        let cards = root.join("app/views/cards");
        std::fs::create_dir_all(&cards).unwrap();
        let views = root.join("app/views");
        let found = |view: &str, line: &str| resolved_names(&Project::unknown(), &views, &format!("cards/{view}"), line);
        let write = |names: &[&str]| {
            for entry in std::fs::read_dir(&cards).unwrap() {
                std::fs::remove_file(entry.unwrap().path()).unwrap();
            }
            for name in names {
                std::fs::write(cards.join(name), "").unwrap();
            }
        };
        let plain = r#"render "cards/card""#;

        write(&["_card.html.erb", "_card.turbo_stream.erb", "_card.erb", "_card.js.erb"]);
        assert_eq!(found("create.turbo_stream.erb", plain), ["_card.turbo_stream.erb", "_card.html.erb", "_card.erb"]);
        let html = r#"render partial: "cards/card", formats: [:html]"#;
        assert_eq!(found("create.turbo_stream.erb", html), ["_card.html.erb", "_card.erb"]);
        let js = r#"render partial: "cards/card", formats: :js"#;
        assert_eq!(found("create.turbo_stream.erb", js), ["_card.js.erb", "_card.erb"]);

        write(&["_a.atom.builder", "_a.html.erb"]);
        assert_eq!(found("x.erb", r#"render "cards/a""#), ["_a.html.erb", "_a.atom.builder"]);

        write(&["_e.erb", "_e.js.erb"]);
        assert_eq!(found("x.js.erb", r#"render "cards/e""#), ["_e.js.erb", "_e.erb"]);

        write(&["_c.html+phone.erb", "_c.js.erb"]);
        let phone = r#"render partial: "cards/c", variants: [:phone]"#;
        assert_eq!(found("x.js.erb", phone), ["_c.js.erb", "_c.html+phone.erb"]);

        write(&["_h.js.haml", "_h.html.erb", "_h.js.erb"]);
        assert_eq!(found("x.js.erb", r#"render partial: "cards/h", handlers: [:erb, :haml]"#), ["_h.js.erb", "_h.js.haml", "_h.html.erb"]);
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
        let views = root.join("app/views");
        let found = |view: &str| {
            resolved_names(&Project::for_template(&users.join(view)), &views, &format!("users/{view}"), r#"render "users/card""#)
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
        let names =
            |kind| render_candidates(&Project::unknown(), &current, kind, &files, &[]).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names(RenderKind::Partial), ["card", "layouts/nav", "users/card"]);
        assert_eq!(names(RenderKind::Template), ["layouts/admin/base", "layouts/application", "users/index"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    // Ruby 3.3 rejects a line starting with `&&` after an expression; the latest grammar takes it.

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

    fn resolved_names(project: &Project, views: &Path, view: &str, render: &str) -> Vec<String> {
        let r = render_refs(render).remove(0);
        resolve(project, &views.join(view), &r).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn nested_partial_names_use_forward_slashes() {
        let (root, views) =
            partial_tree("nested-names", &["users/admin/_card.html.erb", "users/admin/show.html.erb", "users/_row.html.erb"]);
        let project = Project::unknown();
        assert_eq!(resolved_names(&project, &views, "users/index.html.erb", "render \"users/admin/card\""), ["_card.html.erb"]);
        let found = resolve(&project, &views.join("home/show.html.erb"), &render_refs("render \"users/admin/card\"").remove(0));
        assert_eq!(found, [views.join("users/admin/_card.html.erb")]);
        let files = crate::rails::paths::files_under(&views, usize::MAX);
        let current = views.join("users/index.html.erb");
        let names = |kind| render_candidates(&project, &current, kind, &files, &[]).into_iter().map(|(name, _)| name).collect::<Vec<_>>();
        assert_eq!(names(RenderKind::Partial), ["row", "users/admin/card", "users/row"]);
        assert_eq!(names(RenderKind::Template), ["users/admin/show"]);
        let named: Vec<String> = named_templates(&project, &views, &files).into_iter().map(|(name, ..)| name).collect();
        assert!(named.contains(&"users/admin/card".to_string()) && named.iter().all(|name| !name.contains('\\')), "{named:?}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn locale_only_partial_resolves_from_any_format() {
        let (root, views) = partial_tree("locale-only", &["shared/_greeting.de.erb"]);
        for view in ["users/show.html.erb", "users/index.json.jbuilder", "users/show.erb"] {
            assert_eq!(resolved_names(&Project::unknown(), &views, view, r#"render "shared/greeting""#), ["_greeting.de.erb"], "{view}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn locale_and_format_partial_follows_view_format() {
        let (root, views) = partial_tree("locale-format", &["users/_card.en.html.erb", "users/_clip.js.erb", "users/_event.ics.erb"]);
        assert_eq!(resolved_names(&Project::unknown(), &views, "users/show.html.erb", r#"render "card""#), ["_card.en.html.erb"]);
        assert!(resolved_names(&Project::unknown(), &views, "users/index.json.jbuilder", r#"render "card""#).is_empty());
        // `js` has locale shape but is a format: it must not read as format-less and match JSON views.
        assert!(resolved_names(&Project::unknown(), &views, "users/index.json.jbuilder", r#"render "clip""#).is_empty());
        assert_eq!(resolved_names(&Project::unknown(), &views, "users/index.js.erb", r#"render "clip""#), ["_clip.js.erb"]);
        assert!(resolved_names(&Project::unknown(), &views, "users/index.json.jbuilder", r#"render "event""#).is_empty());
        assert_eq!(resolved_names(&Project::unknown(), &views, "users/index.ics.erb", r#"render "event""#), ["_event.ics.erb"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn partial_in_the_views_root() {
        let (root, views) = partial_tree("views-root", &["_note.html.erb", "users/_note.html.erb"]);
        assert_eq!(resolved_names(&Project::unknown(), &views, "users/show.html.erb", r#"render "/note""#), ["_note.html.erb"]);
        let found = resolve(&Project::unknown(), &views.join("users/show.html.erb"), &render_refs(r#"render "/note""#).remove(0));
        assert_eq!(found, [views.join("_note.html.erb")], "the root one, not the folder's");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lookup_options_pick_the_files() {
        let files = ["users/_card.html.erb", "users/_card.html+phone.erb", "users/_card.json.jbuilder", "users/_card.html.haml"];
        let (root, views) = partial_tree("lookup", &files);
        let names = |render: &str| resolved_names(&Project::unknown(), &views, "users/show.html.erb", render);
        assert_eq!(names(r#"render partial: "card", formats: [:json]"#), ["_card.json.jbuilder"]);
        assert_eq!(names(r#"render partial: "card", formats: :json, handlers: [:jbuilder]"#), ["_card.json.jbuilder"]);
        assert_eq!(names(r#"render partial: "card", handlers: %i[haml]"#), ["_card.html.haml"]);
        assert_eq!(names(r#"render partial: "card", variants: [:phone]"#), ["_card.html+phone.erb", "_card.html.erb", "_card.html.haml"]);
        assert_eq!(
            names(r#"render partial: "card", variants: [:tablet]"#),
            ["_card.html.erb", "_card.html.haml"],
            "only the variant-less ones"
        );
        // In the short form the hash is locals, and a computed value names nothing literal.
        assert_eq!(names(r#"render "card", formats: [:json]"#), ["_card.html.erb", "_card.html.haml", "_card.html+phone.erb"]);
        assert_eq!(
            names(r#"render partial: "card", formats: request_formats"#),
            ["_card.html.erb", "_card.html.haml", "_card.html+phone.erb"]
        );
        assert!(names(r#"render partial: "card", formats: [:xml]"#).is_empty(), "Rails raises Missing partial");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lookup_options_apply_to_render_template() {
        let (root, views) = partial_tree("lookup-template", &["users/card.html.erb", "users/card.json.jbuilder"]);
        let names = |render: &str| resolved_names(&Project::unknown(), &views, "users/show.html.erb", render);
        assert_eq!(names(r#"render template: "users/card", formats: [:json]"#), ["card.json.jbuilder"]);
        assert_eq!(names(r#"render template: "users/card", handlers: [:erb]"#), ["card.html.erb"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lookup_handlers_can_exclude_the_format_match() {
        let (root, views) = partial_tree("lookup-handlers", &["users/_card.json.jbuilder", "users/_card.html.erb"]);
        let names = |render: &str| resolved_names(&Project::unknown(), &views, "users/show.html.erb", render);
        assert!(names(r#"render partial: "card", formats: [:json], handlers: [:erb]"#).is_empty());
        assert_eq!(names(r#"render partial: "card", formats: %i(json html), handlers: %i(erb)"#), ["_card.html.erb"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lookup_orders_by_the_given_formats_and_variants() {
        let files = ["users/_card.html.erb", "users/_card.json.jbuilder", "users/_card.erb", "users/_card.en.json.jbuilder"];
        let (root, views) = partial_tree("lookup-order", &files);
        let names = |render: &str| resolved_names(&Project::unknown(), &views, "users/show.html.erb", render);
        assert_eq!(
            names(r#"render partial: "card", formats: [:json, :html]"#),
            ["_card.en.json.jbuilder", "_card.json.jbuilder", "_card.html.erb", "_card.erb"]
        );
        assert_eq!(names(r#"render partial: "card", formats: [:html, :json]"#)[..2], ["_card.html.erb", "_card.en.json.jbuilder"]);
        assert_eq!(names(r#"render partial: "card", formats: [:json]"#), ["_card.en.json.jbuilder", "_card.json.jbuilder", "_card.erb"]);
        std::fs::remove_dir_all(&root).unwrap();

        let files = ["users/_card.html+tablet.erb", "users/_card.html+phone.erb", "users/_card.html.erb"];
        let (root, views) = partial_tree("lookup-variant-order", &files);
        let names = |render: &str| resolved_names(&Project::unknown(), &views, "users/show.html.erb", render);
        assert_eq!(
            names(r#"render partial: "card", variants: [:phone, :tablet]"#),
            ["_card.html+phone.erb", "_card.html+tablet.erb", "_card.html.erb"]
        );
        assert_eq!(names(r#"render partial: "card", variants: [:tablet, :phone]"#)[..2], ["_card.html+tablet.erb", "_card.html+phone.erb"]);
        std::fs::remove_dir_all(&root).unwrap();

        let (root, views) = partial_tree("lookup-handler-order", &["users/_card.json.erb", "users/_card.json.jbuilder"]);
        let names = |render: &str| resolved_names(&Project::unknown(), &views, "users/show.html.erb", render);
        assert_eq!(
            names(r#"render partial: "card", formats: [:json], handlers: [:jbuilder, :erb]"#),
            ["_card.json.jbuilder", "_card.json.erb"]
        );
        assert_eq!(
            names(r#"render partial: "card", formats: [:json], handlers: [:erb, :jbuilder]"#),
            ["_card.json.erb", "_card.json.jbuilder"]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn variant_named_partial_is_not_the_bare_name() {
        let (root, views) = partial_tree("variant-name", &["users/_user.v2.html.erb", "users/_user.html.erb"]);
        assert_eq!(resolved_names(&Project::unknown(), &views, "users/show.html.erb", r#"render "user""#), ["_user.html.erb"]);
        assert_eq!(resolved_names(&Project::unknown(), &views, "users/show.html.erb", r#"render "user.v2""#), ["_user.v2.html.erb"]);
        let only_v2 = partial_tree("variant-only", &["users/_user.v2.html.erb"]);
        assert!(resolved_names(&Project::unknown(), &only_v2.1, "users/show.html.erb", r#"render "user""#).is_empty());
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
        let names =
            |kind| render_candidates(&Project::unknown(), &current, kind, &files, &[]).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names(RenderKind::Partial), ["card", "row", "user.v2", "users/card", "users/row", "users/user.v2"]);
        assert_eq!(names(RenderKind::Template), ["users/index"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
