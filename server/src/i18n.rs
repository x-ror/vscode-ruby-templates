//! Rails i18n keys in `t("…")` calls → `config/locales/**/*.yml`: definition, hover, completion.

use crate::erb::truncate;
use crate::partials::{app_root, views_root};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

// `t "a.b"`, `t(".x")`, `translate`, `I18n.t`; not `obj.t(…)`. `#` keeps interpolated keys out.
const CALL: &str = r#"(?:^|[^\w.@$])(?:I18n\.)?(?:t|translate)\s*\(?\s*["']"#;
static KEY_CALL: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"{CALL}([\w.?!-]+)["']"#)).unwrap());
static KEY_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"{CALL}([\w.?!-]*)$"#)).unwrap());
const VALUE_PREVIEW: usize = 80;

/// One key of a locale file. `key` has no locale prefix; `value` is `None` for a nested mapping.
#[derive(Debug, PartialEq)]
pub struct Translation {
    pub locale: String,
    pub key: String,
    pub line: usize,
    pub value: Option<String>,
}

/// Byte range of the key literal (without quotes) under `col`.
pub fn key_at(line: &str, col: usize) -> Option<(usize, usize)> {
    KEY_CALL
        .captures_iter(line)
        .filter_map(|cap| cap.get(1))
        .map(|m| (m.start(), m.end()))
        .find(|&(s, e)| (s.saturating_sub(1)..=e + 1).contains(&col))
}

/// Byte range of the key literal the cursor is typing in (through its end after the cursor) and the text typed so far.
pub fn completion_prefix(line: &str, col: usize) -> Option<(usize, usize, &str)> {
    let m = KEY_OPEN.captures(line.get(..col)?)?.get(1)?;
    let end = col + line[col..].find(|c: char| !(c.is_alphanumeric() || "_.?!-".contains(c))).unwrap_or(line.len() - col);
    Some((m.start(), end, m.as_str()))
}

/// `users/_form.html.erb` → `users.form`, like ActionView's `@virtual_path.gsub(%r{/_?}, ".")`.
pub fn lazy_scope(current: &Path) -> Option<String> {
    let rel = current.strip_prefix(views_root(current)?).ok()?;
    let stem = crate::partials::template_stem(rel.file_name()?.to_str()?);
    let virtual_path = rel.with_file_name(stem).to_str()?.replace('\\', "/");
    Some(format!("/{virtual_path}").replace("/_", "/").replace('/', ".")[1..].to_string())
}

/// `.title` → `users.index.title` in users/index; other keys as written.
pub fn full_key(current: &Path, key: &str) -> Option<String> {
    if key.starts_with('.') {
        Some(format!("{}{key}", lazy_scope(current)?))
    } else {
        Some(key.to_string())
    }
}

/// The Rails app's `config/locales`, which need not exist yet.
pub fn locales_dir(current: &Path) -> Option<PathBuf> {
    Some(app_root(current)?.parent()?.join("config/locales"))
}

pub fn locale_files(dir: &Path) -> Vec<(PathBuf, Vec<Translation>)> {
    WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "yml" || x == "yaml"))
        .filter_map(|e| Some((e.path().to_path_buf(), parse(&std::fs::read_to_string(e.path()).ok()?))))
        .collect()
}

pub fn lookup<'a>(files: &'a [(PathBuf, Vec<Translation>)], key: &'a str) -> impl Iterator<Item = (&'a Path, &'a Translation)> {
    files.iter().flat_map(|(path, keys)| keys.iter().map(move |t| (path.as_path(), t))).filter(move |(_, t)| t.key == key)
}

/// Markdown with each locale's value of `key` and the locales missing it.
pub fn hover(files: &[(PathBuf, Vec<Translation>)], key: &str) -> Option<String> {
    let found: Vec<&Translation> = lookup(files, key).map(|(_, t)| t).collect();
    if found.is_empty() {
        return None;
    }
    let values: Vec<String> = found
        .iter()
        .map(|t| {
            format!(
                "- **{}**: {}",
                t.locale,
                t.value.as_deref().map_or_else(|| "{…}".to_string(), |v| inline_code(&truncate(v, VALUE_PREVIEW)))
            )
        })
        .collect();
    let mut md = format!("`{key}`\n\n{}", values.join("\n"));
    let locales: BTreeSet<&str> = files.iter().flat_map(|(_, keys)| keys.iter().map(|t| t.locale.as_str())).collect();
    let missing: Vec<String> = locales.into_iter().filter(|l| !found.iter().any(|t| t.locale == *l)).map(|l| format!("`{l}`")).collect();
    if !missing.is_empty() {
        md.push_str(&format!("\n\nMissing in {}", missing.join(", ")));
    }
    Some(md)
}

/// `v` as a Markdown code span, so HTML and Markdown in translations show literally.
fn inline_code(v: &str) -> String {
    let longest_run = v.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest_run + 1);
    let pad = if v.starts_with('`') || v.ends_with('`') { " " } else { "" };
    format!("{fence}{pad}{v}{pad}{fence}")
}

/// Leaf keys of every locale starting with `prefix` and a preview value; `.x` prefixes list the view's lazy keys as `.x`.
pub fn completions(files: &[(PathBuf, Vec<Translation>)], current: &Path, prefix: &str) -> Vec<(String, String)> {
    let scope = if prefix.starts_with('.') { lazy_scope(current) } else { Some(String::new()) };
    let Some(scope) = scope else { return vec![] };
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for t in files.iter().flat_map(|(_, keys)| keys) {
        let Some(value) = &t.value else { continue };
        let Some(label) = t.key.strip_prefix(scope.as_str()).filter(|l| l.starts_with(prefix)) else { continue };
        out.entry(label.to_string()).or_insert_with(|| format!("{}: {}", t.locale, truncate(value, VALUE_PREVIEW)));
    }
    out.into_iter().collect()
}

// ponytail: line-based YAML subset (block mappings, scalars, `|`/`>` blocks); flow mappings `{a: 1}` and
// sequences aren't indexed, aliases show as `*name`. Enough for Rails locale files without a YAML crate.
pub fn parse(text: &str) -> Vec<Translation> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out: Vec<Translation> = Vec::new();
    let mut path: Vec<(usize, String)> = Vec::new();
    // Indent of the last key with a value or sequence item: deeper lines continue it (`|`/`>` blocks, wrapped
    // scalars, item mappings), plus the quote of a quoted scalar still open.
    let mut scalar: Option<(usize, Option<char>)> = None;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim_start();
        if line.is_empty() {
            continue;
        }
        let indent = raw.len() - line.len();
        if let Some((i, quote)) = scalar.filter(|&(i, _)| indent > i) {
            // A sequence's parent key is a mapping (no value), so item lines append nothing.
            if let Some(value) = out.last_mut().and_then(|t| t.value.as_mut()) {
                let (more, open) = match quote {
                    Some(q) => unquote(q, line.trim_end()),
                    None => (line.trim_end().to_string(), false),
                };
                if !value.is_empty() {
                    value.push(' ');
                }
                value.push_str(&more);
                scalar = Some((i, quote.filter(|_| open)));
            }
            continue;
        }
        scalar = None;
        if line.starts_with('#') || line.starts_with("---") {
            continue;
        }
        if line.starts_with('-') {
            scalar = Some((indent, None));
            continue;
        }
        let Some((key, rest)) = split_key(line) else { continue };
        while path.last().is_some_and(|(i, _)| *i >= indent) {
            path.pop();
        }
        path.push((indent, key.to_string()));
        let value = scalar_value(rest);
        if let Some((_, open)) = &value {
            scalar = Some((indent, *open));
        }
        let value = value.map(|(v, _)| v);
        // `I18n.t` splits on `.`, so a key containing one can never be looked up.
        if path.len() < 2 || key == "<<" || path.iter().any(|(_, k)| k.contains('.')) {
            continue;
        }
        let key = path[1..].iter().map(|(_, k)| k.as_str()).collect::<Vec<_>>().join(".");
        out.push(Translation { locale: path[0].1.clone(), key, line: n, value });
    }
    out
}

/// `key: rest` / `"key": rest`.
fn split_key(line: &str) -> Option<(&str, &str)> {
    if let Some(q) = line.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let close = line[1..].find(q)? + 1;
        return Some((&line[1..close], line[close + 1..].strip_prefix(':')?));
    }
    let (colon, _) = line.match_indices(':').find(|(i, _)| line[i + 1..].chars().next().is_none_or(char::is_whitespace))?;
    let key = line[..colon].trim_end();
    (!key.is_empty()).then(|| (key, &line[colon + 1..]))
}

/// The scalar after `key:`, without anchor, quotes or comment, and the quote left open when it continues on
/// the next line; `None` when a mapping follows.
fn scalar_value(rest: &str) -> Option<(String, Option<char>)> {
    let mut rest = rest.trim();
    if rest.starts_with('&') {
        rest = rest.split_once(char::is_whitespace).map_or("", |(_, r)| r.trim_start());
    }
    if rest.is_empty() || rest.starts_with('#') {
        return None;
    }
    if rest.starts_with(['|', '>']) {
        return Some((String::new(), None));
    }
    if let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let (value, open) = unquote(q, &rest[1..]);
        return Some((value, open.then_some(q)));
    }
    Some((rest.split(" #").next().unwrap_or(rest).trim_end().to_string(), None))
}

/// The quoted text up to the first unescaped `q` (`''` in single quotes; `\"`, `\\`, `\n` in double),
/// and whether the closing quote is still to come.
fn unquote(q: char, s: &str) -> (String, bool) {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' if q == '\'' && chars.peek() == Some(&'\'') => {
                chars.next();
                out.push('\'');
            }
            '\\' if q == '"' => match chars.next() {
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            },
            c if c == q => return (out, false),
            c => out.push(c),
        }
    }
    (out, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_key_literals() {
        let keys = |line: &str| KEY_CALL.captures_iter(line).map(|c| c[1].to_string()).collect::<Vec<_>>();
        assert_eq!(
            keys(r#"<%= t("a.b") %> <%= t '.title' %> <%= I18n.t("x") %> <%= translate("y", count: 1) %>"#),
            ["a.b", ".title", "x", "y"]
        );
        assert!(keys(r##"<%= t("a.#{b}") %> <%= post.t("x") %> <%= link_to "x" %> <%= at("x") %> <%= t(:sym) %>"##).is_empty());
        let line = r#"<%= t(".title") %>"#;
        assert_eq!(key_at(line, 8), Some((7, 13)));
        assert_eq!(key_at(line, 2), None);
        assert_eq!(completion_prefix(r#"<%= t(".ti"#, 10), Some((7, 10, ".ti")));
        assert_eq!(completion_prefix(r#"<%= t(".title") %>"#, 9), Some((7, 13, ".t")));
        assert_eq!(completion_prefix(r#"<%= I18n.t ""#, 12), Some((12, 12, "")));
        assert_eq!(completion_prefix(r#"<%= link_to "x"#, 14), None);
    }

    #[test]
    fn lazy_scope_from_view_path() {
        let views = Path::new("/r/app/views");
        assert_eq!(lazy_scope(&views.join("users/index.html.erb")).as_deref(), Some("users.index"));
        assert_eq!(lazy_scope(&views.join("users/_form.html+phone.erb")).as_deref(), Some("users.form"));
        assert_eq!(lazy_scope(&views.join("users/_user.v2.html.erb")).as_deref(), Some("users.user.v2"));
        assert_eq!(lazy_scope(&views.join("admin/_x/_card.html.haml")).as_deref(), Some("admin.x.card"));
        assert_eq!(full_key(&views.join("users/index.html.erb"), "a.b").as_deref(), Some("a.b"));
        assert_eq!(full_key(Path::new("/tmp/x.html.erb"), ".title"), None);
    }

    #[test]
    fn parses_locale_yaml() {
        let yml = concat!(
            "# comment\n---\nen:\n",
            "  defaults: &defaults\n    ok: \"OK\" # trailing\n",
            "  users:\n    <<: *defaults\n    index:\n",
            "      title: Hello # comment\n",
            "      \"quoted.key\": 'It''s'\n",
            "      body: |\n        Line one\n\n        # not a comment\n      after: *alias\n",
            "      days:\n        - Sun\n        - Mon\n",
            "      folded: >-\n        a\n        b\n",
            "      url: http://x.io/a#b\n",
            "      c: \"Hello \\\"x\\\"\" # see \"docs\"\n",
            "      list:\n        - name: A\n          desc: B\n      - x\n",
            "      wrapped: \"first\n        second\"\n",
            "      plain: last\n",
        );
        let got: Vec<(String, usize, Option<String>)> = parse(yml).into_iter().map(|t| (t.key, t.line, t.value)).collect();
        let s = |v: &str| Some(v.to_string());
        assert_eq!(
            got,
            vec![
                ("defaults".into(), 3, None),
                ("defaults.ok".into(), 4, s("OK")),
                ("users".into(), 5, None),
                ("users.index".into(), 7, None),
                ("users.index.title".into(), 8, s("Hello")),
                ("users.index.body".into(), 10, s("Line one # not a comment")),
                ("users.index.after".into(), 14, s("*alias")),
                ("users.index.days".into(), 15, None),
                ("users.index.folded".into(), 18, s("a b")),
                ("users.index.url".into(), 21, s("http://x.io/a#b")),
                ("users.index.c".into(), 22, s("Hello \"x\"")),
                ("users.index.list".into(), 23, None),
                ("users.index.wrapped".into(), 27, s("first second")),
                ("users.index.plain".into(), 29, s("last")),
            ]
        );
        assert!(parse(yml).iter().all(|t| t.locale == "en"));
        assert_eq!(
            parse("\u{feff}en:\n  a: 'It''s'\n  b: \"x\\ny\"\n").into_iter().map(|t| (t.locale, t.value)).collect::<Vec<_>>(),
            [("en".to_string(), s("It's")), ("en".to_string(), s("x\ny"))]
        );
    }

    #[test]
    fn resolves_keys_across_locales() {
        let root = std::env::temp_dir().join(format!("rt-i18n-{}", std::process::id()));
        let locales = root.join("config/locales");
        std::fs::create_dir_all(locales.join("models")).unwrap();
        std::fs::create_dir_all(root.join("app/views/users")).unwrap();
        std::fs::write(locales.join("en.yml"), "en:\n  hello: Hello\n  html: \"<b>`x`</b>\"\n  users:\n    form:\n      title: New user\n")
            .unwrap();
        std::fs::write(locales.join("uk.yml"), "uk:\n  hello: Привіт\n").unwrap();
        std::fs::write(locales.join("models/de.yaml"), "de:\n  users:\n    form:\n      title: Neu\n").unwrap();
        let partial = root.join("app/views/users/_form.html.erb");
        let files = locale_files(&locales_dir(&partial).unwrap());

        let defs: Vec<(PathBuf, usize)> = lookup(&files, "hello").map(|(p, t)| (p.to_path_buf(), t.line)).collect();
        assert_eq!(defs, [(locales.join("en.yml"), 1), (locales.join("uk.yml"), 1)]);
        let key = full_key(&partial, ".title").unwrap();
        assert_eq!(lookup(&files, &key).count(), 2);

        assert_eq!(hover(&files, "hello").unwrap(), "`hello`\n\n- **en**: `Hello`\n- **uk**: `Привіт`\n\nMissing in `de`");
        assert!(hover(&files, "nope").is_none());
        assert_eq!(hover(&files, "html").unwrap().lines().nth(2), Some("- **en**: ``<b>`x`</b>``"));
        assert_eq!(inline_code("`a"), "`` `a ``");

        let labels = |prefix| completions(&files, &partial, prefix).into_iter().map(|(l, _)| l).collect::<Vec<_>>();
        assert_eq!(labels(""), ["hello", "html", "users.form.title"]);
        assert_eq!(labels("users."), ["users.form.title"]);
        assert_eq!(labels("."), [".title"]);
        assert_eq!(completions(&files, &partial, ".t")[0].1, "en: New user");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
