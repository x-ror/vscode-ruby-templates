//! Framework and gem versions of the project a template belongs to, from the nearest `Gemfile.lock`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;

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
}

impl Project {
    fn unknown() -> Self {
        Project { framework: Framework::Unknown, gems: HashMap::new() }
    }

    /// The project of the nearest `Gemfile.lock` above `template`; unknown when there is none.
    pub fn for_template(template: &Path) -> Self {
        template
            .ancestors()
            .skip(1)
            .find_map(|dir| std::fs::read_to_string(dir.join("Gemfile.lock")).ok())
            .map_or_else(Self::unknown, |lock| Self::from_lockfile(&lock))
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
        Project { framework, gems }
    }

    pub fn gem_version(&self, name: &str) -> Option<Version> {
        Version::parse(self.gems.get(name)?)
    }

    /// Does a gem named `name` or `name_*` / `name-*` exist (a gem's own view folder, `spree/` from `spree_core`)?
    pub fn has_gem_family(&self, name: &str) -> bool {
        self.gems.keys().any(|gem| gem == name || gem.strip_prefix(name).is_some_and(|rest| rest.starts_with(['_', '-'])))
    }
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
