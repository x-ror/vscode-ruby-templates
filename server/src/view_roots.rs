//! Every `app/views` folder a Rails app can render from: its own and its engines', packs' and components' under the
//! Rails root, and those of the installed gems that could ship views.

use crate::project::adds_view_paths;
use crate::rails_paths::files_under;
use dashmap::DashMap;
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use walkdir::{DirEntry, WalkDir};

/// Folders whose `app/views` are test fixtures (a dummy app), not roots Rails renders from.
const FIXTURE_FOLDERS: [&str; 2] = ["test", "spec"];
/// A gem's `lib` is searched this deep for the class that makes it an engine.
const ENGINE_SEARCH_DEPTH: usize = 3;
/// `root/packs/name/app/views` is four levels down; one more for nested packs.
const MAX_DEPTH: usize = 6;
const NOT_SEARCHED: [&str; 7] = ["node_modules", ".git", "tmp", "log", "public", "coverage", "storage"];
/// Installed and cached gems; their views come from the lockfile, not from walking.
const NOT_SEARCHED_PATHS: [&str; 2] = ["vendor/bundle", "vendor/cache"];
/// Rails' own gems; their views live in these folders.
const FRAMEWORK_GEMS: [&str; 9] =
    ["rails", "railties", "actionpack", "actionview", "actionmailer", "actioncable", "actionmailbox", "actiontext", "activestorage"];
pub const FRAMEWORK_VIEW_FOLDERS: [&str; 4] = ["rails", "active_storage", "action_mailbox", "action_text"];
/// Folders of `app/` that hold no Ruby code to scan (`app/views` itself is a root).
const NOT_CODE: [&str; 5] = ["views", "assets", "javascript", "frontend", "node_modules"];
/// Git's abbreviated hash is at least 7 characters.
const MIN_COMMIT_HASH: usize = 7;
/// A gem that depends on one of these can add view paths (an engine needs `railties`).
const VIEW_PATH_GEMS: [&str; 3] = ["railties", "actionpack", "actionview"];

/// `(name, version)` of the lockfile's gems, other than Rails' own and path gems, that depend (however indirectly) on
/// something that can add view paths: the ones whose `app/views` has to be found on disk.
pub fn view_capable_gems(lock: &str) -> Vec<(String, String)> {
    let mut specs: Vec<(&str, &str, Vec<&str>)> = Vec::new();
    let (mut in_specs, mut in_path_section) = (false, false);
    for line in lock.lines() {
        if !line.starts_with(' ') {
            (in_specs, in_path_section) = (false, line.trim_end() == "PATH");
        } else if line.trim_end() == "  specs:" {
            in_specs = true;
        } else if let Some(dependency) = in_specs.then(|| line.strip_prefix("      ")).flatten() {
            let (Some((.., deps)), Some(name)) = (specs.last_mut(), dependency.split_whitespace().next()) else { continue };
            deps.push(name);
        } else if let Some((name, version)) = in_specs.then(|| crate::project::spec_line(line)).flatten() {
            if !in_path_section {
                specs.push((name, version, Vec::new()));
            }
        }
    }
    let mut capable: HashSet<&str> = VIEW_PATH_GEMS.into_iter().collect();
    while let Some(&(name, ..)) = specs.iter().find(|(name, _, deps)| !capable.contains(name) && deps.iter().any(|d| capable.contains(d))) {
        capable.insert(name);
    }
    let shipping = specs.iter().filter(|(name, ..)| capable.contains(name) && !FRAMEWORK_GEMS.contains(name));
    shipping.map(|(name, version, _)| (name.to_string(), version.to_string())).collect()
}

/// Where bundler and RubyGems install gems, as the process environment sets it.
#[derive(Default)]
pub struct GemEnv {
    bundle_path: Option<PathBuf>,
    bundle_app_config: Option<PathBuf>,
    home: Option<PathBuf>,
    gem_home: Option<PathBuf>,
    gem_path: Option<OsString>,
}

impl GemEnv {
    pub fn from_process() -> Self {
        let path = |name| std::env::var_os(name).map(PathBuf::from);
        GemEnv {
            bundle_path: path("BUNDLE_PATH"),
            bundle_app_config: path("BUNDLE_APP_CONFIG"),
            home: path("HOME"),
            gem_home: path("GEM_HOME"),
            gem_path: std::env::var_os("GEM_PATH"),
        }
    }
}

/// The view folders of an app: its own (and its engines', packs') and those of the gems that ship views.
#[derive(Debug, Clone)]
pub struct ViewRoots {
    root: PathBuf,
    /// Found on first use: walking the app costs, and most questions are answered without it.
    app: OnceLock<Vec<PathBuf>>,
    gems: Vec<PathBuf>,
    /// The gems' roots that define a Rails engine: the only ones Rails renders from.
    engine_gems: Vec<PathBuf>,
    /// Is every gem that could ship views installed where this looked? When not, `gems` is empty.
    pub gems_known: bool,
}

impl ViewRoots {
    fn app_roots(&self) -> &[PathBuf] {
        self.app.get_or_init(|| under_root(&self.root))
    }

    /// Reads what is read on first use, so a later request on another thread finds it ready.
    pub fn load_app_roots(&self) {
        self.app_roots();
    }

    pub fn has_folder(&self, dir: &str) -> bool {
        self.app_roots().iter().chain(&self.gems).any(|views| views.join(dir).is_dir())
    }

    /// The app's and its engines' roots, by path; a dummy app of a test is none.
    fn rendering_app_roots(&self) -> impl Iterator<Item = &PathBuf> {
        self.app_roots().iter().filter(|views| {
            let relative = views.strip_prefix(&self.root).unwrap_or(views);
            !relative.components().any(|part| FIXTURE_FOLDERS.iter().any(|name| part.as_os_str() == *name))
        })
    }

    /// The roots Rails searches for a name not found in `tried`, in priority order: the app's and its engines' by path,
    /// then the engine gems'.
    pub fn roots_after(&self, tried: &[PathBuf]) -> Vec<&Path> {
        let roots = self.rendering_app_roots().chain(&self.engine_gems);
        roots.filter(|views| !tried.contains(views)).map(PathBuf::as_path).collect()
    }

    /// The app's and its engines' roots other than `own`, the ones whose files completion lists.
    pub fn other_app_roots(&self, own: &Path) -> Vec<&Path> {
        self.rendering_app_roots().filter(|views| *views != own).map(PathBuf::as_path).collect()
    }

    /// Does the app's code add view paths of its own? Scans every Ruby file of the app, which no watcher reports, so the
    /// caller keeps the answer only as long as it keeps the question.
    pub fn app_code_adds_view_paths(&self) -> bool {
        self.app_roots().iter().any(|views| code_adds_view_paths(views))
    }
}

type Discoveries = DashMap<(PathBuf, Vec<(String, String)>), Arc<OnceLock<Arc<ViewRoots>>>>;

/// View roots per Rails root and set of view-capable gems, kept by the language server until a watched file may have
/// changed them. Callers wait for the one discovery in flight, so call from a thread that may block.
#[derive(Default, Debug)]
pub struct Cache(Discoveries);

impl Cache {
    pub fn get(&self, root: &Path, gems: &[(String, String)]) -> Arc<ViewRoots> {
        let cell = Arc::clone(&*self.0.entry((root.to_path_buf(), gems.to_vec())).or_default());
        Arc::clone(cell.get_or_init(|| {
            let roots = find(root, gems, &GemEnv::from_process());
            roots.load_app_roots();
            Arc::new(roots)
        }))
    }

    /// What `get` found, fully loaded, if it has finished; never searches.
    pub fn peek(&self, root: &Path, gems: &[(String, String)]) -> Option<Arc<ViewRoots>> {
        self.0.get(&(root.to_path_buf(), gems.to_vec()))?.get().cloned()
    }

    pub fn clear(&self) {
        self.0.clear();
    }
}

/// The view roots; without the gems' (`gems_known` false) when one isn't installed where this looks (bundler's path,
/// `GEM_HOME`, `GEM_PATH`).
pub fn find(root: &Path, gems: &[(String, String)], env: &GemEnv) -> ViewRoots {
    let installed = installed_gem_views(root, gems, env);
    let gem_views = installed.clone().unwrap_or_default();
    let engine_gems = gem_views.iter().filter(|views| views.is_dir() && is_engine(views)).cloned().collect();
    ViewRoots { root: root.to_path_buf(), app: OnceLock::new(), gems: gem_views, engine_gems, gems_known: installed.is_some() }
}

/// Does the gem whose `app/views` is `views` define a `Rails::Engine`?
fn is_engine(views: &Path) -> bool {
    let Some(lib) = views.parent().and_then(Path::parent).map(|gem| gem.join("lib")) else { return false };
    let files = WalkDir::new(lib)
        .max_depth(ENGINE_SEARCH_DEPTH)
        .into_iter()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "rb"));
    files.into_iter().any(|file| {
        file.file_name() == "engine.rb"
            || std::fs::read_to_string(file.path()).is_ok_and(|text| text.contains("< Rails::Engine") || text.contains("< ::Rails::Engine"))
    })
}

/// Is `file` inside a folder gems install into (a `gems` folder of bundler's or RubyGems' homes, or `bundler/gems`)?
/// Only paths are compared, so it's cheap enough to ask per request.
pub fn is_gem_file(root: Option<&Path>, file: &Path, env: &GemEnv) -> bool {
    if !file.components().any(|part| part.as_os_str() == "gems") {
        return false;
    }
    // Bundler's config is relative to the app, so it is read only for a file that has one.
    let homes = root.map_or_else(|| rubygems_homes(env), |root| gem_homes(root, env));
    let git = homes.iter().filter_map(|home| Some(home.parent()?.join("bundler/gems")));
    // A RubyGems home found by its layout (`…/gems/name-1.0` beside `specifications`), as rbenv and asdf set it up.
    let installed_gem = file.ancestors().filter_map(|dir| dir.parent()).any(|gems| {
        gems.file_name().is_some_and(|name| name == "gems") && gems.parent().is_some_and(|home| home.join("specifications").is_dir())
    });
    installed_gem || homes.iter().cloned().chain(git).any(|dir| file.starts_with(dir))
}

/// The `app/views` of each of `gems`, `None` when one isn't installed.
fn installed_gem_views(root: &Path, gems: &[(String, String)], env: &GemEnv) -> Option<Vec<PathBuf>> {
    if gems.is_empty() {
        return Some(Vec::new());
    }
    let homes = gem_homes(root, env);
    let installed: Vec<PathBuf> = homes.iter().flat_map(|home| subfolders(home)).collect();
    // Git gems install beside the gems folder, as `bundler/gems/name-<sha>`.
    let git_installed: Vec<PathBuf> =
        homes.iter().filter_map(|home| home.parent()).flat_map(|ruby| subfolders(&ruby.join("bundler/gems"))).collect();
    gems.iter()
        .map(|(name, version)| {
            let exact = format!("{name}-{version}");
            let platform = format!("{exact}-");
            let release = installed.iter().find(|dir| folder_name(dir).is_some_and(|n| n == exact || n.starts_with(&platform)));
            let git = || {
                let prefix = format!("{name}-");
                git_installed.iter().find(|dir| folder_name(dir).and_then(|n| n.strip_prefix(&prefix)).is_some_and(is_commit_hash))
            };
            Some(release.or_else(git)?.join("app/views"))
        })
        .collect()
}

fn folder_name(dir: &Path) -> Option<&str> {
    dir.file_name()?.to_str()
}

fn is_commit_hash(text: &str) -> bool {
    text.len() >= MIN_COMMIT_HASH && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Config files are read by `Project`; this is the rest: `prepend_view_path` anywhere in an app's code (controllers,
/// mailers, helpers, models), its `lib` and `config`, and `config.paths` in an engine. A `views` folder inside `app/`
/// other than `app/views` (a theme's) is a view root nothing here lists.
fn code_adds_view_paths(views: &Path) -> bool {
    let Some(app) = views.parent() else { return false };
    let inside_app =
        WalkDir::new(app).into_iter().filter_entry(|entry| !(entry.depth() == 1 && NOT_CODE.iter().any(|name| entry.file_name() == *name)));
    let outside_app = files_under(&app.join("../lib"), usize::MAX).into_iter().chain(files_under(&app.join("../config"), usize::MAX));
    for entry in inside_app.flatten() {
        if entry.depth() > 1 && entry.file_type().is_dir() && entry.file_name() == "views" {
            return true;
        }
        if entry.file_type().is_file() && adds_view_paths_in(entry.path()) {
            return true;
        }
    }
    outside_app.into_iter().any(|file| adds_view_paths_in(&file))
}

fn adds_view_paths_in(file: &Path) -> bool {
    file.extension().is_some_and(|e| e == "rb") && std::fs::read_to_string(file).is_ok_and(|text| adds_view_paths(&text))
}

/// Sorted, so which root wins is the same on every run.
fn under_root(root: &Path) -> Vec<PathBuf> {
    let walk = WalkDir::new(root).max_depth(MAX_DEPTH).into_iter().filter_entry(|entry| entry.depth() == 0 || !not_searched(entry));
    let views =
        walk.flatten().filter(|e| e.file_name() == "views" && e.path().parent().and_then(Path::file_name).is_some_and(|n| n == "app"));
    let mut roots: Vec<PathBuf> = views.map(DirEntry::into_path).collect();
    roots.sort();
    roots
}

/// Folders that hold no views, and the inside of an `app` (below the root) except its `views`.
fn not_searched(entry: &DirEntry) -> bool {
    let in_app = entry.depth() > 1 && entry.path().parent().and_then(Path::file_name).is_some_and(|n| n == "app");
    let named = NOT_SEARCHED.iter().any(|name| entry.file_name() == *name);
    named || NOT_SEARCHED_PATHS.iter().any(|path| entry.path().ends_with(path)) || (in_app && entry.file_name() != "views")
}

/// The `gems` folders gems install into: bundler's path (`vendor/bundle`, `BUNDLE_PATH`, `.bundle/config`, `BUNDLE_APP_CONFIG`, `~/.bundle/config`) and RubyGems'.
/// Bundler resolves relative paths against the app root.
fn gem_homes(root: &Path, env: &GemEnv) -> Vec<PathBuf> {
    let config_dirs = [
        Some(root.join(".bundle")),
        env.bundle_app_config.as_ref().map(|dir| root.join(dir)),
        env.home.as_ref().map(|home| home.join(".bundle")),
    ];
    let configured = config_dirs.into_iter().flatten().filter_map(|dir| {
        let config = std::fs::read_to_string(dir.join("config")).ok()?;
        let value = config.lines().find_map(|line| line.trim().strip_prefix("BUNDLE_PATH:"))?;
        Some(root.join(value.trim().trim_matches(['"', '\''])))
    });
    let from_env = env.bundle_path.as_ref().map(|path| root.join(path));
    let bundles = [root.join("vendor/bundle")].into_iter().chain(from_env).chain(configured);
    let by_ruby_version = bundles.flat_map(|bundle| subfolders(&bundle.join("ruby")).into_iter().map(|ruby| ruby.join("gems")));
    by_ruby_version.chain(rubygems_homes(env)).collect()
}

fn rubygems_homes(env: &GemEnv) -> Vec<PathBuf> {
    let gem_path = env.gem_path.as_ref().map(|paths| std::env::split_paths(paths).collect::<Vec<_>>()).unwrap_or_default();
    env.gem_home.clone().into_iter().chain(gem_path).map(|home| home.join("gems")).collect()
}

fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flatten().map(|entry| entry.path()).filter(|path| path.is_dir()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::write_files;

    #[test]
    fn view_capable_gems_follow_dependencies_transitively() {
        let lock = concat!(
            "PATH\n  remote: .\n  specs:\n    my_engine (0.1.0)\n      railties\n\n",
            "GEM\n  remote: https://rubygems.org/\n  specs:\n    actionview (7.1.0)\n    devise (4.9.4)\n      bcrypt (~> 3.0)\n      railties (>= 4.1)\n",
            "    devise_extra (1.0)\n      devise\n    pg (1.5.0)\n    railties (7.1.0)\n    rails (7.1.0)\n      railties (= 7.1.0)\n",
            "    zz (2.0-x86_64-linux)\n      pg\n\nDEPENDENCIES\n  devise\n  railties\n",
        );
        assert_eq!(view_capable_gems(lock), [("devise".to_string(), "4.9.4".to_string()), ("devise_extra".to_string(), "1.0".to_string())]);
    }

    /// `find` with no environment: tests don't read the developer's gem homes.
    fn find_in(root: &Path, gems: &[(String, String)]) -> ViewRoots {
        find(root, gems, &GemEnv::default())
    }

    fn adds(root: &Path) -> bool {
        find_in(root, &[]).app_code_adds_view_paths()
    }

    #[test]
    fn roots_are_searched_to_a_depth_and_skip_app_insides_and_dependencies() {
        let root = std::env::temp_dir().join(format!("rt-view-roots-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let files = [
            "app/views/a.erb",
            "packs/p/app/views/b.erb",
            "ee/app/views/c.erb",
            "node_modules/x/app/views/d.erb",
            "a/b/c/d/e/f/app/views/g.erb",
            "vendor/bundle/ruby/3.3.0/gems/zz-1.0-java/app/views/h.erb",
        ];
        write_files(&root, &files.map(|file| (file, "")));
        write_files(&root, &[(".bundle/config", "---\nBUNDLE_PATH: \"vendor/bundle\"\n")]);
        let relative = |roots: ViewRoots| {
            let mut names: Vec<String> = roots
                .app_roots()
                .iter()
                .chain(&roots.gems)
                .map(|r| r.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"))
                .collect();
            names.sort();
            names
        };
        assert_eq!(relative(find_in(&root, &[])), ["app/views", "ee/app/views", "packs/p/app/views"]);
        let zz = [("zz".to_string(), "1.0".to_string())];
        assert!(relative(find_in(&root, &zz)).contains(&"vendor/bundle/ruby/3.3.0/gems/zz-1.0-java/app/views".to_string()));
        assert!(!find_in(&root, &[("zz".to_string(), "1".to_string())]).gems_known, "another version");
        write_files(&root, &[("app/controllers/application_controller.rb", "prepend_view_path \"app/views/admin\"\n")]);
        assert!(adds(&root), "a controller adds a view path");
        std::fs::remove_file(root.join("app/controllers/application_controller.rb")).unwrap();
        assert!(!adds(&root));
        for (file, text) in [
            ("app/mailers/application_mailer.rb", "prepend_view_path \"app/views/mailers\"\n"),
            ("packs/p/config/initializers/views.rb", "ActionController::Base.prepend_view_path 'x'\n"),
            ("app/themes/dark/views/a.erb", ""),
        ] {
            write_files(&root, &[(file, text)]);
            assert!(adds(&root), "{file}");
            std::fs::remove_file(root.join(file)).unwrap();
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn git_gems_are_found_beside_the_gems_folder() {
        let root = std::env::temp_dir().join(format!("rt-view-roots-git-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        write_files(&root, &[("app/views/a.erb", ""), ("vendor/bundle/ruby/3.3.0/bundler/gems/devise-0123abcdef01/app/views/x.erb", "")]);
        std::fs::create_dir_all(root.join("vendor/bundle/ruby/3.3.0/gems")).unwrap();
        let found = find_in(&root, &[("devise".to_string(), "4.9".to_string())]);
        assert!(found.gems.contains(&root.join("vendor/bundle/ruby/3.3.0/bundler/gems/devise-0123abcdef01/app/views")));
        assert!(!find_in(&root, &[("dev".to_string(), "1".to_string())]).gems_known);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn gems_are_found_in_an_absolute_bundle_path_and_a_scan_survives_symlink_loops() {
        let base = std::env::temp_dir().join(format!("rt-view-roots-abs-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let (root, bundle) = (base.join("app"), base.join("shared"));
        write_files(&root, &[("app/views/a.erb", "")]);
        write_files(&bundle, &[("ruby/3.3.0/gems/spree_core-4.6.0/app/views/spree/_x.html.erb", "")]);
        write_files(&root, &[(".bundle/config", &format!("BUNDLE_PATH: {}\n", bundle.display()))]);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, root.join("app/loop")).unwrap();
        let spree = [("spree_core".to_string(), "4.6.0".to_string())];
        let found = find_in(&root, &spree);
        assert!(found.gems.contains(&bundle.join("ruby/3.3.0/gems/spree_core-4.6.0/app/views")));
        assert!(!find_in(&root, &[("spree_core".to_string(), "4.6.1".to_string())]).gems_known);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_root_folder_named_like_a_skipped_one_is_still_searched() {
        let base = std::env::temp_dir().join(format!("rt-view-roots-named-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let root = base.join("public");
        write_files(&root, &[("app/views/a.erb", ""), ("storage/app/views/b.erb", ""), ("vendor/bundle/app/views/c.erb", "")]);
        let found = find_in(&root, &[]);
        assert_eq!(found.app_roots(), [root.join("app/views")]);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn relative_bundle_paths_resolve_against_the_app_root() {
        let root = std::env::temp_dir().join(format!("rt-view-roots-rel-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        write_files(&root, &[("shared/ruby/3.3.0/gems/x-1.0/app/views/a.erb", ""), ("app/views/a.erb", "")]);
        let env = GemEnv { bundle_path: Some("shared".into()), ..GemEnv::default() };
        let found = find(&root, &[("x".to_string(), "1.0".to_string())], &env);
        assert_eq!(found.gems, [root.join("shared/ruby/3.3.0/gems/x-1.0/app/views")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn gem_files_are_told_by_path_under_the_gem_homes() {
        let base = std::env::temp_dir().join(format!("rt-view-roots-gemfile-{}", std::process::id()));
        let (root, home) = (base.join("app"), base.join("rubygems"));
        let env = GemEnv { gem_home: Some(home.clone()), ..GemEnv::default() };
        std::fs::create_dir_all(root.join("vendor/bundle/ruby/3.3.0/gems")).unwrap();
        let in_gem = home.join("gems/devise-4.9.4/app/views/devise/_x.html.erb");
        assert!(is_gem_file(Some(&root), &in_gem, &env));
        assert!(is_gem_file(None, &in_gem, &env));
        assert!(is_gem_file(Some(&root), &root.join("vendor/bundle/ruby/3.3.0/gems/x-1/app/views/a.erb"), &env));
        assert!(is_gem_file(Some(&root), &root.join("vendor/bundle/ruby/3.3.0/bundler/gems/x-0123abc/app/views/a.erb"), &env));
        assert!(!is_gem_file(Some(&root), &root.join("app/views/gems/_x.html.erb"), &env), "a views folder named gems");
        assert!(!is_gem_file(Some(&root), &root.join("engines/blog/app/views/a.erb"), &env));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn rbenv_gems_are_told_by_the_specifications_folder_beside_them() {
        let base = std::env::temp_dir().join(format!("rt-view-roots-rbenv-{}", std::process::id()));
        let home = base.join("versions/3.3.0/lib/ruby/gems/3.3.0");
        let env = GemEnv::default();
        std::fs::create_dir_all(home.join("specifications")).unwrap();
        let in_gem = home.join("gems/devise-4.9.4/app/views/devise/_x.html.erb");
        assert!(is_gem_file(None, &in_gem, &env));
        assert!(!is_gem_file(None, &base.join("app/views/gems/x-1/_y.html.erb"), &env));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn only_gems_defining_an_engine_are_rendered_from_and_the_cache_refreshes_when_cleared() {
        let root = std::env::temp_dir().join(format!("rt-view-roots-engines-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let gems = "vendor/bundle/ruby/3.3.0/gems";
        write_files(
            &root,
            &[
                ("app/views/a.erb", ""),
                ("test/dummy/app/views/b.erb", ""),
                (&format!("{gems}/eng-1.0/app/views/x.erb"), ""),
                (&format!("{gems}/eng-1.0/lib/eng/engine.rb"), ""),
                (&format!("{gems}/plain-1.0/app/views/y.erb"), ""),
                (&format!("{gems}/plain-1.0/lib/plain.rb"), "module Plain; end"),
                (&format!("{gems}/inline-1.0/app/views/z.erb"), ""),
                (&format!("{gems}/inline-1.0/lib/inline.rb"), "class E < Rails::Engine; end"),
            ],
        );
        let gem = |name: &str| (name.to_string(), "1.0".to_string());
        let gems_list = [gem("eng"), gem("plain"), gem("inline")];
        let found = find_in(&root, &gems_list);
        let names = |roots: Vec<&Path>| {
            roots.iter().map(|r| r.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")).collect::<Vec<_>>()
        };
        assert_eq!(
            names(found.roots_after(&[])),
            ["app/views", &format!("{gems}/eng-1.0/app/views"), &format!("{gems}/inline-1.0/app/views")],
            "no dummy app, no gem without an engine"
        );
        assert!(!found.has_folder("ghost") && found.gems_known);

        let cache = Cache::default();
        let key: Vec<_> = Vec::new();
        assert!(cache.peek(&root, &key).is_none());
        let first = cache.get(&root, &key);
        assert!(cache.peek(&root, &key).is_some_and(|roots| roots.app.get().is_some()), "ready means the app was walked");
        write_files(&root, &[("engines/new/app/views/n.erb", "")]);
        assert_eq!(names(cache.get(&root, &key).roots_after(&[])), names(first.roots_after(&[])), "kept until cleared");
        cache.clear();
        assert!(cache.peek(&root, &key).is_none());
        assert!(names(cache.get(&root, &key).roots_after(&[])).contains(&"engines/new/app/views".to_string()));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
