//! Slim markup completion: tag and embedded engine names where a tag starts, attribute names
//! where an attribute may start.

use crate::html::{attribute_names, BOOLEAN_ATTRIBUTES, TAGS};
use crate::slim::parser::{Parsed, SLIM_ENGINES};
use crate::util::completion_item;
use tower_lsp_server::ls_types::{CompletionItem, CompletionItemKind, InsertTextFormat};

/// Items for the word ending at byte `offset`; `None` when it is neither a tag nor an attribute name. `stimulus` gives the
/// attributes the element's Stimulus controllers read.
pub fn items(
    text: &str,
    offset: usize,
    parsed: &Parsed,
    extra_engines: &[String],
    stimulus: impl FnOnce() -> Vec<String>,
) -> Option<Vec<CompletionItem>> {
    let before = &text[text[..offset].rfind('\n').map_or(0, |i| i + 1)..offset];
    let tag = word_start(before, |c| c.is_alphanumeric() || matches!(c, '_' | '-' | ':'));
    if parsed.tag_slots.contains(&(offset - tag.len())) && may_start(before, tag) {
        let engines = SLIM_ENGINES.iter().copied().chain(extra_engines.iter().map(String::as_str));
        let engines = engines.map(|name| completion_item(format!("{name}:"), CompletionItemKind::MODULE, "Slim embedded engine"));
        let tags = TAGS.iter().map(|&name| completion_item(name, CompletionItemKind::PROPERTY, "HTML element"));
        return Some(tags.chain(engines).collect());
    }
    let attr = word_start(before, |c| !c.is_whitespace() && !"\"'<>/=()[]{}".contains(c));
    if !parsed.attr_slots.contains(&(offset - attr.len())) || !may_start(before, attr) {
        return None;
    }
    let item = |name: String| {
        let snippet = (!BOOLEAN_ATTRIBUTES.contains(&name.as_str())).then(|| format!("{name}=\"$1\""));
        CompletionItem {
            label: name,
            kind: Some(CompletionItemKind::FIELD),
            insert_text_format: snippet.is_some().then_some(InsertTextFormat::SNIPPET),
            insert_text: snippet,
            ..Default::default()
        }
    };
    Some(attribute_names(stimulus()).map(item).collect())
}

/// The trailing run of `before` made of `is_word` characters.
fn word_start(before: &str, is_word: impl Fn(char) -> bool) -> &str {
    let start = before.char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(before.len(), |(i, _)| i);
    &before[start..]
}

/// An empty word only counts after whitespace or an opening bracket, so typing `.` in `p.` lists nothing.
fn may_start(before: &str, word: &str) -> bool {
    !word.is_empty() || before.is_empty() || before.ends_with(|c: char| c.is_whitespace() || "([{".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slim::parser::parse;

    fn labels(text: &str) -> Option<Vec<String>> {
        let offset = text.find('|').expect("cursor");
        let text = text.replacen('|', "", 1);
        let parsed = parse(&text, &[]);
        items(&text, offset, &parsed, &["mjml".into()], Vec::new).map(|items| items.into_iter().map(|i| i.label).collect())
    }

    #[test]
    fn tag_and_engine_names() {
        let at_line_start = labels("p\n  jav|").unwrap();
        assert!(at_line_start.contains(&"javascript:".into()) && at_line_start.contains(&"mjml:".into()));
        assert!(at_line_start.contains(&"div".into()));
        assert!(labels("li: |").unwrap().contains(&"a".into()));
        assert_eq!(labels("p text tex|"), None);
        assert_eq!(labels("p.|"), None);
        assert_eq!(labels("| sp|"), None);
        assert_eq!(labels("- spa|"), None);
    }

    #[test]
    fn attribute_names() {
        for text in ["a hr|", "a href=\"/\" cl|", "input(type=\"x\" che|", "a |", "a(\n  cl|"] {
            assert!(labels(text).is_some_and(|l| l.contains(&"class".into())), "{text:?}");
        }
        for text in ["p Some text her|", "a href=x|", "a title=\"ti|\"", "= link_to cl|"] {
            assert_eq!(labels(text), None, "{text:?}");
        }
        let turbo = labels("a data-tu|").unwrap();
        assert!(turbo.contains(&"data-turbo-method".into()) && turbo.contains(&"data-turbo-confirm".into()), "{turbo:?}");
        let parsed = parse("div data-controller=\"x\" da", &[]);
        let stimulus = items("div data-controller=\"x\" da", 26, &parsed, &[], || vec!["data-x-target".into()]).unwrap();
        assert!(stimulus.iter().any(|i| i.label == "data-x-target"));
    }

    #[test]
    fn same_html_names_as_erb_and_haml() {
        let tags = labels("p\n  k|").unwrap();
        assert!(tags.contains(&"kbd".into()) && tags.contains(&"wbr".into()), "{tags:?}");
        let attributes = labels("a |").unwrap();
        for name in ["aria-describedby", "popover", "data-target"] {
            assert!(attributes.contains(&name.into()), "{name}");
        }
        assert_eq!(attributes.iter().filter(|name| *name == "disabled").count(), 1);
    }
}
