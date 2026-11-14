//! HTML completion in ERB template text: element names after `<`, the open element after `</`, attribute names
//! inside a start tag.

use crate::erb::html_lint::{inside, non_markup};
use crate::erb::tags;
use crate::html;
use crate::html::{BOOLEAN_ATTRIBUTES, NO_END_TAG, TAGS};
use crate::util::completion_item;
use regex::Regex;
use std::sync::LazyLock;
use tower_lsp_server::ls_types::{CompletionItem, CompletionItemKind, InsertTextFormat};

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<(/?)([a-zA-Z][\w:-]*)").unwrap());

#[derive(Debug, PartialEq)]
pub enum Context<'a> {
    /// `<di|`: the element name typed so far.
    Element(&'a str),
    /// `</|` or `</di|`.
    Close(&'a str),
    /// `<div cla|`: the attribute name typed so far.
    Attribute(&'a str),
}

/// What the cursor at byte `offset` of `text` is completing, in markup (not ERB, comments, `<script>` or `<style>`).
pub fn context(text: &str, offset: usize) -> Option<Context<'_>> {
    let before = text.get(..offset)?;
    let regions = non_markup(before);
    if inside(&regions, offset.saturating_sub(1)) {
        return None;
    }
    let head = before.trim_end_matches(|c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | ':'));
    let typed = &before[head.len()..];
    if head.ends_with("</") {
        return Some(Context::Close(typed));
    }
    if head.ends_with('<') {
        return typed.chars().next().is_none_or(|c| c.is_ascii_alphabetic()).then_some(Context::Element(typed));
    }
    let after_space = head.ends_with(char::is_whitespace);
    (after_space && in_start_tag(head, &regions)).then_some(Context::Attribute(typed))
}

/// The start tag the end of `before` is in, from its `<`: ERB tags and a `<` not opening a tag name are skipped.
pub fn start_tag(before: &str) -> Option<&str> {
    let regions = non_markup(before);
    let open = tag_open(before, &regions)?;
    in_start_tag(before, &regions).then(|| &before[open..])
}

/// Where the last tag before the end of `head` opens: a `<` outside ERB tags followed by a name.
fn tag_open(head: &str, regions: &[(usize, usize)]) -> Option<usize> {
    head.rmatch_indices('<')
        .map(|(at, _)| at)
        .find(|&at| !inside(regions, at) && head[at + 1..].starts_with(|c: char| c.is_ascii_alphabetic()))
}

/// Is the end of `head` between a start tag's name and its `>`, outside quotes and ERB tags?
fn in_start_tag(head: &str, regions: &[(usize, usize)]) -> bool {
    let Some(open) = tag_open(head, regions) else { return false };
    let mut quote = None;
    for (at, c) in head[open..].char_indices().map(|(i, c)| (open + i, c)) {
        if inside(regions, at) {
            continue;
        }
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return false,
            _ => {}
        }
    }
    quote.is_none()
}

fn mask(text: &str, regions: &[(usize, usize)]) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    for &(start, end) in regions {
        bytes[start..end.min(text.len())].iter_mut().filter(|b| **b != b'\n').for_each(|b| *b = tags::ERB_FILL);
    }
    bytes
}

/// Where the tag whose name ends at `from` ends, and whether it is self-closing; `None` while it is still being typed
/// (`<a hre`: no `>` before the next tag). `masked` has its ERB tags and non-markup blanked.
fn start_tag_end(masked: &[u8], from: usize) -> Option<(usize, bool)> {
    let tag = tags::tag_end(masked, from)?;
    let in_quote = |at: usize| tag.quoted.iter().any(|&(start, end)| (start..end).contains(&at));
    let opens_other_tag = (from..tag.end).any(|at| masked[at] == b'<' && !in_quote(at));
    (!opens_other_tag).then_some((tag.end, tag.self_closing))
}

/// The innermost element still open at the end of `before`, by a plain scan of its markup.
fn open_element(before: &str) -> Option<String> {
    let regions = non_markup(before);
    let masked = mask(before, &regions);
    let mut stack: Vec<String> = Vec::new();
    let mut resume = 0;
    for cap in TAG.captures_iter(before) {
        let (Some(whole), Some(name)) = (cap.get(0), cap.get(2)) else { continue };
        if whole.start() < resume || inside(&regions, whole.start()) {
            continue;
        }
        let name = name.as_str().to_ascii_lowercase();
        let Some((tag_end, self_closing)) = start_tag_end(&masked, whole.end()) else { continue };
        resume = tag_end;
        if &cap[1] == "/" {
            if let Some(at) = stack.iter().rposition(|open| *open == name) {
                stack.truncate(at);
            }
        } else if !NO_END_TAG.contains(&name.as_str()) && !self_closing {
            stack.push(name);
        }
    }
    stack.pop()
}

/// `stimulus` gives the attributes the element's Stimulus controllers read, asked only for attribute names.
pub fn items(context: Context, text: &str, offset: usize, stimulus: impl FnOnce() -> Vec<String>) -> Vec<CompletionItem> {
    match context {
        Context::Element(typed) => TAGS
            .iter()
            .filter(|t| t.starts_with(typed))
            .map(|t| completion_item(*t, CompletionItemKind::PROPERTY, "HTML element"))
            .collect(),
        Context::Close(typed) => open_element(&text[..offset])
            .filter(|name| name.starts_with(typed))
            .map(|name| CompletionItem {
                insert_text: Some(format!("{name}>")),
                ..completion_item(format!("/{name}"), CompletionItemKind::PROPERTY, "Close the open element")
            })
            .into_iter()
            .collect(),
        Context::Attribute(typed) => html::attribute_names(stimulus())
            .filter(|a| a.starts_with(typed))
            .map(|name| {
                let snippet = if BOOLEAN_ATTRIBUTES.contains(&name.as_str()) { name.clone() } else { format!("{name}=\"$1\"") };
                CompletionItem {
                    insert_text: Some(snippet),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..completion_item(name, CompletionItemKind::PROPERTY, "HTML attribute")
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_end(text: &str) -> Option<Context<'_>> {
        context(text, text.len())
    }

    #[test]
    fn contexts() {
        assert_eq!(at_end("<p>x</p>\n<di"), Some(Context::Element("di")));
        assert_eq!(at_end("<"), Some(Context::Element("")));
        assert_eq!(at_end("<%"), None);
        assert_eq!(at_end("<div class=\"a\" da"), Some(Context::Attribute("da")));
        assert_eq!(at_end("<div\n  "), Some(Context::Attribute("")));
        assert_eq!(at_end("<div class=\"a b"), None);
        assert_eq!(at_end("<div class=\"<%= x %>\" "), Some(Context::Attribute("")));
        assert_eq!(at_end("<div <%= attrs if a > b %> ti"), Some(Context::Attribute("ti")));
        assert_eq!(at_end("<div>text and"), None);
        assert_eq!(at_end("<ul><li></"), Some(Context::Close("")));
        assert_eq!(at_end("a < b"), None);
        for raw in ["<%# <di", "<!-- <di", "<script>\nfor (let i = 0; i<n; i++) {\n  ", "<script>if (a <b", "<style>.a{} </"] {
            assert_eq!(at_end(raw), None, "{raw}");
        }
        assert_eq!(at_end("<script>x</script>\n<di"), Some(Context::Element("di")));
    }

    #[test]
    fn close_suggests_the_open_element() {
        let close = |text: &str| {
            items(Context::Close(""), text, text.len(), Vec::new).into_iter().map(|i| i.insert_text.unwrap()).collect::<Vec<_>>()
        };
        assert_eq!(close("<div><p>x</p><br><img src=\"a\"/><span>"), ["span>"]);
        assert_eq!(close("<section><% if a %><b>x</b><% end %>"), ["section>"]);
        assert_eq!(close("<div><%= \"<p>\" %>"), ["div>"]);
        assert!(close("<div></div>").is_empty());
        assert_eq!(close("<div><a href=x/ >y"), ["a>"]);
        assert_eq!(close("<div><a href=x/>y"), ["a>"]);
        assert_eq!(close("<div><i />"), ["div>"]);
        assert_eq!(close("<%# <section> %><div><p></p>"), ["div>"]);
        assert_eq!(close("<section>\n  <butt\n  <a hre\n  "), ["section>"]);
        assert_eq!(close("<div title=\"<b>\"><p></p></"), ["div>"]);
        assert_eq!(close("<div <%= attrs if a > b %>><p></p></"), ["div>"]);
        assert_eq!(close("<section><a href=<%= u %>/>y"), ["a>"]);
    }

    #[test]
    fn start_tags_skip_erb_and_text() {
        assert_eq!(
            start_tag(r#"<p>x</p><div data-controller="a" <%= attrs %> data-"#),
            Some(r#"<div data-controller="a" <%= attrs %> data-"#)
        );
        assert_eq!(start_tag(r#"<div title="a < b" data-"#), Some(r#"<div title="a < b" data-"#));
        assert_eq!(start_tag("<div>text "), None);
    }

    #[test]
    fn attribute_snippets() {
        let item = |typed| items(Context::Attribute(typed), "", 0, Vec::new).into_iter().next().map(|i| i.insert_text.unwrap());
        assert_eq!(item("hre").as_deref(), Some("href=\"$1\""));
        assert_eq!(item("disab").as_deref(), Some("disabled"));
        assert!(items(Context::Attribute("data-contr"), "", 0, Vec::new).iter().any(|i| i.label == "data-controller"));
    }
}
