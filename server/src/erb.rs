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
