//! What applies to one template: the `.ruby-templates.toml` files above it (the nearest wins, key by key), then the
//! editor's settings where the files are silent.

use super::file::{parse, ConfigFile, LanguageTable, RuleTable, FILE_NAME};
use super::{by_code, by_name, CountOption, Kind, Rule, EMPTY_LINES, RULES, TRAILING_EMPTY_LINES};
use crate::format::Settings;
use crate::glob::Glob;
use crate::lang::TemplateLang;
use crate::options::Options;
use crate::util::{self, slash_path};
use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_lsp_server::ls_types::Uri;

/// The files read so far, by folder; `None` for a folder without one.
pub type Configs = HashMap<PathBuf, Option<Arc<ConfigFile>>>;

pub struct ResolvedRules {
    format: bool,
    diagnostics: bool,
    align_comments: bool,
    /// Rule names that report nothing: switched off, outside the template's language or disabled by name.
    off: HashSet<String>,
    extra_engines: Vec<String>,
    /// `Layout/EmptyLines`: the longest run of blank lines kept; `None` when the rule is off.
    max_empty_lines: Option<usize>,
    trailing_empty_lines: bool,
    /// Nearest first.
    files: Vec<Arc<ConfigFile>>,
    excluded_by: Option<PathBuf>,
}

pub fn resolve(uri: &Uri, lang: TemplateLang, editor: &Options, configs: &mut Configs) -> ResolvedRules {
    let Some(template) = util::file_path(uri) else { return ResolvedRules::new(Path::new(""), lang, editor, vec![]) };
    ResolvedRules::new(&template, lang, editor, config_files(&template, configs))
}

/// The files in the folders above `template`, up to the first with `root = true`.
fn config_files(template: &Path, configs: &mut Configs) -> Vec<Arc<ConfigFile>> {
    let mut files = Vec::new();
    for dir in template.ancestors().skip(1) {
        let found = configs.entry(dir.to_path_buf()).or_insert_with(|| read(dir)).clone();
        let Some(file) = found else { continue };
        let root = file.root;
        files.push(file);
        if root {
            break;
        }
    }
    files
}

fn read(dir: &Path) -> Option<Arc<ConfigFile>> {
    let path = dir.join(FILE_NAME);
    match std::fs::read_to_string(&path) {
        Ok(text) => Some(Arc::new(parse(path, &text))),
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => None,
        Err(e) => Some(Arc::new(ConfigFile::unreadable(path, &e))),
    }
}

impl ResolvedRules {
    fn new(template: &Path, lang: TemplateLang, editor: &Options, files: Vec<Arc<ConfigFile>>) -> Self {
        let section = section(lang);
        let in_table = |get: fn(&LanguageTable) -> Option<bool>| {
            nearest(&files, |f| f.languages.iter().find(|(name, _)| Some(*name) == section).and_then(|(_, table)| get(table)))
        };
        let in_lists = |get: fn(&LanguageTable) -> Option<&Vec<String>>| {
            nearest(&files, |f| f.languages.iter().find(|(name, _)| Some(*name) == section).and_then(|(_, table)| get(table).cloned()))
        };
        let excluded_by = nearest(&files, |f| f.exclude.as_ref().map(|globs| (f, globs)))
            .filter(|(file, globs)| matches_any(file, globs, template))
            .map(|(file, _)| file.path.clone());
        let disabled = in_lists(|t| t.disabled_diagnostics.as_ref()).unwrap_or_else(|| {
            let codes = editor.disabled(lang).iter();
            let lint_name = |code: &String| match by_code(code) {
                Some(rule) => (rule.kind == Kind::Lint).then(|| rule.name.to_string()),
                None => Some(code.clone()),
            };
            codes.filter_map(lint_name).collect()
        });
        let off: HashSet<String> =
            RULES.iter().filter(|rule| !rule_on(&files, template, lang, rule)).map(|rule| rule.name.to_string()).chain(disabled).collect();
        let max_empty_lines = by_name(EMPTY_LINES)
            .filter(|rule| !off.contains(rule.name))
            .and_then(|rule| rule.options.iter().find(|option| option.key == "max").map(|option| count(&files, rule, option)));
        Self {
            format: in_table(|t| t.format_enabled).unwrap_or_else(|| editor.format_enabled(lang)),
            diagnostics: in_table(|t| t.diagnostics_enabled).unwrap_or_else(|| editor.diagnostics_enabled(lang)),
            align_comments: in_table(|t| t.align_comments).unwrap_or(editor.align_erb_comments),
            extra_engines: in_lists(|t| t.extra_engines.as_ref()).unwrap_or_else(|| editor.extra_engines.clone()),
            max_empty_lines,
            trailing_empty_lines: !off.contains(TRAILING_EMPTY_LINES),
            off,
            files,
            excluded_by,
        }
    }

    /// Whether a diagnostic with this code is reported.
    pub fn reports(&self, code: &str) -> bool {
        !self.off.contains(code)
    }

    pub fn diagnostics_enabled(&self) -> bool {
        self.diagnostics && self.excluded_by.is_none()
    }

    pub fn format_enabled(&self) -> bool {
        self.format
    }

    pub fn extra_engines(&self) -> &[String] {
        &self.extra_engines
    }

    /// The file whose global `exclude` covers the template.
    pub fn excluded_by(&self) -> Option<&Path> {
        self.excluded_by.as_deref()
    }

    /// The config files that apply, nearest first.
    pub fn files(&self) -> &[Arc<ConfigFile>] {
        &self.files
    }

    pub fn format_settings(&self, base: Settings) -> Settings {
        Settings {
            align_comments: self.align_comments,
            max_empty_lines: self.max_empty_lines,
            trailing_empty_lines: self.trailing_empty_lines,
            ..base
        }
    }
}

/// The config table of a language; Skim shares Slim's.
fn section(lang: TemplateLang) -> Option<&'static str> {
    match lang {
        TemplateLang::Erb => Some("erb"),
        TemplateLang::Haml => Some("haml"),
        TemplateLang::Slim | TemplateLang::Skim => Some("slim"),
        TemplateLang::Ruby => Some("jbuilder"),
        TemplateLang::Liquid => None,
    }
}

fn nearest<'a, T>(files: &'a [Arc<ConfigFile>], get: impl Fn(&'a ConfigFile) -> Option<T>) -> Option<T> {
    files.iter().find_map(|file| get(file))
}

fn matches_any(file: &ConfigFile, globs: &[Glob], template: &Path) -> bool {
    let relative = file.path.parent().and_then(|dir| template.strip_prefix(dir).ok());
    relative.is_some_and(|relative| globs.iter().any(|glob| glob.matches(&slash_path(relative))))
}

fn rule_table<'a>(file: &'a ConfigFile, rule: &Rule) -> Option<&'a RuleTable> {
    file.rules.iter().find(|(r, _)| r.name == rule.name).map(|(_, table)| table)
}

/// The option's value in the nearest file that sets it.
fn count(files: &[Arc<ConfigFile>], rule: &Rule, option: &CountOption) -> usize {
    let set = |file: &ConfigFile| rule_table(file, rule)?.counts.iter().find(|(key, _)| *key == option.key).map(|&(_, count)| count);
    nearest(files, set).unwrap_or(option.default)
}

/// The nearest file that sets the rule's `enabled` or its department's decides, the rule's beating the department's within
/// that file; unset means the rule's default. Then the rule's `include` and `exclude`.
fn rule_on(files: &[Arc<ConfigFile>], template: &Path, lang: TemplateLang, rule: &Rule) -> bool {
    let department = |file: &ConfigFile| file.departments.iter().find(|(d, _)| *d == rule.department()).map(|&(_, enabled)| enabled);
    let enabled = nearest(files, |f| rule_table(f, rule).and_then(|table| table.enabled).or_else(|| department(f)))
        .unwrap_or(rule.is_on_by_default(lang));
    let included =
        nearest(files, |f| Some((f, rule_table(f, rule)?.include.as_ref()?))).is_none_or(|(f, globs)| matches_any(f, globs, template));
    let excluded =
        nearest(files, |f| Some((f, rule_table(f, rule)?.exclude.as_ref()?))).is_some_and(|(f, globs)| matches_any(f, globs, template));
    rule.applies_to(lang) && enabled && included && !excluded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::write_files;

    /// A temp project with `files`, removed on drop.
    struct Project(PathBuf);

    impl Project {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let root = std::env::temp_dir().join(format!("rt-rules-{name}-{}", std::process::id()));
            // The top file stops the search, so a file above the temp folder can't change a result.
            let top = |text: &str| format!("root = true\n{text}");
            let others = files.iter().filter(|(file, _)| *file != FILE_NAME);
            let own = files.iter().find(|(file, _)| *file == FILE_NAME).map_or_else(String::new, |(_, text)| text.to_string());
            write_files(&root, &others.copied().collect::<Vec<_>>());
            write_files(&root, &[(FILE_NAME, &top(&own))]);
            Self(root)
        }

        fn resolve(&self, file: &str, lang: TemplateLang, editor: &Options) -> ResolvedRules {
            resolve(&util::file_uri(self.0.join(file)).unwrap(), lang, editor, &mut Configs::new())
        }

        fn erb(&self, file: &str) -> ResolvedRules {
            self.resolve(file, TemplateLang::Erb, &Options::default())
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn editor(init: serde_json::Value) -> Options {
        Options::from_init(Some(&init)).0
    }

    const IMG: &str = "Html/ImgAlt";

    #[test]
    fn nothing_is_off_without_files() {
        let project = Project::new("none", &[("a.html.erb", "")]);
        let rules = project.erb("a.html.erb");
        assert!(rules.reports(IMG) && rules.diagnostics_enabled() && rules.format_enabled());
    }

    #[test]
    fn rule_department_and_default_order() {
        let toml = "[Html]\nenabled = false\n[Html.DuplicateId]\nenabled = true\n[Rails.StrictLocals]\nenabled = false\n";
        let project = Project::new("order", &[(".ruby-templates.toml", toml), ("a.html.erb", "")]);
        let rules = project.erb("a.html.erb");
        assert!(!rules.reports(IMG) && rules.reports("Html/DuplicateId"), "the rule beats its department");
        assert!(!rules.reports("Rails/StrictLocals") && rules.reports("Rails/MissingPartial"));
    }

    #[test]
    fn nested_files_override_key_by_key() {
        let outer = "[erb]\nformat.enabled = false\ndisabled_diagnostics = [\"duplicate-id\"]\n[Html.ImgAlt]\nenabled = false\n";
        let inner = "[erb]\ndisabled_diagnostics = [\"Html/UnclosedElement\"]\n[Html.ImgAlt]\nenabled = true\n";
        let files =
            [(".ruby-templates.toml", outer), ("app/views/.ruby-templates.toml", inner), ("app/views/a.html.erb", ""), ("b.html.erb", "")];
        let project = Project::new("nested", &files);
        let inner_rules = project.erb("app/views/a.html.erb");
        assert!(inner_rules.reports(IMG), "the nearest file wins the rule");
        assert!(!inner_rules.reports("Html/UnclosedElement") && inner_rules.reports("Html/DuplicateId"), "lists are replaced");
        assert!(!inner_rules.format_enabled(), "a key the nearest file leaves out comes from the one above");
        let outer_rules = project.erb("b.html.erb");
        assert!(!outer_rules.reports(IMG) && !outer_rules.reports("Html/DuplicateId") && outer_rules.reports("Html/UnclosedElement"));
    }

    #[test]
    fn globs_are_relative_to_the_file_that_defines_them() {
        let outer = "[Html.ImgAlt]\nexclude = [\"app/views/legacy/**\"]\n";
        let inner = "[Html.DuplicateId]\ninclude = [\"/new/**\"]\n";
        let files = [
            (".ruby-templates.toml", outer),
            ("app/views/.ruby-templates.toml", inner),
            ("app/views/legacy/a.html.erb", ""),
            ("app/views/new/a.html.erb", ""),
            ("app/views/old/a.html.erb", ""),
        ];
        let project = Project::new("globs", &files);
        let legacy = project.erb("app/views/legacy/a.html.erb");
        assert!(!legacy.reports(IMG) && !legacy.reports("Html/DuplicateId"));
        assert!(project.erb("app/views/new/a.html.erb").reports("Html/DuplicateId"));
        let old = project.erb("app/views/old/a.html.erb");
        assert!(old.reports(IMG) && !old.reports("Html/DuplicateId"));
    }

    #[test]
    fn the_nearest_file_that_sets_the_rule_or_its_department_decides() {
        let files = [
            (".ruby-templates.toml", "[Html.ImgAlt]\nenabled = true\n"),
            ("legacy/.ruby-templates.toml", "[Html]\nenabled = false\n"),
            ("legacy/a.html.erb", ""),
            ("b.html.erb", ""),
        ];
        let project = Project::new("dept-files", &files);
        assert!(!project.erb("legacy/a.html.erb").reports(IMG), "the nearer department switch beats the rule above");
        assert!(project.erb("b.html.erb").reports(IMG));
    }

    #[test]
    fn a_file_that_cant_be_read_is_a_problem_not_silence() {
        let project = Project::new("unreadable", &[("a/a.html.erb", ""), ("b/a.html.erb", "")]);
        std::fs::write(project.0.join("a").join(FILE_NAME), [0xff, 0xfe, b'x']).unwrap();
        std::fs::create_dir(project.0.join("b").join(FILE_NAME)).unwrap();
        for dir in ["a", "b"] {
            let rules = project.erb(&format!("{dir}/a.html.erb"));
            let problems = &rules.files()[0].problems;
            assert_eq!(problems.len(), 1, "{dir}");
            assert!(problems[0].message.starts_with("can't read .ruby-templates.toml"), "{}", problems[0].message);
        }
        assert!(project.erb("a/a.html.erb").files()[0].problems[0].message.ends_with("isn't UTF-8"));
    }

    #[test]
    fn root_stops_the_search() {
        let files = [
            (".ruby-templates.toml", "[Html.ImgAlt]\nenabled = false\n"),
            ("app/.ruby-templates.toml", "root = true\n"),
            ("app/a.html.erb", ""),
        ];
        let project = Project::new("root", &files);
        let rules = project.erb("app/a.html.erb");
        assert!(rules.reports(IMG) && rules.files().len() == 1);
    }

    #[test]
    fn global_exclude_silences_the_file() {
        let files = [(".ruby-templates.toml", "exclude = [\"vendor/**\"]\n"), ("vendor/a.html.erb", ""), ("app/a.html.erb", "")];
        let project = Project::new("exclude", &files);
        let vendor = project.erb("vendor/a.html.erb");
        assert!(!vendor.diagnostics_enabled() && vendor.excluded_by().is_some_and(|p| p.ends_with(FILE_NAME)));
        assert!(project.erb("app/a.html.erb").diagnostics_enabled());
    }

    #[test]
    fn the_file_beats_the_editor_which_applies_where_the_file_is_silent() {
        let toml =
            "[erb]\nformat.enabled = true\nformat.align_comments = false\ndisabled_diagnostics = []\n[slim]\nextra_engines = [\"mjml\"]\n";
        let project =
            Project::new("editor", &[(".ruby-templates.toml", toml), ("a.html.erb", ""), ("b.html.slim", ""), ("c.html.haml", "")]);
        let settings = editor(serde_json::json!({
            "erb": { "format": { "enabled": false, "alignComments": true }, "disabledDiagnostics": ["img-missing-alt"] },
            "slim": { "extraEngines": ["other"], "diagnostics": { "enabled": false } },
            "haml": { "diagnostics": { "enabled": false }, "disabledDiagnostics": ["syntax-error"] },
        }));
        let erb = project.resolve("a.html.erb", TemplateLang::Erb, &settings);
        assert!(erb.format_enabled() && erb.reports(IMG) && !erb.format_settings(Settings::default()).align_comments);
        let slim = project.resolve("b.html.slim", TemplateLang::Slim, &settings);
        assert_eq!(slim.extra_engines(), ["mjml"]);
        assert!(!slim.diagnostics_enabled(), "silent in the file: the editor's switch applies");
        let haml = project.resolve("c.html.haml", TemplateLang::Haml, &settings);
        assert!(!haml.diagnostics_enabled() && !haml.reports("Syntax/Template"), "old codes from the editor are normalized");
        let without = Project::new("editor-only", &[("a.html.erb", "")]).resolve("a.html.erb", TemplateLang::Erb, &settings);
        assert!(!without.format_enabled() && !without.reports(IMG) && without.format_settings(Settings::default()).align_comments);
    }

    #[test]
    fn old_codes_and_new_names_both_disable() {
        let toml = "[haml]\ndisabled_diagnostics = [\"void-children\", \"Syntax/IllegalNesting\"]\n";
        let project = Project::new("alias", &[(".ruby-templates.toml", toml), ("a.html.haml", "")]);
        let rules = project.resolve("a.html.haml", TemplateLang::Haml, &Options::default());
        assert!(!rules.reports("Html/VoidChildren") && !rules.reports("Syntax/IllegalNesting") && rules.reports("Syntax/Template"));
    }

    #[test]
    fn a_rule_is_off_outside_its_languages() {
        let project = Project::new("langs", &[("a.html.haml", "")]);
        let haml = project.resolve("a.html.haml", TemplateLang::Haml, &Options::default());
        assert!(!haml.reports(IMG) && haml.reports("Html/VoidChildren"));
    }

    #[test]
    fn formatting_rules_decode_into_the_settings() {
        let project = Project::new("format-rules", &[("a.html.erb", ""), ("b.html.slim", "")]);
        let settings = project.erb("a.html.erb").format_settings(Settings::default());
        assert_eq!((settings.max_empty_lines, settings.trailing_empty_lines), (None, false), "both off for ERB");
        let slim = project.resolve("b.html.slim", TemplateLang::Slim, &Options::default()).format_settings(Settings::default());
        assert_eq!((slim.max_empty_lines, slim.trailing_empty_lines), (None, true), "TrailingEmptyLines on for Slim");
        let editor = editor(serde_json::json!({ "slim": { "disabledDiagnostics": ["Layout/TrailingEmptyLines"] } }));
        let slim = project.resolve("b.html.slim", TemplateLang::Slim, &editor).format_settings(Settings::default());
        assert!(slim.trailing_empty_lines, "the editor's diagnostic list doesn't switch a formatting rule");

        let toml = "[Layout.EmptyLines]\nenabled = true\n[Layout.TrailingEmptyLines]\nenabled = false\n";
        let on = Project::new("format-rules-on", &[(".ruby-templates.toml", toml), ("a.html.erb", "")]);
        let settings = on.erb("a.html.erb").format_settings(Settings::default());
        assert_eq!((settings.max_empty_lines, settings.trailing_empty_lines), (Some(1), false), "max defaults to 1");

        let files = [
            (".ruby-templates.toml", "[Layout]\nenabled = true\n[Layout.EmptyLines]\nmax = 3\n"),
            ("inner/.ruby-templates.toml", "[Layout.EmptyLines]\nmax = 0\nexclude = [\"**/*.slim\"]\n"),
            ("a.html.erb", ""),
            ("inner/a.html.erb", ""),
            ("inner/b.html.slim", ""),
        ];
        let nested = Project::new("format-rules-nested", &files);
        let max = |file: &str, lang| nested.resolve(file, lang, &Options::default()).format_settings(Settings::default()).max_empty_lines;
        assert_eq!(max("a.html.erb", TemplateLang::Erb), Some(3), "the department switch turns a formatting rule on");
        assert_eq!(max("inner/a.html.erb", TemplateLang::Erb), Some(0), "the nearest file sets max");
        assert_eq!(max("inner/b.html.slim", TemplateLang::Slim), None, "its exclude");
        let off = Project::new("format-rules-dept", &[(".ruby-templates.toml", "[Layout]\nenabled = false\n"), ("a.html.slim", "")]);
        let settings = off.resolve("a.html.slim", TemplateLang::Slim, &Options::default()).format_settings(Settings::default());
        assert!(!settings.trailing_empty_lines, "the department switch turns an on-by-default rule off");
    }
}
