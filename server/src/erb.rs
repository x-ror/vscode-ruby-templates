//! ERB diagnostics via tree-sitter-embedded-template + Ruby block balancing.

use crate::ruby::{analyze, code_end, ends_with_block, BlockEffect, CodeEnd};
use crate::util::{coded, error, offset_to_position, warning};
use regex::Regex;
use std::sync::LazyLock;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use tree_sitter::{Node, Parser, Tree};

// Helpers whose block content only reaches the page through their return value.
// `cache`, `content_for`, `capture` are deliberately absent: `<% … do %>` is correct for them.
static OUTPUT_BLOCK_HELPER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(form_with|form_for|form_tag|fields_for|fields|link_to|button_to|button_tag|label_tag|content_tag|turbo_frame_tag|tag\.\w+)(?:\(|\s+[^=\s])").unwrap()
});

/// Stable diagnostic codes; `rubyTemplates.erb.disabledDiagnostics` refers to these.
pub mod code {
    pub const UNCLOSED_TAG: &str = "unclosed-tag";
    pub const MALFORMED_TAG: &str = "malformed-tag";
    pub const STRAY_END: &str = "stray-end";
    pub const UNMATCHED_BRANCH: &str = "unmatched-branch";
    pub const UNCLOSED_BLOCK: &str = "unclosed-block";
    pub const OUTPUT_CONTROL_FLOW: &str = "output-control-flow";
    pub const SILENT_OUTPUT_HELPER: &str = "silent-output-helper";
    pub const UNTERMINATED_STRING: &str = "unterminated-string";
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

pub fn diagnostics(text: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let Some(tree) = parse(text) else { return out };
    let root = tree.root_node();

    // Syntax errors reported by the grammar (unclosed `<%`, stray `%>`).
    let mut cursor = root.walk();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            let (msg, target, code) = if node.is_missing() {
                (format!("Unclosed ERB tag: missing `{}`", node.kind()), node.parent().unwrap_or(node), code::UNCLOSED_TAG)
            } else {
                ("Malformed ERB tag".to_string(), node, code::MALFORMED_TAG)
            };
            let r = node_range(text, target);
            // Point at the opening line only, not the whole rest of the file.
            let r = Range::new(r.start, Position::new(r.start.line, u32::MAX));
            out.push(coded(error(r, msg), code));
        }
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    // Ruby block balance across all code tags.
    struct Opener {
        range: Range,
        snippet: String,
    }
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
        let range = node_range(text, node);
        // A tag left open runs on to the next one's `%>`, swallowing the tags between.
        // One that starts a line counts even inside a string, which an unfinished `"…` usually is.
        let line_start = |at: usize| snippet[..at].rsplit('\n').next().is_some_and(|l| l.trim().is_empty() && snippet[..at].contains('\n'));
        let swallowed = snippet.match_indices("<%").find(|&(at, _)| line_start(at) || code_end(&snippet[..at]) == CodeEnd::Code);
        if swallowed.is_some() {
            let opener = Range::new(range.start, Position::new(range.start.line, u32::MAX));
            out.push(coded(error(opener, "Unclosed ERB tag: missing `%>` before the next `<%`"), code::UNCLOSED_TAG));
            continue;
        }
        let (effect, _, _) = analyze(snippet);
        match effect {
            BlockEffect::Open(n) => {
                for _ in 0..n {
                    openers.push(Opener { range, snippet: snippet.trim().to_string() });
                }
            }
            BlockEffect::Close(n) => {
                for _ in 0..n {
                    if openers.pop().is_none() {
                        out.push(coded(error(range, "Unexpected `end`: no open block"), code::STRAY_END));
                    }
                }
            }
            BlockEffect::Continue => {
                if openers.is_empty() {
                    out.push(coded(error(range, format!("`{}` without an open block", snippet.trim())), code::UNMATCHED_BRANCH));
                }
            }
            BlockEffect::Neutral => {}
        }
        // ERB ends a tag at the first `%>`, even inside a string; the `%>` meant to close it follows on the line.
        // A tag still being typed has no `%>` at all, and a heredoc body may hold lone quotes.
        let rest_of_line = text[node.end_byte()..].split('\n').next().unwrap_or("");
        let cut_short = text[..node.end_byte()].ends_with("%>") && rest_of_line.contains("%>");
        if cut_short && code_end(snippet) == CodeEnd::String && !snippet.contains("<<") {
            let message = "String is not closed before `%>`: ERB ends the tag at the first `%>`, even inside a string";
            out.push(coded(warning(range, message), code::UNTERMINATED_STRING));
        }
        let is_output = text[node.start_byte()..].starts_with("<%=");
        let t = snippet.trim();
        if is_output {
            // `<%= if a then b else c end %>` is one balanced expression.
            let control_flow = t.starts_with("if ") || t.starts_with("unless ") || t == "end" || t.starts_with("end ");
            if control_flow && effect != BlockEffect::Neutral {
                let message = "Control flow in an output tag `<%= %>`; did you mean `<% %>`?";
                out.push(coded(warning(range, message), code::OUTPUT_CONTROL_FLOW));
            }
        } else if let Some(m) = OUTPUT_BLOCK_HELPER.captures(t).filter(|_| ends_with_block(t)) {
            let message = format!("Block helper `{}` in a non-output tag `<% %>` renders nothing; did you mean `<%= %>`?", &m[1]);
            out.push(coded(warning(range, message), code::SILENT_OUTPUT_HELPER));
        }
    }
    for o in openers {
        let message = format!("Block `{}` is never closed: missing `<% end %>`", truncate(&o.snippet, 40));
        out.push(coded(error(o.range, message), code::UNCLOSED_BLOCK));
    }
    // Ruby parse errors would only repeat a broken tag or block structure, so they wait until it is sound.
    if out.iter().all(|d| d.severity != Some(DiagnosticSeverity::ERROR)) {
        out.extend(crate::ruby_syntax::diagnostics(text, &tree).into_iter().map(|d| coded(d, code::RUBY_SYNTAX)));
    }
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
    use tower_lsp::lsp_types::{DiagnosticSeverity, NumberOrString};

    fn only(text: &str) -> Diagnostic {
        let diags = diagnostics(text);
        assert_eq!(diags.len(), 1, "{diags:?}");
        diags.into_iter().next().unwrap()
    }

    fn span(line: u32, start: u32, end: u32) -> Range {
        Range::new(Position::new(line, start), Position::new(line, end))
    }

    #[test]
    fn tag_left_open_before_the_next_one() {
        let d = only("<p>é</p><%= render \"sh\n<%= link_to x %>\n");
        assert_eq!((d.range.start, d.code), (Position::new(0, 8), Some(NumberOrString::String(code::UNCLOSED_TAG.into()))));
        assert!(diagnostics(r#"<%= "<%" + x %>"#).is_empty());
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
        let want = [(5, code::OUTPUT_CONTROL_FLOW), (8, code::STRAY_END), (9, code::UNCLOSED_TAG)];
        assert_eq!(got, want.map(|(line, code)| (line, code.to_string())));
    }

    #[test]
    fn balanced_if_expression_in_output_tag_is_fine() {
        assert!(diagnostics("<%= if a\n  b\nelse\n  c\nend %>").is_empty());
        assert_eq!(only("<%= if a %>x<% end %>").code, Some(NumberOrString::String(code::OUTPUT_CONTROL_FLOW.into())));
    }

    #[test]
    fn percent_gt_inside_a_string_ends_the_tag() {
        let codes = |t: &str| diagnostics(t).into_iter().filter_map(|d| d.code).collect::<Vec<_>>();
        let unterminated = NumberOrString::String(code::UNTERMINATED_STRING.into());
        let ruby_syntax = NumberOrString::String(code::RUBY_SYNTAX.into());
        assert_eq!(codes(r#"<%= "50%>" %>"#), [unterminated.clone(), ruby_syntax.clone()]);
        assert_eq!(codes("<% x = 'a%>' %>"), [unterminated.clone(), ruby_syntax]);
        for clean in [
            r#"<%= render "sh"#,
            "<%= link_to 'x\n<p>%></p>",
            r#"<%= "50%" %>"#,
            "<%= x # don't %>",
            "<% x = <<~T\n  don't\nT\n%>",
            r#"<%= "a\"b" %>"#,
        ] {
            assert!(!codes(clean).contains(&unterminated), "{clean}");
        }
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

    #[test]
    fn return_modifier_does_not_open_a_block() {
        assert!(diagnostics("<% return unless current_user %>\n<p>hi</p>").is_empty());
        assert!(diagnostics("<% return if x %>").is_empty());
        assert!(only("<% if x %>\n<p>hi</p>").message.starts_with("Block `if x` is never closed"));
    }
}
