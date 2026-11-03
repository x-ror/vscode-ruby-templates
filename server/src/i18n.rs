//! Rails i18n keys in `t("…")` calls → `config/locales/**/*.yml`: definition, hover, completion.

use crate::erb::truncate;
use crate::project::Project;
use crate::rails_paths::{app_root, template_stem, views_root};
use crate::util::inline_code;
use regex::Regex;
use std::collections::{BTreeSet, HashMap, HashSet};
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

/// Every key literal on a line: (call start, key start, key end).
pub fn key_calls(line: &str) -> Vec<(usize, usize, usize)> {
    KEY_CALL.captures_iter(line).filter_map(|cap| Some((cap.get(0)?.start(), cap.get(1)?.start(), cap.get(1)?.end()))).collect()
}

/// Byte range of the key literal the cursor is typing in (through its end after the cursor) and the text typed so far.
pub fn completion_prefix(line: &str, col: usize) -> Option<(usize, usize, &str)> {
    let m = KEY_OPEN.captures(line.get(..col)?)?.get(1)?;
    let end = col + line[col..].find(|c: char| !(c.is_alphanumeric() || "_.?!-".contains(c))).unwrap_or(line.len() - col);
    Some((m.start(), end, m.as_str()))
}

/// `users/_form.html.erb` → `users.form`, like ActionView's `@virtual_path.gsub(%r{/_?}, ".")`.
pub fn lazy_scope(project: &Project, current: &Path) -> Option<String> {
    let rel = current.strip_prefix(views_root(current)?).ok()?;
    let stem = template_stem(|f| project.is_format(f), rel.file_name()?.to_str()?);
    let virtual_path = rel.with_file_name(stem).to_str()?.replace('\\', "/");
    Some(format!("/{virtual_path}").replace("/_", "/").replace('/', ".")[1..].to_string())
}

/// `.title` → `users.index.title` in users/index; other keys as written.
pub fn full_key(project: &Project, current: &Path, key: &str) -> Option<String> {
    if key.starts_with('.') {
        Some(format!("{}{key}", lazy_scope(project, current)?))
    } else {
        Some(key.to_string())
    }
}

/// The Rails app's `config/locales`, which need not exist yet.
pub fn locales_dir(current: &Path) -> Option<PathBuf> {
    Some(app_root(current)?.parent()?.join("config/locales"))
}

/// An app's translations sorted by key, for lookups by key and prefix; equal keys keep the files' order.
#[derive(Default)]
pub struct Locales {
    files: Vec<PathBuf>,
    /// (index in `files`, translation).
    by_key: Vec<(usize, Translation)>,
    locales: BTreeSet<String>,
}

impl Locales {
    /// The translations whose key starts with `prefix`, in key order.
    fn starting_with<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = (&'a Path, &'a Translation)> {
        let first = self.by_key.partition_point(|(_, t)| t.key.as_str() < prefix);
        self.by_key[first..].iter().take_while(move |(_, t)| t.key.starts_with(prefix)).map(|(file, t)| (self.files[*file].as_path(), t))
    }
}

pub fn locale_files(dir: &Path) -> Locales {
    let parsed: Vec<(PathBuf, Vec<Translation>)> = WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "yml" || x == "yaml"))
        .filter_map(|e| Some((e.path().to_path_buf(), parse(&std::fs::read_to_string(e.path()).ok()?))))
        .collect();
    let mut files = Vec::with_capacity(parsed.len());
    let mut by_key = Vec::new();
    for (index, (path, translations)) in parsed.into_iter().enumerate() {
        files.push(path);
        by_key.extend(translations.into_iter().map(|t| (index, t)));
    }
    // Stable: the first file's translation of a key stays first.
    by_key.sort_by(|(_, a), (_, b)| a.key.cmp(&b.key));
    let locales = by_key.iter().map(|(_, t)| t.locale.clone()).collect();
    Locales { files, by_key, locales }
}

pub fn lookup<'a>(files: &'a Locales, key: &'a str) -> impl Iterator<Item = (&'a Path, &'a Translation)> {
    files.starting_with(key).filter(move |(_, t)| t.key == key)
}

/// Markdown with each locale's value of `key` and the locales missing it.
pub fn hover(files: &Locales, key: &str) -> Option<String> {
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
    let missing: Vec<String> = files.locales.iter().filter(|l| !found.iter().any(|t| &t.locale == *l)).map(|l| format!("`{l}`")).collect();
    if !missing.is_empty() {
        md.push_str(&format!("\n\nMissing in {}", missing.join(", ")));
    }
    Some(md)
}

/// Leaf keys of every locale starting with `prefix` and a preview value; `.x` prefixes list the view's lazy keys as `.x`.
pub fn completions(project: &Project, files: &Locales, current: &Path, prefix: &str) -> Vec<(String, String)> {
    let scope = if prefix.starts_with('.') { lazy_scope(project, current) } else { Some(String::new()) };
    let Some(scope) = scope else { return vec![] };
    let wanted = format!("{scope}{prefix}");
    let mut out: Vec<(String, String)> = Vec::new();
    for (_, t) in files.starting_with(&wanted) {
        let Some(value) = &t.value else { continue };
        // Keys come sorted, so a repeat of the last label is the same key from a later file.
        let label = &t.key[scope.len()..];
        if out.last().is_none_or(|(last, _)| last != label) {
            out.push((label.to_string(), format!("{}: {}", t.locale, truncate(value, VALUE_PREVIEW))));
        }
    }
    out
}

/// A flow collection or scalar written on one line: `{ a: 1, b: [x, y] }`.
enum Flow {
    Scalar(String),
    Seq(Vec<Flow>),
    Map(Vec<(String, Flow)>),
}

impl Flow {
    fn text(&self) -> String {
        match self {
            Flow::Scalar(text) => text.clone(),
            Flow::Seq(items) => format!("[{}]", items.iter().map(Flow::text).collect::<Vec<_>>().join(", ")),
            Flow::Map(_) => "{…}".to_string(),
        }
    }
}

/// Where an anchored key (`&name`) lives: its locale, its key (empty for a top-level key) and the index in the output
/// where its entries start; they follow each other up to the next key outside it.
type Anchors = HashMap<String, (String, String, usize)>;

// Line-based YAML subset: block mappings, scalars, `|`/`>` blocks, block sequences of scalars, flow collections on one
// line, anchors, aliases and `<<` merges. A list is one key whose value is `[a, b]`, as `t` returns the whole array.
pub fn parse(text: &str) -> Vec<Translation> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out: Vec<Translation> = Vec::new();
    let mut path: Vec<(usize, String)> = Vec::new();
    let mut anchors = Anchors::new();
    // Entries a `<<` merge added: a key the mapping sets itself wins over them.
    let mut merged: HashSet<usize> = HashSet::new();
    // The valueless key a `- item` line right below it would belong to.
    let mut list_key: Option<usize> = None;
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
                let (more, closed) = match quote {
                    Some(q) => unquote(q, line.trim_end()),
                    None => (line.trim_end().to_string(), Some(0)),
                };
                if !value.is_empty() {
                    value.push(' ');
                }
                value.push_str(&more);
                scalar = Some((i, quote.filter(|_| closed.is_none())));
            }
            continue;
        }
        scalar = None;
        if line.starts_with('#') || line.starts_with("---") {
            continue;
        }
        if let Some(item) = line.strip_prefix('-') {
            let listed = list_key.is_some_and(|at| push_list_item(&mut out[at], item.trim()));
            // A list with a non-scalar item is not indexed.
            if !listed {
                if let Some(at) = list_key.take() {
                    out[at].value = None;
                }
            }
            scalar = (!listed).then_some((indent, None));
            continue;
        }
        list_key = None;
        let Some((key, rest)) = split_key(line) else { continue };
        while path.last().is_some_and(|(i, _)| *i >= indent) {
            path.pop();
        }
        path.push((indent, key.to_string()));
        let (anchor, rest) = split_anchor(rest);
        let flow = flow_value(rest);
        let value = if flow.is_some() { None } else { scalar_value(rest) };
        if let Some((_, open)) = &value {
            scalar = Some((indent, *open));
        }
        // `I18n.t` splits on `.`, so a key containing one can never be looked up.
        if path.iter().any(|(_, k)| k.contains('.')) {
            continue;
        }
        let locale = path[0].1.clone();
        if path.len() < 2 {
            if let Some(name) = anchor {
                anchors.insert(name.to_string(), (locale, String::new(), out.len()));
            }
            continue;
        }
        if key == "<<" {
            let target = joined(&path[1..path.len() - 1]);
            for name in rest.split(|c: char| c.is_whitespace() || "[],".contains(c)).filter_map(|word| word.strip_prefix('*')) {
                for (suffix, line, value) in aliased(&out, &anchors, name, &merged) {
                    if let Some(suffix) = suffix.strip_prefix('.') {
                        merged.insert(out.len());
                        let key = if target.is_empty() { suffix.to_string() } else { format!("{target}.{suffix}") };
                        out.push(Translation { locale: locale.clone(), key, line, value });
                    }
                }
            }
            continue;
        }
        let key = joined(&path[1..]);
        if let Some(name) = anchor {
            anchors.insert(name.to_string(), (locale.clone(), key.clone(), out.len()));
        }
        let aliases = rest
            .strip_prefix('*')
            .map(|name| aliased(&out, &anchors, name.split_whitespace().next().unwrap_or_default(), &merged))
            .filter(|found| !found.is_empty());
        if let Some(found) = aliases {
            for (suffix, line, value) in found {
                let line = if suffix.is_empty() { n } else { line };
                out.push(Translation { locale: locale.clone(), key: format!("{key}{suffix}"), line, value });
            }
        } else if let Some(flow) = flow {
            push_flow(&mut out, &locale, key, n, flow);
        } else {
            let value = value.map(|(v, _)| v);
            list_key = value.is_none().then_some(out.len());
            out.push(Translation { locale, key, line: n, value });
        }
    }
    let explicit: HashSet<(&str, &str)> =
        out.iter().enumerate().filter(|(i, _)| !merged.contains(i)).map(|(_, t)| (t.locale.as_str(), t.key.as_str())).collect();
    let shadowed: HashSet<usize> =
        merged.iter().copied().filter(|&i| explicit.contains(&(out[i].locale.as_str(), out[i].key.as_str()))).collect();
    out.into_iter().enumerate().filter(|(i, _)| !shadowed.contains(i)).map(|(_, t)| t).collect()
}

fn joined(path: &[(usize, String)]) -> String {
    path.iter().map(|(_, k)| k.as_str()).collect::<Vec<_>>().join(".")
}

/// The entries at and below the anchored key `name` as (key suffix, line, value); none when `name` is unknown.
/// An entry merged in with `<<` is left out where the range has an entry of its own under the same suffix.
fn aliased(out: &[Translation], anchors: &Anchors, name: &str, merged: &HashSet<usize>) -> Vec<(String, usize, Option<String>)> {
    let Some((locale, key, start)) = anchors.get(name) else { return vec![] };
    let range: Vec<(usize, String, &Translation)> = out[*start..]
        .iter()
        .enumerate()
        .take_while(|(_, t)| &t.locale == locale)
        .map_while(|(i, t)| {
            if key.is_empty() {
                return Some((start + i, format!(".{}", t.key), t));
            }
            let suffix = t.key.strip_prefix(key.as_str())?;
            (suffix.is_empty() || suffix.starts_with('.')).then(|| (start + i, suffix.to_string(), t))
        })
        .collect();
    let own: HashSet<&str> = range.iter().filter(|(i, ..)| !merged.contains(i)).map(|(_, suffix, _)| suffix.as_str()).collect();
    range
        .iter()
        .filter(|(i, suffix, _)| !merged.contains(i) || !own.contains(suffix.as_str()))
        .map(|(_, suffix, t)| (suffix.clone(), t.line, t.value.clone()))
        .collect()
}

/// Adds a flow node under `key`; a mapping is a valueless key with its entries below.
fn push_flow(out: &mut Vec<Translation>, locale: &str, key: String, line: usize, flow: Flow) {
    let Flow::Map(entries) = flow else {
        out.push(Translation { locale: locale.to_string(), key, line, value: Some(flow.text()) });
        return;
    };
    for (name, node) in entries.into_iter().filter(|(name, _)| !name.contains('.')) {
        push_flow(out, locale, format!("{key}.{name}"), line, node);
    }
    out.push(Translation { locale: locale.to_string(), key, line, value: None });
}

/// Appends the scalar `item` of a `- item` line to the list `key` holds; false when the item isn't a scalar.
fn push_list_item(key: &mut Translation, item: &str) -> bool {
    if item.is_empty() || item.starts_with(['#', '|', '>']) || split_key(item).is_some() {
        return false;
    }
    let Some((item, _)) = scalar_value(item) else { return false };
    let item = if item.is_empty() { "\"\"" } else { item.as_str() };
    match &mut key.value {
        Some(list) => {
            list.pop();
            list.push_str(&format!(", {item}]"));
        }
        None => key.value = Some(format!("[{item}]")),
    }
    true
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

/// The anchor name (`&name`) that starts `rest`, and what follows it.
fn split_anchor(rest: &str) -> (Option<&str>, &str) {
    let rest = rest.trim();
    let Some(after) = rest.strip_prefix('&') else { return (None, rest) };
    let (name, tail) = after.split_once(char::is_whitespace).unwrap_or((after, ""));
    (Some(name), tail.trim_start())
}

/// The scalar after `key:`, without quotes or comment, and the quote left open when it continues on
/// the next line; `None` when a mapping follows.
fn scalar_value(rest: &str) -> Option<(String, Option<char>)> {
    let rest = split_anchor(rest).1;
    if rest.is_empty() || rest.starts_with('#') {
        return None;
    }
    if rest.starts_with(['|', '>']) {
        return Some((String::new(), None));
    }
    if let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let (value, closed) = unquote(q, &rest[1..]);
        return Some((value, closed.is_none().then_some(q)));
    }
    Some((rest.split(" #").next().unwrap_or(rest).trim_end().to_string(), None))
}

/// The `{…}` or `[…]` that is all of `rest` but a comment.
fn flow_value(rest: &str) -> Option<Flow> {
    if !rest.starts_with(['{', '[']) {
        return None;
    }
    let (flow, after) = flow_node(rest)?;
    let after = after.trim();
    (after.is_empty() || after.starts_with('#')).then_some(flow)
}

/// The flow node that starts `s` and the text after it.
fn flow_node(s: &str) -> Option<(Flow, &str)> {
    let s = s.trim_start();
    if let Some(mut rest) = s.strip_prefix('[') {
        let mut items = Vec::new();
        loop {
            rest = rest.trim_start();
            if let Some(after) = rest.strip_prefix(']') {
                return Some((Flow::Seq(items), after));
            }
            let (item, after) = flow_node(rest)?;
            items.push(item);
            rest = after.trim_start();
            rest = rest.strip_prefix(',').unwrap_or(rest);
        }
    }
    if let Some(mut rest) = s.strip_prefix('{') {
        let mut entries = Vec::new();
        loop {
            rest = rest.trim_start();
            if let Some(after) = rest.strip_prefix('}') {
                return Some((Flow::Map(entries), after));
            }
            let (key, after) = flow_key(rest)?;
            let after = after.trim_start();
            let (value, after) = if after.starts_with([',', '}']) { (Flow::Scalar(String::new()), after) } else { flow_node(after)? };
            entries.push((key, value));
            rest = after.trim_start();
            rest = rest.strip_prefix(',').unwrap_or(rest);
        }
    }
    if let Some(q) = s.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let (text, closed) = unquote(q, &s[1..]);
        return Some((Flow::Scalar(text), &s[1 + closed?..]));
    }
    let end = s.find([',', ']', '}']).unwrap_or(s.len());
    let text = s[..end].trim_end();
    (!text.is_empty()).then(|| (Flow::Scalar(text.to_string()), &s[end..]))
}

/// The key of a flow mapping entry, quoted or plain, and the text after its colon.
fn flow_key(s: &str) -> Option<(String, &str)> {
    if let Some(q) = s.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let (key, closed) = unquote(q, &s[1..]);
        return Some((key, s[1 + closed?..].trim_start().strip_prefix(':')?));
    }
    let (colon, _) = s.match_indices(':').find(|(i, _)| s[i + 1..].chars().next().is_none_or(|c| c.is_whitespace() || c == ','))?;
    Some((s[..colon].trim_end().to_string(), &s[colon + 1..]))
}

/// The quoted text up to the first unescaped `q` (`''` in single quotes; `\"`, `\\`, `\n` in double), and the
/// bytes through the closing quote, `None` while the closing quote is still to come.
fn unquote(q: char, s: &str) -> (String, Option<usize>) {
    let mut out = String::new();
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\'' if q == '\'' && chars.peek().is_some_and(|&(_, next)| next == '\'') => {
                chars.next();
                out.push('\'');
            }
            '\\' if q == '"' => match chars.next() {
                Some((_, 'n')) => out.push('\n'),
                Some((_, other)) => out.push(other),
                None => {}
            },
            c if c == q => return (out, Some(i + 1)),
            c => out.push(c),
        }
    }
    (out, None)
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
        assert_eq!(lazy_scope(&Project::unknown(), &views.join("users/index.html.erb")).as_deref(), Some("users.index"));
        assert_eq!(lazy_scope(&Project::unknown(), &views.join("users/_form.html+phone.erb")).as_deref(), Some("users.form"));
        assert_eq!(lazy_scope(&Project::unknown(), &views.join("users/_user.v2.html.erb")).as_deref(), Some("users.user.v2"));
        assert_eq!(lazy_scope(&Project::unknown(), &views.join("admin/_x/_card.html.haml")).as_deref(), Some("admin.x.card"));
        assert_eq!(full_key(&Project::unknown(), &views.join("users/index.html.erb"), "a.b").as_deref(), Some("a.b"));
        assert_eq!(full_key(&Project::unknown(), Path::new("/tmp/x.html.erb"), ".title"), None);
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
                ("users.ok".into(), 4, s("OK")),
                ("users.index".into(), 7, None),
                ("users.index.title".into(), 8, s("Hello")),
                ("users.index.body".into(), 10, s("Line one # not a comment")),
                ("users.index.after".into(), 14, s("*alias")),
                ("users.index.days".into(), 15, s("[Sun, Mon]")),
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

    fn parsed(yml: &str) -> Vec<(String, usize, Option<String>)> {
        parse(yml).into_iter().map(|t| (t.key, t.line, t.value)).collect()
    }

    fn entry(key: &str, line: usize, value: Option<&str>) -> (String, usize, Option<String>) {
        (key.to_string(), line, value.map(str::to_string))
    }

    #[test]
    fn parses_flow_mappings_on_one_line() {
        let yml = "en:\n  title: { short: \"A, a\", long: 'B', deep: { x: 1 } } # note\n  after: ok\n  empty: {}\n";
        assert_eq!(
            parsed(yml),
            [
                entry("title.short", 1, Some("A, a")),
                entry("title.long", 1, Some("B")),
                entry("title.deep.x", 1, Some("1")),
                entry("title.deep", 1, None),
                entry("title", 1, None),
                entry("after", 2, Some("ok")),
                entry("empty", 3, None),
            ]
        );
        // An unclosed flow mapping stays the plain text it was.
        assert_eq!(parsed("en:\n  a: { b: 1\n"), [entry("a", 1, Some("{ b: 1"))]);
    }

    #[test]
    fn indexes_a_sequence_as_one_key() {
        let yml = "en:\n  inline: [Sun, \"Mon, day\", 'Tue']\n  block:\n    - Sun\n    - \"Mon\" # c\n  flush:\n  - a\n  items:\n    - name: A\n      desc: B\n    - c\n  next: 1\n";
        assert_eq!(
            parsed(yml),
            [
                entry("inline", 1, Some("[Sun, Mon, day, Tue]")),
                entry("block", 2, Some("[Sun, Mon]")),
                entry("flush", 5, Some("[a]")),
                entry("items", 7, None),
                entry("next", 11, Some("1")),
            ]
        );
    }

    #[test]
    fn resolves_aliases_and_merge_keys_to_the_anchor() {
        let yml = concat!(
            "en:\n",
            "  base: &base\n    ok: OK\n    nested:\n      deep: D\n",
            "  one: &one Hi\n",
            "  page:\n    <<: *base\n    ok: Mine\n    extra: E\n",
            "  copy: *base\n",
            "  hi: *one # c\n",
            "  both:\n    <<: [*one, *base]\n",
            "  lost: *nobody\n",
        );
        assert_eq!(
            parsed(yml),
            [
                entry("base", 1, None),
                entry("base.ok", 2, Some("OK")),
                entry("base.nested", 3, None),
                entry("base.nested.deep", 4, Some("D")),
                entry("one", 5, Some("Hi")),
                entry("page", 6, None),
                entry("page.nested", 3, None),
                entry("page.nested.deep", 4, Some("D")),
                entry("page.ok", 8, Some("Mine")),
                entry("page.extra", 9, Some("E")),
                entry("copy", 10, None),
                entry("copy.ok", 2, Some("OK")),
                entry("copy.nested", 3, None),
                entry("copy.nested.deep", 4, Some("D")),
                entry("hi", 11, Some("Hi")),
                entry("both", 12, None),
                entry("both.ok", 2, Some("OK")),
                entry("both.nested", 3, None),
                entry("both.nested.deep", 4, Some("D")),
                entry("lost", 14, Some("*nobody")),
            ]
        );
    }

    #[test]
    fn quoted_punctuation_and_alias_lookalikes_stay_plain_values() {
        let yml = "en:\n  a: \"x, {b}: #c\"\n  b: \"*not\"\n  c: \"#tag\"\n  d: []\n  e: [a, b] # n\n  f: *later\n  g: &later Hi\n";
        assert_eq!(
            parsed(yml),
            [
                entry("a", 1, Some("x, {b}: #c")),
                entry("b", 2, Some("*not")),
                entry("c", 3, Some("#tag")),
                entry("d", 4, Some("[]")),
                entry("e", 5, Some("[a, b]")),
                entry("f", 6, Some("*later")),
                entry("g", 7, Some("Hi")),
            ]
        );
    }

    #[test]
    fn merges_through_a_merged_anchor_and_scopes_anchors_to_their_locale() {
        let yml = "en:\n  base: &b\n    k: 1\n  mid: &m\n    <<: *b\n    j: 2\n  top:\n    <<: *m\nuk:\n  base: &b\n    k: 2\n  x: *b\n";
        assert_eq!(
            parsed(yml),
            [
                entry("base", 1, None),
                entry("base.k", 2, Some("1")),
                entry("mid", 3, None),
                entry("mid.k", 2, Some("1")),
                entry("mid.j", 5, Some("2")),
                entry("top", 6, None),
                entry("top.k", 2, Some("1")),
                entry("top.j", 5, Some("2")),
                entry("base", 9, None),
                entry("base.k", 10, Some("2")),
                entry("x", 11, None),
                entry("x.k", 10, Some("2")),
            ]
        );
    }

    #[test]
    fn merges_locale_level_and_top_level_anchors() {
        let yml = "en: &en\n  hello: Hi\n  a:\n    b: B\nen-GB:\n  <<: *en\n  hello: Hello\n";
        assert_eq!(
            parsed(yml),
            [
                entry("hello", 1, Some("Hi")),
                entry("a", 2, None),
                entry("a.b", 3, Some("B")),
                entry("a", 2, None),
                entry("a.b", 3, Some("B")),
                entry("hello", 6, Some("Hello")),
            ]
        );
        let yml = "defaults: &defaults\n  ok: OK\nen:\n  <<: *defaults\n  copy: *defaults\n";
        let en: Vec<_> = parse(yml).into_iter().filter(|t| t.locale == "en").map(|t| (t.key, t.line)).collect();
        assert_eq!(en, [("ok".to_string(), 1), ("copy.ok".to_string(), 1)]);
    }

    #[test]
    fn merged_entries_overridden_in_the_anchored_range_are_not_copied() {
        let yml = "defaults: &defaults\n  hello: Default\nen: &en\n  <<: *defaults\n  hello: Hi\nen-GB:\n  <<: *en\n";
        let hello: Vec<_> = parse(yml).into_iter().filter(|t| t.locale == "en-GB").map(|t| (t.key, t.value)).collect();
        assert_eq!(hello, [("hello".to_string(), Some("Hi".to_string()))]);
        let yml = "en:\n  y:\n    m: base\n  a: &x\n    <<: *y\n    m: own\n  d: *x\n";
        let yml = yml.replace("  y:\n", "  y: &y\n");
        let d: Vec<_> = parse(&yml).into_iter().filter(|t| t.key.starts_with("d.")).map(|t| t.value).collect();
        assert_eq!(d, [Some("own".to_string())]);
    }

    #[test]
    fn alias_copies_exactly_the_anchored_subtree() {
        let flow = "en:\n  a: &x { b: 1, c: { d: 2 } }\n  ab: other\n  e: *x\n";
        assert_eq!(
            parsed(flow),
            [
                entry("a.b", 1, Some("1")),
                entry("a.c.d", 1, Some("2")),
                entry("a.c", 1, None),
                entry("a", 1, None),
                entry("ab", 2, Some("other")),
                entry("e.b", 1, Some("1")),
                entry("e.c.d", 1, Some("2")),
                entry("e.c", 1, None),
                entry("e", 3, None),
            ]
        );
        let merged = "base: &y
  m: M
en:
  a: &x
    <<: *y
    c: 1
  b: after
  d: *x
";
        let en: Vec<_> = parse(merged).into_iter().filter(|t| t.locale == "en").map(|t| (t.key, t.line)).collect();
        let keys: Vec<_> = en.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["a", "a.m", "a.c", "b", "d", "d.m", "d.c"]);
        let nested = "en:\n  a: &outer\n    b: &inner\n      c: 1\n    d: 2\n  e: *inner\n  f: *outer\n";
        let keys: Vec<_> = parse(nested).into_iter().map(|t| t.key).collect();
        assert_eq!(keys, ["a", "a.b", "a.b.c", "a.d", "e", "e.c", "f", "f.b", "f.b.c", "f.d"]);
    }

    #[test]
    fn list_items_keep_empty_strings_and_skip_block_scalars() {
        let yml = "en:\n  a:\n    - \"\"\n    - Mon\n    - ''\n  b:\n    - |\n      body\n    - x\n  c: 1\n";
        assert_eq!(parsed(yml), [entry("a", 1, Some("[\"\", Mon, \"\"]")), entry("b", 5, None), entry("c", 9, Some("1"))]);
    }

    #[test]
    fn crlf_and_multibyte_keys_keep_line_numbers_and_values() {
        let yml = "en:\r\n  a: Привіт\r\n  ключ: { х: \"й\" }\r\n  l:\r\n    - a\r\n    - b\r\n  z: &q\r\n    k: 1\r\n  y: *q\r\n";
        assert_eq!(
            parsed(yml),
            [
                entry("a", 1, Some("Привіт")),
                entry("ключ.х", 2, Some("й")),
                entry("ключ", 2, None),
                entry("l", 3, Some("[a, b]")),
                entry("z", 6, None),
                entry("z.k", 7, Some("1")),
                entry("y", 8, None),
                entry("y.k", 7, Some("1")),
            ]
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
        let key = full_key(&Project::unknown(), &partial, ".title").unwrap();
        assert_eq!(lookup(&files, &key).count(), 2);

        assert_eq!(hover(&files, "hello").unwrap(), "`hello`\n\n- **en**: `Hello`\n- **uk**: `Привіт`\n\nMissing in `de`");
        assert!(hover(&files, "nope").is_none());
        assert_eq!(hover(&files, "html").unwrap().lines().nth(2), Some("- **en**: ``<b>`x`</b>``"));
        assert_eq!(inline_code("`a"), "`` `a ``");

        let labels = |prefix| completions(&Project::unknown(), &files, &partial, prefix).into_iter().map(|(l, _)| l).collect::<Vec<_>>();
        assert_eq!(labels(""), ["hello", "html", "users.form.title"]);
        assert_eq!(labels("users."), ["users.form.title"]);
        assert_eq!(labels("."), [".title"]);
        assert_eq!(completions(&Project::unknown(), &files, &partial, ".t")[0].1, "en: New user");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
