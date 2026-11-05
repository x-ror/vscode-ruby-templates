//! Framework, gem and Ruby versions of the project a template belongs to, from the nearest `Gemfile.lock` and `.ruby-version`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::controller_classes::ControllerScan;
use crate::rails_paths::{app_root, files_under, is_known_format};
use crate::strict_locals::{self, Close, RubyContext};
use crate::view_roots;
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framework {
    Rails,
    Sinatra,
    Unknown,
}

/// Projects already loaded during one run, by the files they come from.
pub type Projects = HashMap<ProjectKey, Project>;

/// What `Project::for_template` reads for a template: the nearest `Gemfile.lock`'s folder (with the
/// initializers of the template's app) and the nearest `.ruby-version`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct ProjectKey {
    lock_dir: Option<PathBuf>,
    template_app: Option<PathBuf>,
    ruby_version: Option<PathBuf>,
}

impl ProjectKey {
    pub(crate) fn of(template: &Path) -> Self {
        ProjectKey {
            lock_dir: template.ancestors().skip(1).find(|dir| dir.join("Gemfile.lock").is_file()).map(Path::to_path_buf),
            template_app: app_root(template).and_then(|app| Some(app.parent()?.to_path_buf())),
            ruby_version: template.ancestors().skip(1).map(|dir| dir.join(".ruby-version")).find(|file| file.is_file()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Project {
    pub framework: Framework,
    gems: HashMap<String, String>,
    ruby: Option<Version>,
    /// Format names the app registers in `config/initializers/**/*.rb`, beyond Rails' own.
    formats: Vec<String>,
    /// `config.action_view.prefix_partial_path_with_controller_namespace`, true unless the app's config turns it off.
    pub prefixes_partial_paths: bool,
    /// The folder of the `Gemfile.lock`, which Rails.root is.
    root: Option<PathBuf>,
    /// Ruby files in namespace folders of `app/models` (relative paths), listed on first use: a Project is one app's.
    namespaced_models: OnceLock<Vec<PathBuf>>,
    /// `controller_classes::view_prefixes` by view folder: read from controller files once per Project.
    view_prefixes: Arc<Mutex<HashMap<PathBuf, Option<Vec<String>>>>>,
    /// Each app's controllers (by app folder) read once, on first use.
    controller_scan: Arc<Mutex<HashMap<PathBuf, Arc<ControllerScan>>>>,
    /// Does the app add view folders for every controller (`prepend_view_path` in its config, an engine or path gem with views)?
    pub adds_view_paths: bool,
    /// Gems from the lockfile that could ship views (`view_roots::view_capable_gems`).
    view_capable_gems: Vec<(String, String)>,
    /// Every view root, searched on first use.
    view_roots: OnceLock<Arc<view_roots::ViewRoots>>,
    /// Where the language server keeps view roots between requests; without it they are searched per `Project`.
    view_cache: Option<Arc<view_roots::Cache>>,
    /// Does the app's code add view paths? Read on first use, and not kept beyond this `Project`: no watcher sees it change.
    code_adds_view_paths: OnceLock<bool>,
}

impl Project {
    pub fn unknown() -> Self {
        Project {
            framework: Framework::Unknown,
            gems: HashMap::new(),
            ruby: None,
            formats: Vec::new(),
            prefixes_partial_paths: true,
            root: None,
            namespaced_models: OnceLock::new(),
            view_prefixes: Arc::default(),
            controller_scan: Arc::default(),
            adds_view_paths: false,
            view_capable_gems: Vec::new(),
            view_roots: OnceLock::new(),
            view_cache: None,
            code_adds_view_paths: OnceLock::new(),
        }
    }

    /// The view folders for views in `dir`, from `compute` the first time.
    pub fn view_prefixes_of(&self, dir: &Path, compute: impl FnOnce() -> Option<Vec<String>>) -> Option<Vec<String>> {
        let mut cache = self.view_prefixes.lock().unwrap_or_else(PoisonError::into_inner);
        cache.entry(dir.to_path_buf()).or_insert_with(compute).clone()
    }

    /// The scan of the app at `app`, from `scan` the first time.
    pub fn controller_scan(&self, app: &Path, scan: impl FnOnce() -> ControllerScan) -> Arc<ControllerScan> {
        let mut cache = self.controller_scan.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(cache.entry(app.to_path_buf()).or_insert_with(|| Arc::new(scan())))
    }

    /// The `.rb` files below the first level of `models` (not `concerns/`), relative to it.
    pub fn namespaced_models(&self, models: &Path) -> &[PathBuf] {
        self.namespaced_models.get_or_init(|| {
            let files = WalkDir::new(models).min_depth(2).into_iter().filter_map(Result::ok);
            let relative = files.filter_map(|entry| entry.path().strip_prefix(models).ok().map(Path::to_path_buf));
            relative.filter(|path| path.extension().is_some_and(|ext| ext == "rb") && !path.starts_with("concerns")).collect()
        })
    }

    /// The project of the nearest `Gemfile.lock` above `template`, unknown when there is none, with the formats
    /// registered by the initializers of the app that holds `template` and of the lockfile's app (Mime::SET is
    /// process-wide, so an engine's views see the host app's registrations).
    pub fn for_template(template: &Path) -> Self {
        Self::load(&ProjectKey::of(template))
    }

    /// The project of `template`, loaded once per `ProjectKey` for a run over many templates.
    pub fn memoized<'a>(projects: &'a mut Projects, template: &Path) -> &'a Self {
        projects.entry(ProjectKey::of(template)).or_insert_with_key(Self::load)
    }

    fn load(key: &ProjectKey) -> Self {
        let lock_dir = key.lock_dir.as_deref();
        let lock_text = lock_dir.and_then(|dir| std::fs::read_to_string(dir.join("Gemfile.lock")).ok());
        let project = lock_text.as_deref().map_or_else(Self::unknown, Self::from_lockfile);
        let configs: Vec<AppConfig> =
            key.template_app.iter().map(PathBuf::as_path).chain(lock_dir).map(|app| read_app_config(&app.join("config"))).collect();
        let prefixes_partial_paths = configs.iter().all(|config| config.prefixes_partial_paths);
        let adds_view_paths = configs.iter().any(|config| config.adds_view_paths)
            || lock_dir.zip(lock_text.as_deref()).is_some_and(|(dir, lock)| path_gem_has_views(lock, dir));
        let mut formats: Vec<String> = configs.into_iter().flat_map(|config| config.formats).collect();
        formats.sort();
        formats.dedup();
        let ruby =
            key.ruby_version.as_ref().and_then(|file| std::fs::read_to_string(file).ok()).and_then(|text| ruby_version_file_version(&text));
        Project { formats, prefixes_partial_paths, adds_view_paths, ruby: ruby.or(project.ruby), root: key.lock_dir.clone(), ..project }
    }

    /// Strict locals exist from Rails 7.1; before, the `locals:` line is a plain comment.
    pub fn supports_strict_locals(&self) -> bool {
        self.framework == Framework::Rails && self.gem_version("actionview").is_some_and(|v| v.at_least(strict_locals::SINCE))
    }

    /// The latest `STRICT_LOCALS_REGEX` unless the lockfile names an older actionview.
    pub fn strict_locals_close(&self) -> Close {
        match self.gem_version("actionview") {
            Some(version) if !version.at_least(strict_locals::LAZY_REGEXP_SINCE) => Close::LastOnLine,
            _ => Close::BeforeTagOrLineEnd,
        }
    }

    /// The strict locals `text` declares, read as this project's Rails and Ruby do.
    pub fn strict_locals(&self, text: &str) -> Option<strict_locals::Signature> {
        strict_locals::signature(text, self.strict_locals_close(), self.prism_version())
    }

    pub fn file_strict_locals(&self, file: &Path) -> Option<strict_locals::Signature> {
        strict_locals::file_signature(file, self.strict_locals_close(), self.prism_version())
    }

    /// How Rails evals `template` of this project.
    pub(crate) fn ruby_context(&self, template: &Path) -> RubyContext {
        RubyContext {
            grammar: self.prism_version(),
            strict_locals: self
                .supports_strict_locals()
                .then(|| self.short_identifier(template))
                .flatten()
                .map(|identifier| strict_locals::Rails { close: self.strict_locals_close(), identifier }),
        }
    }

    /// Is `segment` a template format: one of Rails' own or registered by the app?
    pub fn is_format(&self, segment: &str) -> bool {
        is_known_format(segment) || self.formats.iter().any(|f| f == segment)
    }

    pub fn from_lockfile(lock: &str) -> Self {
        let mut gems = HashMap::new();
        let mut in_specs = false;
        let mut in_ruby_section = false;
        let mut ruby = None;
        for line in lock.lines() {
            if !line.starts_with(' ') {
                in_specs = false;
                in_ruby_section = line.trim_end() == "RUBY VERSION";
            } else if let Some(version) = in_ruby_section.then(|| line.trim().strip_prefix("ruby ")).flatten() {
                ruby = ruby_release(version);
            } else if line.trim_end() == "  specs:" {
                in_specs = true;
            } else if let Some((name, version)) = in_specs.then(|| spec_line(line)).flatten() {
                gems.entry(name.to_string()).or_insert_with(|| version.to_string());
            }
        }
        let framework = if gems.contains_key("actionview") {
            Framework::Rails
        } else if gems.contains_key("sinatra") {
            Framework::Sinatra
        } else {
            Framework::Unknown
        };
        Project {
            framework,
            gems,
            ruby,
            formats: Vec::new(),
            prefixes_partial_paths: true,
            root: None,
            namespaced_models: OnceLock::new(),
            view_prefixes: Arc::default(),
            controller_scan: Arc::default(),
            adds_view_paths: false,
            view_capable_gems: view_roots::view_capable_gems(lock),
            view_roots: OnceLock::new(),
            view_cache: None,
            code_adds_view_paths: OnceLock::new(),
        }
    }

    pub fn gem_version(&self, name: &str) -> Option<Version> {
        Version::parse(self.gems.get(name)?)
    }

    /// Rails' `short_identifier` of `template`: its path from the project root, with `/`.
    pub fn short_identifier(&self, template: &Path) -> Option<String> {
        let relative = template.strip_prefix(self.root.as_deref()?).ok()?;
        Some(relative.iter().map(|part| part.to_string_lossy()).collect::<Vec<_>>().join("/"))
    }

    /// The Prism grammar of the project's Ruby, the latest when it names none.
    pub fn prism_version(&self) -> ruby_prism::Version {
        let Some(ruby) = &self.ruby else { return ruby_prism::Version::Latest };
        // Prism 2.1.0 has no grammar before 3.3, which stands in for older Rubies.
        PRISM_GRAMMARS.iter().rfind(|(since, _)| ruby.at_least(since)).map_or(ruby_prism::Version::CRuby3_3, |&(_, grammar)| grammar)
    }

    /// Is `dir` (`users/card`) certainly no view folder: every view root is known, and none has it?
    pub fn lacks_view_folder(&self, dir: &str) -> bool {
        let top = dir.split('/').next().unwrap_or(dir);
        let possible = !self.adds_view_paths && !view_roots::FRAMEWORK_VIEW_FOLDERS.contains(&top) && !self.has_gem_family(top);
        if !possible {
            return false;
        }
        // Reading the app's code is costly; a folder found in some root settles it first.
        let adds_paths =
            || *self.code_adds_view_paths.get_or_init(|| self.view_roots().is_some_and(|roots| roots.app_code_adds_view_paths()));
        self.view_roots().is_some_and(|roots| roots.gems_known && !roots.has_folder(dir) && !adds_paths())
    }

    /// The view folders of the app and of the gems that ship views; `None` outside a Rails root. Searches on first use.
    pub fn view_roots(&self) -> Option<&view_roots::ViewRoots> {
        Some(self.loaded_view_roots()?.as_ref())
    }

    fn loaded_view_roots(&self) -> Option<&Arc<view_roots::ViewRoots>> {
        let root = self.root.as_deref()?;
        Some(self.view_roots.get_or_init(|| match &self.view_cache {
            Some(cache) => cache.get(root, &self.view_capable_gems),
            None => Arc::new(view_roots::find(root, &self.view_capable_gems, &view_roots::GemEnv::from_process())),
        }))
    }

    /// Searches for the view roots, the app's included, and returns them.
    pub fn load_view_roots(&self) -> Option<Arc<view_roots::ViewRoots>> {
        let roots = self.loaded_view_roots()?;
        roots.load_app_roots();
        Some(Arc::clone(roots))
    }

    /// The view roots in the cache, loaded, without searching.
    pub fn cached_view_roots(&self) -> Option<Arc<view_roots::ViewRoots>> {
        self.view_cache.as_ref()?.peek(self.root.as_deref()?, &self.view_capable_gems)
    }

    /// Use `roots` as this project's view roots, so later calls don't reach for the cache again.
    pub fn use_view_roots(&self, roots: Arc<view_roots::ViewRoots>) {
        let _ = self.view_roots.set(roots);
    }

    /// Keep and reuse the view roots in `cache`.
    pub fn with_view_cache(self, cache: Arc<view_roots::Cache>) -> Self {
        Project { view_cache: Some(cache), ..self }
    }

    /// The Rails root's `app/views` when the views folder `own` is part of that app (the app's own, an engine's or a
    /// pack's), which Rails searches before the others. `None` for another Rails app inside the root (`test/dummy`,
    /// a second app of a monorepo).
    pub fn app_views_of(&self, own: &Path) -> Option<PathBuf> {
        let root = self.root.as_deref()?;
        let app_dir = own.parent()?.parent()?;
        let own_app = || app_dir.ancestors().take_while(|dir| *dir != root).any(|dir| dir.join("config/application.rb").is_file());
        (app_dir.starts_with(root) && !own_app()).then(|| root.join("app/views"))
    }

    /// Is `file` in a gem's install folder? Those are read-only.
    pub fn is_gem_file(&self, file: &Path) -> bool {
        view_roots::is_gem_file(self.root.as_deref(), file, &view_roots::GemEnv::from_process())
    }

    /// Does a gem named `name` or `name_*` / `name-*` exist (a gem's own view folder, `spree/` from `spree_core`)?
    pub fn has_gem_family(&self, name: &str) -> bool {
        self.gems.keys().any(|gem| gem == name || gem.strip_prefix(name).is_some_and(|rest| rest.starts_with(['_', '-'])))
    }
}

/// Grammars Prism 2.1.0 vendors after 3.3, by the first Ruby version that uses each.
const PRISM_GRAMMARS: [(&str, ruby_prism::Version); 4] = [
    ("3.4", ruby_prism::Version::CRuby3_4),
    ("3.5", ruby_prism::Version::CRuby4_0),
    ("4.1", ruby_prism::Version::CRuby4_1),
    ("4.2", ruby_prism::Version::Latest),
];

/// The version a `.ruby-version` file names: `3.3.6` or `ruby-3.3.6`, on its first line that isn't blank or a comment.
fn ruby_version_file_version(text: &str) -> Option<Version> {
    let line = text.trim_start_matches(crate::util::BOM).lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with('#'))?;
    let name = line.split_whitespace().next()?;
    ruby_release(name.strip_prefix("ruby-").unwrap_or(name))
}

/// `3.3.6` from `3.3.6p108` or `3.4.0-preview1`; other Ruby engines (`jruby-9.4`) and a bare major have no CRuby release.
fn ruby_release(text: &str) -> Option<Version> {
    let end = text.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(text.len());
    let release = text[..end].trim_end_matches('.');
    release.contains('.').then(|| Version::parse(release)).flatten()
}

const PARTIAL_PREFIX_SETTING: &str = "prefix_partial_path_with_controller_namespace";

/// What an app's config files set that templates depend on.
struct AppConfig {
    /// Names registered by `Mime::Type.register` / `register_alias` in the initializers, as Rails loads them.
    formats: Vec<String>,
    /// False when `config/application.rb`, an environment or an initializer turns the partial path prefix off.
    prefixes_partial_paths: bool,
    /// True when a config file adds view paths: `prepend_view_path`, `config.paths["app/views"]`.
    adds_view_paths: bool,
}

/// The `*.rb` files of `config` that Rails loads, read once for all settings.
fn read_app_config(config: &Path) -> AppConfig {
    let initializers = files_under(&config.join("initializers"), usize::MAX).into_iter().map(|file| (file, true));
    let others = [config.join("application.rb")].into_iter().chain(files_under(&config.join("environments"), 1)).map(|file| (file, false));
    let mut app = AppConfig { formats: Vec::new(), prefixes_partial_paths: true, adds_view_paths: false };
    for (file, is_initializer) in initializers.chain(others).filter(|(file, _)| file.extension().is_some_and(|e| e == "rb")) {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        if is_initializer {
            app.formats.extend(text.lines().filter_map(registered_format).map(str::to_string));
        }
        app.prefixes_partial_paths &= !turns_off_partial_prefix(&text);
        app.adds_view_paths |= adds_view_paths(&text);
    }
    app
}

/// Does the Ruby `text` set `prefix_partial_path_with_controller_namespace = false`?
fn turns_off_partial_prefix(text: &str) -> bool {
    let code = text.lines().filter(|line| !line.trim_start().starts_with('#'));
    code.filter_map(|line| line.split_once(PARTIAL_PREFIX_SETTING)).any(|(_, rest)| {
        let value = rest.trim_start().strip_prefix('=').map(|value| value.split('#').next().unwrap_or(value).trim());
        value == Some("false")
    })
}

/// Does the Ruby `text` call `*_view_path(s)` or touch `paths["app/views"]`?
pub(crate) fn adds_view_paths(text: &str) -> bool {
    let mut code = text.lines().filter(|line| !line.trim_start().starts_with('#'));
    code.any(|line| line.contains("view_path") || line.contains(r#"paths["app/views"]"#) || line.contains("paths['app/views']"))
}

/// Does a gem from the lockfile's `PATH` sections (an engine) ship `app/views`? The app itself (`remote: .`) doesn't count.
fn path_gem_has_views(lock: &str, lock_dir: &Path) -> bool {
    let mut in_path = false;
    lock.lines().any(|line| {
        if !line.starts_with(' ') {
            in_path = line.trim_end() == "PATH";
            return false;
        }
        let remote = line.trim().strip_prefix("remote:").map(str::trim).filter(|remote| !remote.trim_matches(['.', '/']).is_empty());
        in_path && remote.is_some_and(|remote| lock_dir.join(remote).join("app/views").is_dir())
    })
}

// Rails API `Mime::Type.register(string, symbol, …)` and `register_alias(string, symbol, …)` (actionpack mime_type.rb).
fn registered_format(line: &str) -> Option<&str> {
    let call = line.trim_start().strip_prefix("::").unwrap_or(line.trim_start()).strip_prefix("Mime::Type.register")?;
    let args = call.strip_prefix("_alias").unwrap_or(call).trim_start();
    let args = args.strip_prefix('(').unwrap_or(args).trim_start();
    let quote = args.chars().next().filter(|q| matches!(q, '"' | '\''))?;
    let after_type = args[1..].split_once(quote)?.1;
    let symbol = after_type.trim_start().strip_prefix(',')?.trim_start().strip_prefix(':')?;
    let end = symbol.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(symbol.len());
    (end > 0).then(|| &symbol[..end])
}

/// `    name (1.2.3)` or `    name (1.2.3-x86_64-linux)`; the 6-space dependency lines don't match.
pub(crate) fn spec_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("    ").filter(|r| !r.starts_with(' '))?;
    let (name, version) = rest.split_once(" (")?;
    let version = version.strip_suffix(')')?;
    Some((name, version.split_once('-').map_or(version, |(v, _platform)| v)))
}

/// A RubyGems version: dotted numbers, where a segment with letters (`rc1` → `rc`, `1`) marks a prerelease.
#[derive(Debug, Clone, Eq)]
pub struct Version(Vec<Segment>);

// Prerelease words sort below numbers, so `8.0.0.rc1 < 8.0.0` (the missing segment counts as 0).
// Zeros before the first word are dropped first, as in Gem::Version#canonical_segments: `1.0.0.a == 1.0.a`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Segment {
    Word(String),
    Number(u64),
}

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let mut segments = Vec::new();
        for part in text.split('.') {
            let mut rest = part;
            while let Some(first) = rest.chars().next() {
                let numeric = first.is_ascii_digit();
                let end = rest.find(|c: char| c.is_ascii_digit() != numeric).unwrap_or(rest.len());
                let (piece, tail) = rest.split_at(end);
                segments.push(if numeric {
                    Segment::Number(piece.parse().ok()?)
                } else if piece.chars().all(|c| c.is_ascii_alphabetic()) {
                    Segment::Word(piece.to_string())
                } else {
                    return None;
                });
                rest = tail;
            }
        }
        let first_word = segments.iter().position(|s| matches!(s, Segment::Word(_))).unwrap_or(segments.len());
        let kept = segments[..first_word].iter().rposition(|s| *s != Segment::Number(0)).map_or(1, |last| last + 1).min(first_word);
        segments.drain(kept..first_word);
        (!segments.is_empty()).then_some(Version(segments))
    }

    /// `self >= other`, `other` being a version literal like `"7.1"`.
    pub fn at_least(&self, other: &str) -> bool {
        Version::parse(other).is_some_and(|other| *self >= other)
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let zero = Segment::Number(0);
        let len = self.0.len().max(other.0.len());
        (0..len)
            .map(|i| self.0.get(i).unwrap_or(&zero).cmp(other.0.get(i).unwrap_or(&zero)))
            .find(|ord| ord.is_ne())
            .unwrap_or(Ordering::Equal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framework(lock: &str) -> Framework {
        Project::from_lockfile(lock).framework
    }

    #[test]
    fn framework_from_gems() {
        let rails = "GEM\n  remote: https://rubygems.org/\n  specs:\n    actionview (7.1.3)\n      builder (~> 3.1)\n    rails (7.1.3)\n";
        assert_eq!(framework(rails), Framework::Rails);
        assert_eq!(framework("GEM\n  specs:\n    sinatra (4.0.0)\n"), Framework::Sinatra);
        assert_eq!(framework("GEM\n  specs:\n    sinatra (4.0.0)\n    actionview (7.1.3)\n"), Framework::Rails);
        assert_eq!(framework("GEM\n  specs:\n    rack (3.0.0)\n"), Framework::Unknown);
        assert_eq!(framework(""), Framework::Unknown);
    }

    #[test]
    fn spec_lines_only() {
        let lock = concat!(
            "GIT\n  remote: x\n  specs:\n    forked (1.0.0)\n\n",
            "PATH\n  remote: .\n  specs:\n    local (0.1.0)\n\n",
            "GEM\n  specs:\n    nokogiri (1.16.0-x86_64-linux)\n    nokogiri (1.16.0-arm64-darwin)\n      racc (~> 1.4)\n    slim (5.2.1)\n\n",
            "DEPENDENCIES\n    notgem (1.0)\n  slim\n",
        );
        let project = Project::from_lockfile(lock);
        assert_eq!(project.gem_version("nokogiri"), Version::parse("1.16.0"));
        assert_eq!(project.gem_version("forked"), Version::parse("1.0.0"));
        assert_eq!(project.gem_version("local"), Version::parse("0.1.0"));
        assert!(project.gem_version("slim").is_some());
        assert!(project.gem_version("racc").is_none());
        assert!(project.gem_version("notgem").is_none());
        assert!(project.has_gem_family("forked"));
        assert!(!project.has_gem_family("fork"));
    }

    #[test]
    fn nearest_lockfile_wins() {
        let root = std::env::temp_dir().join(format!("rt-project-{}", std::process::id()));
        let inner = root.join("engine/app/views/users");
        std::fs::create_dir_all(&inner).unwrap();
        let template = inner.join("show.html.erb");
        assert_eq!(Project::for_template(&template).framework, Framework::Unknown);
        std::fs::write(root.join("Gemfile.lock"), "GEM\n  specs:\n    actionview (7.1.3)\n").unwrap();
        assert_eq!(Project::for_template(&template).framework, Framework::Rails);
        std::fs::write(root.join("engine/Gemfile.lock"), "GEM\n  specs:\n    sinatra (4.0.0)\n").unwrap();
        assert_eq!(Project::for_template(&template).framework, Framework::Sinatra);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn registered_format_lines() {
        let name = |line| registered_format(line);
        assert_eq!(name(r#"Mime::Type.register "text/richtext", :rtf"#), Some("rtf"));
        assert_eq!(name(r#"  Mime::Type.register_alias "text/html", :mobile"#), Some("mobile"));
        assert_eq!(name("Mime::Type.register_alias 'text/html', :phone_x"), Some("phone_x"));
        assert_eq!(name(r#"Mime::Type.register("application/x-md", :md, %w[text/x-md], %w[markdown])"#), Some("md"));
        assert_eq!(name(r#"Mime::Type.register_alias("text/html", :tablet, [:tab])"#), Some("tablet"));
        assert_eq!(name(r#"::Mime::Type.register "a/b", :ab # trailing"#), Some("ab"));
        assert_eq!(name(r#"# Mime::Type.register "a/b", :ab"#), None);
        assert_eq!(name(r#"Mime::Type.registered "a/b", :ab"#), None);
        assert_eq!(name(r#"Mime::Type.register "a/b", "ab""#), None);
        assert_eq!(name("Mime::Type.register"), None);
        assert_eq!(name("config.x = 1"), None);
    }

    #[test]
    fn partial_prefix_setting_is_read_from_application_environments_and_initializers() {
        let off = "config.action_view.prefix_partial_path_with_controller_namespace = false\n";
        let prefixes_in = |file: &str, text: &str| {
            let root = std::env::temp_dir().join(format!("rt-project-prefix-{}-{}", std::process::id(), file.replace('/', "_")));
            std::fs::create_dir_all(root.join(file).parent().unwrap()).unwrap();
            std::fs::write(root.join(file), text).unwrap();
            let found = read_app_config(&root.join("config")).prefixes_partial_paths;
            std::fs::remove_dir_all(&root).unwrap();
            found
        };
        assert!(!prefixes_in("config/application.rb", off));
        assert!(!prefixes_in("config/environments/production.rb", off), "any environment counts");
        assert!(!prefixes_in("config/initializers/a/b.rb", off));
        assert!(prefixes_in("config/application.rb", "config.action_view.prefix_partial_path_with_controller_namespace = true\n"));
        assert!(prefixes_in("config/application.rb", &format!("  # {off}")));
        assert!(prefixes_in("config/locales/x.rb", off), "only the config files Rails loads");
    }

    #[test]
    fn partial_prefix_setting_with_a_trailing_comment_is_off() {
        let setting = |value: &str| format!("config.action_view.prefix_partial_path_with_controller_namespace = {value}\n");
        assert!(turns_off_partial_prefix(&setting("false # legacy")));
        assert!(!turns_off_partial_prefix(&setting("true # was false")));
    }

    #[test]
    fn view_paths_added_app_wide() {
        assert!(adds_view_paths("ActionController::Base.prepend_view_path Rails.root.join(\"themes\")\n"));
        assert!(adds_view_paths("config.paths[\"app/views\"] << \"x\"\n"));
        assert!(!adds_view_paths("# prepend_view_path \"x\"\nconfig.x = 1\n"));
        let root = std::env::temp_dir().join(format!("rt-project-pathgem-{}", std::process::id()));
        std::fs::create_dir_all(root.join("engines/admin/app/views")).unwrap();
        std::fs::create_dir_all(root.join("app/views")).unwrap();
        let lock = |remote: &str| format!("PATH\n  remote: {remote}\n  specs:\n    admin (0.1.0)\n\nGEM\n  remote: engines/admin\n");
        assert!(path_gem_has_views(&lock("engines/admin"), &root));
        assert!(!path_gem_has_views(&lock("."), &root), "the app itself");
        assert!(!path_gem_has_views(&lock("engines/other"), &root));
        assert!(!path_gem_has_views("GEM\n  remote: engines/admin\n", &root));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn app_registered_formats() {
        let root = std::env::temp_dir().join(format!("rt-project-formats-{}", std::process::id()));
        let initializers = root.join("config/initializers");
        let template = root.join("app/views/users/show.html.erb");
        std::fs::create_dir_all(&initializers).unwrap();
        std::fs::create_dir_all(template.parent().unwrap()).unwrap();
        assert!(!Project::for_template(&template).is_format("mobile"));
        std::fs::write(initializers.join("mime_types.rb"), "# comment\nMime::Type.register_alias \"text/html\", :mobile\n").unwrap();
        std::fs::write(initializers.join("notes.txt"), "Mime::Type.register \"a/b\", :ignored\n").unwrap();
        std::fs::create_dir_all(initializers.join("mime")).unwrap();
        std::fs::write(initializers.join("mime/types.rb"), "Mime::Type.register \"text/x-md\", :md\n").unwrap();
        let project = Project::for_template(&template);
        assert!(project.is_format("mobile") && project.is_format("md") && project.is_format("html"));
        assert!(!project.is_format("ignored") && !project.is_format("de"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn engine_sees_host_app_formats() {
        let root = std::env::temp_dir().join(format!("rt-project-engine-{}", std::process::id()));
        let template = root.join("engines/admin/app/views/x/show.mobile.erb");
        std::fs::create_dir_all(template.parent().unwrap()).unwrap();
        std::fs::create_dir_all(root.join("config/initializers")).unwrap();
        std::fs::write(root.join("Gemfile.lock"), "GEM\n  specs:\n    actionview (7.1.3)\n").unwrap();
        std::fs::write(root.join("config/initializers/mime.rb"), "Mime::Type.register_alias \"text/html\", :mobile\n").unwrap();
        let project = Project::for_template(&template);
        assert!(project.is_format("mobile"));
        assert_eq!(project.framework, Framework::Rails);
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn grammar(ruby: &str) -> ruby_prism::Version {
        Project { ruby: Version::parse(ruby), ..Project::unknown() }.prism_version()
    }

    #[test]
    fn grammar_follows_the_ruby_version() {
        use ruby_prism::Version::*;
        assert_eq!(Project::unknown().prism_version(), Latest);
        for (ruby, expected) in [
            ("2.7.8", CRuby3_3),
            ("3.2.9", CRuby3_3),
            ("3.3.0", CRuby3_3),
            ("3.3.6", CRuby3_3),
            ("3.4.0", CRuby3_4),
            ("3.4.9", CRuby3_4),
            ("3.5.0", CRuby4_0),
            ("4.0.2", CRuby4_0),
            ("4.1.0", CRuby4_1),
            ("4.1.9", CRuby4_1),
            ("4.2.0", Latest),
            ("5.0", Latest),
        ] {
            assert_eq!(grammar(ruby), expected, "{ruby}");
        }
    }

    #[test]
    fn ruby_version_texts() {
        let release = ruby_version_file_version;
        assert_eq!(release("3.3.6\n"), Version::parse("3.3.6"));
        assert_eq!(release("ruby-3.3.6\n"), Version::parse("3.3.6"));
        assert_eq!(release("# pinned\n\n  3.4.1 # comment\n"), Version::parse("3.4.1"));
        assert_eq!(release("3.4.0-preview1"), Version::parse("3.4.0"));
        assert_eq!(release("jruby-9.4.5.0"), None);
        assert_eq!(release("system"), None);
        assert_eq!(release("3"), None);
        assert_eq!(release("\u{feff}3.3.6\n"), Version::parse("3.3.6"));
        assert_eq!(release(""), None);
        let lock = "GEM\n  specs:\n    rack (3.0.0)\n\nRUBY VERSION\n   ruby 3.3.6p108\n\nBUNDLED WITH\n   2.5.0\n";
        assert_eq!(Project::from_lockfile(lock).ruby, Version::parse("3.3.6"));
        assert_eq!(Project::from_lockfile("RUBY VERSION\n   ruby 3.4.0\n").ruby, Version::parse("3.4.0"));
        assert_eq!(Project::from_lockfile("GEM\n  specs:\n    rack (3.0.0)\n").ruby, None);
        assert_eq!(Project::from_lockfile("BUNDLED WITH\n   ruby 3.3.6\n").ruby, None);
    }

    #[test]
    fn nearest_ruby_version_file_beats_the_lockfile() {
        let root = std::env::temp_dir().join(format!("rt-project-ruby-{}", std::process::id()));
        let template = root.join("engine/app/views/users/show.html.erb");
        std::fs::create_dir_all(template.parent().unwrap()).unwrap();
        let ruby = || Project::for_template(&template).ruby;
        assert_eq!(ruby(), None);
        std::fs::write(root.join("Gemfile.lock"), "RUBY VERSION\n   ruby 3.4.1p10\n").unwrap();
        assert_eq!(ruby(), Version::parse("3.4.1"));
        std::fs::write(root.join(".ruby-version"), "3.3.6\n").unwrap();
        assert_eq!(ruby(), Version::parse("3.3.6"));
        std::fs::write(root.join("engine/.ruby-version"), "ruby-3.2.2\n").unwrap();
        assert_eq!(ruby(), Version::parse("3.2.2"));
        std::fs::write(root.join("engine/.ruby-version"), "jruby-9.4.5.0\n").unwrap();
        assert_eq!(ruby(), Version::parse("3.4.1"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn version_ordering() {
        assert!(v("7.0.8") < v("7.1"));
        assert!(v("7.1.0") == v("7.1"));
        assert!(v("7.10") > v("7.9"));
        assert!(v("8.0.0.rc1") < v("8.0.0"));
        assert!(v("8.0.0.rc1") < v("8.0.0.rc2"));
        assert!(v("8.0.0.beta1") < v("8.0.0.rc1"));
        assert!(v("8.0.0.rc1") > v("7.2.2"));
        assert!(v("1.0.0.a") == v("1.0.a"));
        assert!(v("7.1.0.rc1") == v("7.1.rc1"));
        assert!(v("2.0.0.beta") == v("2.0.beta.0"));
        assert!(v("1.0.a") < v("1.0"));
        assert!(v("0.0.0") == v("0") && v("0.0.0").at_least("0"));
        assert!(v("0.0.0") < v("0.0.1"));
        assert_eq!(Version::parse(""), None);
        assert_eq!(Version::parse("1.x_y"), None);
    }

    #[test]
    fn version_gates() {
        assert!(!v("7.0.8").at_least("7.1"));
        assert!(v("7.1.0").at_least("7.1"));
        assert!(v("7.1.0.rc1").at_least("7.0"));
        assert!(!v("7.1.0.rc1").at_least("7.1"));
        assert!(!v("8.0.1").at_least("8.0.2"));
        assert!(v("8.0.2").at_least("8.0.2"));
        assert!(!v("8.0.2.rc1").at_least("8.0.2"));
        assert!(v("8.1.0").at_least("8.0.2"));
    }

    fn rails_app(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("rt-project-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        crate::util::write_files(&root, &[("Gemfile.lock", "GEM\n  specs:\n    actionview (7.1.0)\n")]);
        crate::util::write_files(&root, files);
        root
    }

    #[test]
    fn view_paths_added_by_code_are_read_per_project_not_kept_in_the_cache() {
        let root = rails_app("adds-paths", &[("app/views/users/show.html.erb", "")]);
        let (cache, view) = (Arc::new(view_roots::Cache::default()), root.join("app/views/users/show.html.erb"));
        let project = || Project::for_template(&view).with_view_cache(Arc::clone(&cache));
        assert!(project().lacks_view_folder("ghost"));
        crate::util::write_files(&root, &[("app/controllers/application_controller.rb", "prepend_view_path \"app/views/themes\"\n")]);
        assert!(!project().lacks_view_folder("ghost"), "found without the cache being cleared");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_project_keeps_the_view_roots_it_was_given() {
        let root = rails_app("given-roots", &[("app/views/users/show.html.erb", "")]);
        let (cache, view) = (Arc::new(view_roots::Cache::default()), root.join("app/views/users/show.html.erb"));
        let project = Project::for_template(&view).with_view_cache(Arc::clone(&cache));
        assert!(project.cached_view_roots().is_none());
        let found = project.load_view_roots().unwrap();
        let cached = project.cached_view_roots().unwrap();
        assert!(Arc::ptr_eq(&found, &cached));
        let fresh = Project::for_template(&view).with_view_cache(Arc::clone(&cache));
        fresh.use_view_roots(cached);
        cache.clear();
        assert!(Arc::ptr_eq(&found, fresh.loaded_view_roots().unwrap()), "a cleared cache doesn't make it search again");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
