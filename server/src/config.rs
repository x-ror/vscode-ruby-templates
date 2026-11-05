//! The indentation width of a file, from the project's `.editorconfig`. The editor's tab size goes on top in the
//! language server (`Settings::with_editor`). Every other property is ignored and listed in the notes.

use crate::format::Settings;
use crate::util::slash_path;
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
    path.file_name()?;
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
        let relative = slash_path(relative);
        let mut applies = false;
        let mut read = false;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(['#', ';'])) {
            if let Some(glob) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                applies = section_matches(glob, &relative).unwrap_or_else(|| {
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

/// A section glob without `/` matches the file name in any folder (an implicit `**/` prefix); with one, the path from the file's folder.
/// `None` when the glob can't be read.
fn section_matches(glob: &str, relative: &str) -> Option<bool> {
    let glob = match glob.strip_prefix('/') {
        Some(anchored) => anchored.to_string(),
        None if glob.contains('/') => glob.to_string(),
        None => format!("**/{glob}"),
    };
    let (re, ranges) = glob_regex(&glob)?;
    let Some(found) = re.captures(relative) else { return Some(false) };
    // A range in an alternative that wasn't taken has no group and passes.
    let in_range =
        |(i, &(a, b)): (usize, &(i64, i64))| found.get(i + 1).is_none_or(|m| m.as_str().parse::<i64>().is_ok_and(|n| (a..=b).contains(&n)));
    Some(ranges.iter().enumerate().all(in_range))
}

/// The index of the first unescaped `close` at or after `from`, counting `open` pairs when given.
fn find_closing(chars: &[char], from: usize, open: Option<char>, close: char) -> Option<usize> {
    let mut depth = 0;
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            c if Some(c) == open => depth += 1,
            c if c == close && depth <= 1 => return Some(i),
            c if c == close => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// The inside of a `[…]` as a regex class: `!` negates, `\x` is a literal `x`.
fn class_body(inner: &[char]) -> String {
    let (negated, inner) = inner.strip_prefix(&['!']).map_or((false, inner), |rest| (true, rest));
    let mut body = String::from(if negated { "^" } else { "" });
    let mut chars = inner.iter();
    while let Some(&c) = chars.next() {
        match (c, chars.as_slice().first()) {
            ('\\', Some(&next)) => {
                body.push_str(&regex::escape(&next.to_string()));
                chars.next();
            }
            ('[' | '\\' | '&' | '~' | '^', _) => {
                body.push('\\');
                body.push(c);
            }
            _ => body.push(c),
        }
    }
    body
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
            '*' if i == 0 && chars[1..].starts_with(&['*', '/']) => {
                re.push_str("(?:.*/)?");
                i += 2;
            }
            '*' if chars.get(i + 1) == Some(&'*') => {
                re.push_str(".*");
                i += 1;
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '/' if chars[i + 1..].starts_with(&['*', '*', '/']) => {
                re.push_str("(?:/|/.*/)");
                i += 3;
            }
            '[' => match find_closing(&chars, i + 1, None, ']').filter(|&close| !chars[i..close].contains(&'/')) {
                Some(close) => {
                    re.push_str(&format!("[{}]", class_body(&chars[i + 1..close])));
                    i = close;
                }
                None => re.push_str("\\["),
            },
            '{' => match find_closing(&chars, i, Some('{'), '}') {
                Some(close) => {
                    let inner: String = chars[i + 1..close].iter().collect();
                    if let Some((a, b)) = inner.split_once("..").and_then(|(a, b)| Some((a.parse::<i64>().ok()?, b.parse::<i64>().ok()?))) {
                        ranges.push((a.min(b), a.max(b)));
                        re.push_str("(-?(?:0|[1-9][0-9]*))");
                        i = close;
                    } else if inner.contains(',') {
                        re.push_str("(?:");
                        braces += 1;
                    } else {
                        re.push_str(&regex::escape(&format!("{{{inner}}}")));
                        i = close;
                    }
                }
                None => re.push_str("\\{"),
            },
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
        let text = "[*]\nindent_style = space\ntab_width = 8\ntrim_trailing_whitespace = yes\n[[z-a].erb]\nindent_size = 3\n";
        let project = Project::new("tabw", &[(".editorconfig", text), ("a.html.erb", "")]);
        let resolved = project.settings("a.html.erb");
        assert_eq!(resolved.settings.indent_size, 2, "tab_width is the tab's width, not the indent");
        assert!(resolved.notes.iter().any(|n| n == ".editorconfig: ignored trim_trailing_whitespace = yes"), "{:?}", resolved.notes);
        assert!(resolved.notes.iter().any(|n| n.contains("ignored section [[z-a].erb]")), "{:?}", resolved.notes);
    }

    #[test]
    fn globs() {
        let m = |glob, path: &str| section_matches(glob, path).unwrap();
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

    fn matches(glob: &str, path: &str) -> bool {
        section_matches(glob, path).unwrap()
    }

    fn assert_cases(cases: &[(&str, &str, bool)]) {
        for &(glob, path, expected) in cases {
            assert_eq!(matches(glob, path), expected, "[{glob}] against {path}");
        }
    }

    // editorconfig-core-test glob: star, question, star_star
    #[test]
    fn core_star_question_star_star() {
        assert_cases(&[
            ("a*e.c", "ace.c", true),
            ("a*e.c", "abcde.c", true),
            ("a*e.c", "a e.c", true),
            ("Bar/*", "Bar/foo.txt", true),
            ("Bar/*", "Bar/foo/baz.txt", false),
            ("som?.c", "some.c", true),
            ("som?.c", "som.c", false),
            ("som?.c", "som/.c", false),
            ("a**z.c", "amnz.c", true),
            ("a**z.c", "am/nz.c", true),
            ("a**z.c", "a/z.c", true),
            ("b/**z.c", "b/z.c", true),
            ("b/**z.c", "b/mn/z.c", true),
            ("c**/z.c", "c/z.c", true),
            ("c**/z.c", "cmn/z.c", true),
            ("d/**/z.c", "d/z.c", true),
            ("d/**/z.c", "d/mn/z.c", true),
            ("d/**/z.c", "dmn/z.c", false),
            ("d/**/z.c", "d/mnz.c", false),
            ("b/**z.c", "bm/nz.c", false),
            ("c**/z.c", "cmnz.c", false),
            ("c**/z.c", "c/mnz.c", false),
            ("app/**/*.erb", "app/x.erb", true),
            ("app/**/*.erb", "app/a/b/x.erb", true),
            ("**/z.c", "z.c", true),
            ("**/z.c", "a/z.c", true),
            ("**/z.c", "az.c", false),
            ("d/**/z.c", "d/e/z.c", true),
            ("d/**/z.c", "d/e/f/z.c", true),
        ]);
    }

    // editorconfig-core-test glob: brackets (incl. unmatched and slash inside)
    #[test]
    fn core_brackets() {
        assert_cases(&[
            ("[ab].a", "a.a", true),
            ("[ab].a", "ab.a", false),
            ("[!ab].b", "c.b", true),
            ("[!ab].b", "a.b", false),
            ("[a-c].c", "b.c", true),
            ("[a-c].c", "d.c", false),
            ("ab[e/]cd.i", "ab[e/]cd.i", true),
            ("ab[e/]cd.i", "abecd.i", false),
            ("ab[/c", "ab[/c", true),
            ("ab[c.l", "ab[c.l", true),
            ("ab[c.l", "abc.l", false),
            ("[\\]ab].g", "].g", true),
            ("[\\]ab].g", "a.g", true),
            ("[ab]].g", "b].g", true),
            ("[!\\]ab].g", "c.g", true),
            ("[!\\]ab].g", "].g", false),
            ("[!ab]].g", "c].g", true),
            ("[-ab].f", "-.f", true),
            ("[abd-g].e", "e.e", true),
            ("[!d-g].d", "f.d", false),
            ("ab[/c", "ab/c", false),
        ]);
    }

    // editorconfig-core-test glob: braces, numeric_range, and the spec's literal cases
    #[test]
    fn core_braces_and_numeric_ranges() {
        assert_cases(&[
            ("*.{js,py}", "x/a.py", true),
            ("*.{js,py}", "a.rb", false),
            ("{single}.b", "{single}.b", true),
            ("{single}.b", "single.b", false),
            ("{}.c", "{}.c", true),
            ("a{b.d", "a{b.d", true),
            ("a{b,c.d", "a{b,c.d", true),
            ("a{b,c.d", "ab", false),
            ("a}b.e", "a}b.e", true),
            ("a,b.f", "a,b.f", true),
            ("{a,{b,c}}.g", "a.g", true),
            ("{a,{b,c}}.g", "c.g", true),
            ("{a,{b,c}}.g", "d.g", false),
            ("{a,b}{1..3}.h", "b2.h", true),
            ("{a,b}{1..3}.h", "b4.h", false),
            ("{a,{1..3}}.g", "a.g", true),
            ("{a,{1..3}}.g", "2.g", true),
            ("{a,{1..3}}.g", "4.g", false),
            ("a{b,c,}.d", "a.d", true),
            ("a{b,c,}.d", "ac.d", true),
            ("a{b,c,}.d", "a,.d", false),
            ("a{,b,,c,}.e", "ab.e", true),
            ("{word,{also},this}.g", "{also}.g", true),
            ("{word,{also},this}.g", "word,this}.g", false),
            ("{{a,b},c}.k", "b.k", true),
            ("{{a,b},c}.k", "{a,b}.k", false),
            ("{a,{b,c}}.l", "{b,c}.l", false),
            ("{},b}.h", "{},b}.h", true),
            ("{{,b,c{d}.i", "{{,b,c{d}.i", true),
            ("{{,b,c{d}.i", "b.i", false),
            ("{{,b,c{d}.i", "c{d.i", false),
            ("{a\\,b,cd}.txt", "a,b.txt", true),
            ("{a\\,b,cd}.txt", "a.txt", false),
            ("{e,\\},f}.txt", "}.txt", true),
            ("{e,\\},f}.txt", "f.txt", true),
            ("{g,\\\\,i}.txt", "\\.txt", true),
            ("{g,\\\\,i}.txt", "i.txt", true),
            ("{some,a{*c,b}[ef]}.j", "abe.j", true),
            ("{some,a{*c,b}[ef]}.j", "abcf.j", true),
            ("{some,a{*c,b}[ef]}.j", "abg.j", false),
            ("{some,a{*c,b}[ef]}.j", "ae.j", false),
            ("{a,b\\}", "{a,b}", true),
            ("{a,b\\}", "a", false),
            ("{a\\}", "{a}", true),
            ("{aardvark..antelope}", "{aardvark..antelope}", true),
            ("{aardvark..antelope}", "agreement", false),
            ("{3..120}", "3", true),
            ("{3..120}", "60", true),
            ("{3..120}", "121", false),
            ("{3..120}", "2", false),
            ("{3..120}", "5a", false),
            ("{-3..3}", "-2", true),
            ("{-3..3}", "0", true),
            ("{-3..3}", "-4", false),
            ("{-3..3}", "4", false),
            ("{3..-3}", "-3", true),
            ("{1..9}", "07", false),
            ("{3..120}", "060", false),
            ("{0..9}", "0", true),
            ("{-3..3}", "-03", false),
            ("{1..9}", "010", false),
            ("{a..c}.x", "a..c.x", false),
        ]);
    }

    // editorconfig-core-test glob: escaped characters, and regex metacharacters in names
    #[test]
    fn core_escapes_and_literals() {
        assert_cases(&[
            ("a\\*b.txt", "a*b.txt", true),
            ("a\\*b.txt", "axb.txt", false),
            ("a\\?b", "a?b", true),
            ("a\\?b", "axb", false),
            ("a\\[b]", "a[b]", true),
            ("a\\{b,c}", "a{b,c}", true),
            ("a\\{b,c}", "ab", false),
            ("a+b(1).rb", "a+b(1).rb", true),
            ("a+b(1).rb", "aab(1).rb", false),
            ("a|b", "a", false),
            ("a|b", "a|b", true),
        ]);
    }

    // editorconfig-core-test filetree: sections without `/` match the basename anywhere, with `/` are anchored to the file's folder
    #[test]
    fn core_path_separator_anchoring() {
        assert_cases(&[
            ("*.py", "a/b/c.py", true),
            ("c.py", "a/b/c.py", true),
            ("/c.py", "c.py", true),
            ("/c.py", "a/c.py", false),
            ("/a/c.py", "a/c.py", true),
            ("/a/c.py", "x/a/c.py", false),
            ("a/c.py", "a/c.py", true),
            ("a/c.py", "x/a/c.py", false),
            ("a/*.py", "a/b/c.py", false),
            ("a/**.py", "a/b/c.py", true),
            ("**.py", "a/b/c.py", true),
            ("**/c.py", "a/b/c.py", true),
            ("*", "a/b/c.py", true),
            ("/*", "a/b.py", false),
        ]);
    }
}
