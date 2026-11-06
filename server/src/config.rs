//! The indentation width of a file, from the project's `.editorconfig`. The editor's tab size goes on top in the
//! language server (`Settings::with_editor`). Every other property is ignored and listed in the notes.

use crate::format::Settings;
use crate::glob::Glob;
use crate::util::slash_path;
use std::path::Path;

/// Settings for one file, and what was read or ignored to get them.
pub struct Resolved {
    pub settings: Settings,
    pub notes: Vec<String>,
}

pub fn settings_for(path: &Path) -> Resolved {
    let mut notes = Vec::new();
    let defaults = Settings::default();
    let indent_size = editorconfig_indent_size(path, &mut notes).unwrap_or(defaults.indent_size);
    Resolved { settings: Settings { indent_size, ..defaults }, notes }
}

/// The indentation width from the `.editorconfig` properties that apply to `path`: files from the root (or the topmost `root = true`) down,
/// later sections and nearer files winning, as the EditorConfig spec has it.
fn editorconfig_indent_size(path: &Path, notes: &mut Vec<String>) -> Option<usize> {
    path.file_name()?;
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
    indent_size(&properties)
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

fn indent_size(properties: &[(String, String)]) -> Option<usize> {
    let get = |key: &str| properties.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).filter(|v| *v != "unset");
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

/// Whether `indent_size` reads this property; any other is listed in the notes.
fn is_understood(key: &str, value: &str) -> bool {
    match key {
        "indent_style" => matches!(value, "space" | "tab"),
        "indent_size" => value == "tab" || positive_number(value).is_some(),
        "tab_width" => positive_number(value).is_some(),
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
    fn editorconfig_tab_width_and_unreadable_values() {
        let text = "[*]\nindent_style = space\ntab_width = 8\ntrim_trailing_whitespace = yes\n[[z-a].erb]\nindent_size = 3\n";
        let project = Project::new("tabw", &[(".editorconfig", text), ("a.html.erb", "")]);
        let resolved = project.settings("a.html.erb");
        assert_eq!(resolved.settings.indent_size, 2, "tab_width is the tab's width, not the indent");
        assert!(resolved.notes.iter().any(|n| n == ".editorconfig: ignored trim_trailing_whitespace = yes"), "{:?}", resolved.notes);
        assert!(resolved.notes.iter().any(|n| n.contains("ignored section [[z-a].erb]")), "{:?}", resolved.notes);
    }
}
