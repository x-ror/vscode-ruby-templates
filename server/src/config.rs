//! Formatter settings from the project's files, lowest precedence first: our defaults, the template language's
//! linter config (`.erb_lint.yml`, `.haml-lint.yml`, `.slim-lint.yml`) for the few linters that mean what one of our
//! settings means, then `.editorconfig`. The editor's options go on top in the language server
//! (`Settings::with_editor`). Everything else in those files is ignored and listed in the notes.

use crate::format::{IndentStyle, LineEnding, Settings};
use crate::lang::TemplateLang;
use regex::Regex;
use std::path::{Path, PathBuf};

/// Settings for one file, and what was read or ignored to get them.
pub struct Resolved {
    pub settings: Settings,
    pub notes: Vec<String>,
}

/// One source's opinion; `None` leaves the setting to the sources below it.
#[derive(Default, Debug, PartialEq)]
struct Overrides {
    indent_style: Option<IndentStyle>,
    indent_size: Option<usize>,
    end_of_line: Option<LineEnding>,
    final_newline: Option<bool>,
    trim_trailing_whitespace: Option<bool>,
    collapse_blank_lines: Option<bool>,
    trim_trailing_blank_lines: Option<bool>,
}

impl Overrides {
    fn apply(&self, s: Settings) -> Settings {
        Settings {
            indent_style: self.indent_style.unwrap_or(s.indent_style),
            indent_size: self.indent_size.unwrap_or(s.indent_size),
            end_of_line: self.end_of_line.or(s.end_of_line),
            final_newline: self.final_newline.or(s.final_newline),
            trim_trailing_whitespace: self.trim_trailing_whitespace.unwrap_or(s.trim_trailing_whitespace),
            collapse_blank_lines: self.collapse_blank_lines.unwrap_or(s.collapse_blank_lines),
            trim_trailing_blank_lines: self.trim_trailing_blank_lines.unwrap_or(s.trim_trailing_blank_lines),
        }
    }
}

pub fn settings_for(path: &Path, lang: TemplateLang) -> Resolved {
    let mut notes = Vec::new();
    let mut settings = Settings::default();
    if let Some((file, text)) = linter_config(path, lang) {
        notes.push(format!("read {}", file.display()));
        settings = linter_overrides(&text, lang, &file, &mut notes).apply(settings);
    }
    let editorconfig = editorconfig_overrides(path, &mut notes);
    Resolved { settings: editorconfig.apply(settings), notes }
}

// ---------------------------------------------------------------- linter configs

/// The nearest linter config of `lang`'s linter, from the file's folder up.
fn linter_config(path: &Path, lang: TemplateLang) -> Option<(PathBuf, String)> {
    let names: &[&str] = match lang {
        TemplateLang::Erb => &[".erb_lint.yml", ".erb-lint.yml"],
        TemplateLang::Haml => &[".haml-lint.yml"],
        TemplateLang::Slim | TemplateLang::Skim => &[".slim-lint.yml"],
        TemplateLang::Ruby => return None,
    };
    path.ancestors().skip(1).find_map(|dir| {
        names.iter().find_map(|name| {
            let file = dir.join(name);
            std::fs::read_to_string(&file).ok().map(|text| (file, text))
        })
    })
}

/// A linter we map, the options of it we read, and the setting it checks.
struct Mapped {
    name: &'static str,
    options: &'static [&'static str],
    set: fn(&mut Overrides, &Yaml),
}

// erb_lint's FinalNewline also flags more than one newline at the end.
const ERB_LINT: &[Mapped] = &[
    Mapped {
        name: "FinalNewline",
        options: &["present"],
        set: |o, l| {
            let present = l.flag("present").unwrap_or(true);
            o.final_newline = Some(present);
            o.trim_trailing_blank_lines = present.then_some(true);
        },
    },
    Mapped { name: "TrailingWhitespace", options: &[], set: |o, _| o.trim_trailing_whitespace = Some(true) },
    Mapped { name: "ExtraNewline", options: &[], set: |o, _| o.collapse_blank_lines = Some(true) },
    Mapped { name: "SpaceIndentation", options: &[], set: |o, _| o.indent_style = Some(IndentStyle::Space) },
];
const HAML_LINT: &[Mapped] = &[
    Mapped { name: "FinalNewline", options: &["present"], set: |o, l| o.final_newline = Some(l.flag("present").unwrap_or(true)) },
    Mapped { name: "TrailingWhitespace", options: &[], set: |o, _| o.trim_trailing_whitespace = Some(true) },
    Mapped { name: "TrailingEmptyLines", options: &[], set: |o, _| o.trim_trailing_blank_lines = Some(true) },
];
const SLIM_LINT: &[Mapped] = &[
    Mapped { name: "TrailingWhitespace", options: &[], set: |o, _| o.trim_trailing_whitespace = Some(true) },
    Mapped { name: "Tab", options: &[], set: |o, _| o.indent_style = Some(IndentStyle::Space) },
];

/// Linters that sound like ours but aren't, and why.
const NOT_OURS: &[(&str, &str)] = &[
    ("SpaceAroundErbTag", "it changes what is inside ERB tags"),
    ("Indentation", "Haml files keep their own indent unit"),
    ("EmptyLines", "Slim compiles every line, blank ones included"),
    ("TrailingBlankLines", "Slim compiles every line, blank ones included"),
];

fn linter_overrides(text: &str, lang: TemplateLang, file: &Path, notes: &mut Vec<String>) -> Overrides {
    let config = Yaml::parse(text);
    let (mapped, defaults_on) = match lang {
        // erb_lint runs only the linters it is told to, unless `EnableDefaultLinters` turns its defaults on.
        TemplateLang::Erb => (ERB_LINT, config.get("EnableDefaultLinters").and_then(Yaml::as_bool).unwrap_or(false)),
        TemplateLang::Haml => (HAML_LINT, true),
        _ => (SLIM_LINT, true),
    };
    let shown = file.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut out = Overrides::default();
    let empty = Yaml::Map(vec![]);
    let linters = config.get("linters").unwrap_or(&empty);
    for m in mapped {
        let entry = linters.get(m.name);
        // A linter whose settings can't be read has no say.
        let on = match entry {
            None => Some(defaults_on),
            Some(Yaml::Scalar(value)) if !value.is_empty() => None,
            Some(l) => l.get("enabled").map_or(Some(defaults_on), Yaml::as_bool),
        };
        let Some(on) = on else {
            notes.push(format!("{shown}: ignored linters.{}: can't read its settings", m.name));
            continue;
        };
        for (key, _) in entry.map_or(&[][..], Yaml::entries).iter().filter(|(k, _)| k != "enabled" && !m.options.contains(&k.as_str())) {
            notes.push(format!("{shown}: ignored linters.{}.{key}", m.name));
        }
        if on {
            (m.set)(&mut out, entry.unwrap_or(&empty));
            notes.push(format!("{shown}: {} applies", m.name));
        }
    }
    for (name, _) in linters.entries() {
        if mapped.iter().any(|m| m.name == name) {
            continue;
        }
        let why = NOT_OURS.iter().find(|(n, _)| *n == name).map_or("not a whitespace rule of ours", |(_, why)| why);
        notes.push(format!("{shown}: ignored linters.{name}: {why}"));
    }
    for (key, _) in config.entries().iter().filter(|(k, _)| k != "linters" && k != "EnableDefaultLinters") {
        notes.push(format!("{shown}: ignored {key}"));
    }
    out
}

/// The part of YAML linter configs use: nested mappings of plain or quoted scalars. Sequences, flow collections,
/// anchors and multi-line scalars are read as opaque values.
#[derive(Debug, PartialEq)]
enum Yaml {
    Scalar(String),
    Map(Vec<(String, Yaml)>),
}

impl Yaml {
    fn parse(text: &str) -> Self {
        let lines: Vec<(usize, &str)> = text
            .lines()
            .map(|l| strip_comment(l).trim_end())
            .filter(|l| !l.trim().is_empty() && *l != "---" && *l != "...")
            .map(|l| (l.len() - l.trim_start().len(), l.trim_start()))
            .collect();
        Self::block(&lines, &mut 0, 0)
    }

    fn block(lines: &[(usize, &str)], i: &mut usize, indent: usize) -> Self {
        let mut entries = Vec::new();
        while let Some(&(at, line)) = lines.get(*i) {
            if at < indent {
                break;
            }
            *i += 1;
            let Some((key, value)) = line.split_once(':').filter(|_| !line.starts_with('-')) else { continue };
            let value = value.trim();
            let child = if let Some(flow) = value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
                Self::flow(flow)
            } else if value.is_empty() {
                match lines.get(*i) {
                    Some(&(deeper, _)) if deeper > at => Self::block(lines, i, deeper),
                    _ => Self::Scalar(String::new()),
                }
            } else {
                Self::Scalar(unquote(value).to_string())
            };
            // Lines nested deeper than this key's value (a sequence under it) belong to it.
            while lines.get(*i).is_some_and(|&(deeper, _)| deeper > at) {
                *i += 1;
            }
            entries.push((unquote(key.trim()).to_string(), child));
        }
        Self::Map(entries)
    }

    /// `{ enabled: false, present: true }`, without nesting.
    fn flow(inner: &str) -> Self {
        let pairs = inner.split(',').filter_map(|pair| pair.split_once(':'));
        Self::Map(pairs.map(|(k, v)| (unquote(k.trim()).to_string(), Self::Scalar(unquote(v.trim()).to_string()))).collect())
    }

    fn get(&self, key: &str) -> Option<&Yaml> {
        self.entries().iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn entries(&self) -> &[(String, Yaml)] {
        match self {
            Self::Map(entries) => entries,
            Self::Scalar(_) => &[],
        }
    }

    /// YAML 1.1 booleans, as Ruby's Psych reads them.
    fn as_bool(&self) -> Option<bool> {
        let Self::Scalar(s) = self else { return None };
        match s.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" => Some(true),
            "false" | "no" | "off" => Some(false),
            _ => None,
        }
    }

    fn flag(&self, key: &str) -> Option<bool> {
        self.get(key)?.as_bool()
    }
}

/// `key: value # comment` without the comment; a `#` inside quotes stays.
fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '#') if i == 0 || line[..i].ends_with([' ', '\t']) => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(s: &str) -> &str {
    for q in ['"', '\''] {
        if let Some(inner) = s.strip_prefix(q).and_then(|s| s.strip_suffix(q)) {
            return inner;
        }
    }
    s
}

// ---------------------------------------------------------------- .editorconfig

/// The `.editorconfig` properties that apply to `path`: files from the root (or the topmost `root = true`) down,
/// later sections and nearer files winning, as the EditorConfig spec has it.
fn editorconfig_overrides(path: &Path, notes: &mut Vec<String>) -> Overrides {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { return Overrides::default() };
    let mut files = Vec::new();
    for dir in path.ancestors().skip(1) {
        let file = dir.join(".editorconfig");
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        let root = editorconfig_is_root(&text);
        files.push((dir.to_path_buf(), file, text));
        if root {
            break;
        }
    }
    let mut properties: Vec<(String, String)> = Vec::new();
    for (dir, file, text) in files.iter().rev() {
        let Ok(relative) = path.strip_prefix(dir) else { continue };
        let relative = relative.to_string_lossy().replace('\\', "/");
        let mut applies = false;
        let mut read = false;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(['#', ';'])) {
            if let Some(glob) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                applies = section_matches(glob, &relative, &name).unwrap_or_else(|| {
                    notes.push(format!(".editorconfig: ignored section [{glob}]: can't read the glob"));
                    false
                });
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            if applies {
                read = true;
                let key = key.trim().to_ascii_lowercase();
                properties.retain(|(k, _)| *k != key);
                properties.push((key, value.trim().to_ascii_lowercase()));
            }
        }
        if read {
            notes.push(format!("read {}", file.display()));
        }
    }
    let get = |key: &str| properties.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).filter(|v| *v != "unset");
    let flag = |key: &str| get(key).and_then(|v| v.parse::<bool>().ok());
    let number = |key: &str| get(key).and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0);
    let tab_width = number("tab_width");
    let tabs = get("indent_style") == Some("tab");
    // `tab_width` stands in for `indent_size` only with tabs, as the spec has it.
    let indent_size = match get("indent_size") {
        Some("tab") => tab_width,
        Some(_) => number("indent_size"),
        None => tab_width.filter(|_| tabs),
    };
    let readable = |key: &str, value: &str| match key {
        "indent_style" => matches!(value, "space" | "tab"),
        "indent_size" => value == "tab" || number(key).is_some(),
        "tab_width" => number(key).is_some(),
        "end_of_line" => matches!(value, "lf" | "crlf"),
        "insert_final_newline" | "trim_trailing_whitespace" => flag(key).is_some(),
        _ => false,
    };
    for (key, value) in properties.iter().filter(|(k, v)| v != "unset" && !readable(k, v)) {
        notes.push(format!(".editorconfig: ignored {key} = {value}"));
    }
    Overrides {
        indent_style: match get("indent_style") {
            Some("space") => Some(IndentStyle::Space),
            Some("tab") => Some(IndentStyle::Tab),
            _ => None,
        },
        indent_size,
        end_of_line: match get("end_of_line") {
            Some("lf") => Some(LineEnding::Lf),
            Some("crlf") => Some(LineEnding::CrLf),
            _ => None,
        },
        final_newline: flag("insert_final_newline"),
        trim_trailing_whitespace: flag("trim_trailing_whitespace"),
        ..Overrides::default()
    }
}

fn editorconfig_is_root(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .take_while(|l| !l.starts_with('['))
        .any(|l| l.split_once('=').is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("root") && v.trim().eq_ignore_ascii_case("true")))
}

/// A section glob without `/` matches the file name in any folder; with one, the path from the file's folder.
/// `None` when the glob can't be read.
fn section_matches(glob: &str, relative: &str, name: &str) -> Option<bool> {
    let (glob, subject) = match glob.strip_prefix('/') {
        Some(anchored) => (anchored, relative),
        None if glob.contains('/') => (glob, relative),
        None => (glob, name),
    };
    let (re, ranges) = glob_regex(glob)?;
    let Some(found) = re.captures(subject) else { return Some(false) };
    let in_range = |(i, &(a, b)): (usize, &(i64, i64))| found[i + 1].parse::<i64>().is_ok_and(|n| (a..=b).contains(&n));
    Some(ranges.iter().enumerate().all(in_range))
}

/// The EditorConfig glob as an anchored regex: `*`, `**`, `?`, `[…]`, `[!…]`, `{a,b}`, `{1..9}`; each numeric
/// range is a capture group, checked against its bounds by the caller.
fn glob_regex(glob: &str) -> Option<(Regex, Vec<(i64, i64)>)> {
    let mut ranges = Vec::new();
    let mut re = String::from("^");
    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    let mut braces = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') => {
                re.push_str(".*");
                i += 1;
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '[' => {
                let close = chars[i..].iter().position(|&c| c == ']')? + i;
                let inner: String = chars[i + 1..close].iter().collect();
                let inner = inner.strip_prefix('!').map_or_else(|| inner.clone(), |rest| format!("^{rest}"));
                re.push_str(&format!("[{}]", inner.replace('\\', "\\\\")));
                i = close;
            }
            '{' => {
                let close = chars[i..].iter().position(|&c| c == '}')? + i;
                let inner: String = chars[i + 1..close].iter().collect();
                if let Some((a, b)) = inner.split_once("..").and_then(|(a, b)| Some((a.parse::<i64>().ok()?, b.parse::<i64>().ok()?))) {
                    ranges.push((a.min(b), a.max(b)));
                    re.push_str("([+-]?[0-9]+)");
                    i = close;
                } else if inner.contains(',') {
                    re.push_str("(?:");
                    braces += 1;
                } else {
                    re.push_str(&regex::escape(&format!("{{{inner}}}")));
                    i = close;
                }
            }
            ',' if braces > 0 => re.push('|'),
            '}' if braces > 0 => {
                re.push(')');
                braces -= 1;
            }
            '\\' if i + 1 < chars.len() => {
                re.push_str(&regex::escape(&chars[i + 1].to_string()));
                i += 1;
            }
            c => re.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    re.push('$');
    Some((Regex::new(&re).ok()?, ranges))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp project with `files`, removed on drop.
    struct Project(PathBuf);

    impl Project {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let root = std::env::temp_dir().join(format!("rt-config-{name}-{}", std::process::id()));
            for (file, text) in files {
                let path = root.join(file);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            Self(root)
        }

        fn settings(&self, file: &str) -> Resolved {
            let path = self.0.join(file);
            settings_for(&path, TemplateLang::from_uri_or_id(&tower_lsp::lsp_types::Url::from_file_path(&path).unwrap(), None).unwrap())
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn defaults_without_config() {
        let project = Project::new("none", &[("app/views/a.html.erb", "")]);
        let resolved = project.settings("app/views/a.html.erb");
        assert_eq!(resolved.settings, Settings::default());
    }

    #[test]
    fn editorconfig_sections_and_precedence() {
        let outer = "root = true\n[*]\nindent_style = space\nindent_size = 2\ninsert_final_newline = true\nmax_line_length = 80\n";
        let inner = "[*.erb]\nindent_size = 4\nend_of_line = crlf\ntrim_trailing_whitespace = false\n[*.slim]\nindent_style = tab\n";
        let project = Project::new("ec", &[(".editorconfig", outer), ("app/views/.editorconfig", inner), ("app/views/a.html.erb", "")]);
        let resolved = project.settings("app/views/a.html.erb");
        let s = resolved.settings;
        assert_eq!((s.indent_style, s.indent_size, s.end_of_line), (IndentStyle::Space, 4, Some(LineEnding::CrLf)));
        assert_eq!((s.final_newline, s.trim_trailing_whitespace), (Some(true), false));
        assert!(resolved.notes.iter().any(|n| n == ".editorconfig: ignored max_line_length = 80"), "{:?}", resolved.notes);
    }

    #[test]
    fn editorconfig_stops_at_root_and_handles_tabs_and_unset() {
        let above = "[*]\nindent_size = 8\nend_of_line = crlf\n";
        let root =
            "root = true\n[*.{erb,haml}]\nindent_style = tab\nindent_size = tab\ntab_width = 3\n[lib/**.erb]\nindent_style = unset\n";
        let files = [(".editorconfig", above), ("app/.editorconfig", root), ("app/a.html.erb", ""), ("app/lib/x/b.html.erb", "")];
        let project = Project::new("root", &files);
        let s = project.settings("app/a.html.erb").settings;
        assert_eq!((s.indent_style, s.indent_size, s.end_of_line), (IndentStyle::Tab, 3, None), "nothing above the root");
        assert_eq!(project.settings("app/lib/x/b.html.erb").settings.indent_style, IndentStyle::Space, "unset falls back to the default");
    }

    #[test]
    fn linters_map_only_what_they_mean() {
        let erb_lint = "---\nEnableDefaultLinters: true\nlinters:\n  ExtraNewline:\n    enabled: true\n  FinalNewline:\n    present: false\n  TrailingWhitespace:\n    enabled: false\n  SpaceAroundErbTag:\n    enabled: true\n  Rubocop:\n    enabled: true\n    rubocop_config:\n      inherit_from:\n        - .rubocop.yml\n";
        let project = Project::new("lint", &[(".erb_lint.yml", erb_lint), ("app/views/a.html.erb", "")]);
        let resolved = project.settings("app/views/a.html.erb");
        let s = resolved.settings;
        assert!(s.collapse_blank_lines);
        assert_eq!(s.final_newline, Some(false));
        assert!(s.trim_trailing_whitespace, "a disabled linter has no opinion; the default stays");
        let ignored: Vec<&String> = resolved.notes.iter().filter(|n| n.contains("ignored")).collect();
        assert_eq!(ignored.len(), 2, "{ignored:?}");
        assert!(ignored[0].contains("SpaceAroundErbTag: it changes what is inside ERB tags"));
    }

    #[test]
    fn erb_lint_without_defaults_runs_only_listed_linters() {
        let project = Project::new("nodefault", &[(".erb-lint.yml", "linters:\n  FinalNewline:\n    enabled: true\n"), ("a.html.erb", "")]);
        let s = project.settings("a.html.erb").settings;
        assert_eq!((s.final_newline, s.collapse_blank_lines), (Some(true), false));
    }

    #[test]
    fn haml_and_slim_linters_are_on_by_default() {
        let project = Project::new(
            "hamlslim",
            &[
                (".haml-lint.yml", "linters:\n  Indentation:\n    width: 4 # spaces\n"),
                (".slim-lint.yml", "linters:\n  TrailingWhitespace:\n    enabled: false\n"),
                ("a.haml", ""),
                ("a.slim", ""),
            ],
        );
        let haml = project.settings("a.haml");
        assert!(haml.settings.trim_trailing_blank_lines);
        assert!(haml.notes.iter().any(|n| n.contains("ignored linters.Indentation: Haml files keep their own indent unit")));
        let slim = project.settings("a.slim").settings;
        assert_eq!(slim.indent_style, IndentStyle::Space);
    }

    #[test]
    fn editorconfig_beats_linters() {
        let project = Project::new(
            "beats",
            &[
                (".erb_lint.yml", "linters:\n  FinalNewline:\n    enabled: true\n    present: true\n"),
                (".editorconfig", "[*]\ninsert_final_newline = false\n"),
                ("a.html.erb", ""),
            ],
        );
        assert_eq!(project.settings("a.html.erb").settings.final_newline, Some(false));
    }

    #[test]
    fn yaml_booleans_flow_maps_and_unreadable_linters() {
        let haml = "linters:\n  TrailingEmptyLines: { enabled: false }\n  FinalNewline:\n    present: False\n  TrailingWhitespace: maybe\n";
        let project = Project::new("bools", &[(".haml-lint.yml", haml), ("a.haml", "")]);
        let resolved = project.settings("a.haml");
        assert!(!resolved.settings.trim_trailing_blank_lines);
        assert_eq!(resolved.settings.final_newline, Some(false));
        assert!(resolved.notes.iter().any(|n| n.contains("ignored linters.TrailingWhitespace: can't read")), "{:?}", resolved.notes);
        let erb = "EnableDefaultLinters: yes\nlinters:\n  TrailingWhitespace:\n    enabled: no\n    exclude:\n      - legacy/**\n";
        let project = Project::new("bools-erb", &[(".erb_lint.yml", erb), ("a.html.erb", "")]);
        let resolved = project.settings("a.html.erb");
        assert!(resolved.settings.collapse_blank_lines && resolved.settings.trim_trailing_blank_lines, "defaults on, FinalNewline too");
        assert!(resolved.notes.iter().any(|n| n.contains("ignored linters.TrailingWhitespace.exclude")), "{:?}", resolved.notes);
    }

    #[test]
    fn editorconfig_tab_width_and_unreadable_values() {
        let text = "[*]\nindent_style = space\ntab_width = 8\ninsert_final_newline = yes\n[x[.erb]\nindent_size = 3\n";
        let project = Project::new("tabw", &[(".editorconfig", text), ("a.html.erb", "")]);
        let resolved = project.settings("a.html.erb");
        assert_eq!(resolved.settings.indent_size, 2, "tab_width is the tab's width, not the indent");
        assert!(resolved.notes.iter().any(|n| n == ".editorconfig: ignored insert_final_newline = yes"), "{:?}", resolved.notes);
        assert!(resolved.notes.iter().any(|n| n.contains("ignored section [x[.erb]")), "{:?}", resolved.notes);
    }

    #[test]
    fn globs() {
        let m = |glob, path: &str| section_matches(glob, path, path.rsplit('/').next().unwrap()).unwrap();
        assert!(m("*.erb", "app/views/a.html.erb"));
        assert!(m("*.{erb,haml}", "x/a.haml"));
        assert!(!m("*.{erb,haml}", "x/a.slim"));
        assert!(m("app/**.erb", "app/views/a.erb"));
        assert!(!m("/a.erb", "x/a.erb"));
        assert!(m("[!_]*.erb", "show.erb"));
        assert!(!m("[!_]*.erb", "_card.erb"));
        assert!(m("file{1..3}.erb", "file2.erb"));
        assert!(!m("file{1..3}.erb", "file4.erb"));
        assert!(m("file{0..99999999}.erb", "file12345.erb"), "a range is checked, not spelled out");
        assert!(!m("*.erb", "a.erb.tt"));
    }

    #[test]
    fn yaml_subset() {
        let y = Yaml::parse("a: 1\nb:\n  c: \"x # y\" # note\n  d:\n    - 1\n    - 2\n  e: 'z'\nf: [1, 2]\n");
        assert_eq!(y.get("a"), Some(&Yaml::Scalar("1".into())));
        assert_eq!(y.get("b").and_then(|b| b.get("c")), Some(&Yaml::Scalar("x # y".into())));
        assert_eq!(y.get("b").and_then(|b| b.get("e")), Some(&Yaml::Scalar("z".into())));
        assert_eq!(y.get("f"), Some(&Yaml::Scalar("[1, 2]".into())));
    }
}
