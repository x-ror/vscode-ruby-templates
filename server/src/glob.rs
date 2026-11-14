//! EditorConfig-style globs: `*`, `**`, `?`, `[…]`, `[!…]`, `{a,b}`, `{1..9}`, matched against a `/`-separated relative path.

use regex::Regex;

/// A compiled glob. One without `/` matches the file name in any folder (an implicit `**/` prefix); a leading or inner `/`
/// anchors it to the start of the path.
#[derive(Debug)]
pub struct Glob {
    regex: Regex,
    ranges: Vec<(i64, i64)>,
}

impl Glob {
    /// `None` when the pattern can't be read.
    pub fn new(pattern: &str) -> Option<Glob> {
        let pattern = match pattern.strip_prefix('/') {
            Some(anchored) => anchored.to_string(),
            None if pattern.contains('/') => pattern.to_string(),
            None => format!("**/{pattern}"),
        };
        let (regex, ranges) = glob_regex(&pattern)?;
        Some(Glob { regex, ranges })
    }

    pub fn matches(&self, relative: &str) -> bool {
        let Some(found) = self.regex.captures(relative) else { return false };
        // A range in an alternative that wasn't taken has no group and passes.
        let in_range = |(i, &(a, b)): (usize, &(i64, i64))| {
            found.get(i + 1).is_none_or(|m| m.as_str().parse::<i64>().is_ok_and(|n| (a..=b).contains(&n)))
        };
        self.ranges.iter().enumerate().all(in_range)
    }
}

/// The index of `]` closing a character class `[` at `open`, when valid (no `/`).
fn closing_class(chars: &[char], open: usize) -> Option<usize> {
    let mut i = open + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            ']' => return Some(i),
            '/' => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

/// The `}` closing the `{` at `open` (nested braces, classes and `\x` escapes skipped) and whether it has a top-level comma.
fn closing_brace(chars: &[char], open: usize) -> Option<(usize, bool)> {
    let mut depth = 0;
    let mut comma = false;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '[' => i = closing_class(chars, i).unwrap_or(i),
            '{' => depth += 1,
            '}' if depth == 1 => return Some((i, comma)),
            '}' => depth -= 1,
            ',' if depth == 1 => comma = true,
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
    let mut brace_ends: Vec<usize> = Vec::new();
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
            '[' => match closing_class(&chars, i) {
                Some(close) => {
                    re.push_str(&format!("[{}]", class_body(&chars[i + 1..close])));
                    i = close;
                }
                None => re.push_str("\\["),
            },
            '{' => match closing_brace(&chars, i) {
                Some((close, has_comma)) => {
                    let inner: String = chars[i + 1..close].iter().collect();
                    if let Some((a, b)) = inner.split_once("..").and_then(|(a, b)| Some((a.parse::<i64>().ok()?, b.parse::<i64>().ok()?))) {
                        ranges.push((a.min(b), a.max(b)));
                        re.push_str("(-?(?:0|[1-9][0-9]*))");
                        i = close;
                    } else if has_comma {
                        re.push_str("(?:");
                        brace_ends.push(close);
                    } else {
                        re.push_str("\\{");
                    }
                }
                None => re.push_str("\\{"),
            },
            ',' if !brace_ends.is_empty() => re.push('|'),
            '}' if brace_ends.last() == Some(&i) => {
                re.push(')');
                brace_ends.pop();
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

    #[test]
    fn globs() {
        let m = matches;
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
        Glob::new(glob).unwrap().matches(path)
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
            ("{a{b,c}}.x", "{ab}.x", true),
            ("{a{b,c}}.x", "{ac}.x", true),
            ("{a{b,c}}.x", "ab.x", false),
            ("{[a,b]}.x", "{a}.x", true),
            ("{[a,b]}.x", "a.x", false),
            ("{[}a],b}.x", "}.x", true),
            ("{a\\,b}.x", "{a,b}.x", true),
            ("{a\\,b}.x", "a.x", false),
            ("{x*}.c", "{xyz}.c", true),
            ("{x*}.c", "xyz.c", false),
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
