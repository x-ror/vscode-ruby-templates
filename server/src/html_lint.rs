//! HTML checks for ERB views with an HTML host: `alt` text and duplicate ids. Names follow Herb's rules
//! (`html-img-require-alt`, `html-no-duplicate-ids`).

use crate::erb;
use crate::ruby::{analyze, BlockEffect};
use crate::util::{coded, warning, LineIndex};
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;
use tower_lsp::lsp_types::{Diagnostic, Range};

/// Stable diagnostic codes, shared with ERB's `rubyTemplates.erb.disabledDiagnostics`.
pub mod code {
    pub const IMG_MISSING_ALT: &str = "img-missing-alt";
    pub const DUPLICATE_ID: &str = "duplicate-id";
}

const ALT_RULE: &str = "the alt attribute must be specified (HTML 4.8.4.4.1)";
/// Set by a markup generator that couldn't get alt text; conformance checkers ignore the missing `alt` then (4.8.4.4).
const NO_ALT_AVAILABLE: &str = "generator-unable-to-provide-required-alt";

/// Elements whose body is text, not markup.
pub const RAW_TEXT: &[&str] = &["script", "style", "textarea"];
static START_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<([a-zA-Z][\w:-]*)").unwrap());
static IMAGE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bimage_tag\b").unwrap());
static ALT_OPTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\balt:|:alt\s*=>|["']alt["']\s*=>|\*\*"#).unwrap());

/// Sorted byte ranges of a template that aren't markup: ERB tags (comments and delimiters included), HTML comments and
/// raw-text element bodies. Each runs to the end of the text when it isn't closed.
pub fn non_markup(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = text[at..].find('<').map(|i| at + i) {
        let rest = &text[i..];
        let till = |close: &str, from: usize| text[from..].find(close).map_or(text.len(), |e| from + e + close.len());
        if rest.starts_with("<%") {
            let end = till("%>", i + 2);
            out.push((i, end));
            at = end;
        } else if rest.starts_with("<!--") {
            let end = till("-->", i + 4);
            out.push((i, end));
            at = end;
        } else if let Some(name) = RAW_TEXT.iter().find(|n| rest.get(1..=n.len()).is_some_and(|r| r.eq_ignore_ascii_case(n))) {
            // The body starts after the start tag's `>`, skipping ERB in its attributes.
            let mut body = i + 1;
            while let Some(gt) = text[body..].find(['>', '<']).map(|e| body + e) {
                if text[gt..].starts_with("<%") {
                    body = till("%>", gt + 2);
                    continue;
                }
                body = gt + 1;
                break;
            }
            let close = text[body..]
                .match_indices("</")
                .map(|(e, _)| body + e)
                .find(|&e| text.get(e + 2..e + 2 + name.len()).is_some_and(|n| n.eq_ignore_ascii_case(name)))
                .unwrap_or(text.len());
            out.push((body, close));
            at = close.max(i + 1);
        } else {
            at = i + 1;
        }
    }
    out
}

/// Is byte `at` inside one of `regions` (sorted, as `non_markup` returns them)?
pub fn inside(regions: &[(usize, usize)], at: usize) -> bool {
    let next = regions.partition_point(|&(start, _)| start <= at);
    next > 0 && at < regions[next - 1].1
}

struct Attribute {
    name: String,
    value: Option<(usize, usize)>,
}

struct StartTag {
    name: String,
    start: usize,
    attrs: Vec<Attribute>,
    /// ERB in an attribute-name position (`<img <%= attrs %>>`) may add any attribute.
    dynamic: bool,
}

pub fn diagnostics(text: &str) -> Vec<Diagnostic> {
    let regions = non_markup(text);
    let spans = erb::code_spans(text);
    let tags = start_tags(text, &regions);
    let index = LineIndex::new(text);
    let range = |start: usize, end: usize| Range::new(index.position(start), index.position(end));
    let mut out = Vec::new();
    for tag in tags.iter().filter(|t| t.name == "img") {
        let tag_range = range(tag.start, tag.start + 4);
        let has_alt = tag.attrs.iter().any(|a| a.name == "alt" || (a.name == NO_ALT_AVAILABLE && a.value.is_none_or(|(s, e)| s == e)));
        if !tag.dynamic && !has_alt {
            out.push(coded(warning(tag_range, format!("`<img>` without `alt`: {ALT_RULE}")), code::IMG_MISSING_ALT));
        }
    }
    out.extend(image_tags_without_alt(text, &spans, &range));
    out.extend(duplicate_ids(text, &regions, &spans, &tags, &range));
    out
}

/// Start tags in markup, with their attributes.
fn start_tags(text: &str, regions: &[(usize, usize)]) -> Vec<StartTag> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(cap) = START_TAG.captures_at(text, from) {
        let (Some(whole), Some(name)) = (cap.get(0), cap.get(1)) else { break };
        from = whole.end();
        if inside(regions, whole.start()) {
            continue;
        }
        let name = name.as_str().to_ascii_lowercase();
        let Some((tag, end)) = parse_attributes(text, regions, whole.start(), whole.end(), &name) else { continue };
        from = end;
        out.push(tag);
    }
    out
}

/// The byte just past the character at `pos`.
fn next_char(text: &str, pos: usize) -> usize {
    pos + text[pos..].chars().next().map_or(1, char::len_utf8)
}

/// Attributes from byte `pos` to the tag's `>`; `None` for a tag that never closes.
fn parse_attributes(text: &str, regions: &[(usize, usize)], start: usize, mut pos: usize, name: &str) -> Option<(StartTag, usize)> {
    let mut tag = StartTag { name: name.to_string(), start, attrs: Vec::new(), dynamic: false };
    loop {
        pos = text.len() - text[pos..].trim_start().len();
        let rest = &text[pos..];
        let c = rest.chars().next()?;
        if rest.starts_with("<%") {
            tag.dynamic = true;
            pos = erb_end(text, pos)?;
            continue;
        }
        if c == '>' || rest.starts_with("/>") {
            return Some((tag, pos + if c == '>' { 1 } else { 2 }));
        }
        if c == '<' {
            return None;
        }
        let name_end = rest.find(|c: char| c.is_whitespace() || matches!(c, '=' | '>' | '/' | '<')).map_or(text.len(), |e| pos + e);
        let attr = text[pos..name_end].to_ascii_lowercase();
        pos = if name_end > pos { name_end } else { next_char(text, pos) };
        let mut value = None;
        if text[pos..].trim_start().starts_with('=') {
            pos += text[pos..].find('=')? + 1;
            pos = text.len() - text[pos..].trim_start().len();
            let (v, end) = match text[pos..].chars().next()? {
                q @ ('"' | '\'') => {
                    let close = quoted_end(text, regions, pos + 1, q)?;
                    ((pos + 1, close), close + 1)
                }
                // An unquoted value may be (or hold) ERB, whose `%>` doesn't end the tag.
                _ => {
                    let mut end = pos;
                    while let Some(c) = text[end..].chars().next().filter(|c| !c.is_whitespace() && *c != '>') {
                        end = if text[end..].starts_with("<%") { erb_end(text, end)? } else { end + c.len_utf8() };
                    }
                    ((pos, end), end)
                }
            };
            value = Some(v);
            pos = end;
        }
        tag.attrs.push(Attribute { name: attr, value });
    }
}

/// The byte past the `%>` that closes the ERB tag opening at `pos`.
fn erb_end(text: &str, pos: usize) -> Option<usize> {
    text[pos..].find("%>").map(|e| pos + e + 2)
}

/// The closing quote of a value starting at `pos`; quotes inside ERB tags don't count.
fn quoted_end(text: &str, regions: &[(usize, usize)], pos: usize, quote: char) -> Option<usize> {
    text[pos..].match_indices(quote).map(|(i, _)| pos + i).find(|&at| !inside(regions, at))
}

/// `image_tag` calls in output tags whose options give no `alt:` (a `**splat` may).
fn image_tags_without_alt<'a>(
    text: &'a str,
    spans: &'a [(usize, usize)],
    range: &'a impl Fn(usize, usize) -> Range,
) -> impl Iterator<Item = Diagnostic> + 'a {
    spans.iter().filter_map(move |&(s, e)| {
        let code = &text[s..e.min(text.len())];
        let call = IMAGE_TAG.find(code)?;
        (!ALT_OPTION.is_match(&code[call.end()..])).then(|| {
            coded(
                warning(range(s + call.start(), s + call.end()), format!("`image_tag` without `alt:`: {ALT_RULE}")),
                code::IMG_MISSING_ALT,
            )
        })
    })
}

/// Static ids used twice that no ERB branch separates; `<template>` content isn't in the document yet.
fn duplicate_ids(
    text: &str,
    regions: &[(usize, usize)],
    spans: &[(usize, usize)],
    tags: &[StartTag],
    range: &impl Fn(usize, usize) -> Range,
) -> Vec<Diagnostic> {
    let templates = template_bodies(text, regions);
    let ids = tags
        .iter()
        .filter(|t| !inside(&templates, t.start))
        .flat_map(|t| &t.attrs)
        .filter(|a| a.name == "id")
        .filter_map(|a| a.value)
        .filter(|&(s, e)| s < e && !text[s..e].contains("<%"));
    // Each id's latest earlier occurrence.
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::new();
    for (s, e) in ids {
        let id = &text[s..e];
        if let Some(first) = seen.insert(id, s) {
            if !branch_between(text, spans, first, s) {
                let message = format!(
                    "Duplicate id `{id}` (first on line {}): the value must be unique amongst all the IDs in the element's tree (HTML 3.2.6)",
                    range(first, first).start.line + 1
                );
                out.push(coded(warning(range(s, e), message), code::DUPLICATE_ID));
            }
        }
    }
    out
}

fn template_bodies(text: &str, regions: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(open) = lower[at..].find("<template").map(|i| at + i) {
        at = open + 1;
        if inside(regions, open) {
            continue;
        }
        let close = lower[open..].find("</template").map_or(text.len(), |e| open + e);
        out.push((open, close));
        at = close.max(at);
    }
    out
}

/// Any ERB control flow (`if`, `else`, `when`, a loop) between the two offsets, which may render only one of them.
fn branch_between(text: &str, spans: &[(usize, usize)], from: usize, to: usize) -> bool {
    let first = spans.partition_point(|&(s, _)| s <= from);
    spans[first..].iter().take_while(|&&(s, _)| s < to).any(|&(s, e)| {
        let code = &text[s..e.min(text.len())];
        !matches!(analyze(code).0, BlockEffect::Neutral)
            || code.split_whitespace().next().is_some_and(|w| matches!(w, "else" | "elsif" | "when" | "in"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(text: &str) -> Vec<(u32, String)> {
        diagnostics(text).into_iter().map(|d| (d.range.start.line, crate::util::code(&d).unwrap_or_default().to_string())).collect()
    }

    #[test]
    fn img_alt_and_image_tag() {
        assert_eq!(codes("<img src=\"/a.png\">"), [(0, code::IMG_MISSING_ALT.to_string())]);
        assert!(codes("<img src=\"/a.png\" alt=\"\">\n<img <%= attrs %>>\n<!-- <img src=x> -->\n<%# <img src=x> %>").is_empty());
        assert_eq!(codes("<%= image_tag \"a.png\", class: \"x\" %>"), [(0, code::IMG_MISSING_ALT.to_string())]);
        assert!(codes("<img src=\"/a.png\" generator-unable-to-provide-required-alt=\"\">").is_empty());
        assert!(codes("<%= image_tag \"a.png\", alt: \"A\" %><%= image_tag(x, **opts) %>").is_empty());
        assert!(codes("<script>const s = '<img src=x>'</script>").is_empty());
        assert!(codes("<img src=\"/a.png\" generator-unable-to-provide-required-alt>").is_empty());
        assert_eq!(codes("<img src=\"/a.png\" generator-unable-to-provide-required-alt=\"x\">").len(), 1);
        // A no-break or ideographic space after the name is part of an attribute name, not a crash.
        assert_eq!(codes("<img\u{a0}src=\"a\">\n<div\u{3000}class=\"x\">"), [(0, code::IMG_MISSING_ALT.to_string())]);
    }

    #[test]
    fn messages_quote_the_standard() {
        let message = |text: &str| diagnostics(text).remove(0).message;
        assert_eq!(message("<img src=x>"), "`<img>` without `alt`: the alt attribute must be specified (HTML 4.8.4.4.1)");
        assert_eq!(message("<%= image_tag \"a\" %>"), "`image_tag` without `alt:`: the alt attribute must be specified (HTML 4.8.4.4.1)");
        assert_eq!(
            message("<i id=a></i>\n<b id=a></b>"),
            "Duplicate id `a` (first on line 1): the value must be unique amongst all the IDs in the element's tree (HTML 3.2.6)"
        );
    }

    #[test]
    fn duplicate_ids_outside_branches() {
        assert_eq!(codes("<div id=\"a\"></div>\n<p id=\"a\"></p>"), [(1, code::DUPLICATE_ID.to_string())]);
        assert!(codes("<% if x %><div id=\"a\"></div><% else %><div id=\"a\"></div><% end %>").is_empty());
        assert!(codes("<div id=\"a-<%= i %>\"></div><div id=\"a-<%= i %>\"></div>").is_empty());
        assert!(codes("<div id=\"a\" data-x=\"<%= \"id='a'\" %>\"></div>").is_empty());
        assert!(codes("<%# <div id=\"a\"></div><div id=\"a\"></div> %>").is_empty());
        assert!(codes("<template><div id=\"a\"></div></template><div id=\"a\"></div>").is_empty());
    }

    #[test]
    fn regions_and_lookup() {
        let text = "<p><%# c %><!-- x --><script type=\"<%= t %>\">a < b</script><b>";
        let regions = non_markup(text);
        let at = |needle: &str| text.find(needle).unwrap();
        assert!(inside(&regions, at("c %")) && inside(&regions, at("x --")) && inside(&regions, at("a < b")));
        assert!(!inside(&regions, at("<p>")) && !inside(&regions, at("<b>")) && !inside(&regions, at("</script>")));
        assert!(inside(&non_markup("<style>.a{"), 9));
    }
}
