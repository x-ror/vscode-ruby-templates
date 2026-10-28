//! The indentation width of a file, from the project's `.editorconfig`. The editor's tab size goes on top in the
//! language server (`Settings::with_editor`). Every other property is ignored and listed in the notes.

use crate::format::Settings;
use regex::Regex;
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
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned())?;
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
        _ => false,
    };
    for (key, value) in properties.iter().filter(|(k, v)| v != "unset" && !readable(k, v)) {
        notes.push(format!(".editorconfig: ignored {key} = {value}"));
    }
    indent_size
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
        let text = "[*]\nindent_style = space\ntab_width = 8\ntrim_trailing_whitespace = yes\n[x[.erb]\nindent_size = 3\n";
        let project = Project::new("tabw", &[(".editorconfig", text), ("a.html.erb", "")]);
        let resolved = project.settings("a.html.erb");
        assert_eq!(resolved.settings.indent_size, 2, "tab_width is the tab's width, not the indent");
        assert!(resolved.notes.iter().any(|n| n == ".editorconfig: ignored trim_trailing_whitespace = yes"), "{:?}", resolved.notes);
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
}
