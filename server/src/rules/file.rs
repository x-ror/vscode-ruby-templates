//! `.ruby-templates.toml`, read against the rule registry. A part that can't be read is dropped with a `Problem`; the rest is kept.

use super::{by_code, by_name, departments, ChoiceOption, CountOption, Kind, ListOption, Rule};
use crate::glob::Glob;
use crate::util::LineIndex;
use std::ops::Range as Span;
use std::path::PathBuf;
use toml::de::{DeString, DeTable, DeValue};
use toml::Spanned;
use tower_lsp_server::ls_types::Range;

pub const FILE_NAME: &str = ".ruby-templates.toml";

/// The language tables, and the keys each takes.
const LANGUAGE_TABLES: [(&str, &[&str]); 4] = [
    ("erb", &["format.enabled", "format.align_comments", "diagnostics.enabled", "disabled_diagnostics"]),
    ("haml", &["format.enabled", "diagnostics.enabled", "disabled_diagnostics"]),
    ("slim", &["extra_engines", "format.enabled", "diagnostics.enabled", "disabled_diagnostics"]),
    ("jbuilder", &["disabled_diagnostics"]),
];
const RULE_KEYS: [&str; 3] = ["enabled", "include", "exclude"];

#[derive(Debug)]
pub struct Problem {
    pub range: Range,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct RuleTable {
    pub enabled: Option<bool>,
    pub include: Option<Vec<Glob>>,
    pub exclude: Option<Vec<Glob>>,
    /// The rule's whole-number options that the table sets, by key.
    pub counts: Vec<(&'static str, usize)>,
    /// The rule's word options that the table sets, by key.
    pub choices: Vec<(&'static str, &'static str)>,
    /// The rule's list options that the table sets, by key.
    pub lists: Vec<(&'static str, Vec<String>)>,
}

/// One of `[erb]`, `[haml]`, `[slim]` (Skim too), `[jbuilder]`.
#[derive(Debug, Default)]
pub struct LanguageTable {
    pub format_enabled: Option<bool>,
    pub align_comments: Option<bool>,
    pub diagnostics_enabled: Option<bool>,
    /// Rule names, aliases already resolved.
    pub disabled_diagnostics: Option<Vec<String>>,
    pub extra_engines: Option<Vec<String>>,
}

/// What one file says. Its globs are relative to the folder it is in.
#[derive(Debug)]
pub struct ConfigFile {
    pub path: PathBuf,
    pub root: bool,
    pub exclude: Option<Vec<Glob>>,
    pub languages: Vec<(&'static str, LanguageTable)>,
    pub departments: Vec<(&'static str, bool)>,
    pub rules: Vec<(&'static Rule, RuleTable)>,
    pub problems: Vec<Problem>,
}

type Key<'a> = &'a Spanned<DeString<'a>>;
type Value<'a> = &'a Spanned<DeValue<'a>>;

impl ConfigFile {
    /// A file that exists but can't be read: no settings, one problem.
    pub fn unreadable(path: PathBuf, error: &std::io::Error) -> ConfigFile {
        let message = if error.kind() == std::io::ErrorKind::InvalidData {
            format!("can't read {FILE_NAME}: it isn't UTF-8")
        } else {
            format!("can't read {FILE_NAME}: {error}")
        };
        ConfigFile { problems: vec![Problem { range: Range::default(), message }], ..ConfigFile::empty(path) }
    }

    fn empty(path: PathBuf) -> ConfigFile {
        ConfigFile { path, root: false, exclude: None, languages: vec![], departments: vec![], rules: vec![], problems: vec![] }
    }
}

pub fn parse(path: PathBuf, text: &str) -> ConfigFile {
    let mut file = ConfigFile { path, root: false, exclude: None, languages: vec![], departments: vec![], rules: vec![], problems: vec![] };
    let mut reader = Reader { index: LineIndex::new(text), problems: vec![] };
    match DeTable::parse(text) {
        Ok(table) => reader.table(&mut file, table.get_ref()),
        Err(error) => reader.problem(error.span().unwrap_or(0..0), format!("invalid TOML: {}", error.message())),
    }
    reader.problems.sort_by_key(|p| (p.range.start.line, p.range.start.character));
    file.problems = reader.problems;
    file
}

struct Reader<'a> {
    index: LineIndex<'a>,
    problems: Vec<Problem>,
}

impl Reader<'_> {
    fn problem(&mut self, span: Span<usize>, message: String) {
        self.problems.push(Problem { range: self.index.range(span.start, span.end), message });
    }

    fn table(&mut self, file: &mut ConfigFile, table: &DeTable) {
        for (key, value) in table {
            let name: &str = key.get_ref();
            if name == "root" {
                file.root = self.boolean(name, value).unwrap_or(false);
            } else if name == "exclude" {
                file.exclude = self.globs(name, value);
            } else if let Some((section, allowed)) = LANGUAGE_TABLES.iter().find(|(section, _)| *section == name) {
                let language = self.language(section, allowed, value);
                file.languages.push((section, language));
            } else if name.starts_with(|c: char| c.is_ascii_uppercase()) || name.contains('/') {
                self.department(file, key, value);
            } else {
                let message = format!(
                    "unknown key `{name}`; it takes root, exclude, the tables [erb], [haml], [slim] and [jbuilder], and a department such as [Html]"
                );
                self.problem(key.span(), message);
            }
        }
    }

    fn language(&mut self, section: &str, allowed: &[&str], value: Value) -> LanguageTable {
        let mut language = LanguageTable::default();
        let Some(table) = self.as_table(section, value) else { return language };
        for (path, key, value) in leaves(table, allowed) {
            if !allowed.contains(&path.as_str()) {
                let home = LANGUAGE_TABLES.iter().find(|(_, keys)| keys.contains(&path.as_str()));
                let message = match home {
                    Some((other, _)) => format!("`{path}` belongs in [{other}]"),
                    None => format!("unknown key `{path}` in [{section}]; it takes {}", allowed.join(", ")),
                };
                self.problem(key.span(), message);
                continue;
            }
            match path.as_str() {
                "format.enabled" => language.format_enabled = self.boolean(&path, value),
                "format.align_comments" => language.align_comments = self.boolean(&path, value),
                "diagnostics.enabled" => language.diagnostics_enabled = self.boolean(&path, value),
                "extra_engines" => language.extra_engines = self.strings(&path, value).map(|v| v.into_iter().map(|(s, _)| s).collect()),
                _ => language.disabled_diagnostics = self.rule_names(&path, value),
            }
        }
        language
    }

    fn department(&mut self, file: &mut ConfigFile, key: Key, value: Value) {
        let name: &str = key.get_ref();
        if name.contains('/') {
            return self.problem(key.span(), format!("write it as [{}]", name.replace('/', ".")));
        }
        let Some(department) = departments().find(|d| *d == name) else {
            let known = departments().collect::<Vec<_>>().join(", ");
            return self.problem(key.span(), format!("unknown department `{name}`; the departments are {known}"));
        };
        let Some(table) = self.as_table(name, value) else { return };
        for (sub, value) in table {
            let sub_name: &str = sub.get_ref();
            if sub_name == "enabled" {
                file.departments.extend(self.boolean(sub_name, value).map(|enabled| (department, enabled)));
            } else if value.get_ref().is_table() {
                let full = format!("{department}/{sub_name}");
                match by_name(&full) {
                    Some(rule) => {
                        let rule_table = self.rule_table(&format!("{department}.{sub_name}"), rule, value);
                        file.rules.push((rule, rule_table));
                    }
                    None => self.problem(sub.span(), format!("unknown rule `{full}`")),
                }
            } else {
                let message =
                    format!("unknown key `{sub_name}` in [{department}]; it takes enabled, or a rule table such as [{department}.Name]");
                self.problem(sub.span(), message);
            }
        }
    }

    fn rule_table(&mut self, table_name: &str, rule: &'static Rule, value: Value) -> RuleTable {
        let mut parsed = RuleTable::default();
        let Some(table) = self.as_table(table_name, value) else { return parsed };
        for (key, value) in table {
            match key.get_ref().as_ref() {
                "enabled" => parsed.enabled = self.boolean("enabled", value),
                "include" => parsed.include = self.globs("include", value),
                "exclude" => parsed.exclude = self.globs("exclude", value),
                other => {
                    let count = rule.options.iter().find(|option| option.key == other);
                    let choice = rule.choices.iter().find(|choice| choice.key == other);
                    let list = rule.lists.iter().find(|list| list.key == other);
                    if let Some(option) = count {
                        parsed.counts.extend(self.count(option, value));
                    } else if let Some(option) = choice {
                        parsed.choices.extend(self.choice(option, value));
                    } else if let Some(option) = list {
                        parsed.lists.extend(self.words(option, value));
                    } else {
                        let own = rule.options.iter().map(|option| option.key);
                        let own = own.chain(rule.choices.iter().map(|choice| choice.key)).chain(rule.lists.iter().map(|list| list.key));
                        let takes = RULE_KEYS.into_iter().chain(own).collect::<Vec<_>>();
                        let message = format!("unknown key `{other}` in [{table_name}]; it takes {}", takes.join(", "));
                        self.problem(key.span(), message);
                    }
                }
            }
        }
        parsed
    }

    fn count(&mut self, option: &CountOption, value: Value) -> Option<(&'static str, usize)> {
        let count = value.get_ref().as_integer().and_then(|n| usize::from_str_radix(n.as_str(), n.radix()).ok());
        if count.is_none() {
            self.problem(value.span(), format!("`{}` must be a whole number, 0 or more", option.key));
        }
        count.map(|count| (option.key, count))
    }

    fn choice(&mut self, option: &ChoiceOption, value: Value) -> Option<(&'static str, &'static str)> {
        let word = value.get_ref().as_str().and_then(|word| option.values.iter().find(|&&allowed| allowed == word));
        if word.is_none() {
            let got = value.get_ref().as_str().map_or_else(|| value.get_ref().type_str().to_string(), |word| format!("\"{word}\""));
            let message = format!("`{}` must be one of {}, got {got}", option.key, option.values.join(", "));
            self.problem(value.span(), message);
        }
        word.map(|&word| (option.key, word))
    }

    /// The words of a list option: element names, lower-cased.
    fn words(&mut self, option: &ListOption, value: Value) -> Option<(&'static str, Vec<String>)> {
        let words = self.strings(option.key, value)?;
        let mut kept = Vec::new();
        for (word, span) in words {
            if word.is_empty() || !word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | ':' | '_')) {
                self.problem(span, format!("`{word}` is not an element name"));
            } else {
                kept.push(word.to_ascii_lowercase());
            }
        }
        Some((option.key, kept))
    }

    fn as_table<'v>(&mut self, name: &str, value: &'v Spanned<DeValue<'v>>) -> Option<&'v DeTable<'v>> {
        let table = value.get_ref().as_table();
        if table.is_none() {
            self.problem(value.span(), format!("`{name}` must be a table"));
        }
        table
    }

    fn boolean(&mut self, name: &str, value: Value) -> Option<bool> {
        let boolean = value.get_ref().as_bool();
        if boolean.is_none() {
            self.problem(value.span(), format!("`{name}` must be true or false"));
        }
        boolean
    }

    /// The strings of a list, with their spans; `None` when it isn't a list of strings.
    fn strings(&mut self, name: &str, value: Value) -> Option<Vec<(String, Span<usize>)>> {
        let items: Option<Vec<_>> = value
            .get_ref()
            .as_array()
            .and_then(|items| items.iter().map(|item| Some((item.get_ref().as_str()?.to_string(), item.span()))).collect());
        if items.is_none() {
            self.problem(value.span(), format!("`{name}` must be a list of strings"));
        }
        items
    }

    fn globs(&mut self, name: &str, value: Value) -> Option<Vec<Glob>> {
        let patterns = self.strings(name, value)?;
        let mut globs = Vec::new();
        for (pattern, span) in patterns {
            match Glob::new(&pattern) {
                Some(glob) => globs.push(glob),
                None => self.problem(span, format!("can't read the glob `{pattern}`")),
            }
        }
        Some(globs)
    }

    /// Rule names from names or old codes; the ones that are neither are dropped.
    fn rule_names(&mut self, name: &str, value: Value) -> Option<Vec<String>> {
        let mut names = Vec::new();
        for (code, span) in self.strings(name, value)? {
            match by_code(&code) {
                Some(rule) if rule.kind == Kind::Lint => names.push(rule.name.to_string()),
                Some(rule) => self.problem(
                    span,
                    format!("`{}` formats, it has no diagnostic; switch it in [{}]", rule.name, rule.name.replace('/', ".")),
                ),
                None => self.problem(span, format!("unknown rule `{code}`")),
            }
        }
        Some(names)
    }
}

/// The entries of `table` with tables flattened one level, as `format.enabled` is written under `[erb]`.
/// An empty table stays one entry unless a key in `allowed` lives under it, so `[erb.unknown]` is still reported.
fn leaves<'a>(table: &'a DeTable<'a>, allowed: &[&str]) -> Vec<(String, Key<'a>, Value<'a>)> {
    let mut out = Vec::new();
    for (key, value) in table {
        let name: &str = key.get_ref();
        match value.get_ref().as_table() {
            Some(inner)
                if !inner.is_empty() || allowed.iter().any(|key| key.strip_prefix(name).is_some_and(|rest| rest.starts_with('.'))) =>
            {
                out.extend(inner.iter().map(|(sub, v)| (format!("{name}.{}", sub.get_ref()), sub, v)))
            }
            _ => out.push((name.to_string(), key, value)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn read(text: &str) -> ConfigFile {
        parse(Path::new("/p/.ruby-templates.toml").to_path_buf(), text)
    }

    fn messages(file: &ConfigFile) -> Vec<String> {
        file.problems.iter().map(|p| format!("{}:{}: {}", p.range.start.line + 1, p.range.start.character + 1, p.message)).collect()
    }

    #[test]
    fn a_valid_file_is_read_whole() {
        let file = read(
            r#"
root = true
exclude = ["vendor/**", "tmp/**"]

[erb]
format.enabled = false
format.align_comments = true
diagnostics.enabled = true
disabled_diagnostics = ["Html/ImgAlt", "unclosed-element"]

[slim]
extra_engines = ["mjml"]

[Html]
enabled = false

[Html.ImgAlt]
enabled = true
include = ["app/views/**"]
exclude = ["**/*.haml"]
"#,
        );
        assert!(file.problems.is_empty(), "{:?}", messages(&file));
        assert!(file.root && file.exclude.as_ref().is_some_and(|globs| globs.len() == 2));
        let erb = &file.languages.iter().find(|(name, _)| *name == "erb").unwrap().1;
        assert_eq!((erb.format_enabled, erb.align_comments, erb.diagnostics_enabled), (Some(false), Some(true), Some(true)));
        assert_eq!(erb.disabled_diagnostics.as_deref(), Some(&["Html/ImgAlt".to_string(), "Html/UnclosedElement".to_string()][..]));
        assert_eq!(file.languages.iter().find(|(name, _)| *name == "slim").unwrap().1.extra_engines, Some(vec!["mjml".to_string()]));
        assert_eq!(file.departments, [("Html", false)]);
        let (rule, table) = &file.rules[0];
        assert_eq!(rule.name, "Html/ImgAlt");
        assert!(table.enabled == Some(true) && table.include.is_some() && table.exclude.is_some());
    }

    #[test]
    fn bad_parts_are_dropped_and_the_rest_kept() {
        let file = read(
            r#"
[Html.ImgAltt]
enabled = false

[Html.DuplicateId]
enabeld = false
exclude = ["[z-a].erb"]

[Html.ImgAlt]
enabled = "no"

[Rails.StrictLocals]
enabled = false

[Html]
colour = 1
"#,
        );
        assert_eq!(
            messages(&file),
            [
                "2:7: unknown rule `Html/ImgAltt`",
                "6:1: unknown key `enabeld` in [Html.DuplicateId]; it takes enabled, include, exclude",
                "7:12: can't read the glob `[z-a].erb`",
                "10:11: `enabled` must be true or false",
                "16:1: unknown key `colour` in [Html]; it takes enabled, or a rule table such as [Html.Name]",
            ]
        );
        assert_eq!(
            file.rules.iter().map(|(rule, _)| rule.name).collect::<Vec<_>>(),
            ["Html/DuplicateId", "Html/ImgAlt", "Rails/StrictLocals"]
        );
        assert_eq!(file.rules[0].1.exclude.as_ref().map(Vec::len), Some(0));
    }

    #[test]
    fn invalid_toml_is_one_problem_with_its_position() {
        let file = read("root = true\n[erb\nformat.enabled = false\n");
        assert_eq!(file.problems.len(), 1, "{:?}", messages(&file));
        assert!(file.problems[0].message.starts_with("invalid TOML"), "{:?}", messages(&file));
        assert_eq!(file.problems[0].range.start.line, 1);
        assert!(!file.root && file.languages.is_empty());
    }

    #[test]
    fn a_quoted_rule_name_gets_the_fix() {
        let file = read("[\"Html/ImgAlt\"]\nenabled = false\n");
        assert_eq!(messages(&file), ["1:2: write it as [Html.ImgAlt]"]);
        assert!(file.rules.is_empty());
    }

    #[test]
    fn a_rule_with_options_reads_them_and_checks_them() {
        let file = read("[Layout.EmptyLines]\nenabled = true\nmax = 2\n[Layout.TrailingEmptyLines]\nenabled = false\n");
        assert!(file.problems.is_empty(), "{:?}", messages(&file));
        assert_eq!(file.rules[0].1.counts, [("max", 2)]);
        assert!(file.rules[1].1.counts.is_empty());
        let file = read("[Layout.EmptyLines]\nmax = 0\n");
        assert_eq!(file.rules[0].1.counts, [("max", 0)]);
        for (value, bad) in [("-1", true), ("1.5", true), ("\"2\"", true), ("true", true), ("[1]", true), ("0x10", false)] {
            let file = read(&format!("[Layout.EmptyLines]\nmax = {value}\n"));
            let want = ["2:7: `max` must be a whole number, 0 or more".to_string()];
            assert_eq!(messages(&file), if bad { &want[..] } else { &[] }, "max = {value}");
            assert_eq!(file.rules[0].1.counts.is_empty(), bad, "max = {value}");
        }
    }

    #[test]
    fn a_style_is_one_of_its_words() {
        let file = read(
            "[Layout.EmptyLinesAroundBranches]\nstyle = \"empty_lines\"\n[Layout.EmptyLinesAroundBlockBody]\nstyle = \"no_empty_lines\"\n",
        );
        assert!(file.problems.is_empty(), "{:?}", messages(&file));
        let style = |name: &str| file.rules.iter().find(|(rule, _)| rule.name == name).map(|(_, table)| table.choices.clone());
        assert_eq!(style("Layout/EmptyLinesAroundBranches"), Some(vec![("style", "empty_lines")]));
        assert_eq!(style("Layout/EmptyLinesAroundBlockBody"), Some(vec![("style", "no_empty_lines")]));
        for (value, got) in
            [("\"keep\"", "\"keep\""), ("true", "boolean"), ("1", "integer"), ("\"\"", "\"\""), ("[\"empty_lines\"]", "array")]
        {
            let file = read(&format!("[Layout.EmptyLinesAroundElementBody]\nstyle = {value}\n"));
            let want = [format!("2:9: `style` must be one of no_empty_lines, empty_lines, got {got}")];
            assert_eq!(messages(&file), want, "style = {value}");
            assert!(file.rules[0].1.choices.is_empty(), "style = {value}");
        }
    }

    #[test]
    fn a_list_option_is_a_list_of_element_names() {
        let file = read("[Layout.ElementContentOnOwnLine]\ntags = [\"Div\", \"my-element\"]\n");
        assert!(file.problems.is_empty(), "{:?}", messages(&file));
        assert_eq!(file.rules[0].1.lists, [("tags", vec!["div".to_string(), "my-element".to_string()])]);
        let file = read("[Layout.ElementContentOnOwnLine]\ntags = []\n");
        assert_eq!(file.rules[0].1.lists, [("tags", Vec::<String>::new())]);
        let file = read("[Layout.ElementContentOnOwnLine]\ntags = \"div\"\n");
        assert_eq!(messages(&file), ["2:8: `tags` must be a list of strings"]);
        let file = read("[Layout.ElementContentOnOwnLine]\ntags = [\"div\", \"<p>\", \"\"]\n");
        assert_eq!(messages(&file), ["2:16: `<p>` is not an element name", "2:23: `` is not an element name"]);
        assert_eq!(file.rules[0].1.lists, [("tags", vec!["div".to_string()])]);
    }

    #[test]
    fn unknown_keys_list_what_the_rule_takes() {
        let file = read("[Layout.EmptyLines]\nmaximum = 1\n[Layout.TrailingEmptyLines]\nmax = 1\n");
        assert_eq!(
            messages(&file),
            [
                "2:1: unknown key `maximum` in [Layout.EmptyLines]; it takes enabled, include, exclude, max",
                "4:1: unknown key `max` in [Layout.TrailingEmptyLines]; it takes enabled, include, exclude",
            ]
        );
        let file = read("[Layout.EmptyLinesAroundBranches]\nmax = 1\n");
        assert_eq!(
            messages(&file),
            ["2:1: unknown key `max` in [Layout.EmptyLinesAroundBranches]; it takes enabled, include, exclude, style"]
        );
    }

    #[test]
    fn a_formatting_rule_is_not_a_disabled_diagnostic() {
        let file = read("[erb]\ndisabled_diagnostics = [\"Layout/EmptyLines\", \"Html/ImgAlt\"]\n");
        assert_eq!(messages(&file), ["2:25: `Layout/EmptyLines` formats, it has no diagnostic; switch it in [Layout.EmptyLines]"]);
        assert_eq!(file.languages[0].1.disabled_diagnostics.as_deref(), Some(&["Html/ImgAlt".to_string()][..]));
    }

    #[test]
    fn languages_take_their_own_keys() {
        let file = read(
            "[erb]\nextra_engines = [\"mjml\"]\nformat.enable = true\n[jbuilder]\nformat.enabled = false\n[haml]\ndisabled_diagnostics = [\"nope\", 3]\n[Nope]\n",
        );
        assert_eq!(
            messages(&file),
            [
                "2:1: `extra_engines` belongs in [slim]",
                "3:8: unknown key `format.enable` in [erb]; it takes format.enabled, format.align_comments, diagnostics.enabled, disabled_diagnostics",
                "5:8: `format.enabled` belongs in [erb]",
                "7:24: `disabled_diagnostics` must be a list of strings",
                "8:2: unknown department `Nope`; the departments are Syntax, Html, A11y, Rails, Turbo, Stimulus, Layout, Style",
            ]
        );
        let empty_sub = read("[erb]\n[erb.unknown]\n");
        assert_eq!(
            messages(&empty_sub),
            ["2:6: unknown key `unknown` in [erb]; it takes format.enabled, format.align_comments, diagnostics.enabled, disabled_diagnostics"]
        );
    }

    #[test]
    fn empty_subtables_of_known_keys_are_valid() {
        let file = read("[erb.format]\n[erb]\ndiagnostics = {}\n[haml.diagnostics]\n");
        assert_eq!(messages(&file), Vec::<String>::new());
    }
}
