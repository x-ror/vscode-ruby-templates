//! Every `app/views` folder a Rails app can render from: its own and its engines', packs' and components' under the
//! Rails root, and those of the installed gems that could ship views.

use crate::project::adds_view_paths;
use crate::rails_paths::files_under;
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use walkdir::{DirEntry, WalkDir};

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
    app: Vec<PathBuf>,
    gems: Vec<PathBuf>,
    code_adds_view_paths: OnceLock<bool>,
}

impl ViewRoots {
    pub fn has_folder(&self, dir: &str) -> bool {
        self.app.iter().chain(&self.gems).any(|views| views.join(dir).is_dir())
    }

    /// Does the app's code add view paths of its own? Read on first use: it scans every Ruby file of the app.
    pub fn app_code_adds_view_paths(&self) -> bool {
        *self.code_adds_view_paths.get_or_init(|| self.app.iter().any(|views| code_adds_view_paths(views)))
    }
}

/// The view roots, or `None` when one of `gems` isn't installed where this looks (bundler's path, `GEM_HOME`, `GEM_PATH`).
pub fn find(root: &Path, gems: &[(String, String)], env: &GemEnv) -> Option<ViewRoots> {
    let gem_views = installed_gem_views(root, gems, env)?;
    Some(ViewRoots { app: under_root(root), gems: gem_views, code_adds_view_paths: OnceLock::new() })
}

/// The `app/views` of each of `gems`, `None` when one isn't installed (checked before any scan of the app: it usually fails).
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

fn under_root(root: &Path) -> Vec<PathBuf> {
    let walk = WalkDir::new(root).max_depth(MAX_DEPTH).into_iter().filter_entry(|entry| entry.depth() == 0 || !not_searched(entry));
    let views =
        walk.flatten().filter(|e| e.file_name() == "views" && e.path().parent().and_then(Path::file_name).is_some_and(|n| n == "app"));
    views.map(DirEntry::into_path).collect()
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
    let gem_path = env.gem_path.as_ref().map(|paths| std::env::split_paths(paths).collect::<Vec<_>>()).unwrap_or_default();
    let rubygems = env.gem_home.clone().into_iter().chain(gem_path).map(|home| home.join("gems"));
    by_ruby_version.chain(rubygems).collect()
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
    fn find_in(root: &Path, gems: &[(String, String)]) -> Option<ViewRoots> {
        find(root, gems, &GemEnv::default())
    }

    fn adds(root: &Path) -> bool {
        find_in(root, &[]).unwrap().app_code_adds_view_paths()
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
            let mut names: Vec<String> =
                roots.app.iter().chain(&roots.gems).map(|r| r.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")).collect();
            names.sort();
            names
        };
        assert_eq!(relative(find_in(&root, &[]).unwrap()), ["app/views", "ee/app/views", "packs/p/app/views"]);
        let zz = [("zz".to_string(), "1.0".to_string())];
        assert!(relative(find_in(&root, &zz).unwrap()).contains(&"vendor/bundle/ruby/3.3.0/gems/zz-1.0-java/app/views".to_string()));
        assert!(find_in(&root, &[("zz".to_string(), "1".to_string())]).is_none(), "another version");
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
        let found = find_in(&root, &[("devise".to_string(), "4.9".to_string())]).unwrap();
        assert!(found.gems.contains(&root.join("vendor/bundle/ruby/3.3.0/bundler/gems/devise-0123abcdef01/app/views")));
        assert!(find_in(&root, &[("dev".to_string(), "1".to_string())]).is_none());
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
        let found = find_in(&root, &spree).unwrap();
        assert!(found.gems.contains(&bundle.join("ruby/3.3.0/gems/spree_core-4.6.0/app/views")));
        assert!(find_in(&root, &[("spree_core".to_string(), "4.6.1".to_string())]).is_none());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_root_folder_named_like_a_skipped_one_is_still_searched() {
        let base = std::env::temp_dir().join(format!("rt-view-roots-named-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let root = base.join("public");
        write_files(&root, &[("app/views/a.erb", ""), ("storage/app/views/b.erb", ""), ("vendor/bundle/app/views/c.erb", "")]);
        let found = find_in(&root, &[]).unwrap();
        assert_eq!(found.app, [root.join("app/views")]);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn relative_bundle_paths_resolve_against_the_app_root() {
        let root = std::env::temp_dir().join(format!("rt-view-roots-rel-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        write_files(&root, &[("shared/ruby/3.3.0/gems/x-1.0/app/views/a.erb", ""), ("app/views/a.erb", "")]);
        let env = GemEnv { bundle_path: Some("shared".into()), ..GemEnv::default() };
        let found = find(&root, &[("x".to_string(), "1.0".to_string())], &env).unwrap();
        assert_eq!(found.gems, [root.join("shared/ruby/3.3.0/gems/x-1.0/app/views")]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
