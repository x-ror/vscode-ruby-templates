//! Haml editing features on one line: where Ruby is, what to complete, and hover docs for filters and prefixes.

use crate::haml_indent::{haml_code_start, is_haml_tag, tag_attr_groups, tag_head_len, AttrGroup, HAML_FILTERS};
use crate::helpers::{ARIA_KEYS, DATA_KEYS, HTML_ATTRIBUTES, HTML_TAGS};
use crate::ruby;
use crate::util::{completion_item, leading_ws};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, InsertTextFormat};

/// What the cursor is completing, besides Ruby.
#[derive(Debug, PartialEq, Eq)]
pub enum Completion<'a> {
    /// `:jav` at line start.
    Filter(&'a str),
    /// `%sec` at line start.
    Tag(&'a str),
    /// A key position in a tag's `{}` hash, or a name position in its `()`. `rocket`: an old-style `:key =>` key.
    Attribute { typed: &'a str, place: AttrPlace, rocket: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrPlace {
    Hash,
    DataHash,
    AriaHash,
    Html,
}

/// Ruby is the code after `-`/`=`/`~`/`!=`/`&=` (on its own or after a tag head), a tag's `{}` hash or `[]`
/// object reference, or an open `#{` interpolation.
pub fn in_ruby(line: &str, col: usize) -> bool {
    let col = col.min(line.len());
    if let Some(start) = haml_code_start(line).filter(|&start| start <= col) {
        return !ruby::comment_before(&line[start..col]);
    }
    let ws = leading_ws(line).len();
    let content = &line[ws..];
    if content.starts_with("-#") {
        return false;
    }
    if let Some(rel) = col.checked_sub(ws).filter(|_| is_haml_tag(content)) {
        if let Some(g) = tag_attr_groups(content).into_iter().find(|g| inside(g, rel)) {
            return g.open != '(';
        }
    }
    let before = &line[..col];
    before.rfind("#{").is_some_and(|open| !before[open..].contains('}'))
}

fn inside(group: &AttrGroup, col: usize) -> bool {
    col > group.start && group.end.is_none_or(|end| col < end)
}

pub fn completion_context(line: &str, col: usize) -> Option<Completion<'_>> {
    let ws = leading_ws(line).len();
    let before = line.get(ws..col)?;
    let name_chars = |s: &str, extra: &[char]| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || extra.contains(&c));
    if let Some(typed) = before.strip_prefix(':').filter(|t| name_chars(t, &[])) {
        return Some(Completion::Filter(typed));
    }
    if let Some(typed) = before.strip_prefix('%').filter(|t| name_chars(t, &['-'])) {
        return Some(Completion::Tag(typed));
    }
    if !is_haml_tag(before) {
        return None;
    }
    let group = tag_attr_groups(before).pop().filter(|g| g.end.is_none())?;
    let inner = &before[group.start + 1..];
    let (typed, place) = match group.open {
        '{' => hash_key(inner)?,
        '(' => (html_attr_name(inner)?, AttrPlace::Html),
        _ => return None,
    };
    let (typed, rocket) = typed.strip_prefix(':').map_or((typed, false), |t| (t, true));
    Some(Completion::Attribute { typed, place, rocket })
}

/// Items for a `completion_context`.
pub fn completions(context: Completion) -> Vec<CompletionItem> {
    match context {
        Completion::Filter(typed) => HAML_FILTERS
            .iter()
            .filter(|name| name.starts_with(typed))
            .map(|f| completion_item(*f, CompletionItemKind::MODULE, "Haml filter"))
            .collect(),
        Completion::Tag(typed) => HTML_TAGS
            .iter()
            .filter(|name| name.starts_with(typed))
            .map(|t| completion_item(*t, CompletionItemKind::PROPERTY, "HTML element"))
            .collect(),
        Completion::Attribute { typed, place, rocket } => {
            let names: Vec<String> = match place {
                AttrPlace::Hash => HTML_ATTRIBUTES.iter().chain(&["data", "aria"]).map(|a| a.to_string()).collect(),
                AttrPlace::DataHash => DATA_KEYS.iter().map(|k| k.to_string()).collect(),
                AttrPlace::AriaHash => ARIA_KEYS.iter().map(|k| k.to_string()).collect(),
                AttrPlace::Html => HTML_ATTRIBUTES
                    .iter()
                    .map(|a| a.to_string())
                    .chain(ARIA_KEYS.iter().map(|k| format!("aria-{k}")))
                    .chain(DATA_KEYS.iter().map(|k| format!("data-{}", k.replace('_', "-"))))
                    .collect(),
            };
            names
                .into_iter()
                .filter(|n| n.starts_with(typed))
                .map(|name| {
                    // The typed `:` of an old-style key stays; the snippet replaces only the name after it.
                    let key = if name.contains('-') { format!("\"{name}\"") } else { name.clone() };
                    let separator = if rocket { " =>" } else { ":" };
                    let snippet = match place {
                        AttrPlace::Html => format!("{name}=\"$1\""),
                        AttrPlace::Hash if name == "data" || name == "aria" => format!("{key}{separator} {{$1}}"),
                        _ => format!("{key}{separator} $1"),
                    };
                    CompletionItem {
                        insert_text: Some(snippet),
                        insert_text_format: Some(InsertTextFormat::SNIPPET),
                        ..completion_item(name, CompletionItemKind::PROPERTY, "HTML attribute")
                    }
                })
                .collect()
        }
    }
}

/// The partial key being typed in an attribute hash (with its `:` if old-style), and which hash (`data:`, `aria:` or the tag's own).
fn hash_key(inner: &str) -> Option<(&str, AttrPlace)> {
    // Open brackets as (bracket, offset after it); the outer `{` is implicit.
    let mut open: Vec<(char, usize)> = vec![('{', 0)];
    let mut segment = Some(0);
    let mut quote = None;
    let mut chars = inner.char_indices();
    while let Some((i, ch)) = chars.next() {
        match (quote, ch) {
            (Some(_), '\\') => {
                chars.next();
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(ch),
            (None, '{' | '(' | '[') => {
                open.push((ch, i + 1));
                segment = Some(i + 1);
            }
            // After a nested value closes, the key is behind us until the next `,`.
            (None, '}' | ')' | ']') => {
                open.pop();
                segment = None;
            }
            (None, ',') => segment = Some(i + 1),
            _ => {}
        }
    }
    let (bracket, after) = *open.last()?;
    if quote.is_some() || bracket != '{' {
        return None;
    }
    let typed = inner[segment?..].trim_start();
    if !typed.strip_prefix(':').unwrap_or(typed).chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    if open.len() == 1 {
        return Some((typed, AttrPlace::Hash));
    }
    let owner = inner[..after - 1].trim_end();
    let place = if ["data:", ":data =>", ":data=>", "\"data\":"].iter().any(|k| owner.ends_with(k)) {
        AttrPlace::DataHash
    } else if ["aria:", ":aria =>", ":aria=>", "\"aria\":"].iter().any(|k| owner.ends_with(k)) {
        AttrPlace::AriaHash
    } else {
        return None;
    };
    Some((typed, place))
}

/// The partial attribute name being typed in `(…)`, or `None` inside a value.
fn html_attr_name(inner: &str) -> Option<&str> {
    let mut quote = None;
    for ch in inner.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None => {}
        }
    }
    if quote.is_some() {
        return None;
    }
    let start = inner
        .char_indices()
        .rev()
        .find(|&(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':')))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let preceded_by_space = start == 0 || inner[..start].ends_with(char::is_whitespace);
    preceded_by_space.then(|| &inner[start..])
}

const FILTER_DOCS: &[(&str, &str)] = &[
    ("plain", "Plain text, not parsed as Haml. `#{}` is interpolated and **not** escaped."),
    ("escaped", "Plain text, HTML-escaped (interpolated `#{}` values too)."),
    (
        "preserve",
        "Plain text with newlines turned into `&#x000A;`, so later indentation can't change it. `#{}` is interpolated, not escaped.",
    ),
    ("cdata", "Wraps the text in `<![CDATA[ … ]]>`. `#{}` is interpolated, not escaped."),
    ("javascript", "Wraps the text in `<script>`. `#{}` is interpolated and **not** escaped: use `#{j value}` or `#{value.to_json}`."),
    ("css", "Wraps the text in `<style>`. `#{}` is interpolated, not escaped."),
    ("ruby", "Runs the Ruby code; nothing is output unless the code writes to the buffer."),
    ("erb", "Renders the text as ERB (via Tilt)."),
    (
        "markdown",
        "Renders Markdown to HTML via Tilt; needs a Markdown gem (`redcarpet`, `kramdown`, `commonmarker`, …). `#{}` is interpolated.",
    ),
    ("sass", "Compiles indented Sass into `<style>` via Tilt; needs `sass-embedded` (or `sassc`). `#{}` is interpolated."),
    ("scss", "Compiles SCSS into `<style>` via Tilt; needs `sass-embedded` (or `sassc`). `#{}` is interpolated."),
    ("less", "Compiles Less into `<style>` via Tilt; needs the `less` gem. `#{}` is interpolated."),
    ("coffee", "Compiles CoffeeScript into `<script>` via Tilt; needs the `coffee-script` gem. `#{}` is interpolated."),
    ("coffeescript", "Alias of `:coffee`: compiles CoffeeScript into `<script>`; needs the `coffee-script` gem."),
];

/// Line-start prefixes, longest first so `!=` wins over `!`.
const PREFIX_DOCS: &[(&str, &str)] = &[
    ("!==", "Interpolated plain text, **not** escaped: `#{}` values are output as-is."),
    ("&==", "Interpolated plain text, HTML-escaped."),
    ("!=", "Evaluates Ruby and outputs the result **without** HTML escaping."),
    ("&=", "Evaluates Ruby and outputs the result HTML-escaped (the default for `=` in Rails)."),
    ("!~", "Like `~`, without HTML escaping."),
    ("&~", "Like `~`, HTML-escaped."),
    ("==", "Interpolated plain text: `#{}` is evaluated and HTML-escaped; the rest is not Ruby."),
    ("-#", "Haml comment: this line and everything nested under it are left out of the output."),
    ("=", "Evaluates Ruby and outputs the result, HTML-escaped."),
    ("~", "Like `=`, but newlines inside `<pre>`, `<textarea>` and `<code>` in the result are preserved (`find_and_preserve`)."),
    ("-", "Runs Ruby without output. Nested lines form the block; Haml adds the `end`."),
    ("&", "Plain text, HTML-escaped, with `#{}` interpolation."),
    ("!", "Plain text, **not** escaped, with `#{}` interpolation."),
];

const WHITESPACE_DOCS: &[(char, &str)] = &[
    ('<', "Removes whitespace **inside** this tag: its content sits directly against the opening and closing tags."),
    ('>', "Removes whitespace **around** this tag: it sits directly against its neighbours and parent."),
];

/// Markdown hover for a filter name or a Haml line prefix (`&=`, `!=`, `~`, `<`, `>`, `==`, `&`, `!`, …) at `col`.
pub fn hover(line: &str, col: usize) -> Option<String> {
    let ws = leading_ws(line).len();
    let content = &line[ws..];
    if let Some(name) = content.strip_prefix(':').map(str::trim_end) {
        let (name, doc) = FILTER_DOCS.iter().find(|(f, _)| *f == name)?;
        return (ws..ws + 1 + name.len()).contains(&col).then(|| format!("**Haml filter** `:{name}`\n\n{doc}"));
    }
    let mut at = ws;
    if is_haml_tag(content) {
        at += tag_head_len(content)?;
        let after_head = &line[at..];
        let removal_len = after_head.len() - after_head.trim_start_matches(['<', '>']).len();
        if (at..at + removal_len).contains(&col) {
            let doc = WHITESPACE_DOCS.iter().find(|(c, _)| line[col..].starts_with(*c))?.1;
            return Some(format!("**Haml whitespace removal** `{}`\n\n{doc}", &line[col..col + 1]));
        }
        at += removal_len;
    } else if content.starts_with("!!!") {
        return None;
    }
    let rest = &line[at..];
    let (prefix, doc) = PREFIX_DOCS.iter().find(|(p, _)| rest.starts_with(p))?;
    // After a tag only output prefixes apply; `-` and `-#` there are text.
    if at != ws && prefix.starts_with('-') {
        return None;
    }
    // Haml reads `&`/`!` as a prefix only before a space or `#{` (`&amp;` is text).
    if matches!(*prefix, "&" | "!") && !(rest[1..].starts_with(' ') || rest[1..].starts_with("#{")) {
        return None;
    }
    (at..at + prefix.len()).contains(&col).then(|| format!("**Haml** `{prefix}`\n\n{doc}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `|` in `src` marks the cursor.
    fn split(src: &str) -> (String, usize) {
        let col = src.find('|').expect("cursor");
        (src.replacen('|', "", 1), col)
    }

    #[test]
    fn ruby_only_in_code() {
        for (src, ruby) in [
            ("= link_|", true),
            ("- x = foo|", true),
            ("%p= link_|", true),
            ("%a{href: user_pa|}", true),
            ("%a{href: user_pa|", true),
            ("%a[@us|er]", true),
            ("%p see #{link_|", true),
            ("%p see (link_|", false),
            ("%p see {link_|", false),
            ("%a(href=\"x\") (cli|", false),
            ("%a(hre|f=\"x\")", false),
            ("%p text = foo|", false),
            ("%p{a: 1} done #{x} and |", false),
            ("-# link_|", false),
            ("- # link_|", false),
            ("plain (text|", false),
            ("%p== hi #{fo|", true),
        ] {
            let (line, col) = split(src);
            assert_eq!(in_ruby(&line, col), ruby, "{src:?}");
        }
    }

    #[test]
    fn completion_contexts() {
        use AttrPlace::*;
        let attr = |typed, place| Some(Completion::Attribute { typed, place, rocket: false });
        let rocket = |typed, place| Some(Completion::Attribute { typed, place, rocket: true });
        for (src, want) in [
            (":jav|", Some(Completion::Filter("jav"))),
            ("  :|", Some(Completion::Filter(""))),
            ("%sec|", Some(Completion::Tag("sec"))),
            ("%|", Some(Completion::Tag(""))),
            ("%p.a|", None),
            ("%a{hr|", attr("hr", Hash)),
            ("%a{href: x, ti|", attr("ti", Hash)),
            ("%a{href: foo(1, |", None),
            ("%a{href: \"x, |", None),
            ("%a{href: |", None),
            ("%div{data: {turbo_f|", attr("turbo_f", DataHash)),
            ("%div{data: {a: 1}, |", attr("", Hash)),
            ("%div{data: {a: 1} |", None),
            ("%div{:aria => {lab|", attr("lab", AriaHash)),
            ("%div{x: {y|", None),
            ("%a{:hr|", rocket("hr", Hash)),
            ("%a{:href => x, :|", rocket("", Hash)),
            ("%a{:data => {tur|", attr("tur", DataHash)),
            ("%a{:data => {:tur|", rocket("tur", DataHash)),
            ("%p 50%|", None),
            (".box(cla|", attr("cla", Html)),
            ("%a(href=\"x\" ti|", attr("ti", Html)),
            ("%a(href=\"x |", None),
            ("%a(href=fo|", None),
            ("%a{x: 1}(|", attr("", Html)),
            ("%a{x: 1} te|", None),
            ("%p text :x|", None),
        ] {
            let (line, col) = split(src);
            assert_eq!(completion_context(&line, col), want, "{src:?}");
        }
    }

    #[test]
    fn every_position_of_the_sample_is_safe() {
        let sample = include_str!("../../test/sample.html.haml");
        for line in sample.lines().chain(["%a{x: \"é", "  %p(é", "é:css", "%p<é"]) {
            for (col, _) in line.char_indices().chain([(line.len(), ' ')]) {
                in_ruby(line, col);
                completion_context(line, col);
                hover(line, col);
            }
        }
        crate::outline::haml_document_symbols(sample);
        crate::outline::haml_folding_ranges(sample);
    }

    #[test]
    fn every_filter_is_documented() {
        let documented: Vec<&str> = FILTER_DOCS.iter().map(|(name, _)| *name).collect();
        assert_eq!(documented, crate::haml_indent::HAML_FILTERS);
    }

    #[test]
    fn hover_docs() {
        let head = |src: &str| {
            let (line, col) = split(src);
            hover(&line, col).map(|md| md.lines().next().unwrap_or("").to_string())
        };
        assert_eq!(head(":java|script").as_deref(), Some("**Haml filter** `:javascript`"));
        assert_eq!(head("  |:css").as_deref(), Some("**Haml filter** `:css`"));
        assert_eq!(head(":unknown|"), None);
        assert_eq!(head("|&= x").as_deref(), Some("**Haml** `&=`"));
        assert_eq!(head("!|= x").as_deref(), Some("**Haml** `!=`"));
        assert_eq!(head("|== hi").as_deref(), Some("**Haml** `==`"));
        assert_eq!(head("|~ x").as_deref(), Some("**Haml** `~`"));
        assert_eq!(head("|& text").as_deref(), Some("**Haml** `&`"));
        assert_eq!(head("|! text").as_deref(), Some("**Haml** `!`"));
        assert_eq!(head("|&#{x}").as_deref(), Some("**Haml** `&`"));
        assert_eq!(head("|&amp; x"), None);
        assert_eq!(head("|!important"), None);
        assert_eq!(head("%p{a: 1}<|>= x").as_deref(), Some("**Haml whitespace removal** `>`"));
        assert_eq!(head("%p|<= x").as_deref(), Some("**Haml whitespace removal** `<`"));
        assert_eq!(head("%p<>|!= x").as_deref(), Some("**Haml** `!=`"));
        assert_eq!(head("%p|- x"), None);
        assert_eq!(head("|!!! 5"), None);
        assert_eq!(head("= x |= y"), None);
    }
}
