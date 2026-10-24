//! HTML checks for ERB views with an HTML host: `alt` text, duplicate ids, `image_tag` for computed sources and
//! unescaped output. Names follow Herb's rules (`html-img-require-alt`, `html-no-duplicate-ids`,
//! `erb-prefer-image-tag-helper`, `erb-no-unsafe-raw`).

use crate::erb;
use crate::ruby::{analyze, BlockEffect};
use crate::util::{coded, offset_to_position, warning};
use regex::Regex;
use std::sync::LazyLock;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Range};

/// Stable diagnostic codes, shared with ERB's `rubyTemplates.erb.disabledDiagnostics`.
pub mod code {
    pub const IMG_MISSING_ALT: &str = "img-missing-alt";
    pub const DUPLICATE_ID: &str = "duplicate-id";
    pub const PREFER_IMAGE_TAG: &str = "prefer-image-tag";
    pub const UNSAFE_OUTPUT: &str = "unsafe-output";
}

const RAW_TEXT: &[&str] = &["script", "style", "textarea"];
static START_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<([a-zA-Z][\w:-]*)").unwrap());
static IMAGE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bimage_tag\b").unwrap());
static ALT_OPTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\balt:|:alt\s*=>|["']alt["']\s*=>|\*\*"#).unwrap());
static RAW_CALL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[^\w.])raw[\s(]|\.html_safe\b").unwrap());

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
    let spans = erb::code_spans(text);
    let tags = start_tags(text, &spans);
    let range = |start: usize, end: usize| Range::new(offset_to_position(text, start), offset_to_position(text, end));
    let mut out = Vec::new();
    for tag in tags.iter().filter(|t| t.name == "img") {
        let tag_range = range(tag.start, tag.start + 4);
        if !tag.dynamic && !tag.attrs.iter().any(|a| a.name == "alt") {
            out.push(coded(warning(tag_range, "`<img>` without `alt` text"), code::IMG_MISSING_ALT));
        }
        let computed = tag.attrs.iter().find(|a| a.name == "src").and_then(|a| a.value).is_some_and(|(s, e)| text[s..e].contains("<%="));
        if computed {
            let message = "Computed `src`: `image_tag` escapes it and takes `alt:` as an option";
            out.push(info(coded(warning(tag_range, message), code::PREFER_IMAGE_TAG)));
        }
    }
    out.extend(image_tags_without_alt(text, &spans, &range));
    out.extend(duplicate_ids(text, &spans, &tags, &range));
    out.extend(unsafe_output(text, &spans, &range));
    out
}

fn info(mut d: Diagnostic) -> Diagnostic {
    d.severity = Some(DiagnosticSeverity::INFORMATION);
    d
}

fn in_code(spans: &[(usize, usize)], at: usize) -> bool {
    spans.iter().any(|&(s, e)| (s.saturating_sub(2)..=e).contains(&at))
}

/// Start tags outside ERB code, comments and raw-text elements, with their attributes.
fn start_tags(text: &str, spans: &[(usize, usize)]) -> Vec<StartTag> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(cap) = START_TAG.captures_at(text, from) {
        let (Some(whole), Some(name)) = (cap.get(0), cap.get(1)) else { break };
        from = whole.end();
        if in_code(spans, whole.start()) || in_comment(text, whole.start()) {
            continue;
        }
        let name = name.as_str().to_ascii_lowercase();
        let Some((tag, end)) = parse_attributes(text, spans, whole.start(), whole.end(), &name) else { continue };
        from = end;
        if RAW_TEXT.contains(&name.as_str()) {
            from = text[end..].to_ascii_lowercase().find(&format!("</{name}")).map_or(text.len(), |at| end + at);
        }
        out.push(tag);
    }
    out
}

fn in_comment(text: &str, at: usize) -> bool {
    text[..at].rfind("<!--").is_some_and(|open| !text[open..at].contains("-->"))
}

/// Attributes from byte `pos` to the tag's `>`; `None` for a tag that never closes.
fn parse_attributes(text: &str, spans: &[(usize, usize)], start: usize, mut pos: usize, name: &str) -> Option<(StartTag, usize)> {
    let bytes = text.as_bytes();
    let mut tag = StartTag { name: name.to_string(), start, attrs: Vec::new(), dynamic: false };
    loop {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let b = *bytes.get(pos)?;
        if text[pos..].starts_with("<%") {
            tag.dynamic = true;
            pos = text[pos..].find("%>").map(|e| pos + e + 2)?;
            continue;
        }
        if b == b'>' || text[pos..].starts_with("/>") {
            return Some((tag, pos + if b == b'>' { 1 } else { 2 }));
        }
        if b == b'<' {
            return None;
        }
        let name_end = text[pos..].find(|c: char| c.is_whitespace() || matches!(c, '=' | '>' | '/' | '<')).map_or(text.len(), |e| pos + e);
        let attr = text[pos..name_end].to_ascii_lowercase();
        pos = name_end.max(pos + 1);
        let mut value = None;
        if text[pos..].trim_start().starts_with('=') {
            pos += text[pos..].find('=')? + 1;
            pos += text[pos..].len() - text[pos..].trim_start().len();
            value = Some(match bytes.get(pos)? {
                q @ (b'"' | b'\'') => {
                    let close = quoted_end(text, spans, pos + 1, *q)?;
                    let v = (pos + 1, close);
                    pos = close + 1;
                    v
                }
                _ => {
                    let end = text[pos..].find(|c: char| c.is_whitespace() || c == '>').map_or(text.len(), |e| pos + e);
                    let v = (pos, end);
                    pos = end;
                    v
                }
            });
        }
        tag.attrs.push(Attribute { name: attr, value });
    }
}

/// The closing quote of a value starting at `pos`; quotes inside ERB tags don't count.
fn quoted_end(text: &str, spans: &[(usize, usize)], pos: usize, quote: u8) -> Option<usize> {
    text[pos..].bytes().enumerate().map(|(i, b)| (pos + i, b)).find(|&(at, b)| b == quote && !in_code(spans, at)).map(|(at, _)| at)
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
        (!ALT_OPTION.is_match(&code[call.end()..]))
            .then(|| coded(warning(range(s + call.start(), s + call.end()), "`image_tag` without `alt:`"), code::IMG_MISSING_ALT))
    })
}

/// Static ids used twice in a row of markup that no ERB branch separates.
fn duplicate_ids(text: &str, spans: &[(usize, usize)], tags: &[StartTag], range: &impl Fn(usize, usize) -> Range) -> Vec<Diagnostic> {
    let ids = tags
        .iter()
        .flat_map(|t| &t.attrs)
        .filter(|a| a.name == "id")
        .filter_map(|a| a.value)
        .filter(|&(s, e)| s < e && !text[s..e].contains("<%"));
    let mut seen: Vec<(&str, usize)> = Vec::new();
    let mut out = Vec::new();
    for (s, e) in ids {
        let id = &text[s..e];
        if let Some(&(_, first)) = seen.iter().rev().find(|(seen_id, _)| *seen_id == id) {
            if !branch_between(text, spans, first, s) {
                let message = format!("Duplicate id `{id}` (first on line {})", offset_to_position(text, first).line + 1);
                out.push(coded(warning(range(s, e), message), code::DUPLICATE_ID));
            }
        }
        seen.push((id, s));
    }
    out
}

/// Any ERB control flow (`if`, `else`, `when`, a loop) between the two offsets, which may render only one of them.
fn branch_between(text: &str, spans: &[(usize, usize)], from: usize, to: usize) -> bool {
    spans.iter().filter(|&&(s, _)| s > from && s < to).any(|&(s, e)| {
        let code = &text[s..e.min(text.len())];
        !matches!(analyze(code).0, BlockEffect::Neutral)
            || code.split_whitespace().next().is_some_and(|w| matches!(w, "else" | "elsif" | "when" | "in"))
    })
}

/// `<%==`, `raw(…)` and `.html_safe`: output that skips HTML escaping.
fn unsafe_output<'a>(
    text: &'a str,
    spans: &'a [(usize, usize)],
    range: &'a impl Fn(usize, usize) -> Range,
) -> impl Iterator<Item = Diagnostic> + 'a {
    spans.iter().filter_map(move |&(s, e)| {
        let code = &text[s..e.min(text.len())];
        // `code_spans` starts after `<%`, so `<%==` leaves `==` in front of the code.
        let raw_tag = code.starts_with("==");
        let at = if raw_tag { Some((s - 2, s + 2)) } else { RAW_CALL.find(code).map(|m| (s + m.start(), s + m.end())) };
        at.map(|(a, b)| {
            info(coded(warning(range(a, b), "Unescaped output: make sure the value can't hold user input"), code::UNSAFE_OUTPUT))
        })
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
        assert!(codes("<img src=\"/a.png\" alt=\"\">\n<img <%= attrs %>>\n<!-- <img src=x> -->").is_empty());
        assert_eq!(codes("<img src=\"<%= image_path('a') %>\" alt=\"A\">"), [(0, code::PREFER_IMAGE_TAG.to_string())]);
        assert_eq!(codes("<%= image_tag \"a.png\", class: \"x\" %>"), [(0, code::IMG_MISSING_ALT.to_string())]);
        assert!(codes("<%= image_tag \"a.png\", alt: \"A\" %><%= image_tag(x, **opts) %>").is_empty());
        assert!(codes("<script>const s = '<img src=x>'</script>").is_empty());
    }

    #[test]
    fn duplicate_ids_outside_branches() {
        assert_eq!(codes("<div id=\"a\"></div>\n<p id=\"a\"></p>"), [(1, code::DUPLICATE_ID.to_string())]);
        assert!(codes("<% if x %><div id=\"a\"></div><% else %><div id=\"a\"></div><% end %>").is_empty());
        assert!(codes("<div id=\"a-<%= i %>\"></div><div id=\"a-<%= i %>\"></div>").is_empty());
        assert!(codes("<div id=\"a\" data-x=\"<%= \"id='a'\" %>\"></div>").is_empty());
    }

    #[test]
    fn unsafe_output_markers() {
        let unsafe_lines = |t: &str| codes(t).into_iter().filter(|(_, c)| c == code::UNSAFE_OUTPUT).map(|(l, _)| l).collect::<Vec<_>>();
        assert_eq!(unsafe_lines("<%== x %>\n<%= raw(x) %>\n<%= x.html_safe %>\n<%= draw(x) %>\n<%= x.raw %>"), [0, 1, 2]);
    }
}
