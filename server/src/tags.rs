//! Unclosed and mismatched HTML tags in ERB templates with an HTML host.

use crate::erb;
use crate::ruby::{analyze, BlockEffect};
use crate::util::{coded, offset_to_position, warning};
use regex::Regex;
use std::sync::LazyLock;
use tower_lsp::lsp_types::{Diagnostic, Range};

/// Stable diagnostic codes, shared with ERB's `rubyTemplates.erb.disabledDiagnostics`.
pub mod code {
    pub const UNEXPECTED_CLOSE_TAG: &str = "unexpected-close-tag";
    pub const MISMATCHED_CLOSE_TAG: &str = "mismatched-close-tag";
    pub const UNCLOSED_ELEMENT: &str = "unclosed-element";
}

pub const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];
// End tags HTML lets you omit; never tracked, so they can't be reported.
pub(crate) const OPTIONAL_END: &[&str] = &[
    "li", "p", "td", "tr", "th", "option", "dt", "dd", "thead", "tbody", "tfoot", "colgroup", "rp", "rt", "optgroup", "html", "head",
    "body",
];
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title"];

// Block helpers whose body is JS/CSS, not HTML; the source has no literal `<script>` to skip.
static RAW_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(javascript_tag|tag\.(script|style)|content_tag[ (]\s*:(script|style))\b").unwrap());

#[derive(Clone)]
pub(crate) struct Tag {
    pub name: String,
    pub start: usize,
    pub end: usize,
}

/// An open ERB block: the tag stack on entry and the tag names each finished branch left open.
/// `in_branch` is false between `case x` and its first `when`, which is not a branch.
pub(crate) struct Block {
    pub entry: Vec<Tag>,
    branch_ends: Vec<Vec<String>>,
    has_else: bool,
    in_branch: bool,
}

pub(crate) struct ErbEvent<'a> {
    pub at: usize,
    pub end: usize,
    pub effect: BlockEffect,
    pub keyword: &'a str,
}

/// An ERB template's ERB tags and HTML markup, in source order.
pub(crate) enum Token<'a> {
    Erb(ErbEvent<'a>),
    /// A start tag; `pairs` is false for void, self-closing and optional-end elements, which the stack skips.
    Open {
        tag: Tag,
        pairs: bool,
        quoted: Vec<(usize, usize)>,
    },
    Close {
        tag: Tag,
        quoted: Vec<(usize, usize)>,
    },
    /// `<script>`, `<style>`, `<textarea>`, `<title>` and their unparsed body, up to `</name` (`None`: never closed).
    Raw {
        tag: Tag,
        close: Option<(usize, usize)>,
    },
    /// `<!-- … -->`, `<!DOCTYPE …>`, `<?xml …?>`.
    Comment {
        start: usize,
        end: usize,
    },
    /// A `<name` whose `>` never comes: nothing after it can be read as HTML.
    Unterminated,
}

fn names(stack: &[Tag]) -> Vec<String> {
    stack.iter().map(|t| t.name.clone()).collect()
}

fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

struct TagEnd {
    end: usize,
    self_closing: bool,
    quoted: Vec<(usize, usize)>,
}

/// Byte offset just past the `>` closing a tag (skipping quoted attribute values), whether it is
/// self-closing, and the quoted values' spans. A `/` ending an unquoted value (`<a href=/>`) belongs to the
/// value, not to `/>`.
fn tag_end(html: &[u8], from: usize) -> Option<TagEnd> {
    let mut i = from;
    let mut prev = 0u8;
    let mut unquoted = false;
    let mut quoted = Vec::new();
    while let Some(&b) = html.get(i) {
        match b {
            b'>' => return Some(TagEnd { end: i + 1, self_closing: prev == b'/' && !unquoted, quoted }),
            b'"' | b'\'' if prev == b'=' => {
                let close = find(html, i + 1, &[b])?;
                quoted.push((i, close + 1));
                i = close;
            }
            _ if b.is_ascii_whitespace() => unquoted = false,
            _ if prev == b'=' => unquoted = true,
            _ => {}
        }
        if !b.is_ascii_whitespace() {
            prev = b;
        }
        i += 1;
    }
    None
}

/// HTML with ERB tags blanked out, plus every ERB tag as an event, in order.
fn split(text: &str) -> Option<(Vec<u8>, Vec<ErbEvent<'_>>)> {
    let tree = erb::parse(text)?;
    let root = tree.root_node();
    // Broken ERB is reported elsewhere; tag positions would be guesswork.
    if root.has_error() {
        return None;
    }
    let mut html = text.as_bytes().to_ascii_lowercase();
    let mut events = Vec::new();
    let mut cursor = root.walk();
    for node in root.children(&mut cursor) {
        if node.kind() == "content" {
            continue;
        }
        html[node.byte_range()].iter_mut().filter(|b| **b != b'\n').for_each(|b| *b = b' ');
        let mut c2 = node.walk();
        let code = node.children(&mut c2).find(|n| n.kind() == "code");
        let (effect, keyword) = match code {
            Some(code) => {
                let snippet = &text[code.byte_range()];
                // ponytail: a JS/CSS block body anywhere skips the whole file; per-block raw spans would keep checking.
                if RAW_BLOCK.is_match(snippet) {
                    return None;
                }
                (analyze(snippet).0, snippet.split_whitespace().next().unwrap_or(""))
            }
            None => (BlockEffect::Neutral, ""),
        };
        events.push(ErbEvent { at: node.start_byte(), end: node.end_byte(), effect, keyword });
    }
    Some((html, events))
}

/// The template's tokens; `None` for broken ERB or a JS/CSS block helper, where tag positions are guesswork.
pub(crate) fn tokens(text: &str) -> Option<Vec<Token<'_>>> {
    let (html, events) = split(text)?;
    let mut events = events.into_iter().peekable();
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(start) = find(&html, i, b"<") {
        tokens.extend(std::iter::from_fn(|| events.next_if(|e| e.at < start)).map(Token::Erb));
        i = start + 1;
        if html[start..].starts_with(b"<!--") {
            i = find(&html, start + 4, b"-->").map_or(html.len(), |p| p + 3);
            tokens.push(Token::Comment { start, end: i });
            continue;
        }
        if html[start..].starts_with(b"<!") || html[start..].starts_with(b"<?") {
            i = find(&html, start, b">").map_or(html.len(), |p| p + 1);
            tokens.push(Token::Comment { start, end: i });
            continue;
        }
        let closing = html.get(i) == Some(&b'/');
        let name_start = i + closing as usize;
        if !html.get(name_start).is_some_and(u8::is_ascii_alphabetic) {
            continue;
        }
        let name_len =
            html[name_start..].iter().take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.')).count();
        let name = String::from_utf8_lossy(&html[name_start..name_start + name_len]).into_owned();
        let Some(TagEnd { end, self_closing, quoted }) = tag_end(&html, name_start + name_len) else {
            tokens.push(Token::Unterminated);
            return Some(tokens);
        };
        i = end;
        let tag = Tag { name, start, end };
        if closing {
            tokens.push(Token::Close { tag, quoted });
        } else if RAW_TEXT.contains(&tag.name.as_str()) && !self_closing {
            let close = format!("</{}", tag.name);
            let close = find(&html, end, close.as_bytes()).map(|close_start| {
                let after = close_start + close.len();
                (close_start, after + (html.get(after) == Some(&b'>')) as usize)
            });
            i = close.map_or(html.len(), |(_, close_end)| close_end);
            tokens.push(Token::Raw { tag, close });
        } else {
            let name = tag.name.as_str();
            let pairs = !self_closing && !VOID.contains(&name) && !OPTIONAL_END.contains(&name);
            tokens.push(Token::Open { tag, pairs, quoted });
        }
    }
    tokens.extend(events.map(Token::Erb));
    Some(tokens)
}

/// Track the tag stack through ERB branches. Returns false when the stack is no longer certain; the blocks are closed
/// anyway, and the stack is the one the last branch left.
// ponytail: a block whose branches (including the implicit empty `else` of `if`/loops) leave different tags
// open makes the stack path-dependent, so checking stops there. Conditional wrappers
// (`<% if a %><div><% end %>…<% if a %></div><% end %>`) are therefore unchecked; per-path stacks would cover them.
pub(crate) fn apply(event: &ErbEvent, stack: &mut Vec<Tag>, blocks: &mut Vec<Block>) -> bool {
    match event.effect {
        BlockEffect::Open(n) => {
            for _ in 0..n {
                let in_branch = event.keyword != "case";
                blocks.push(Block { entry: stack.clone(), branch_ends: Vec::new(), has_else: false, in_branch });
            }
        }
        BlockEffect::Continue => {
            let Some(block) = blocks.last_mut() else { return false };
            if block.in_branch {
                block.branch_ends.push(names(stack));
            }
            block.in_branch = true;
            block.has_else |= event.keyword == "else";
            *stack = block.entry.clone();
        }
        BlockEffect::Close(n) => {
            let mut agree = true;
            for _ in 0..n {
                let Some(block) = blocks.pop() else { return false };
                let open = names(stack);
                agree &= block.branch_ends.iter().all(|b| *b == open) && (block.has_else || names(&block.entry) == open);
            }
            return agree;
        }
        BlockEffect::Neutral => {}
    }
    true
}

/// A matched `<name …>…</name>` pair, as byte offsets.
pub struct Element {
    pub name: String,
    pub start: usize,
    pub open_end: usize,
    pub close_start: usize,
    pub end: usize,
}

/// `partial`: split-wrapper partials (`_header` opens a `<div>` that `_footer` closes) are legitimate,
/// so only mismatches are reported there.
pub fn diagnostics(text: &str, partial: bool) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    scan(text, partial, &mut out, &mut Vec::new());
    out
}

/// Matched element pairs up to the first point where the tag stack becomes uncertain.
pub fn elements(text: &str) -> Vec<Element> {
    let mut elements = Vec::new();
    scan(text, true, &mut Vec::new(), &mut elements);
    elements
}

/// The open and close tag names of the element whose name holds byte `offset`, for linked editing.
pub fn linked_names(text: &str, offset: usize) -> Option<[(usize, usize); 2]> {
    elements(text).into_iter().find_map(|e| {
        let open = (e.start + 1, e.start + 1 + e.name.len());
        let close = (e.close_start + 2, e.close_start + 2 + e.name.len());
        // An implied end (`<li>a<li>`) has no `</li>` to edit along.
        let names = |(s, end): (usize, usize)| text.get(s..end).is_some_and(|n| n.eq_ignore_ascii_case(&e.name));
        let holds = |(s, end): (usize, usize)| (s..=end).contains(&offset);
        (names(open) && names(close) && (holds(open) || holds(close))).then_some([open, close])
    })
}

fn scan(text: &str, partial: bool, out: &mut Vec<Diagnostic>, elements: &mut Vec<Element>) {
    let Some(tokens) = tokens(text) else { return };
    let range = |start, end| Range::new(offset_to_position(text, start), offset_to_position(text, end));
    let mut stack: Vec<Tag> = Vec::new();
    // Optional-end elements with an explicit end tag; paired for `elements` only, invisible to diagnostics.
    let mut optional: Vec<Tag> = Vec::new();
    let mut blocks: Vec<Block> = Vec::new();
    for token in tokens {
        match token {
            Token::Erb(event) => {
                if !apply(&event, &mut stack, &mut blocks) {
                    return;
                }
            }
            Token::Comment { .. } | Token::Raw { close: None, .. } => {}
            Token::Unterminated => return,
            Token::Raw { tag, close: Some((close_start, end)) } => {
                elements.push(Element { name: tag.name, start: tag.start, open_end: tag.end, close_start, end })
            }
            Token::Open { tag, pairs, .. } => {
                if pairs {
                    stack.push(tag);
                } else if OPTIONAL_END.contains(&tag.name.as_str()) {
                    optional.push(tag);
                }
            }
            Token::Close { tag, .. } if VOID.contains(&tag.name.as_str()) => {}
            Token::Close { tag: Tag { name, start, end }, .. } if OPTIONAL_END.contains(&name.as_str()) => {
                if let Some(at) = optional.iter().rposition(|t| t.name == name) {
                    elements.push(Element { name, start: optional[at].start, open_end: optional[at].end, close_start: start, end });
                    optional.truncate(at);
                }
            }
            Token::Close { tag: Tag { name, start, end }, .. } => match stack.last() {
                None if partial => {}
                None => {
                    out.push(coded(warning(range(start, end), format!("Unexpected closing tag `</{name}>`")), code::UNEXPECTED_CLOSE_TAG))
                }
                Some(top) if top.name == name => {
                    elements.push(Element { name, start: top.start, open_end: top.end, close_start: start, end });
                    stack.pop();
                }
                Some(top) => {
                    let line = offset_to_position(text, top.start).line + 1;
                    let message = format!("Closing tag `</{name}>` doesn't match `<{}>` (line {line})", top.name);
                    out.push(coded(warning(range(start, end), message), code::MISMATCHED_CLOSE_TAG));
                    // ponytail: first mismatch only; recovery guesses would cascade into noise.
                    return;
                }
            },
        }
    }
    if blocks.is_empty() && !partial {
        let unclosed = |t: &Tag| coded(warning(range(t.start, t.end), format!("`<{}>` is never closed", t.name)), code::UNCLOSED_ELEMENT);
        out.extend(stack.iter().map(unclosed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Position;

    fn diagnostics(text: &str) -> Vec<Diagnostic> {
        super::diagnostics(text, false)
    }

    fn messages(text: &str) -> Vec<String> {
        diagnostics(text).into_iter().map(|d| d.message).collect()
    }

    #[test]
    fn reports_mismatch_stray_and_unclosed() {
        assert_eq!(messages("<div>\n  <span>x</div>"), ["Closing tag `</div>` doesn't match `<span>` (line 2)"]);
        assert_eq!(messages("<div></div></span>"), ["Unexpected closing tag `</span>`"]);
        assert_eq!(messages("<section><div></div>"), ["`<section>` is never closed"]);
        // Tags crossing a block boundary inconsistently with the outside are still caught.
        assert_eq!(messages("<% if a %><div><% else %><div><% end %></span>"), ["Closing tag `</span>` doesn't match `<div>` (line 1)"]);
        assert_eq!(
            messages("<% case x %>\n<% when 1 %><a><% else %><a><% end %></span>"),
            ["Closing tag `</span>` doesn't match `<a>` (line 2)"]
        );
        assert_eq!(messages("<% items.each do |i| %><div><% end %></div>"), Vec::<String>::new());
        assert_eq!(messages("<ul><% items.each do |i| %><li><%= i %><% end %>"), ["`<ul>` is never closed"]);
    }

    #[test]
    fn review_false_positives() {
        assert!(messages("<a href=/>Home</a><a href=/users/>U</a>").is_empty());
        assert!(messages("<br/><input disabled/><x-icon a=b />").is_empty());
        assert!(messages("<%= javascript_tag do %>\n el.innerHTML = \"<span>\" + x + \"</div>\"; if (a<b) {}\n<% end %>").is_empty());
        assert!(messages("<%= content_tag(:script) do %>\"</div>\"<% end %>").is_empty());
        let header = "<div class=\"wrapper\"><header>x</header>";
        assert!(super::diagnostics(header, true).is_empty());
        assert!(super::diagnostics("</div></body>", true).is_empty());
        assert_eq!(super::diagnostics("<div></span>", true).len(), 1);
    }

    #[test]
    fn position_uses_utf16_after_multibyte() {
        let d = diagnostics("😀é<div>\né</span>");
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].range, Range::new(Position::new(1, 1), Position::new(1, 8)));
    }

    #[test]
    fn no_false_positives() {
        for ok in [
            "<% if a %><div class=\"x\"><% else %><div class=\"y\"><% end %>body</div>",
            "<% if a %><div class=\"x\"><% elsif b %><div><% else %><div><% end %></div>",
            "<% case x %><% when 1 %><a href=\"/1\"><% when 2 %><a href=\"/2\"><% else %><a><% end %>x</a>",
            "<div><% if a %></div><div class=\"b\"><% end %></div>",
            "<% if a %><div><% end %>x<% if a %></div><% end %>",
            "<% if a %><div><% else %><span><% end %></div>",
            "<br><img src=\"x\"><input type=\"text\"><hr/>",
            "<svg><path d=\"M0\" /><use href=\"#a\"/></svg>",
            "<!-- <div> --><!DOCTYPE html><?xml version=\"1.0\"?>",
            "<script>if (a < b) { x = \"</div>\" }</script><style>a > b {}</style>",
            "<textarea><div></textarea><title>a <b></title>",
            "<div title=\"a > b\" data-x='</span>'>x</div>",
            "<div class=\"<%= x > 1 ? 'a' : 'b' %>\" <%= attrs %>>x</div>",
            "<ul><li>a<li>b</ul><p>one<p>two<table><tr><td>1</table>",
            "<html><head><body><DIV>x</div>",
            "<%= form_with model: @u do |f| %><div><%= f.text_field :x %></div><% end %>",
            "<% content = capture do %><span>x</span><% end %><%# <div> %>",
            "<%= raw '<div>' %><% x = '</span>' %>",
            "a < b && c <= d",
            "<% if a %><div><% end %>", // path-dependent: unchecked
        ] {
            assert!(diagnostics(ok).is_empty(), "{ok}: {:?}", messages(ok));
        }
    }

    #[test]
    fn elements_pair_optional_end_and_raw_text() {
        let pairs: Vec<(String, usize, usize)> =
            elements("<ul><li>a<li>b</li></ul><script>x</script>").into_iter().map(|e| (e.name, e.start, e.end)).collect();
        assert_eq!(pairs, [("li".to_string(), 9, 19), ("ul".to_string(), 0, 24), ("script".to_string(), 24, 42)]);
    }

    #[test]
    fn linked_tag_names() {
        let text = "<div class=\"a\"><% if x %><SPAN>b</span><% end %></div><ul><li>a<li>b</ul>";
        let at = |needle: &str| text.find(needle).unwrap() + 1;
        let names = |offset| linked_names(text, offset).map(|r| r.map(|(s, e)| &text[s..e]));
        assert_eq!(names(at("div")), Some(["div", "div"]));
        assert_eq!(names(text.rfind("div").unwrap()), Some(["div", "div"]));
        assert_eq!(names(at("SPAN")), Some(["SPAN", "span"]));
        assert_eq!(names(at("li")), None);
        assert_eq!(names(at("class")), None);
    }

    #[test]
    fn unterminated_raw_text_close_does_not_swallow_next_tag() {
        assert!(diagnostics("<script>x</script\n<div>y</div>").is_empty());
    }

    #[test]
    fn broken_erb_is_skipped() {
        assert!(diagnostics("<div><% if a %>").is_empty());
        assert!(diagnostics("<div><% foo").is_empty());
    }
}
