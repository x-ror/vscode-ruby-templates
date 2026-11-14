//! The indentation width and style and the final newline of a file, from the project's `.editorconfig`. The editor's tab size and
//! `insertSpaces` go on top in the language server (`Settings::with_editor`). Every other property is ignored and listed in the notes.

use crate::format::{IndentStyle, Settings};
use crate::glob::Glob;
use crate::util::slash_path;
use std::path::Path;

/// Settings for one file, and what was read or ignored to get them.
pub struct Resolved {
    pub settings: Settings,
    /// `insert_final_newline = true`: turns `Layout/TrailingEmptyLines` on where it is off by default (ERB).
    pub insert_final_newline: bool,
    pub notes: Vec<String>,
}

pub fn settings_for(path: &Path) -> Resolved {
    let mut notes = Vec::new();
    let defaults = Settings::default();
    let properties = editorconfig_properties(path, &mut notes);
    let settings = Settings {
        indent_size: indent_size(&properties).unwrap_or(defaults.indent_size),
        indent_style: indent_style(&properties).unwrap_or(defaults.indent_style),
        ..defaults
    };
    Resolved { settings, insert_final_newline: property(&properties, "insert_final_newline") == Some("true"), notes }
}

/// The `.editorconfig` properties that apply to `path`: files from the root (or the topmost `root = true`) down, later sections
/// and nearer files winning, as the EditorConfig spec has it.
fn editorconfig_properties(path: &Path, notes: &mut Vec<String>) -> Vec<(String, String)> {
    if path.file_name().is_none() {
        return Vec::new();
    }
    let mut properties = Vec::new();
    for (dir, text) in editorconfig_files(path).iter().rev() {
        let Ok(relative) = path.strip_prefix(dir) else { continue };
        if read_matching_sections(text, &slash_path(relative), &mut properties, notes) {
            notes.push(format!("read {}", dir.join(".editorconfig").display()));
        }
    }
    for (key, value) in properties.iter().filter(|(k, v)| v != "unset" && !is_understood(k, v)) {
        notes.push(format!(".editorconfig: ignored {key} = {value}"));
    }
    properties
}

/// The folders above `path` that hold an `.editorconfig`, with its text: nearest first, up to the first `root = true`.
fn editorconfig_files(path: &Path) -> Vec<(&Path, String)> {
    let mut files = Vec::new();
    for dir in path.ancestors().skip(1) {
        let Ok(text) = std::fs::read_to_string(dir.join(".editorconfig")) else { continue };
        let root = editorconfig_is_root(&text);
        files.push((dir, text));
        if root {
            break;
        }
    }
    files
}

/// Sets `properties` (lowercased) from the sections of one `.editorconfig` that match `relative`; true when any did.
fn read_matching_sections(text: &str, relative: &str, properties: &mut Vec<(String, String)>, notes: &mut Vec<String>) -> bool {
    let mut applies = false;
    let mut read = false;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(['#', ';'])) {
        if let Some(glob) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            applies = Glob::new(glob).map(|glob| glob.matches(relative)).unwrap_or_else(|| {
                notes.push(format!(".editorconfig: ignored section [{glob}]: can't read the glob"));
                false
            });
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        if !applies {
            continue;
        }
        read = true;
        let key = key.trim().to_ascii_lowercase();
        properties.retain(|(k, _)| *k != key);
        properties.push((key, value.trim().to_ascii_lowercase()));
    }
    read
}

fn property<'a>(properties: &'a [(String, String)], key: &str) -> Option<&'a str> {
    properties.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).filter(|v| *v != "unset")
}

fn indent_style(properties: &[(String, String)]) -> Option<IndentStyle> {
    match property(properties, "indent_style") {
        Some("tab") => Some(IndentStyle::Tabs),
        Some("space") => Some(IndentStyle::Spaces),
        _ => None,
    }
}

fn indent_size(properties: &[(String, String)]) -> Option<usize> {
    let get = |key: &str| property(properties, key);
    let number = |key: &str| get(key).and_then(positive_number);
    let tab_width = number("tab_width");
    let tabs = get("indent_style") == Some("tab");
    // `tab_width` stands in for `indent_size` only with tabs, as the spec has it.
    match get("indent_size") {
        Some("tab") => tab_width,
        Some(_) => number("indent_size"),
        None => tab_width.filter(|_| tabs),
    }
}

/// Whether the settings read this property; any other is listed in the notes.
fn is_understood(key: &str, value: &str) -> bool {
    match key {
        "indent_style" => matches!(value, "space" | "tab"),
        "indent_size" => value == "tab" || positive_number(value).is_some(),
        "tab_width" => positive_number(value).is_some(),
        // `false` changes nothing: the final newline is only ever added.
        "insert_final_newline" => value == "true",
        _ => false,
    }
}

fn positive_number(value: &str) -> Option<usize> {
    value.parse().ok().filter(|&n| n > 0)
}

fn editorconfig_is_root(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .take_while(|l| !l.starts_with('['))
        .any(|l| l.split_once('=').is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("root") && v.trim().eq_ignore_ascii_case("true")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
            settings_for(&self.0.join(file))
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
        let outer = "root = true\n[*]\nindent_style = space\nindent_size = 2\nmax_line_length = 80\n";
        let inner = "[*.erb]\nindent_size = 4\nend_of_line = crlf\n[*.slim]\nindent_size = 8\n";
        let project = Project::new("ec", &[(".editorconfig", outer), ("app/views/.editorconfig", inner), ("app/views/a.html.erb", "")]);
        let resolved = project.settings("app/views/a.html.erb");
        assert_eq!(resolved.settings.indent_size, 4);
        for ignored in ["max_line_length = 80", "end_of_line = crlf"] {
            assert!(resolved.notes.iter().any(|n| *n == format!(".editorconfig: ignored {ignored}")), "{ignored}: {:?}", resolved.notes);
        }
    }

    #[test]
    fn editorconfig_stops_at_root_and_handles_tabs_and_unset() {
        let above = "[*]\nindent_size = 8\n";
        let root = "root = true\n[*.{erb,haml}]\nindent_style = tab\nindent_size = tab\ntab_width = 3\n[lib/**.erb]\nindent_style = unset\nindent_size = unset\n";
        let files = [(".editorconfig", above), ("app/.editorconfig", root), ("app/a.html.erb", ""), ("app/lib/x/b.html.erb", "")];
        let project = Project::new("root", &files);
        assert_eq!(project.settings("app/a.html.erb").settings.indent_size, 3, "nothing above the root");
        assert_eq!(project.settings("app/lib/x/b.html.erb").settings.indent_size, 2, "unset falls back to the default");
    }

    #[test]
    fn editorconfig_indent_style_sets_the_style() {
        let text = "[*]\nindent_style = tab\n[*.slim]\nindent_style = space\n[legacy/*]\nindent_style = unset\n";
        let project = Project::new("style", &[(".editorconfig", text), ("a.html.erb", ""), ("b.html.slim", ""), ("legacy/c.html.erb", "")]);
        let style = |file: &str| project.settings(file).settings.indent_style;
        assert_eq!(
            (style("a.html.erb"), style("b.html.slim"), style("legacy/c.html.erb")),
            (IndentStyle::Tabs, IndentStyle::Spaces, IndentStyle::Spaces)
        );
        assert!(project.settings("a.html.erb").notes.iter().all(|n| !n.contains("ignored")), "indent_style is understood");
    }

    #[test]
    fn editorconfig_insert_final_newline_true_is_read() {
        let text = "[*.erb]\ninsert_final_newline = true\n[*.slim]\ninsert_final_newline = false\n";
        let project = Project::new("newline", &[(".editorconfig", text), ("a.html.erb", ""), ("b.html.slim", ""), ("c.html.haml", "")]);
        let newline = |file: &str| project.settings(file).insert_final_newline;
        assert_eq!((newline("a.html.erb"), newline("b.html.slim"), newline("c.html.haml")), (true, false, false));
        let notes = project.settings("a.html.erb").notes;
        assert!(notes.iter().all(|n| !n.contains("ignored")), "{notes:?}");
    }

    #[test]
    fn editorconfig_tab_width_and_unreadable_values() {
        let text = "[*]\nindent_style = space\ntab_width = 8\ntrim_trailing_whitespace = yes\n[[z-a].erb]\nindent_size = 3\n";
        let project = Project::new("tabw", &[(".editorconfig", text), ("a.html.erb", "")]);
        let resolved = project.settings("a.html.erb");
        assert_eq!(resolved.settings.indent_size, 2, "tab_width is the tab's width, not the indent");
        assert!(resolved.notes.iter().any(|n| n == ".editorconfig: ignored trim_trailing_whitespace = yes"), "{:?}", resolved.notes);
        assert!(resolved.notes.iter().any(|n| n.contains("ignored section [[z-a].erb]")), "{:?}", resolved.notes);
    }
}
