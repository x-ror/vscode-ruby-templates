//! Framework and gem versions of the project a template belongs to, from the nearest `Gemfile.lock`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::rails_paths::{app_root, files_under, is_known_format};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framework {
    Rails,
    Sinatra,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub framework: Framework,
    gems: HashMap<String, String>,
    /// Format names the app registers in `config/initializers/**/*.rb`, beyond Rails' own.
    formats: Vec<String>,
}

impl Project {
    pub fn unknown() -> Self {
        Project { framework: Framework::Unknown, gems: HashMap::new(), formats: Vec::new() }
    }

    /// The project of the nearest `Gemfile.lock` above `template`, unknown when there is none, with the formats
    /// registered by the initializers of the app that holds `template` and of the lockfile's app (Mime::SET is
    /// process-wide, so an engine's views see the host app's registrations).
    pub fn for_template(template: &Path) -> Self {
        let lock_dir = template.ancestors().skip(1).find(|dir| dir.join("Gemfile.lock").is_file());
        let project = lock_dir
            .and_then(|dir| std::fs::read_to_string(dir.join("Gemfile.lock")).ok())
            .map_or_else(Self::unknown, |lock| Self::from_lockfile(&lock));
        let template_app = app_root(template).and_then(|app| Some(app.parent()?.to_path_buf()));
        let mut formats: Vec<String> = template_app
            .iter()
            .map(PathBuf::as_path)
            .chain(lock_dir)
            .flat_map(|app| registered_formats(&app.join("config/initializers")))
            .collect();
        formats.sort();
        formats.dedup();
        Project { formats, ..project }
    }

    /// Is `segment` a template format: one of Rails' own or registered by the app?
    pub fn is_format(&self, segment: &str) -> bool {
        is_known_format(segment) || self.formats.iter().any(|f| f == segment)
    }

    pub fn from_lockfile(lock: &str) -> Self {
        let mut gems = HashMap::new();
        let mut in_specs = false;
        for line in lock.lines() {
            if !line.starts_with(' ') {
                in_specs = false;
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
        Project { framework, gems, formats: Vec::new() }
    }

    pub fn gem_version(&self, name: &str) -> Option<Version> {
        Version::parse(self.gems.get(name)?)
    }

    /// Does a gem named `name` or `name_*` / `name-*` exist (a gem's own view folder, `spree/` from `spree_core`)?
    pub fn has_gem_family(&self, name: &str) -> bool {
        self.gems.keys().any(|gem| gem == name || gem.strip_prefix(name).is_some_and(|rest| rest.starts_with(['_', '-'])))
    }
}

/// Names registered by `Mime::Type.register` / `register_alias` in the `*.rb` files under `dir`, as Rails loads them.
fn registered_formats(dir: &Path) -> Vec<String> {
    files_under(dir, usize::MAX)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "rb"))
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|text| text.lines().filter_map(registered_format).map(str::to_string).collect::<Vec<_>>())
        .collect()
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
fn spec_line(line: &str) -> Option<(&str, &str)> {
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
}
