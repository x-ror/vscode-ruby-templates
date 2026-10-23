//! ERB diagnostics via tree-sitter-embedded-template + Ruby block balancing.

use crate::ruby::{analyze, BlockEffect};
use crate::util::{error, offset_to_position, warning};
use tower_lsp::lsp_types::{Diagnostic, Position, Range};
use tree_sitter::{Node, Parser};

fn node_range(text: &str, node: Node) -> Range {
    Range::new(offset_to_position(text, node.start_byte()), offset_to_position(text, node.end_byte()))
}

pub fn diagnostics(text: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_embedded_template::LANGUAGE.into()).is_err() {
        return out;
    }
    let Some(tree) = parser.parse(text, None) else { return out };
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
        if is_output {
            let t = snippet.trim();
            if t.starts_with("if ") || t.starts_with("unless ") || t == "end" || t.starts_with("end ") {
                out.push(warning(range, "Control flow in an output tag `<%= %>`; did you mean `<% %>`?"));
            }
        }
    }
    for o in openers {
        out.push(error(o.range, format!("Block `{}` is never closed: missing `<% end %>`", truncate(&o.snippet, 40))));
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_string() }
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
    fn never_closed_block_truncates_long_snippet() {
        let d = only("é\n  😀<% items_with_a_very_long_name.each_with_index do |item, i| %>");
        assert_eq!(d.message, "Block `items_with_a_very_long_name.each_with_in…` is never closed: missing `<% end %>`");
        assert_eq!(d.range.start, Position::new(1, 4));
    }
}
