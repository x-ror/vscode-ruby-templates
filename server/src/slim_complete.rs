//! Slim markup completion: tag and embedded engine names where a tag starts, attribute names
//! where an attribute may start.

use crate::helpers::DATA_KEYS;
use crate::html::{ATTRIBUTES, BOOLEAN_ATTRIBUTES, TAGS};
use crate::slim_parser::{Parsed, SLIM_ENGINES};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, InsertTextFormat};

/// Items for the word ending at byte `offset`; `None` when it is neither a tag nor an attribute name. `stimulus` are the
/// attributes the element's Stimulus controllers read.
pub fn items(text: &str, offset: usize, parsed: &Parsed, extra_engines: &[String], stimulus: &[String]) -> Option<Vec<CompletionItem>> {
    let before = &text[text[..offset].rfind('\n').map_or(0, |i| i + 1)..offset];
    let tag = word_start(before, |c| c.is_alphanumeric() || matches!(c, '_' | '-' | ':'));
    if parsed.tag_slots.contains(&(offset - tag.len())) && may_start(before, tag) {
        let engines = SLIM_ENGINES.iter().copied().chain(extra_engines.iter().map(String::as_str));
        let engines = engines.map(|name| CompletionItem {
            label: format!("{name}:"),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some("Slim embedded engine".into()),
            ..Default::default()
        });
        let tags = TAGS.iter().map(|name| CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some("HTML element".into()),
            ..Default::default()
        });
        return Some(tags.chain(engines).collect());
    }
    let attr = word_start(before, |c| !c.is_whitespace() && !"\"'<>/=()[]{}".contains(c));
    if !parsed.attr_slots.contains(&(offset - attr.len())) || !may_start(before, attr) {
        return None;
    }
    let data = DATA_KEYS.iter().map(|key| format!("data-{}", key.replace('_', "-"))).filter(|name| !ATTRIBUTES.contains(&name.as_str()));
    let names = ATTRIBUTES.iter().map(|name| name.to_string()).chain(data).chain(stimulus.iter().cloned());
    let valued = names.map(|name| CompletionItem {
        label: name.clone(),
        kind: Some(CompletionItemKind::FIELD),
        insert_text: Some(format!("{name}=\"$1\"")),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    });
    let boolean = BOOLEAN_ATTRIBUTES.iter().map(|name| CompletionItem {
        label: name.to_string(),
        kind: Some(CompletionItemKind::FIELD),
        ..Default::default()
    });
    Some(valued.chain(boolean).collect())
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
    use crate::slim_parser::parse;

    fn labels(text: &str) -> Option<Vec<String>> {
        let offset = text.find('|').expect("cursor");
        let text = text.replacen('|', "", 1);
        let parsed = parse(&text, &[]);
        items(&text, offset, &parsed, &["mjml".into()], &[]).map(|items| items.into_iter().map(|i| i.label).collect())
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
        let stimulus = items("div data-controller=\"x\" da", 26, &parsed, &[], &["data-x-target".into()]).unwrap();
        assert!(stimulus.iter().any(|i| i.label == "data-x-target"));
    }
}
