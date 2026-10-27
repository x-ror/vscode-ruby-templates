//! ERB diagnostics via tree-sitter-embedded-template, plus the Ruby syntax errors of `erb_ruby`.

use crate::checks::RubyContext;
use crate::erb_ruby;
use crate::util::{coded, error, offset_to_position};
use tower_lsp::lsp_types::{Diagnostic, Position, Range};
use tree_sitter::{Node, Parser, Tree};

/// Stable diagnostic codes; `rubyTemplates.erb.disabledDiagnostics` refers to these.
pub mod code {
    pub const UNCLOSED_TAG: &str = "unclosed-tag";
    pub const RUBY_SYNTAX: &str = "ruby-syntax";
}

fn node_range(text: &str, node: Node) -> Range {
    Range::new(offset_to_position(text, node.start_byte()), offset_to_position(text, node.end_byte()))
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
    let mut out = Vec::new();
    let Some(tree) = parse(text) else { return out };
    let root = tree.root_node();

    // The grammar marks a `<%` without `%>` with a MISSING `%>`, or with an ERROR node at the document start.
    let mut cursor = root.walk();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let unclosed = if node.is_missing() {
            Some(node.parent().unwrap_or(node))
        } else {
            let source = &text[node.byte_range()];
            (node.is_error() && source.starts_with("<%") && !source.contains("%>")).then_some(node)
        };
        if let Some(tag) = unclosed {
            let r = node_range(text, tag);
            // Point at the opening line only, not the whole rest of the file.
            let r = Range::new(r.start, Position::new(r.start.line, u32::MAX));
            out.push(coded(error(r, "Unclosed ERB tag: missing `%>`"), code::UNCLOSED_TAG));
        }
        stack.extend(node.children(&mut cursor));
    }

    out.extend(
        erb_ruby::diagnostics(text, ruby.grammar, ruby.strict_locals_identifier.as_deref())
            .into_iter()
            .map(|d| coded(d, code::RUBY_SYNTAX)),
    );
    out
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

    fn only(text: &str) -> Diagnostic {
        let diags = diagnostics(text);
        assert_eq!(diags.len(), 1, "{diags:?}");
        diags.into_iter().next().unwrap()
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
        let want =
            [(0, code::RUBY_SYNTAX), (5, code::RUBY_SYNTAX), (5, code::RUBY_SYNTAX), (5, code::RUBY_SYNTAX), (9, code::UNCLOSED_TAG)];
        assert_eq!(got, want.map(|(line, code)| (line, code.to_string())));
    }

    #[test]
    fn balanced_template_is_clean() {
        assert!(diagnostics("<% if x %>é<% else %>😀<% end %>\n<%= link_to 'a', b %>").is_empty());
    }

    #[test]
    fn unclosed_tag_points_at_opener_after_emoji() {
        let d = only("😀 <% foo");
        assert_eq!(d.message, "Unclosed ERB tag: missing `%>`");
        assert_eq!(d.range, span(0, 3, u32::MAX));
    }

    #[test]
    fn unclosed_tag_on_second_line_after_multibyte() {
        let d = only("é\né <%= foo");
        assert_eq!(d.message, "Unclosed ERB tag: missing `%>`");
        assert_eq!(d.range, span(1, 2, u32::MAX));
    }

    #[test]
    fn unclosed_tag_at_document_start() {
        for text in ["<% foo", "<%", "<%= foo", "<%- foo", "<%# x", "<%== x", "<%# 😀"] {
            let d = only(text);
            assert_eq!(d.message, "Unclosed ERB tag: missing `%>`", "{text}");
            assert_eq!(d.range, span(0, 0, u32::MAX), "{text}");
        }
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
