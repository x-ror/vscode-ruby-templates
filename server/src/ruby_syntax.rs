//! Ruby syntax errors inside ERB tags, found by parsing the template's Ruby the way Erubi compiles it.

use crate::util::{error, offset_to_position};
use tower_lsp::lsp_types::{Diagnostic, Range};
use tree_sitter::{Node, Parser, Tree};

/// One broken tag tends to cascade; the first few errors are the useful ones.
const MAX_ERRORS: usize = 3;
const NEAR_CHARS: usize = 20;

/// The template with every byte outside a code tag blanked (newlines kept) and a `;` where each tag closes,
/// as Erubi joins tags; offsets stay those of the template, so errors map back without a table.
pub fn compiled_ruby(text: &str, template: &Tree) -> String {
    let mut out = vec![b' '; text.len()];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            out[i] = b'\n';
        }
    }
    let mut cursor = template.walk();
    for tag in template.root_node().children(&mut cursor) {
        if tag.kind() != "directive" && tag.kind() != "output_directive" {
            continue;
        }
        let mut inner = tag.walk();
        let Some(code) = tag.children(&mut inner).find(|n| n.kind() == "code") else { continue };
        out[code.byte_range()].copy_from_slice(&text.as_bytes()[code.byte_range()]);
        if let Some(close) = out.get_mut(code.end_byte()).filter(|b| **b != b'\n') {
            *close = b';';
        }
    }
    // Only whole code ranges were copied, so the bytes are the template's UTF-8 split at ASCII delimiters.
    String::from_utf8(out).unwrap_or_default()
}

/// Syntax errors in the Ruby of `text`'s `<% %>` and `<%= %>` tags.
pub fn diagnostics(text: &str, template: &Tree) -> Vec<Diagnostic> {
    if !template.root_node().children(&mut template.walk()).any(|n| n.kind() == "directive" || n.kind() == "output_directive") {
        return vec![];
    }
    let ruby = compiled_ruby(text, template);
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_ruby::LANGUAGE.into()).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(&ruby, None) else { return vec![] };
    let mut errors = Vec::new();
    collect_errors(tree.root_node(), &mut errors);
    errors.dedup_by_key(|n| n.start_position().row);
    errors
        .into_iter()
        .take(MAX_ERRORS)
        .map(|node| {
            let range = Range::new(offset_to_position(text, node.start_byte()), offset_to_position(text, node.end_byte()));
            let near = ruby[node.byte_range()].split_whitespace().next().unwrap_or("");
            let message = match (node.is_missing(), near) {
                (true, _) => format!("Ruby syntax error: missing `{}`", node.kind()),
                (false, "") => "Ruby syntax error".to_string(),
                (false, near) => format!("Ruby syntax error near `{}`", near.chars().take(NEAR_CHARS).collect::<String>()),
            };
            error(range, message)
        })
        .collect()
}

/// Outermost error nodes in document order; an error's own children are part of it.
fn collect_errors<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    if node.is_error() || node.is_missing() {
        out.push(node);
        return;
    }
    if !node.has_error() {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_errors(child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(text: &str) -> Vec<(u32, u32, String)> {
        let template = crate::erb::parse(text).unwrap();
        diagnostics(text, &template).into_iter().map(|d| (d.range.start.line, d.range.start.character, d.message)).collect()
    }

    #[test]
    fn tags_join_like_erubi() {
        let ruby =
            compiled_ruby("<p>é</p><% if a %>x<%= b %><% end %>", &crate::erb::parse("<p>é</p><% if a %>x<%= b %><% end %>").unwrap());
        assert_eq!(ruby, "            if a ;      b ;    end ; ");
        assert!(errors("<% items.each do |i| %>\n<%= i %>\n<% end %>\n<%= x = <<~T\n  don't\nT\n%>").is_empty());
        assert!(errors("<% case x %>\n<% when 1 %>a<% else %>b<% end %>").is_empty());
        assert!(errors("<p>plain html with ( and ' quotes</p>").is_empty());
    }

    #[test]
    fn errors_point_into_the_tag() {
        assert_eq!(errors("é<%= link_to \"x\", foo( %>\n<p></p>\n"), [(0, 21, "Ruby syntax error near `(`".to_string())]);
        let two_statements = errors("<% x = 1 y = 2 %>");
        assert_eq!(two_statements.len(), 1, "{two_statements:?}");
        assert_eq!(two_statements[0].0, 0);
    }
}
