//! ERB diagnostics via tree-sitter-embedded-template + Ruby block balancing.

use crate::ruby::{analyze, ends_with_block, BlockEffect};
use crate::util::{error, offset_to_position, warning};
use once_cell::sync::Lazy;
use regex::Regex;
use tower_lsp::lsp_types::{Diagnostic, Position, Range};
use tree_sitter::{Node, Parser, Tree};

// Helpers whose block content only reaches the page through their return value.
// `cache`, `content_for`, `capture` are deliberately absent: `<% … do %>` is correct for them.
static OUTPUT_BLOCK_HELPER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(form_with|form_for|form_tag|fields_for|fields|link_to|button_to|button_tag|label_tag|content_tag|turbo_frame_tag|tag\.\w+)(?:\(|\s+[^=\s])").unwrap()
});

fn node_range(text: &str, node: Node) -> Range {
    Range::new(offset_to_position(text, node.start_byte()), offset_to_position(text, node.end_byte()))
}

pub fn parse(text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_embedded_template::LANGUAGE.into()).ok()?;
    parser.parse(text, None)
}

pub fn diagnostics(text: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let Some(tree) = parse(text) else { return out };
    let root = tree.root_node();

    // Syntax errors reported by the grammar (unclosed `<%`, stray `%>`).
    let mut cursor = root.walk();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            let (msg, target) = if node.is_missing() {
                (format!("Unclosed ERB tag: missing `{}`", node.kind()), node.parent().unwrap_or(node))
            } else {
                ("Malformed ERB tag".to_string(), node)
            };
            let r = node_range(text, target);
            // Point at the opening line only, not the whole rest of the file.
            let r = Range::new(r.start, Position::new(r.start.line, u32::MAX));
            out.push(error(r, msg));
        }
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    // Ruby block balance across all code tags.
    struct Opener { range: Range, snippet: String }
    let mut openers: Vec<Opener> = Vec::new();
    let mut cursor = root.walk();
    for node in root.children(&mut cursor) {
        let kind = node.kind();
        if kind != "directive" && kind != "output_directive" {
            continue;
        }
        let mut c2 = node.walk();
        let Some(code) = node.children(&mut c2).find(|n| n.kind() == "code") else { continue };
        let snippet = &text[code.byte_range()];
        let (effect, _, _) = analyze(snippet);
        let range = node_range(text, node);
        match effect {
            BlockEffect::Open(n) => {
                for _ in 0..n {
                    openers.push(Opener { range, snippet: snippet.trim().to_string() });
                }
            }
            BlockEffect::Close(n) => {
                for _ in 0..n {
                    if openers.pop().is_none() {
                        out.push(error(range, "Unexpected `end`: no open block"));
                    }
                }
            }
            BlockEffect::Continue => {
                if openers.is_empty() {
                    out.push(error(range, format!("`{}` without an open block", snippet.trim())));
                }
            }
            BlockEffect::Neutral => {}
        }
        let is_output = text[node.start_byte()..].starts_with("<%=");
        let t = snippet.trim();
        if is_output {
            if t.starts_with("if ") || t.starts_with("unless ") || t == "end" || t.starts_with("end ") {
                out.push(warning(range, "Control flow in an output tag `<%= %>`; did you mean `<% %>`?"));
            }
        } else if let Some(m) = OUTPUT_BLOCK_HELPER.captures(t).filter(|_| ends_with_block(t)) {
            out.push(warning(range, format!("Block helper `{}` in a non-output tag `<% %>` renders nothing; did you mean `<%= %>`?", &m[1])));
        }
    }
    for o in openers {
        out.push(error(o.range, format!("Block `{}` is never closed: missing `<% end %>`", truncate(&o.snippet, 40))));
    }
    out
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_string() }
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
    opener.or_else(|| line.match_indices("%>").find_map(|(i, _)| {
        let start = if i > 0 && matches!(line.as_bytes()[i - 1], b'-' | b'=') { i - 1 } else { i };
        token_at(start, CLOSERS)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::DiagnosticSeverity;

    fn only(text: &str) -> Diagnostic {
        let diags = diagnostics(text);
        assert_eq!(diags.len(), 1, "{diags:?}");
        diags.into_iter().next().unwrap()
    }

    fn span(line: u32, start: u32, end: u32) -> Range {
        Range::new(Position::new(line, start), Position::new(line, end))
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
    fn malformed_tag() {
        // The grammar only yields an ERROR node (vs. a MISSING `%>`) for an unclosed tag at document start.
        let d = only("<%# 😀");
        assert_eq!(d.message, "Malformed ERB tag");
        assert_eq!(d.range, span(0, 0, u32::MAX));
    }

    #[test]
    fn unexpected_end_uses_utf16_columns() {
        // 😀 is 2 UTF-16 units, é is 1: tag starts at column 4.
        let d = only("😀é <% end %>");
        assert_eq!(d.message, "Unexpected `end`: no open block");
        assert_eq!(d.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(d.range, span(0, 4, 13));
    }

    #[test]
    fn continuation_without_block() {
        assert_eq!(only("<% elsif y %>").message, "`elsif y` without an open block");
    }

    #[test]
    fn control_flow_in_output_tag_warns() {
        let d = only("<% if x %><%= end %>");
        assert_eq!(d.message, "Control flow in an output tag `<%= %>`; did you mean `<% %>`?");
        assert_eq!(d.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(d.range, span(0, 10, 20));
    }

    #[test]
    fn block_helper_in_non_output_tag_warns() {
        let d = only("<% form_with model: @u do |f| %><% end %>");
        assert_eq!(d.message, "Block helper `form_with` in a non-output tag `<% %>` renders nothing; did you mean `<%= %>`?");
        assert_eq!(d.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(d.range, span(0, 0, 32));
        assert!(only("<%- tag.div(class: 'x') do %><% end %>").message.contains("`tag.div`"));
        assert!(only("<% link_to path do %><% end %>").message.contains("`link_to`"));
    }

    #[test]
    fn block_helper_false_positives() {
        for ok in [
            "<%= form_with model: @u do |f| %><% end %>",
            "<% @items.each do |i| %><% end %>",
            "<% cache @u do %><% end %>",
            "<% content_for :title do %><% end %>",
            "<% if link_to %><% end %>",
            "<% fields.each do |f| %><% end %>",
            "<% fields = list.map do |f| %><% end %>",
            "<% link_to_thing do %><% end %>",
            "<% link_to 'a', b %>",
            "<%# form_with do %>",
        ] {
            assert!(diagnostics(ok).is_empty(), "{ok}: {:?}", diagnostics(ok));
        }
    }

    #[test]
    fn never_closed_block_truncates_long_snippet() {
        let d = only("é\n  😀<% items_with_a_very_long_name.each_with_index do |item, i| %>");
        assert_eq!(d.message, "Block `items_with_a_very_long_name.each_with_in…` is never closed: missing `<% end %>`");
        assert_eq!(d.range.start, Position::new(1, 4));
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
