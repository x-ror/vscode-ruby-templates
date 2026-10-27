//! ERB diagnostics: the Ruby syntax errors of `erb_ruby` and the HTML errors Erubi's text leaves; tree-sitter-embedded-template for the rest.

use crate::checks::RubyContext;
use crate::erb_ruby;
use crate::util::{coded, offset_to_position, warning};
use tower_lsp::lsp_types::{Diagnostic, Position, Range};
use tree_sitter::{Parser, Tree};

/// Stable diagnostic codes; `rubyTemplates.erb.disabledDiagnostics` refers to these.
pub mod code {
    pub const UNCLOSED_TAG: &str = "unclosed-tag";
    pub const RUBY_SYNTAX: &str = "ruby-syntax";
}

pub fn parse(text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_embedded_template::LANGUAGE.into()).ok()?;
    parser.parse(text, None)
}

/// `<%=foo%>` → `<%= foo %>`: one space inside the delimiters of each one-line code tag, as `(start, end, tag)`.
pub fn spacing_edits(text: &str) -> Vec<(usize, usize, String)> {
    let Some(tree) = parse(text) else { return vec![] };
    let mut cursor = tree.root_node().walk();
    let mut out = Vec::new();
    for tag in tree.root_node().children(&mut cursor) {
        if tag.kind() != "directive" && tag.kind() != "output_directive" {
            continue;
        }
        let mut inner = tag.walk();
        let Some(code) = tag.children(&mut inner).find(|n| n.kind() == "code") else { continue };
        let source = &text[code.byte_range()];
        if source.contains('\n') || source.trim().is_empty() || !text[..tag.end_byte()].ends_with("%>") {
            continue;
        }
        let spaced = format!("{} {} {}", &text[tag.start_byte()..code.start_byte()], source.trim(), &text[code.end_byte()..tag.end_byte()]);
        if spaced != text[tag.byte_range()] {
            out.push((tag.start_byte(), tag.end_byte(), spaced));
        }
    }
    out
}

pub fn diagnostics(text: &str, ruby: &RubyContext) -> Vec<Diagnostic> {
    erb_ruby::diagnostics(text, ruby.grammar, ruby.strict_locals_identifier.as_deref())
        .into_iter()
        .map(|d| coded(d, code::RUBY_SYNTAX))
        .collect()
}

/// Elements whose content the tokenizer reads as raw text or RCDATA, up to their end tag (`noscript` with scripting on).
const TEXT_ELEMENTS: [&str; 9] = ["script", "style", "xmp", "iframe", "noembed", "noframes", "noscript", "textarea", "title"];
const FOREIGN_ROOTS: [&str; 2] = ["svg", "math"];

/// The attribute sub-states of the tokenizer (13.2.5.32 to 13.2.5.40) that decide where a quote opens a value.
enum Attr {
    BeforeName,
    Name,
    AfterName,
    BeforeValue,
    UnquotedValue,
}

struct Tag {
    closing: bool,
    name: String,
    attr: Attr,
    /// The last character was a `/` outside a value (the self-closing start tag state).
    self_closing: bool,
}

impl Tag {
    fn new(closing: bool, name: String) -> Self {
        Tag { closing, name, attr: Attr::BeforeName, self_closing: false }
    }
}

#[derive(Default)]
enum State {
    #[default]
    Data,
    Comment,
    BogusComment,
    Cdata,
    Tag(Tag),
    QuotedValue(char, Tag),
    RawText(&'static str),
    Plaintext,
}

/// A tokenizer state kept across the text pieces. Limits: tree construction is approximated by counting `<svg>`/`<math>`
/// tags (no HTML integration points), and script data's escaped states (`<script><!--<script></script>`) are not modelled.
#[derive(Default)]
struct Html {
    state: State,
    foreign_depth: usize,
}

impl Html {
    /// Moves over `piece` and returns the offsets of the `<%` it reads in the data state.
    fn feed(&mut self, piece: &str) -> Vec<usize> {
        let mut openers = Vec::new();
        let mut pos = 0;
        while pos < piece.len() {
            let rest = &piece[pos..];
            if matches!(self.state, State::Data) && rest.starts_with("<%") {
                openers.push(pos);
            }
            let state = std::mem::take(&mut self.state);
            let (consumed, next) = self.step(state, rest);
            self.state = next;
            pos += consumed;
        }
        openers
    }

    /// Bytes of `rest` the state consumes, and the state after them.
    fn step(&mut self, state: State, rest: &str) -> (usize, State) {
        match state {
            State::Data if rest.starts_with('<') => self.tag_open(rest),
            State::Data => (rest.find('<').unwrap_or(rest.len()), State::Data),
            State::Comment => past_first(rest, &["-->", "--!>"], State::Comment),
            State::BogusComment => past_first(rest, &[">"], State::BogusComment),
            State::Cdata => past_first(rest, &["]]>"], State::Cdata),
            State::Tag(tag) => self.in_tag(tag, rest),
            State::QuotedValue(quote, tag) => quoted_value(quote, tag, rest),
            State::RawText(element) => raw_text(element, rest),
            State::Plaintext => (rest.len(), State::Plaintext),
        }
    }

    /// `rest` starts with `<`: the tag open (13.2.5.6) and markup declaration open (13.2.5.42) states.
    fn tag_open(&self, rest: &str) -> (usize, State) {
        let after = &rest[1..];
        if let Some(comment) = after.strip_prefix("!--") {
            // `<!-->` and `<!--->` are empty comments.
            let empty = if comment.starts_with("->") { 2 } else { usize::from(comment.starts_with('>')) };
            return (4 + empty, if empty > 0 { State::Data } else { State::Comment });
        }
        if self.foreign_depth > 0 && after.starts_with("![CDATA[") {
            return (9, State::Cdata);
        }
        let (closing, name_text) = after.strip_prefix('/').map_or((false, after), |name| (true, name));
        if name_text.starts_with(|c: char| c.is_ascii_alphabetic()) {
            let length = name_text.find(ends_tag_name).unwrap_or(name_text.len());
            let name = name_text[..length].to_ascii_lowercase();
            return (1 + usize::from(closing) + length, State::Tag(Tag::new(closing, name)));
        }
        if after.starts_with(['!', '?']) || (closing && !name_text.starts_with('>')) {
            return (2, State::BogusComment);
        }
        (1, State::Data)
    }

    /// One character of a tag after its name.
    fn in_tag(&mut self, mut tag: Tag, rest: &str) -> (usize, State) {
        let Some(c) = rest.chars().next() else { return (0, State::Tag(tag)) };
        let self_closing = std::mem::take(&mut tag.self_closing);
        if c == '>' {
            return (1, self.tag_end(tag, self_closing));
        }
        tag.attr = match (&tag.attr, c) {
            (Attr::BeforeName | Attr::Name | Attr::AfterName, '/') => {
                tag.self_closing = true;
                Attr::BeforeName
            }
            (Attr::BeforeName, c) if c.is_ascii_whitespace() => Attr::BeforeName,
            (Attr::BeforeName, _) => Attr::Name,
            (Attr::Name, c) if c.is_ascii_whitespace() => Attr::AfterName,
            (Attr::Name | Attr::AfterName, '=') => Attr::BeforeValue,
            (Attr::Name, _) => Attr::Name,
            (Attr::AfterName, c) if c.is_ascii_whitespace() => Attr::AfterName,
            (Attr::AfterName, _) => Attr::Name,
            (Attr::BeforeValue, c) if c.is_ascii_whitespace() => Attr::BeforeValue,
            (Attr::BeforeValue, '"' | '\'') => return (1, State::QuotedValue(c, tag)),
            (Attr::BeforeValue, _) => Attr::UnquotedValue,
            (Attr::UnquotedValue, c) if c.is_ascii_whitespace() => Attr::BeforeName,
            (Attr::UnquotedValue, _) => Attr::UnquotedValue,
        };
        (c.len_utf8(), State::Tag(tag))
    }

    /// The state after a tag's `>`; inside `<svg>`/`<math>` no element's content is raw text.
    fn tag_end(&mut self, tag: Tag, self_closing: bool) -> State {
        if FOREIGN_ROOTS.contains(&tag.name.as_str()) {
            if tag.closing {
                self.foreign_depth = self.foreign_depth.saturating_sub(1);
            } else {
                self.foreign_depth += usize::from(!self_closing);
            }
            return State::Data;
        }
        if tag.closing || self.foreign_depth > 0 {
            return State::Data;
        }
        if tag.name == "plaintext" {
            return State::Plaintext;
        }
        TEXT_ELEMENTS.iter().find(|&&t| t == tag.name).map_or(State::Data, |&t| State::RawText(t))
    }
}

fn ends_tag_name(c: char) -> bool {
    c.is_ascii_whitespace() || c == '/' || c == '>'
}

/// Past the first of `ends` in `rest`, back in the data state; without one, all of `rest` in `state`.
fn past_first(rest: &str, ends: &[&str], state: State) -> (usize, State) {
    match ends.iter().filter_map(|end| rest.find(end).map(|i| i + end.len())).min() {
        Some(end) => (end, State::Data),
        None => (rest.len(), state),
    }
}

fn quoted_value(quote: char, tag: Tag, rest: &str) -> (usize, State) {
    match rest.find(quote) {
        Some(i) => (i + 1, State::Tag(Tag { attr: Attr::BeforeName, ..tag })),
        None => (rest.len(), State::QuotedValue(quote, tag)),
    }
}

/// Up to and including the end tag name of `element` (`</script` followed by whitespace, `/` or `>`).
fn raw_text(element: &'static str, rest: &str) -> (usize, State) {
    let close = format!("</{element}");
    let end = rest
        .as_bytes()
        .windows(close.len() + 1)
        .position(|window| window[..close.len()].eq_ignore_ascii_case(close.as_bytes()) && ends_tag_name(char::from(window[close.len()])));
    match end {
        Some(i) => (i + close.len(), State::Tag(Tag::new(true, element.to_string()))),
        None => (rest.len(), State::RawText(element)),
    }
}

/// The `<%` Erubi leaves as text reaches an HTML host as text, where `<` followed by `%` is a parse error (13.2.5.6)
/// when the tokenizer is in the data state. Complete tags are invisible to it.
pub fn html_diagnostics(text: &str) -> Vec<Diagnostic> {
    let (texts, tail_at) = erb_ruby::rendered_texts(text);
    let mut html = Html::default();
    for rendered in &texts {
        html.feed(rendered);
    }
    html.feed(&text[tail_at..])
        .into_iter()
        .map(|at| {
            let start = offset_to_position(text, tail_at + at);
            let range = Range::new(start, Position::new(start.line, u32::MAX));
            let message =
                "invalid-first-character-of-tag-name: `<%` without `%>` is text, and `<` followed by `%` starts no tag (HTML 13.2.5.6)";
            coded(warning(range, message), code::UNCLOSED_TAG)
        })
        .collect()
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Ruby code of each ERB tag as sorted `(start, end)` byte bounds: `start` is just after `<%`, `end` the `>` of
/// `%>` (an offset there still belongs to the tag), or the text's end when unclosed. `<%#` comments and `<%%`
/// literals are left out. Like ERB, a tag ends at the first `%>`, even inside a string.
pub fn code_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut pos = 0;
    while let Some(i) = text[pos..].find("<%") {
        let open = pos + i;
        let close = text[open + 2..].find("%>").map(|c| open + 2 + c);
        let tag = &text[open..];
        if !tag.starts_with("<%#") && !tag.starts_with("<%%") {
            spans.push((open + 2, close.map_or(text.len(), |c| c + 1)));
        }
        let Some(close) = close else { break };
        pos = close + 2;
    }
    spans
}

/// Start of the span from `code_spans` that contains byte `offset`.
pub fn span_at(spans: &[(usize, usize)], offset: usize) -> Option<usize> {
    let after = spans.partition_point(|&(start, _)| start <= offset);
    let (start, end) = spans[..after].last()?;
    (offset <= *end).then_some(*start)
}

// Longest first. Semantics are Erubi's as configured by Rails (`trim: true`, `<%==` unescaped).
const OPENERS: &[(&str, &str)] = &[
    ("<%==", "**`<%==`** outputs the value **without** HTML escaping (like `raw`)."),
    ("<%=", "**`<%=`** outputs the value, HTML-escaped unless it is `html_safe`."),
    ("<%-", "**`<%-`** runs Ruby without output; Erubi treats it like `<%` (stdlib ERB with `trim_mode: '-'` strips the indentation before it)."),
    ("<%#", "**`<%#`** ERB comment: neither run nor output."),
    ("<%%", "**`<%%`** literal `<%`: the tag is output as text, not run."),
    ("<%", "**`<%`** runs Ruby without output; alone on a line it leaves no blank line."),
];
const CLOSERS: &[(&str, &str)] = &[
    ("-%>", "**`-%>`** closes the tag and drops the newline after `<%= … -%>` (code tags alone on a line drop it anyway)."),
    ("=%>", "**`=%>`** closes the tag; Erubi treats it like `-%>`."),
    ("%>", "**`%>`** closes the ERB tag."),
];

/// Markdown explaining the ERB tag delimiter under byte `col` of `line`.
pub fn tag_doc(line: &str, col: usize) -> Option<&'static str> {
    let token_at = |start: usize, tokens: &[(&str, &'static str)]| {
        let rest = &line[start..];
        tokens.iter().find(|(t, _)| rest.starts_with(t)).filter(|(t, _)| (start..start + t.len()).contains(&col)).map(|&(_, doc)| doc)
    };
    let opener = line.match_indices("<%").find_map(|(i, _)| token_at(i, OPENERS));
    opener.or_else(|| {
        line.match_indices("%>").find_map(|(i, _)| {
            let start = if i > 0 && matches!(line.as_bytes()[i - 1], b'-' | b'=') { i - 1 } else { i };
            token_at(start, CLOSERS)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostics(text: &str) -> Vec<Diagnostic> {
        super::diagnostics(text, &RubyContext::outside_project())
    }

    fn span(line: u32, start: u32, end: u32) -> Range {
        Range::new(Position::new(line, start), Position::new(line, end))
    }

    #[test]
    fn tag_spacing() {
        let fixed = |t: &str| spacing_edits(t).into_iter().map(|(_, _, tag)| tag).collect::<Vec<_>>();
        assert_eq!(fixed("<%=foo%>é<%if a-%><%==  raw %><%- x =%>"), ["<%= foo %>", "<% if a -%>", "<%== raw %>"]);
        assert!(fixed("<%= ok %><% x = 1 %><%# note %><%%= literal %>\n<% a\n  b %><%= x").is_empty());
    }

    #[test]
    fn broken_fixture() {
        let mut got: Vec<(u32, String)> = diagnostics(include_str!("../../test/broken/bad.html.erb"))
            .into_iter()
            .map(|d| (d.range.start.line, crate::util::code(&d).unwrap_or_default().to_string()))
            .collect();
        got.sort();
        let want = [(0, code::RUBY_SYNTAX), (5, code::RUBY_SYNTAX), (5, code::RUBY_SYNTAX), (5, code::RUBY_SYNTAX)];
        assert_eq!(got, want.map(|(line, code)| (line, code.to_string())));
        let html = html_diagnostics(include_str!("../../test/broken/bad.html.erb"));
        assert_eq!(html.iter().map(|d| d.range.start.line).collect::<Vec<_>>(), [9]);
    }

    #[test]
    fn balanced_template_is_clean() {
        assert!(diagnostics("<% if x %>é<% else %>😀<% end %>\n<%= link_to 'a', b %>").is_empty());
    }

    const UNCLOSED_MESSAGE: &str =
        "invalid-first-character-of-tag-name: `<%` without `%>` is text, and `<` followed by `%` starts no tag (HTML 13.2.5.6)";

    fn unclosed(text: &str) -> Vec<Range> {
        html_diagnostics(text)
            .into_iter()
            .map(|d| {
                assert_eq!(d.message, UNCLOSED_MESSAGE);
                assert_eq!(d.severity, Some(tower_lsp::lsp_types::DiagnosticSeverity::WARNING));
                assert_eq!(crate::util::code(&d), Some(code::UNCLOSED_TAG));
                d.range
            })
            .collect()
    }

    #[test]
    fn unclosed_tag_at_document_start() {
        for text in ["<% foo", "<%", "<%= foo", "<%- foo", "<%# x", "<%== x", "<%# 😀"] {
            assert_eq!(unclosed(text), [span(0, 0, u32::MAX)], "{text}");
        }
    }

    #[test]
    fn unclosed_tag_after_text_and_a_closed_tag() {
        assert_eq!(unclosed("<p>hi</p> <% foo"), [span(0, 10, u32::MAX)]);
        assert_eq!(unclosed("<% a %>\n<p><%= b %> <%= c"), [span(1, 12, u32::MAX)]);
    }

    #[test]
    fn unclosed_tag_after_multibyte() {
        assert_eq!(unclosed("😀 <% foo"), [span(0, 3, u32::MAX)]);
        assert_eq!(unclosed("é\né <%= foo"), [span(1, 2, u32::MAX)]);
    }

    #[test]
    fn every_opener_in_the_text_left_over_is_reported() {
        assert_eq!(unclosed("<% a %>\n<% b\n<%= c"), [span(1, 0, u32::MAX), span(2, 0, u32::MAX)]);
    }

    #[test]
    fn only_the_data_state_reports() {
        for text in [
            "<script>\nif (a <% b) {}\n</script>",
            "<style>a <% b",
            "<!-- <% old -->",
            "<!-- <% old",
            "<p title=\"5 <% 6\">x</p>",
            "<p title='5 <% 6",
            "<p data-a=<% b>",
            "<textarea>a <% b</textarea>",
            "<title>a <% b",
            "<!DOCTYPE <% html>",
            "<?xml <% ?>",
            "</ <% x>",
            "<plaintext>a <% b",
            "<xmp>a <% b",
            "<SCRIPT>a <% b",
            "<script>a</scripty> <% b",
            "<noscript>a <% b",
            "<a href=<%= u %>\ntitle=\"x>y <% z\">",
            "<p title=\"<%# c %>\n<% z\">",
            "<p title=é>x</p> <svg><![CDATA[ a > <% b ]]></svg>",
            "<p data-é=\"<% é\">x",
            "<p a=\"é\" b='<% 😀'>",
            "<!-- é <% -->",
            "<script>é <% </script",
            "<p a=b c=\"<% d\">",
            "<svg><![CDATA[ a > <% b ]]></svg>",
        ] {
            assert!(unclosed(text).is_empty(), "{text}");
        }
        for text in [
            "<script>a</script> <% b",
            "<script>a</SCRIPT\n> <% b",
            "<!-- c --> <% b",
            "<p title=\"x\"> <% b",
            "<textarea></textarea><% b",
            "<!-->\n<% b",
            "<p>a <% b %> c</p> <% d",
            "<script>x</script><% a %> <% b",
            "<p title=é>x</p> <% b",
            "<% x %>\n<% y",
            "<a href=<%= u -%>\ntitle=\"x>y <% z\">",
            "<p a=\"x\">\n  <% x %>\n<%= a -%>\n<%= u =%>\nb\"> <% y",
            "<p data-é=\"1\"> <% y",
            "<!-- é --> <% y",
            "<p a=b=\"x> <% y",
            "<p a==\"x> <% y",
            "<p a = = \"x> <% y",
            "<p =\"x> <% y",
            "<svg><style>a <% y",
            "<svg><script>a <% y",
            "<svg></svg><style>a</style> <% y",
            "<svg/><style>a</style> <% y",
        ] {
            assert_eq!(unclosed(text).len(), 1, "{text}");
        }
    }

    #[test]
    fn lazy_match_leaves_no_text_for_html() {
        assert!(unclosed("<% a <% b %>").is_empty());
        assert!(unclosed("<% a %><%% lit %> <%# c %>").is_empty());
        assert!(unclosed("").is_empty());
    }

    #[test]
    fn code_spans_cross_lines() {
        let code_start = |text: &str, offset| span_at(&code_spans(text), offset);
        let text = "<p>\n<%= link_to \"x\",\n  user_path(u) %> after <%# note %> <%% lit %> <% open";
        let at = |needle: &str| text.find(needle).unwrap();
        assert_eq!(code_start(text, at("user_path")), Some(at("<%=") + 2));
        assert_eq!(code_start(text, at("%> after") + 1), Some(at("<%=") + 2));
        assert_eq!(code_start(text, at("after")), None);
        assert_eq!(code_start(text, at("note")), None);
        assert_eq!(code_start(text, at("lit")), None);
        assert_eq!(code_start(text, at("open")), Some(at("<% open") + 2));
        assert_eq!(code_start(text, at("<p>")), None);
    }

    #[test]
    fn tag_docs() {
        let line = "<%== a -%> <%= b =%> <%- c %> <%# d %> <%% e %> <% f %>";
        let doc = |needle: &str, shift: usize| tag_doc(line, line.find(needle).unwrap() + shift).map(|d| d.split("**").nth(1).unwrap());
        assert_eq!(doc("<%==", 3), Some("`<%==`"));
        assert_eq!(doc("-%>", 0), Some("`-%>`"));
        assert_eq!(doc("-%>", 2), Some("`-%>`"));
        assert_eq!(doc("<%=", 0), Some("`<%==`"));
        assert_eq!(doc("<%= b", 2), Some("`<%=`"));
        assert_eq!(doc("=%>", 1), Some("`=%>`"));
        assert_eq!(doc("<%-", 2), Some("`<%-`"));
        assert_eq!(doc("<%#", 2), Some("`<%#`"));
        assert_eq!(doc("<%%", 2), Some("`<%%`"));
        assert_eq!(doc("<% f", 1), Some("`<%`"));
        assert_eq!(doc("f %>", 2), Some("`%>`"));
        assert_eq!(doc("<% f", 2), None);
        assert_eq!(doc("a -%>", 0), None);
    }
}
