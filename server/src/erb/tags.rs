//! Unclosed and mismatched HTML tags in ERB templates with an HTML host.

use crate::erb;
use crate::html::{NO_END_TAG, TEXT_ELEMENTS};
use crate::ruby::{analyze, BlockEffect};
use crate::util::{coded, warning, LineIndex};
use regex::Regex;
use std::sync::{Arc, LazyLock};
use tower_lsp_server::ls_types::{Diagnostic, Range};

/// Stable diagnostic codes, shared with ERB's `rubyTemplates.erb.disabledDiagnostics`.
pub mod code {
    pub const UNEXPECTED_CLOSE_TAG: &str = "Html/UnexpectedCloseTag";
    pub const MISMATCHED_CLOSE_TAG: &str = "Html/MismatchedCloseTag";
    pub const UNCLOSED_ELEMENT: &str = "Html/UnclosedElement";
}

// End tags HTML lets you omit; never tracked, so they can't be reported.
pub(crate) const OPTIONAL_END: &[&str] = &[
    "li", "p", "td", "tr", "th", "option", "dt", "dd", "thead", "tbody", "tfoot", "colgroup", "caption", "rp", "rt", "optgroup", "html",
    "head", "body",
];
// Start tags that close an open `p` (13.2.6.4.7), so its own end tag later has nothing to close.
#[rustfmt::skip]
const CLOSES_P: &[&str] = &[
    "address", "article", "aside", "blockquote", "center", "details", "dialog", "dir", "div", "dl", "fieldset", "figcaption", "figure",
    "footer", "header", "hgroup", "main", "menu", "nav", "ol", "p", "search", "section", "summary", "ul", "h1", "h2", "h3", "h4", "h5",
    "h6", "pre", "listing", "form", "li", "dd", "dt", "plaintext", "table", "hr", "xmp",
];
// Inside these a start tag doesn't close a `p` opened outside (13.2.6.4.7 "has an element in button scope").
const SCOPE_BOUNDARY: &[&str] = &["button", "applet", "object", "template", "marquee", "table", "td", "th", "caption", "svg", "math"];
const HTML_END_TAG_RULE: &str = "HTML 13.2.6.4.7";
const HEADINGS: [&str; 6] = ["h1", "h2", "h3", "h4", "h5", "h6"];
/// Past this many a file is broken beyond what more warnings would help with.
const MAX_TAG_DIAGNOSTICS: usize = 20;

// Block helpers whose body is JS/CSS, not HTML; the source has no literal `<script>` to skip.
static RAW_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(javascript_tag|tag\.(script|style)|content_tag[ (]\s*:(script|style))\b").unwrap());

// A `{ … }` block has no `end` for `raw_block_bodies` to find, so its helper is left unchecked like the formatter does.
static BRACE_BLOCK_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\s*(\|[^|]*\|\s*)?$").unwrap());

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
    tokens_of(text, &analyzed(text), true).map_or_else(Vec::new, |tokens| scan_tokens(text, tokens, partial).0)
}

/// Matched element pairs up to the first point where the tag stack becomes uncertain.
pub fn elements(text: &str) -> Vec<Element> {
    elements_of(text, &analyzed(text))
}

pub(crate) fn elements_of<'a>(text: &'a str, analyzed: &[(erb::ruby::ErbTag<'a>, BlockEffect)]) -> Vec<Element> {
    tokens_of(text, analyzed, true).map_or_else(Vec::new, |tokens| scan_tokens(text, tokens, true).1)
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

/// Every Erubi tag with its block effect; comments and `<%%` literals are `Neutral`.
pub(crate) fn analyzed(text: &str) -> Vec<(erb::ruby::ErbTag<'_>, BlockEffect)> {
    erb::ruby::tags(text)
        .into_iter()
        .map(|t| {
            let effect = if t.is_literal() || t.is_comment() { BlockEffect::Neutral } else { analyze(&text[t.code.clone()]) };
            (t, effect)
        })
        .collect()
}

/// The template's tokens; `None` for broken ERB or a JS/CSS block helper, which the formatter can't re-indent.
pub(crate) fn tokens(text: &str) -> Option<Vec<Token<'_>>> {
    tokens_of(text, &analyzed(text), false)
}

#[derive(Clone)]
pub(crate) struct Tag {
    pub name: String,
    pub start: usize,
    pub end: usize,
    /// Set on a tag an `if` chain left open: only a later chain at the same level with the same conditions closes it.
    pub wrapper: Option<Arc<Wrapper>>,
}

pub(crate) struct Wrapper {
    /// The normalised condition of each branch (`else` for the last one, if any).
    chain: Vec<String>,
    level: usize,
    /// The tags each branch left open, unmarked.
    branches: Vec<Vec<Tag>>,
}

/// How the branches of the ERB blocks an event closed ended, worst last.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Paths {
    Agree,
    /// A conditional wrapper: the stack is certain, but the block has no single end to line up with.
    Wrapper,
    Differ,
}

/// An open ERB block: the tag stack and `p` state on entry, and those each finished branch left.
/// `in_branch` is false between `case x` and its first `when`, which is not a branch.
pub(crate) struct Block {
    pub entry: Vec<Tag>,
    entry_p: PState,
    branch_ends: Vec<Vec<Tag>>,
    branch_p_ends: Vec<PState>,
    has_else: bool,
    in_branch: bool,
    /// The conditions of an `if`/`unless` chain's branches so far; `None` for any other block.
    chain: Option<Vec<String>>,
    /// The wrapper whose open tags this chain's branches have closed so far, in the same order.
    closing: Option<Arc<Wrapper>>,
}

/// Whether a `p` element is open, tracked through ERB branches like the tag stack.
#[derive(Clone, Copy, PartialEq, Default)]
pub(crate) enum PState {
    #[default]
    Closed,
    Open,
    /// Branches disagree, or a scope boundary hides whether a start tag closed it: its end tag isn't checked.
    Unknown,
}

pub(crate) struct ErbEvent<'a> {
    pub at: usize,
    pub end: usize,
    pub effect: BlockEffect,
    pub keyword: &'a str,
    pub code: &'a str,
    pub comment: bool,
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

/// The tag diagnostics and the matched elements of `tokens`, up to the first point where the tag stack becomes uncertain.
fn scan_tokens(text: &str, tokens: Vec<Token<'_>>, partial: bool) -> (Vec<Diagnostic>, Vec<Element>) {
    let mut out = Vec::new();
    let mut elements = Vec::new();
    let index = LineIndex::new(text);
    let range = |start, end| index.range(start, end);
    let mut stack: Vec<Tag> = Vec::new();
    // Optional-end elements with an explicit end tag; paired for `elements` only, invisible to diagnostics.
    let mut optional: Vec<Tag> = Vec::new();
    let mut blocks: Vec<Block> = Vec::new();
    let mut p = PState::Closed;
    // Elements a stray end tag's message already names: `<div>…</dvi>` is one mistake.
    let mut named: Vec<usize> = Vec::new();
    for token in tokens {
        if out.len() >= MAX_TAG_DIAGNOSTICS {
            return (out, elements);
        }
        match token {
            Token::Erb(event) => {
                if apply(&event, &mut stack, &mut p, &mut blocks) == Paths::Differ {
                    return (out, elements);
                }
            }
            Token::Comment { .. } | Token::Raw { close: None, .. } => {}
            Token::Unterminated => return (out, elements),
            Token::Raw { tag, close: Some((close_start, end)) } => {
                elements.push(Element { name: tag.name, start: tag.start, open_end: tag.end, close_start, end })
            }
            Token::Open { tag, pairs, .. } => {
                if CLOSES_P.contains(&tag.name.as_str()) {
                    p = match p {
                        _ if tag.name == "p" => PState::Open,
                        PState::Open if scope_boundary_open(&stack, &optional) => PState::Unknown,
                        PState::Open => PState::Closed,
                        other => other,
                    };
                }
                if pairs {
                    stack.push(tag);
                } else if OPTIONAL_END.contains(&tag.name.as_str()) {
                    optional.push(tag);
                }
            }
            Token::Close { tag: Tag { name, start, end, .. }, .. } if NO_END_TAG.contains(&name.as_str()) => {
                out.push(void_close_tag(&name, range(start, end)));
            }
            Token::Close { tag: Tag { name, start, end, .. }, .. } if OPTIONAL_END.contains(&name.as_str()) => {
                // `<p>a<div>b</div></p>`: the `div` closed the `p`, and the parser inserts an empty one for this end tag.
                if name == "p" {
                    if p == PState::Closed && !partial {
                        out.push(unexpected_close_tag(&name, range(start, end)));
                    }
                    p = PState::Closed;
                }
                if let Some(at) = optional.iter().rposition(|t| t.name == name) {
                    elements.push(Element { name, start: optional[at].start, open_end: optional[at].end, close_start: start, end });
                    optional.truncate(at);
                }
            }
            Token::Close { tag: Tag { name, start, end, .. }, .. } => match stack.last() {
                None if partial => {}
                None => out.push(unexpected_close_tag(&name, range(start, end))),
                Some(top) if top.wrapper.is_some() && (top.name != name || !closes_in_matching_if(top, &blocks)) => return (out, elements),
                Some(top) if top.name == name => {
                    elements.push(Element { name, start: top.start, open_end: top.end, close_start: start, end });
                    stack.pop();
                }
                Some(top) => {
                    let line = index.line_of(top.start) + 1;
                    let message =
                        format!("End tag `</{name}>` doesn't close the open `<{}>` (line {line}) ({HTML_END_TAG_RULE})", top.name);
                    out.push(coded(warning(range(start, end), message), code::MISMATCHED_CLOSE_TAG));
                    let heading = |name: &str| HEADINGS.contains(&name);
                    if heading(&name) && heading(&top.name) {
                        // Any heading's end tag closes the open heading (13.2.6.4.7).
                        stack.pop();
                    } else if stack.iter().any(|t| t.name == name || (heading(&name) && heading(&t.name))) {
                        // Crossing an open element could close more than one way, so checking stops.
                        // Recovering past it would guess, and cascade into noise.
                        return (out, elements);
                    } else {
                        // A stray end tag is dropped, as browsers do.
                        named.push(top.start);
                    }
                }
            },
        }
    }
    if blocks.is_empty() && !partial {
        let unclosed = |t: &Tag| {
            coded(warning(range(t.start, t.end), format!("`<{}>` is never closed ({HTML_END_TAG_RULE})", t.name)), code::UNCLOSED_ELEMENT)
        };
        // A wrapper's closing `if` may be in another file (layout, partial).
        let room = MAX_TAG_DIAGNOSTICS - out.len();
        out.extend(stack.iter().filter(|t| t.wrapper.is_none() && !named.contains(&t.start)).take(room).map(unclosed));
    }
    (out, elements)
}

fn scope_boundary_open(stack: &[Tag], optional: &[Tag]) -> bool {
    stack.iter().chain(optional).any(|t| SCOPE_BOUNDARY.contains(&t.name.as_str()))
}

fn unexpected_close_tag(name: &str, range: Range) -> Diagnostic {
    let message = format!("Unexpected end tag `</{name}>`: no open element of that name ({HTML_END_TAG_RULE})");
    coded(warning(range, message), code::UNEXPECTED_CLOSE_TAG)
}

fn void_close_tag(name: &str, range: Range) -> Diagnostic {
    coded(warning(range, format!("End tag `</{name}>` for a void element (HTML 13.1.2)")), code::UNEXPECTED_CLOSE_TAG)
}

/// `skip_raw_blocks`: keep the tokens around JS/CSS block helpers and leave their bodies out.
fn tokens_of<'a>(text: &'a str, analyzed: &[(erb::ruby::ErbTag<'a>, BlockEffect)], skip_raw_blocks: bool) -> Option<Vec<Token<'a>>> {
    let (html, events) = split(text, analyzed, skip_raw_blocks)?;
    let mut events = events.into_iter().peekable();
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(start) = find(&html, i, b"<") {
        tokens.extend(std::iter::from_fn(|| events.next_if(|e| e.at < start)).map(Token::Erb));
        i = start + 1;
        if html[start..].starts_with(b"<!--") {
            i = find(&html, start + "<!--".len(), b"-->").map_or(html.len(), |p| p + "-->".len());
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
        let tag = Tag { name, start, end, wrapper: None };
        if closing {
            tokens.push(Token::Close { tag, quoted });
        } else if TEXT_ELEMENTS.contains(&tag.name.as_str()) && !self_closing {
            let close = format!("</{}", tag.name);
            let close = find(&html, end, close.as_bytes()).map(|close_start| {
                let after = close_start + close.len();
                (close_start, after + (html.get(after) == Some(&b'>')) as usize)
            });
            i = close.map_or(html.len(), |(_, close_end)| close_end);
            tokens.push(Token::Raw { tag, close });
        } else {
            let name = tag.name.as_str();
            let pairs = !self_closing && !NO_END_TAG.contains(&name) && !OPTIONAL_END.contains(&name);
            tokens.push(Token::Open { tag, pairs, quoted });
        }
    }
    tokens.extend(events.map(Token::Erb));
    Some(tokens)
}

fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

pub(crate) struct TagEnd {
    end: usize,
    pub(crate) self_closing: bool,
    quoted: Vec<(usize, usize)>,
}

/// Byte offset just past the `>` closing a tag (skipping quoted attribute values), whether it is
/// self-closing, and the quoted values' spans. A `/` ending an unquoted value (`<a href=/>`) belongs to the
/// value, not to `/>`, and a `/` apart from the `>` (`<a href=x/ >`) closes nothing.
pub(crate) fn tag_end(html: &[u8], from: usize) -> Option<TagEnd> {
    let mut i = from;
    let mut prev = 0u8;
    let mut last = 0u8;
    let mut unquoted = false;
    let mut quoted = Vec::new();
    while let Some(&b) = html.get(i) {
        match b {
            b'>' => return Some(TagEnd { end: i + 1, self_closing: last == b'/' && !unquoted, quoted }),
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
        last = b;
        i += 1;
    }
    None
}

/// HTML with ERB tags blanked out, plus every ERB tag as an event, in order. Tags are Erubi's; a `<%%` literal
/// renders as text, so it stays in the HTML. With `skip_raw_blocks` the JS/CSS block helpers' bodies are blanked and
/// their events dropped; without, any such helper makes the result `None`.
fn split<'a>(
    text: &'a str,
    analyzed: &[(erb::ruby::ErbTag<'a>, BlockEffect)],
    skip_raw_blocks: bool,
) -> Option<(Vec<u8>, Vec<ErbEvent<'a>>)> {
    // An unclosed `<%` is reported elsewhere; tag positions would be guesswork. `<%%` is text.
    let tail = &text[analyzed.last().map_or(0, |(t, _)| t.end)..];
    if tail.match_indices("<%").any(|(at, _)| !tail[at..].starts_with("<%%")) {
        return None;
    }
    let mut html = text.as_bytes().to_ascii_lowercase();
    let mut events = Vec::new();
    for (tag, effect) in analyzed.iter().filter(|(t, _)| !t.is_literal()) {
        html[tag.start..tag.end].iter_mut().filter(|b| **b != b'\n').for_each(|b| *b = b' ');
        let (effect, keyword, code) = if tag.is_comment() {
            (BlockEffect::Neutral, "", "")
        } else {
            let snippet = &text[tag.code.clone()];
            if RAW_BLOCK.is_match(snippet) && (!skip_raw_blocks || BRACE_BLOCK_OPEN.is_match(snippet)) {
                return None;
            }
            (*effect, snippet.split_whitespace().next().unwrap_or(""), snippet)
        };
        events.push(ErbEvent { at: tag.start, end: tag.end, effect, keyword, code, comment: tag.is_comment() });
    }
    if skip_raw_blocks {
        for (from, to) in raw_block_bodies(text, analyzed) {
            html[from..to].iter_mut().filter(|b| **b != b'\n').for_each(|b| *b = b' ');
            events.retain(|e| e.at < from || e.at >= to);
        }
    }
    Some((html, events))
}

/// Body byte ranges of the JS/CSS block helpers (`javascript_tag do … end`), up to their matching `end`. When the
/// depth never closes (an inner `if` lacks its `end`), up to the last `end` before the next markup line that is no
/// more indented than the helper; failing that, the end of the text.
fn raw_block_bodies(text: &str, analyzed: &[(erb::ruby::ErbTag<'_>, BlockEffect)]) -> Vec<(usize, usize)> {
    let mut bodies = Vec::new();
    for (i, (tag, effect)) in analyzed.iter().enumerate() {
        let BlockEffect::Open(opened) = *effect else { continue };
        if tag.is_literal() || tag.is_comment() || !RAW_BLOCK.is_match(&text[tag.code.clone()]) {
            continue;
        }
        let mut depth = opened;
        let close = analyzed[i + 1..].iter().find_map(|(t, effect)| {
            match *effect {
                BlockEffect::Open(n) => depth += n,
                BlockEffect::Close(n) => depth = depth.saturating_sub(n),
                _ => {}
            }
            (depth == 0).then_some(t.start)
        });
        let close = close.unwrap_or_else(|| unbalanced_body_end(text, tag.start, opened, &analyzed[i + 1..]));
        bodies.push((tag.end, close));
    }
    bodies
}

fn unbalanced_body_end(text: &str, helper_start: usize, opened: usize, after: &[(erb::ruby::ErbTag<'_>, BlockEffect)]) -> usize {
    let indentation = |line: &str| line.len() - line.trim_start().len();
    let helper_line = &text[text[..helper_start].rfind('\n').map_or(0, |i| i + 1)..];
    let helper_indent = indentation(helper_line);
    let markup_line = text[helper_start..].match_indices('\n').map(|(i, _)| helper_start + i + 1).find(|&start| {
        let line = text[start..].lines().next().unwrap_or("");
        let Some(tag) = line.trim_start().strip_prefix('<') else { return false };
        tag.strip_prefix('/').unwrap_or(tag).starts_with(|c: char| c.is_ascii_alphabetic()) && indentation(line) <= helper_indent
    });
    let Some(markup_line) = markup_line else { return text.len() };
    // Only an `end` that could close the helper itself: a balanced inner `if`'s end is JS body.
    let mut depth = opened;
    let mut last_end = None;
    for (t, effect) in after.iter().take_while(|(t, _)| t.end <= markup_line) {
        match *effect {
            BlockEffect::Open(n) => depth += n,
            BlockEffect::Close(n) => {
                if depth <= opened {
                    last_end = Some(t.start);
                }
                depth = depth.saturating_sub(n);
            }
            _ => {}
        }
    }
    last_end.unwrap_or(markup_line)
}

/// Track the tag stack and `p` state through ERB branches. When the branches disagree the blocks are closed anyway,
/// and the stack is the one the last branch left.
// A block whose branches (including the implicit empty `else` of `if`/loops) leave different tags
// open makes the stack path-dependent, so checking stops there. The exception is a conditional wrapper: an
// `if`/`unless` chain that opens tags and a later chain with the same conditions at the same level that closes them.
pub(crate) fn apply(event: &ErbEvent, stack: &mut Vec<Tag>, p: &mut PState, blocks: &mut Vec<Block>) -> Paths {
    match event.effect {
        BlockEffect::Open(n) => {
            for _ in 0..n {
                let in_branch = event.keyword != "case";
                let entry = stack.clone();
                let chain = matches!(event.keyword, "if" | "unless").then(|| vec![condition_key(event.keyword, event.code)]);
                let closing = chain.as_ref().and_then(|chain| closing_wrapper(stack, blocks.len(), &chain[0]));
                if let Some(wrapper) = &closing {
                    reopen_branch(stack, wrapper, 0);
                }
                let (branch_ends, branch_p_ends) = (Vec::new(), Vec::new());
                blocks.push(Block { entry, entry_p: *p, branch_ends, branch_p_ends, has_else: false, in_branch, chain, closing });
            }
        }
        BlockEffect::Continue => {
            let Some(block) = blocks.last_mut() else { return Paths::Differ };
            if block.in_branch {
                block.branch_ends.push(stack.clone());
                block.branch_p_ends.push(*p);
            }
            block.in_branch = true;
            block.has_else |= event.keyword == "else";
            *stack = block.entry.clone();
            *p = block.entry_p;
            let key = if event.keyword == "else" { ELSE.to_string() } else { condition_key(event.keyword, event.code) };
            match block.chain.as_mut() {
                Some(chain) if matches!(event.keyword, "elsif" | "else") => chain.push(key),
                _ => block.chain = None,
            }
            let branch = block.chain.as_ref().map_or(0, |c| c.len() - 1);
            let same_branch = |w: &Arc<Wrapper>| block.chain.as_ref().is_some_and(|c| w.chain.get(branch) == Some(&c[branch]));
            block.closing = block.closing.take().filter(same_branch);
            if let Some(wrapper) = &block.closing {
                reopen_branch(stack, wrapper, branch);
            }
        }
        BlockEffect::Close(n) => {
            let mut paths = Paths::Agree;
            for _ in 0..n {
                let Some(block) = blocks.pop() else { return Paths::Differ };
                let ends: Vec<Vec<String>> = block.branch_ends.iter().map(|b| names(b)).collect();
                let now = names(stack);
                let level = blocks.len();
                paths = paths.max(if paths_agree(&ends, &names(&block.entry), block.has_else, &now) {
                    Paths::Agree
                } else if closes_wrapper(&block, &ends, &now) || mark_wrapper(&block, stack, level) {
                    Paths::Wrapper
                } else {
                    Paths::Differ
                });
                if !paths_agree(&block.branch_p_ends, &block.entry_p, block.has_else, p) {
                    *p = PState::Unknown;
                }
            }
            return paths;
        }
        BlockEffect::Neutral => {}
    }
    Paths::Agree
}

const ELSE: &str = "else";

/// A branch condition with `unless`, `!` and `not` folded into one form, so `if !x` and `unless x` compare equal.
/// Only a plain method or variable is negated; for anything with operators the text is kept as written.
fn condition_key(keyword: &str, code: &str) -> String {
    let condition = code.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
    let condition = condition.strip_suffix(" then").unwrap_or(&condition);
    let mut negated = keyword == "unless";
    let mut rest = condition;
    while let Some(inner) = rest.strip_prefix('!').or_else(|| rest.strip_prefix("not ")) {
        negated = !negated;
        rest = inner.trim_start();
    }
    let plain = !rest.is_empty() && rest.chars().all(|c| c.is_alphanumeric() || "_@$.?:()[]'\"".contains(c));
    match (plain, negated) {
        (true, true) => format!("!{rest}"),
        (true, false) => rest.to_string(),
        (false, _) if keyword == "unless" => format!("!({condition})"),
        (false, _) => condition.to_string(),
    }
}

/// The wrapper whose open tags end `stack`, if the chain `level` deep starting with `key` is the one that closes them.
fn closing_wrapper(stack: &[Tag], level: usize, key: &str) -> Option<Arc<Wrapper>> {
    let wrapper = stack.last()?.wrapper.as_ref()?;
    let tail = stack.iter().rev().take_while(|t| t.wrapper.as_ref().is_some_and(|w| Arc::ptr_eq(w, wrapper))).count();
    (wrapper.level == level && wrapper.chain.first().map(String::as_str) == Some(key) && wrapper.branches.last()?.len() == tail)
        .then(|| wrapper.clone())
}

/// Swap the open tags of the wrapper's last branch for the ones `branch` left, which this branch is to close.
fn reopen_branch(stack: &mut Vec<Tag>, wrapper: &Arc<Wrapper>, branch: usize) {
    let kept = stack.len() - wrapper.branches.last().map_or(0, Vec::len);
    stack.truncate(kept);
    stack.extend(wrapper.branches[branch].iter().cloned().map(|tag| Tag { wrapper: Some(wrapper.clone()), ..tag }));
}

/// Every branch of `block`, the same chain as the wrapper it closed tags of, ended without them.
fn closes_wrapper(block: &Block, ends: &[Vec<String>], now: &[String]) -> bool {
    let Some(wrapper) = &block.closing else { return false };
    if block.chain.as_ref() != Some(&wrapper.chain) {
        return false;
    }
    let kept = block.entry.len() - wrapper.branches.last().map_or(0, Vec::len);
    let base = names(&block.entry[..kept]);
    paths_agree(ends, &base, block.has_else, now)
}

/// An `if`/`unless` chain whose branches leave tags open (and mark them): every branch keeps the entry stack and adds to it.
fn mark_wrapper(block: &Block, stack: &mut [Tag], level: usize) -> bool {
    let Some(chain) = &block.chain else { return false };
    let entry = &block.entry;
    let ends = block.branch_ends.iter().map(Vec::as_slice).chain(std::iter::once(&*stack));
    let extras: Option<Vec<Vec<Tag>>> = ends
        .map(|end| {
            let extra = end.get(entry.len()..)?;
            let intact = end.iter().zip(entry).all(|(a, b)| a.name == b.name);
            (intact && extra.iter().all(|t| t.wrapper.is_none())).then(|| extra.to_vec())
        })
        .collect();
    // The stack holds the last branch's tags, which carry the mark.
    let Some(branches) = extras.filter(|b| b.last().is_some_and(|extra| !extra.is_empty())) else { return false };
    let wrapper = Arc::new(Wrapper { chain: chain.clone(), level, branches });
    stack[entry.len()..].iter_mut().for_each(|t| t.wrapper = Some(wrapper.clone()));
    true
}

/// Whether `tag`, a wrapper's, is being closed inside the chain that matches its conditions.
fn closes_in_matching_if(tag: &Tag, blocks: &[Block]) -> bool {
    let Some(wrapper) = &tag.wrapper else { return true };
    blocks.len() == wrapper.level + 1 && blocks.last().and_then(|b| b.closing.as_ref()).is_some_and(|w| Arc::ptr_eq(w, wrapper))
}

/// Every finished branch, and the fall-through when there's no `else`, ends in `now`.
fn paths_agree<T: PartialEq<U>, U: ?Sized>(branch_ends: &[T], entry: &T, has_else: bool, now: &U) -> bool {
    branch_ends.iter().all(|b| b == now) && (has_else || entry == now)
}

fn names(stack: &[Tag]) -> Vec<String> {
    stack.iter().map(|t| t.name.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp_server::ls_types::Position;

    fn diagnostics(text: &str) -> Vec<Diagnostic> {
        super::diagnostics(text, false)
    }

    fn messages(text: &str) -> Vec<String> {
        diagnostics(text).into_iter().map(|d| d.message).collect()
    }

    #[test]
    fn unclosed_literal_is_text() {
        assert_eq!(messages("<div>\n<%# a %>\n<span>\n</div>\n<%% open\n").len(), 1);
    }

    #[test]
    fn reports_mismatch_stray_and_unclosed() {
        assert_eq!(messages("<div>\n  <span>x</div>"), ["End tag `</div>` doesn't close the open `<span>` (line 2) (HTML 13.2.6.4.7)"]);
        assert_eq!(messages("<div></div></span>"), ["Unexpected end tag `</span>`: no open element of that name (HTML 13.2.6.4.7)"]);
        assert_eq!(messages("<section><div></div>"), ["`<section>` is never closed (HTML 13.2.6.4.7)"]);
        // Tags crossing a block boundary inconsistently with the outside are still caught.
        assert_eq!(
            messages("<% if a %><div><% else %><div><% end %></span>"),
            ["End tag `</span>` doesn't close the open `<div>` (line 1) (HTML 13.2.6.4.7)"]
        );
        assert_eq!(
            messages("<% case x %>\n<% when 1 %><a><% else %><a><% end %></span>"),
            ["End tag `</span>` doesn't close the open `<a>` (line 2) (HTML 13.2.6.4.7)"]
        );
        assert_eq!(messages("<% items.each do |i| %><div><% end %></div>"), Vec::<String>::new());
        assert_eq!(messages("<ul><% items.each do |i| %><li><%= i %><% end %>"), ["`<ul>` is never closed (HTML 13.2.6.4.7)"]);
    }

    #[test]
    fn a_stray_end_tag_is_dropped_and_checking_goes_on() {
        let stray = "End tag `</span>` doesn't close the open `<div>` (line 1) (HTML 13.2.6.4.7)";
        let unexpected = "Unexpected end tag `</b>`: no open element of that name (HTML 13.2.6.4.7)";
        assert_eq!(messages("<div></span><p>x</p></div></b>"), [stray, unexpected]);
        assert_eq!(messages("<div></span></div><section>"), [stray, "`<section>` is never closed (HTML 13.2.6.4.7)"]);
        assert_eq!(messages("<div><% if a %></span><% end %></div></b>"), [stray, unexpected]);
        assert_eq!(super::diagnostics("<div></span></div>", true).len(), 1);
        assert_eq!(messages("<div></dvi>").len(), 1, "the typo'd end tag already names the unclosed element");
    }

    #[test]
    fn an_end_tag_crossing_an_open_element_stops_checking() {
        let crossed = "End tag `</div>` doesn't close the open `<span>` (line 1) (HTML 13.2.6.4.7)";
        assert_eq!(messages("<div><span></div></b>"), [crossed]);
        assert_eq!(messages("<div><span></div><i>"), [crossed]);
    }

    #[test]
    fn a_heading_end_tag_closes_the_open_heading() {
        let mismatch = "End tag `</h2>` doesn't close the open `<h3>` (line 1) (HTML 13.2.6.4.7)";
        assert_eq!(messages("<div><h3>a</h2></div><b>"), [mismatch, "`<b>` is never closed (HTML 13.2.6.4.7)"]);
        assert_eq!(messages("<h3><b>a</h2></b>").len(), 1, "a heading below another element stops checking");
    }

    #[test]
    fn end_tags_close_through_implied_end_elements() {
        assert!(messages("<ul><li>a<li>b</ul><dl><dt>a<dd>b</dl><table><tr><td>a<td>b</table><select><option>a</select>").is_empty());
        assert_eq!(messages("<ul><li>a<li>b</ul></i>").len(), 1);
    }

    #[test]
    fn diagnostics_per_file_are_capped() {
        let strays = "<div></span></div>".repeat(MAX_TAG_DIAGNOSTICS * 2);
        assert_eq!(messages(&strays).len(), MAX_TAG_DIAGNOSTICS);
        let unclosed = "<section>".repeat(MAX_TAG_DIAGNOSTICS * 2);
        assert_eq!(messages(&unclosed).len(), MAX_TAG_DIAGNOSTICS);
        assert_eq!(messages(&"</b>".repeat(MAX_TAG_DIAGNOSTICS + 1)).len(), MAX_TAG_DIAGNOSTICS);
    }

    #[test]
    fn void_and_optional_end_elements_are_not_unclosed() {
        for ok in [
            "<area><base><br><col><embed><hr><img><input><link><meta><param><source><track><wbr>",
            "<dl><dt>a<dd>b<dt>c</dl><select><optgroup><option>a<option>b</select>",
            "<table><caption>c<colgroup><col><thead><tr><th>h<tbody><tr><td>a<td>b<tfoot><tr><td>f</table>",
            "<ruby>a<rt>b<rp>c</ruby><p>a<p>b",
        ] {
            assert!(messages(ok).is_empty(), "{ok}: {:?}", messages(ok));
        }
        assert_eq!(
            messages("<br></br><img></img>"),
            ["End tag `</br>` for a void element (HTML 13.1.2)", "End tag `</img>` for a void element (HTML 13.1.2)"]
        );
    }

    #[test]
    fn end_tag_of_a_p_a_block_start_tag_closed() {
        let unexpected = ["Unexpected end tag `</p>`: no open element of that name (HTML 13.2.6.4.7)"];
        assert_eq!(messages("<p>a<div>b</div></p>"), unexpected);
        assert_eq!(messages("<p>a<ul><li>b</ul></p>"), unexpected);
        assert_eq!(messages("<div>x</p></div>"), unexpected);
        assert_eq!(messages("<p>a</p></p>"), unexpected);
        assert!(messages("<p>a<span>b</span></p><p>c<p>d</p>").is_empty());
        assert!(super::diagnostics("<div>x</p></div>", true).is_empty());
    }

    #[test]
    fn p_end_tag_inside_scope_boundaries_and_erb_branches() {
        for ok in [
            "<p><button><div>x</div></button></p>",
            "<p><object><div>x</div></object></p>",
            "<p><template><div>x</div></template></p>",
            "<p><svg><foreignObject><div>x</div></foreignObject></svg></p>",
            "<table><tr><td><p>a<div>b</div></p></table>",
            "<% if a %><p>x<% else %><div>y</div><% end %></p>",
            "<% if a %><div>y</div><% else %><p>x<% end %></p>",
            "<p><% if a %><div>x</div><% end %></p>",
            "<p><% if a %><b>x</b><% else %><i>y</i><% end %></p>",
            "<p><% items.each do |i| %><%= i %><% end %></p>",
            "<% if a %><p>x<% else %><p>y<% end %></p>",
        ] {
            assert!(messages(ok).is_empty(), "{ok}: {:?}", messages(ok));
        }
        assert_eq!(messages("<p><% if a %><div>x</div><% else %><div>y</div><% end %></p>").len(), 1);
        assert_eq!(messages("<% if a %><div>x</div><% else %><span>y</span><% end %></p>").len(), 1);
    }

    #[test]
    fn a_slash_apart_from_the_closing_bracket_is_not_self_closing() {
        assert!(messages("<a href=x/ >y</a>").is_empty());
        assert!(messages("<a href=x/>y</a>").is_empty());
        assert!(messages("<div/ >y</div>").is_empty());
        assert_eq!(messages("<div/>y</div>").len(), 1);
        assert_eq!(messages("<a href=x />y</a>").len(), 1);
    }

    #[test]
    fn review_false_positives() {
        assert!(messages("<a href=/>Home</a><a href=/users/>U</a>").is_empty());
        assert!(messages("<br/><input disabled/><x-icon a=b />").is_empty());
        assert!(messages("<img src=x/><br/ ><div/ >x</div>").is_empty());
        assert!(messages("<%= javascript_tag do %>\n el.innerHTML = \"<span>\" + x + \"</div>\"; if (a<b) {}\n<% end %>").is_empty());
        assert!(messages("<%= content_tag(:script) do %>\"</div>\"<% end %>").is_empty());
        let header = "<div class=\"wrapper\"><header>x</header>";
        assert!(super::diagnostics(header, true).is_empty());
        assert!(super::diagnostics("</div></body>", true).is_empty());
        assert_eq!(super::diagnostics("<div></span>", true).len(), 1);
    }

    #[test]
    fn js_block_helper_skips_only_its_body() {
        let js = "<%= javascript_tag do %>\n x = \"<span>\" + \"</div>\";\n<% end %>";
        assert_eq!(messages(&format!("<div>\n{js}\n<span>x</div>")).len(), 1);
        assert_eq!(messages(&format!("{js}<section>")), ["`<section>` is never closed (HTML 13.2.6.4.7)"]);
        let nested = "<%= javascript_tag do %><% if a %>\"</b>\"<% end %>\"<i>\"<% end %>";
        assert_eq!(messages(&format!("{nested}</span>")).len(), 1);
        assert!(messages(&format!("<div>{js}{js}</div>")).is_empty());
        assert!(messages("<%= javascript_tag do %>\"</div>\"<div>").is_empty(), "unclosed helper skips the rest");
        let lacking_end = "<div>\n  <%= javascript_tag do %>\n    <% if x %>\n    f(\"</p>\");\n  <% end %>\n  <span></div>\n";
        assert_eq!(messages(lacking_end).len(), 1, "an inner `if` without `end` ends the body at the next markup line");
        assert!(messages("<%= javascript_tag do %>\n  <% if x %>\n    <b>\n<% end %>\n").is_empty());
        let template_literal =
            "<%= javascript_tag do %>\n  <% if x %>\n    el.innerHTML = `\n      <div>\n    `;\n<% end %>\n<span></div>\n";
        assert_eq!(messages(template_literal).len(), 1, "a markup line deeper than the helper stays in the body");
        let balanced_inner =
            "<div>\n<%= javascript_tag do %>\n  <% if x %>\n    a();\n  <% end %>\n  <% if y %>\n    b(\"</p>\");\n</div>\n";
        assert!(messages(balanced_inner).is_empty(), "a balanced inner `if`'s end is not the helper's");
        assert_eq!(messages(&lacking_end.replace('\n', "\r\n")).len(), 1, "CRLF");
    }

    #[test]
    fn every_js_css_helper_form_skips_its_body_but_not_the_rest() {
        for open in [
            "<%= javascript_tag do %>",
            "<%= javascript_tag(nonce: true) do %>",
            "<%= tag.script do %>",
            "<%= tag.style(media: \"print\") do %>",
            "<%= content_tag :style do %>",
            "<%= content_tag(:script, type: \"x\") do %>",
            "<%- javascript_tag do -%>",
        ] {
            let body = format!("{open}\n s = \"<b></i>\"; // <% if a %>\n<% end %>\n<% end -%>");
            assert!(messages(&format!("<div>{body}</div>")).is_empty(), "{open}");
            assert_eq!(messages(&format!("<i></b>{body}")).len(), 1, "{open}: error before");
            assert_eq!(messages(&format!("{body}<i></b>")).len(), 1, "{open}: error after");
        }
        assert_eq!(messages("<%= content_tag :div do %><b></i><% end %>").len(), 1, "other helpers stay checked");
        for open in ["<%= javascript_tag { %>", "<%= tag.script() { %>", "<%= tag.script { |t| %>"] {
            let text = format!("<div>\n{open}\n  var s = \"</p>\";\n<% }} %>\n</div>\n");
            assert!(messages(&text).is_empty(), "{open}");
        }
    }

    #[test]
    fn elements_after_a_js_helper_keep_their_positions() {
        let text = "<%= javascript_tag do %>\"</b>\"<% end %><ul><li>a</li></ul>";
        let mut found: Vec<_> = elements(text).into_iter().map(|e| e.name).collect();
        found.sort();
        assert_eq!(found, ["li", "ul"]);
    }

    #[test]
    fn literal_raw_text_blocks_skip_only_their_body() {
        for ok in [
            "<script type=\"text/template\"><div></script><p>x</p>",
            "<SCRIPT>if (a<b) {}</SCRIPT><div></div>",
            "<script>x = \"<%= y %></div>\"</script><div></div>",
            "<script>x = '<% if a %>'</script><div></div>",
        ] {
            assert!(messages(ok).is_empty(), "{ok}: {:?}", messages(ok));
        }
        assert_eq!(messages("<script>a</script>\n<div>\n<span>x</div>").len(), 1);
        assert!(messages("<script>\n<div>").is_empty());
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
    fn conditional_wrappers_are_balanced() {
        let wrap = "<% if link %><a href=\"<%= link %>\"><% end %>\n<b>x</b>\n<% if link %></a><% end %>";
        assert!(messages(wrap).is_empty());
        assert_eq!(messages(&format!("{wrap}</span>")).len(), 1, "error after");
        assert_eq!(messages("<% if link %><a><% end %><div></span></div><% if link %></a><% end %>").len(), 1, "error between");
        assert!(messages("<% unless x %><a><% end %>y<% unless x %></a><% end %>").is_empty());
        assert!(messages("<%if  link%><a><%end%>y<% if link %></a><% end %>").is_empty(), "whitespace is normalised");
        let nested = "<% if a %><div><% end %><% if b %><span><% end %>x<% if b %></span><% end %><% if a %></div><% end %>";
        assert_eq!(messages(&format!("{nested}</i>")).len(), 1);
        assert_eq!(messages("<div><% if a %><p><% end %></i><% if a %></p><% end %></div>").len(), 1);
        assert!(messages("<% if a %><div><% end %><% if b %></div><% end %></i>").is_empty(), "different condition stops");
        assert!(messages("<% if a %><div><% end %></div></i>").is_empty(), "plain close of a wrapper stops");
        assert!(messages("<% if a %><div><% end %><% unless a %></div><% end %></i>").is_empty(), "if vs unless stops");
        assert!(messages("<% if a %><div><% end %><% if a %><% if c %></div><% end %><% end %></i>").is_empty(), "other level stops");
        assert_eq!(messages("<% if a %><div><% end %><span></div>").len(), 1, "real mismatch above a wrapper");
    }

    #[test]
    fn conditional_wrapper_conditions_ignore_erb_trim_markers_and_then() {
        assert_eq!(messages("<%- if a -%><div><%- end -%>x<%- if a -%></div><%- end -%></i>").len(), 1);
        assert_eq!(messages("<% if a -%><div><% end %>x<% if a %></div><% end %></i>").len(), 1);
        assert_eq!(messages("<% if a then %><div><% end %>x<% if a then %></div><% end %></i>").len(), 1);
    }

    #[test]
    fn conditional_wrapper_with_several_tags_and_in_a_loop() {
        assert_eq!(messages("<% if a %><div><span><% end %>x<% if a %></span></div><% end %></i>").len(), 1);
        let looped = "<% items.each do |i| %><% if i %><div><% end %>x<% if i %></div><% end %><% end %>";
        assert!(messages(looped).is_empty());
        assert_eq!(messages(&format!("{looped}</i>")).len(), 1);
    }

    #[test]
    fn if_else_chain_wrapper_is_balanced() {
        let open = "<% if link %><a href=\"<%= link %>\"><% else %><span><% end %>\n  x\n";
        let close = "<% if link %></a><% else %></span><% end %>";
        assert!(messages(&format!("{open}{close}")).is_empty());
        assert_eq!(messages(&format!("{open}{close}</i>")).len(), 1, "error after");
        assert_eq!(messages(&format!("{open}<div></i></div>{close}")).len(), 1, "error between");
        let three = "<% if a %><b><% elsif c %><i><% else %><u><% end %>x<% if a %></b><% elsif c %></i><% else %></u><% end %>";
        assert_eq!(messages(&format!("{three}</s>")).len(), 1);
        let no_else = "<% if a %><b><% elsif c %><i><% end %>x<% if a %></b><% elsif c %></i><% end %>";
        assert_eq!(messages(&format!("{no_else}</s>")).len(), 1);
        let empty_middle = "<% if a %><b><% elsif c %><% else %><u><% end %>x<% if a %></b><% elsif c %><% else %></u><% end %>";
        assert_eq!(messages(&format!("{empty_middle}</s>")).len(), 1);
        let unclosed = "<% if a %><b><% else %><i><% end %>x<% if a %></b><% else %></i><% end %><p></p><u>";
        assert_eq!(messages(unclosed), ["`<u>` is never closed (HTML 13.2.6.4.7)"]);
    }

    #[test]
    fn if_else_chain_wrapper_stops_when_the_closing_chain_differs() {
        let open = "<% if a %><b><% else %><i><% end %>x";
        let over_closed = "<% if a %></b></i><% else %></i><% end %>";
        assert_eq!(messages(&format!("{open}{over_closed}")).len(), 1, "a real error inside a matching branch");
        let empty_last = "<% if a %><b><% else %><% end %>x<% if a %></b><% else %><% end %></s>";
        assert!(messages(empty_last).is_empty(), "no tag to carry the mark");
        for close in [
            "<% if a %></i><% else %></b><% end %>",
            "<% if c %></b><% else %></i><% end %>",
            "<% if a %></b><% end %>",
            "<% if a %></b><% elsif c %></i><% else %></i><% end %>",
            "<% if a %></b><% else %><% end %>",
            "<% if a %><% else %><% end %>",
            "<% unless a %></b><% else %></i><% end %>",
            "</i>",
        ] {
            assert!(messages(&format!("{open}{close}</s>")).is_empty(), "{close}");
        }
    }

    #[test]
    fn unless_else_chain_wrapper_closes_with_the_negated_if_else() {
        let open = "<% unless a %><b><% else %><i><% end %>x";
        for close in ["<% unless a %></b><% else %></i><% end %>", "<% if !a %></b><% else %></i><% end %>"] {
            assert!(messages(&format!("{open}{close}")).is_empty(), "{close}");
            assert_eq!(messages(&format!("{open}{close}</s>")).len(), 1, "{close}");
        }
    }

    #[test]
    fn stray_end_tags_in_conditional_branches_are_each_reported() {
        let text = "<div><% if a %></span><% else %></em><% end %></div>";
        assert_eq!(messages(text).len(), 2);
        assert_eq!(messages("<div><% if a %><b><% end %></span></div>").len(), 0, "path-dependent stack stops checking");
    }

    #[test]
    fn negated_conditions_match_unless() {
        for (open, close) in [
            ("if !admin", "unless admin"),
            ("if not admin", "unless admin"),
            ("unless !admin", "if admin"),
            ("unless admin", "if ! admin"),
            ("if !user.admin?", "unless user.admin?"),
            ("if !admin then", "unless admin"),
            ("unless a && b", "if !(a && b)"),
            ("if a && b", "if a && b"),
        ] {
            let text = format!("<% {open} %><div class=\"x\"><% end %>y<% {close} %></div><% end %>");
            assert_eq!(messages(&format!("{text}</i>")).len(), 1, "{open} / {close}");
        }
        for (open, close) in [
            ("if !a && b", "unless a && b"),
            ("if not a and b", "unless a and b"),
            ("if admin", "if !admin"),
            ("if !a", "unless !a"),
            ("unless a || b", "if !a || b"),
        ] {
            let text = format!("<% {open} %><div><% end %>y<% {close} %></div><% end %></i>");
            assert!(messages(&text).is_empty(), "{open} / {close}");
        }
    }

    #[test]
    fn conditional_wrapper_is_not_marked_twice() {
        let text = "<% if x %>\n<% if a %><div><% end %>\n<% end %>\n<% if x %></div><% end %></i>";
        assert!(messages(text).is_empty(), "path-dependent stack: checking stops instead of trusting a re-marked wrapper");
    }

    #[test]
    fn conditional_wrapper_close_before_open_is_unexpected() {
        assert_eq!(messages("<% if a %></div><% end %><% if a %><div><% end %>").len(), 1);
    }

    #[test]
    fn elements_pair_across_a_conditional_wrapper() {
        let text = "<% if a %><div><% end %>x<% if a %></div><% end %><p></p>";
        let pairs: Vec<(String, usize, usize)> = elements(text).into_iter().map(|e| (e.name, e.start, e.end)).collect();
        assert_eq!(pairs, [("div".to_string(), 10, 41), ("p".to_string(), 50, 57)]);
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
