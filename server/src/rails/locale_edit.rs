//! Adds a key to a locale file as one text insertion, so that comments, quotes, anchors and line endings stay as they are.
//! Reads the file the way `i18n::parse` does, line by line; a file this can't place the key in with certainty (flow collections,
//! aliases, anchors, merges, lists, several documents) gets no edit.

use super::i18n::{scalar_value, split_anchor, split_key};
use std::cmp::Ordering;
use std::ops::Range;

const DEFAULT_INDENT: usize = 2;
/// Plain scalars that YAML 1.1, which Psych reads, makes something else than a string.
const NOT_STRINGS: [&str; 10] = ["y", "n", "yes", "no", "on", "off", "true", "false", "null", "~"];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Key,
    /// A `- item`.
    Item,
    /// Part of the value of a key or item above: a block scalar, a wrapped scalar, the entries of an item.
    Continuation,
}

struct Entry<'a> {
    key: &'a str,
    quoted: bool,
    /// A scalar, a flow collection, an alias or a tag follows the colon.
    has_value: bool,
    anchored: bool,
}

struct Line<'a> {
    /// The offset where the line ends, before its line ending.
    end: usize,
    indent: usize,
    kind: Kind,
    entry: Option<Entry<'a>>,
}

/// The offset in `text` (without a BOM) and the text to insert there to add `path` (a locale and the keys below it) with the
/// scalar `value`; none when the key exists, conflicts with a value or can't be placed with certainty.
pub fn add_key(text: &str, path: &[&str], value: &str) -> Option<(usize, String)> {
    let lines = read_lines(text)?;
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let unit = indent_unit(&lines);
    let mut range = 0..lines.len();
    let mut parent: Option<usize> = None;
    for (depth, name) in path.iter().enumerate() {
        let missing = &path[depth..];
        if range.is_empty() {
            return Some(match parent {
                Some(at) => (lines[at].end, format!("{eol}{}", block(missing, lines[at].indent + unit, unit, value, eol))),
                None => {
                    let separator = if text.is_empty() || text.ends_with('\n') { "" } else { eol };
                    (text.len(), format!("{separator}{}{eol}", block(missing, 0, unit, value, eol)))
                }
            });
        }
        let (indent, keys) = mapping_keys(&lines, &range)?;
        if keys.iter().any(|&at| lines[at].entry.as_ref().is_some_and(|entry| entry.key == "<<")) {
            return None;
        }
        let found = keys.into_iter().find(|&at| lines[at].entry.as_ref().is_some_and(|entry| names(entry, name)));
        let Some(at) = found else {
            return Some((lines[range.end - 1].end, format!("{eol}{}", block(missing, indent, unit, value, eol))));
        };
        let entry = lines[at].entry.as_ref()?;
        if depth + 1 == path.len() || entry.has_value || entry.anchored {
            return None;
        }
        range = at + 1..block_end(&lines, &range, at);
        parent = Some(at);
    }
    None
}

fn read_lines(text: &str) -> Option<Vec<Line<'_>>> {
    let mut lines: Vec<Line> = Vec::new();
    let mut scalar_owner: Option<usize> = None;
    let mut offset = 0;
    for raw in text.split_inclusive('\n') {
        let content = raw.strip_suffix('\n').map_or(raw, |line| line.strip_suffix('\r').unwrap_or(line));
        let end = offset + content.len();
        offset += raw.len();
        let body = content.trim_start_matches(' ');
        let indent = content.len() - body.len();
        if body.starts_with('\t') {
            return None;
        }
        if body.trim().is_empty() {
            continue;
        }
        if scalar_owner.is_some_and(|owner| indent > owner) {
            lines.push(Line { end, indent, kind: Kind::Continuation, entry: None });
            continue;
        }
        scalar_owner = None;
        if body.starts_with('#') || (body.trim_end() == "---" && lines.is_empty()) {
            continue;
        }
        if body.starts_with("---") || body.starts_with("...") || body.starts_with('%') {
            return None;
        }
        if body == "-" || body.starts_with("- ") {
            scalar_owner = Some(indent);
            lines.push(Line { end, indent, kind: Kind::Item, entry: None });
            continue;
        }
        let (key, rest) = split_key(body)?;
        let quoted = body.starts_with(['"', '\'']);
        let unreadable = if quoted { key.contains('\\') } else { key.starts_with(['{', '[', '&', '*', '!', '?', '|', '>', '@', '`']) };
        if unreadable {
            return None;
        }
        let has_value = scalar_value(rest).is_some();
        if has_value {
            scalar_owner = Some(indent);
        }
        let anchored = split_anchor(rest).0.is_some();
        lines.push(Line { end, indent, kind: Kind::Key, entry: Some(Entry { key, quoted, has_value, anchored }) });
    }
    Some(lines)
}

/// The indentation one level of nesting adds: what the first nested key has over its parent.
fn indent_unit(lines: &[Line]) -> usize {
    let nested = lines.windows(2).find(|pair| pair[0].kind == Kind::Key && pair[1].kind == Kind::Key && pair[1].indent > pair[0].indent);
    nested.map_or(DEFAULT_INDENT, |pair| pair[1].indent - pair[0].indent)
}

/// The indentation of the block mapping `range` holds and the lines of its keys; none when it holds a list or is misindented.
fn mapping_keys(lines: &[Line], range: &Range<usize>) -> Option<(usize, Vec<usize>)> {
    let first = &lines[range.start];
    if first.kind != Kind::Key {
        return None;
    }
    let mut keys = Vec::new();
    for at in range.clone() {
        let line = &lines[at];
        match (line.kind, line.indent.cmp(&first.indent)) {
            // An item at its key's indent is that key's list, as Psych dumps one.
            (Kind::Continuation, _) | (_, Ordering::Greater) | (Kind::Item, Ordering::Equal) => {}
            (_, Ordering::Less) => return None,
            (Kind::Key, Ordering::Equal) => keys.push(at),
        }
    }
    Some((first.indent, keys))
}

/// The line after the last one that belongs to the key on line `at` of `range`: those nested below it, and a list at its own indent.
fn block_end(lines: &[Line], range: &Range<usize>, at: usize) -> usize {
    let indent = lines[at].indent;
    let leaves =
        |line: &Line| line.kind != Kind::Continuation && (line.indent < indent || (line.indent == indent && line.kind == Kind::Key));
    (at + 1..range.end).find(|&next| leaves(&lines[next])).unwrap_or(range.end)
}

/// Whether `entry` is the key `name`: a plain key that YAML reads as a boolean, null or number isn't a string.
fn names(entry: &Entry, name: &str) -> bool {
    entry.key == name && (entry.quoted || !needs_quotes(name))
}

fn needs_quotes(key: &str) -> bool {
    let starts_like_a_word = key.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_');
    let plain = key.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '?' | '!'));
    !starts_like_a_word || !plain || NOT_STRINGS.iter().any(|word| key.eq_ignore_ascii_case(word))
}

fn key_text(key: &str) -> String {
    if needs_quotes(key) {
        format!("\"{}\"", key.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        key.to_string()
    }
}

/// The lines for `names` nested one in the next from `indent`, the last with `value`.
fn block(names: &[&str], indent: usize, unit: usize, value: &str, eol: &str) -> String {
    let last = names.len() - 1;
    let lines = names.iter().enumerate().map(|(depth, name)| {
        let spaces = " ".repeat(indent + depth * unit);
        let key = key_text(name);
        if depth == last {
            format!("{spaces}{key}: {value}")
        } else {
            format!("{spaces}{key}:")
        }
    });
    lines.collect::<Vec<_>>().join(eol)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` with the key `a.b.c` (below the locale `en`) added as `value`, or none when no edit is made.
    fn added(text: &str, keys: &str) -> Option<String> {
        let path: Vec<&str> = std::iter::once("en").chain(keys.split('.')).collect();
        let (offset, inserted) = add_key(text, &path, "\"\"")?;
        Some(format!("{}{inserted}{}", &text[..offset], &text[offset..]))
    }

    const LOCALE: &str = "en:\n  users:\n    index:\n      title: Users\n    show:\n      name: Name\n  layouts:\n    menu: Menu\n";

    #[test]
    fn a_key_goes_after_the_last_entry_of_its_parent() {
        assert_eq!(
            added(LOCALE, "users.index.heading").unwrap(),
            "en:\n  users:\n    index:\n      title: Users\n      heading: \"\"\n    show:\n      name: Name\n  layouts:\n    menu: Menu\n"
        );
        assert_eq!(
            added(LOCALE, "users.show.age").unwrap(),
            "en:\n  users:\n    index:\n      title: Users\n    show:\n      name: Name\n      age: \"\"\n  layouts:\n    menu: Menu\n"
        );
        assert_eq!(added(LOCALE, "layouts.title").unwrap(), format!("{LOCALE}    title: \"\"\n"));
    }

    #[test]
    fn missing_parents_are_created_with_the_files_indentation() {
        assert_eq!(added(LOCALE, "users.edit.title").unwrap(), "en:\n  users:\n    index:\n      title: Users\n    show:\n      name: Name\n    edit:\n      title: \"\"\n  layouts:\n    menu: Menu\n");
        assert_eq!(added(LOCALE, "pages.home.title").unwrap(), format!("{LOCALE}  pages:\n    home:\n      title: \"\"\n"));
        let four = "en:\n    users:\n        index:\n            title: Users\n";
        assert_eq!(added(four, "admin.users.title").unwrap(), format!("{four}    admin:\n        users:\n            title: \"\"\n"));
        let odd = "en:\n   a: 1\n";
        assert_eq!(added(odd, "b.c").unwrap(), "en:\n   a: 1\n   b:\n      c: \"\"\n");
    }

    #[test]
    fn a_parent_without_entries_gets_the_first_one() {
        assert_eq!(
            added("en:\n  users:\n  layouts:\n    a: 1\n", "users.title").unwrap(),
            "en:\n  users:\n    title: \"\"\n  layouts:\n    a: 1\n"
        );
        assert_eq!(added("en:\n  users: # none yet\n", "users.title.x").unwrap(), "en:\n  users: # none yet\n    title:\n      x: \"\"\n");
    }

    #[test]
    fn an_empty_file_or_one_without_the_locale_gets_it() {
        assert_eq!(added("", "users.title").unwrap(), "en:\n  users:\n    title: \"\"\n");
        assert_eq!(added("# only a comment", "title").unwrap(), "# only a comment\nen:\n  title: \"\"\n");
        assert_eq!(added("---\n", "title").unwrap(), "---\nen:\n  title: \"\"\n");
        assert_eq!(added("fr:\n  title: Titre\n", "title").unwrap(), "fr:\n  title: Titre\nen:\n  title: \"\"\n");
    }

    #[test]
    fn line_endings_and_a_missing_final_one_are_kept() {
        assert_eq!(added("en:\r\n  users:\r\n    a: 1\r\n", "users.b").unwrap(), "en:\r\n  users:\r\n    a: 1\r\n    b: \"\"\r\n");
        assert_eq!(
            added("en:\r\n  users:\r\n    a: 1\r\n", "pages.b").unwrap(),
            "en:\r\n  users:\r\n    a: 1\r\n  pages:\r\n    b: \"\"\r\n"
        );
        assert_eq!(added("en:\n  users:\n    a: 1", "users.b").unwrap(), "en:\n  users:\n    a: 1\n    b: \"\"");
        assert_eq!(added("en:", "b").unwrap(), "en:\n  b: \"\"");
        assert_eq!(added("fr:\n  a: 1", "b").unwrap(), "fr:\n  a: 1\nen:\n  b: \"\"");
    }

    #[test]
    fn comments_and_block_scalars_stay_whole() {
        let text = "en:\n  # users\n  users:\n    a: |\n      one\n      # not a comment\n      two\n    # trailing note\n  other: 1\n";
        assert_eq!(
            added(text, "users.b").unwrap(),
            "en:\n  # users\n  users:\n    a: |\n      one\n      # not a comment\n      two\n    b: \"\"\n    # trailing note\n  other: 1\n"
        );
        let wrapped = "en:\n  users:\n    a: \"first\n      second\" # note\n    c: plain\n      wrapped\n";
        assert_eq!(
            added(wrapped, "users.d").unwrap(),
            "en:\n  users:\n    a: \"first\n      second\" # note\n    c: plain\n      wrapped\n    d: \"\"\n"
        );
        assert_eq!(added("en:\n  a: 1 # one\n", "b").unwrap(), "en:\n  a: 1 # one\n  b: \"\"\n");
    }

    #[test]
    fn keys_that_yaml_would_read_as_something_else_are_quoted() {
        assert_eq!(added("en:\n  a: 1\n", "no").unwrap(), "en:\n  a: 1\n  \"no\": \"\"\n");
        assert_eq!(added("en:\n  a: 1\n", "On").unwrap(), "en:\n  a: 1\n  \"On\": \"\"\n");
        assert_eq!(added("en:\n  a: 1\n", "yes.sure").unwrap(), "en:\n  a: 1\n  \"yes\":\n    sure: \"\"\n");
        assert_eq!(added("en:\n  a: 1\n", "1st").unwrap(), "en:\n  a: 1\n  \"1st\": \"\"\n");
        assert_eq!(added("en:\n  a: 1\n", "a:b").unwrap(), "en:\n  a: 1\n  \"a:b\": \"\"\n");
        assert_eq!(added("en:\n  a: 1\n", "say \"hi\"").unwrap(), "en:\n  a: 1\n  \"say \\\"hi\\\"\": \"\"\n");
        assert_eq!(added("en:\n  a: 1\n", "-x").unwrap(), "en:\n  a: 1\n  \"-x\": \"\"\n");
        assert_eq!(added("en:\n  a: 1\n", "save?").unwrap(), "en:\n  a: 1\n  save?: \"\"\n");
        assert_eq!(added("en:\n  a: 1\n", "date_picker-x").unwrap(), "en:\n  a: 1\n  date_picker-x: \"\"\n");
    }

    #[test]
    fn existing_quoted_keys_are_found_and_plain_booleans_are_not_strings() {
        let quoted = "en:\n  \"no\":\n    a: 1\n  'yes':\n    b: 1\n";
        assert_eq!(added(quoted, "no.c").unwrap(), "en:\n  \"no\":\n    a: 1\n    c: \"\"\n  'yes':\n    b: 1\n");
        assert_eq!(added(quoted, "yes.c").unwrap(), "en:\n  \"no\":\n    a: 1\n  'yes':\n    b: 1\n    c: \"\"\n");
        // `no:` is `false` to Psych, so the string key `no` is a new one.
        assert_eq!(added("en:\n  no:\n    a: 1\n", "no.c").unwrap(), "en:\n  no:\n    a: 1\n  \"no\":\n    c: \"\"\n");
    }

    #[test]
    fn nothing_is_added_where_the_file_would_change_meaning() {
        for (label, text, keys) in [
            ("exists", LOCALE, "users.index.title"),
            ("exists as a group", LOCALE, "users.index"),
            ("scalar parent", LOCALE, "users.index.title.x"),
            ("alias parent", "en:\n  base: &base\n    a: 1\n  users: *base\n", "users.b"),
            ("anchored parent", "en:\n  users: &base\n    a: 1\n  other: *base\n", "users.b"),
            ("anchored ancestor", "en:\n  users: &base\n    index:\n      a: 1\n", "users.index.b"),
            ("merge in the parent", "en:\n  defaults: &d\n    a: 1\n  users:\n    <<: *d\n    b: 1\n", "users.c"),
            ("merge in an ancestor", "en:\n  users:\n    <<: *d\n", "users.index.title"),
            ("flow parent", "en:\n  users: { a: 1 }\n", "users.b"),
            ("flow sequence parent", "en:\n  users: [a, b]\n", "users.b"),
            ("flow root", "{ en: { a: 1 } }\n", "b"),
            ("list parent", "en:\n  users:\n    - a\n    - b\n", "users.c"),
            ("list at the key's indent", "en:\n  users:\n  - a\n", "users.c"),
            ("tab indentation", "en:\n\tusers:\n\t\ta: 1\n", "users.b"),
            ("several documents", "en:\n  a: 1\n---\nfr:\n  a: 1\n", "b"),
            ("document end", "en:\n  a: 1\n...\n", "b"),
            ("explicit key", "en:\n  ? a\n  : 1\n", "b"),
            ("escaped key", "en:\n  \"a\\tb\": 1\n", "c"),
            ("misindented", "en:\n    a: 1\n  b: 2\n", "c"),
        ] {
            assert_eq!(added(text, keys), None, "{label}");
        }
    }

    #[test]
    fn a_list_elsewhere_in_the_file_is_no_obstacle() {
        let text = "en:\n  days:\n    - Sun\n    - Mon\n  users:\n    a: 1\n  items:\n  - x\n  - y: 1\n    z: 2\n";
        assert_eq!(
            added(text, "users.b").unwrap(),
            "en:\n  days:\n    - Sun\n    - Mon\n  users:\n    a: 1\n    b: \"\"\n  items:\n  - x\n  - y: 1\n    z: 2\n"
        );
        assert_eq!(added(text, "last").unwrap(), format!("{text}  last: \"\"\n"));
    }
}
