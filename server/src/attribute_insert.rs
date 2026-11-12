//! The edit that gives an element an attribute with an empty value, for the quick fixes that add `alt` or `id`: where it goes
//! and how the template's language spells it.

use crate::attributes::{last_argument_bytes, Element};
use crate::lang::TemplateLang;
use crate::util::find_balanced;
use ruby_prism::{CallNode, Visit};

/// The byte offset and the text that add the attribute `name` with an empty value to `element` of the template `text`; none when
/// the template doesn't show where.
pub(crate) fn empty_attribute(lang: TemplateLang, text: &str, element: &Element, name: &str) -> Option<(usize, String)> {
    if let Some(end) = element.last_argument_end {
        return Some((end, format!(", {name}: \"\"")));
    }
    let head = element.head_end?;
    match lang {
        TemplateLang::Erb => Some((head, format!(" {name}=\"\""))),
        TemplateLang::Haml => in_haml_tag(text, head, name),
        TemplateLang::Slim => in_slim_tag(text, head, name),
        _ => None,
    }
}

/// `%img{src: "a"}` takes `alt: ""` in its hash, `%img(src="a")` takes `alt=""` in its list, and a tag with neither a list.
fn in_haml_tag(text: &str, head: usize, name: &str) -> Option<(usize, String)> {
    let mut at = head;
    loop {
        match text[at..].chars().next() {
            // An object reference sets the id and the class, and holds no attributes.
            Some('[') => at += find_balanced(&text[at..], '[', ']')?,
            Some('{') => return inside_wrapper(text, at, '}', &format!("{name}: \"\""), ", "),
            Some('(') => return inside_wrapper(text, at, ')', &format!("{name}=\"\""), " "),
            _ => return Some((at, format!("({name}=\"\")"))),
        }
    }
}

/// `img(src="a")` and its `[]` and `{}` forms take `alt=""` inside; a tag without a wrapper takes ` alt=""` after its head.
fn in_slim_tag(text: &str, head: usize, name: &str) -> Option<(usize, String)> {
    let entry = format!("{name}=\"\"");
    match text[head..].chars().next() {
        Some('(') => inside_wrapper(text, head, ')', &entry, " "),
        Some('[') => inside_wrapper(text, head, ']', &entry, " "),
        Some('{') => inside_wrapper(text, head, '}', &entry, " "),
        _ => Some((head, format!(" {entry}"))),
    }
}

/// Appends `entry` after the last entry of the group that opens at `open` and ends with `close`; `separator` follows an entry
/// that has none after it.
fn inside_wrapper(text: &str, open: usize, close: char, entry: &str, separator: &str) -> Option<(usize, String)> {
    let opener = text[open..].chars().next()?;
    let end = open + find_balanced(&text[open..], opener, close)? - close.len_utf8();
    let content = text[open + opener.len_utf8()..end].trim_end();
    let inner_start = open + opener.len_utf8();
    if content.trim_start().is_empty() {
        return Some((inner_start, entry.to_string()));
    }
    let separator = if content.ends_with(',') { " " } else { separator };
    Some((inner_start + content.len(), format!("{separator}{entry}")))
}

/// Where `image_tag "a.png", class: "x"` takes `, alt: ""`: the end of the call's last argument, in `code` that starts with
/// the call and may go on. None when the code doesn't parse or the call has no arguments.
pub(crate) fn image_tag_arguments_end(code: &str) -> Option<usize> {
    // `link_to image_tag(a), url`: what follows the call's parentheses is another call's.
    let call = code.strip_prefix("image_tag").filter(|rest| rest.starts_with('(')).and_then(|rest| find_balanced(rest, '(', ')'));
    let code = call.map_or(code, |len| &code[.."image_tag".len() + len]);
    let parsed = ruby_prism::parse(code.as_bytes());
    if parsed.errors().next().is_some() {
        return None;
    }
    let mut finder = ImageTag { end: None };
    finder.visit(&parsed.node());
    finder.end
}

struct ImageTag {
    end: Option<usize>,
}

impl<'pr> Visit<'pr> for ImageTag {
    fn visit_call_node(&mut self, call: &CallNode<'pr>) {
        let starts_the_code = call.message_loc().is_some_and(|name| name.start() == 0);
        if self.end.is_none() && starts_the_code && call.receiver().is_none() && call.name().as_slice() == b"image_tag" {
            let arguments: Vec<_> = call.arguments().into_iter().flat_map(|list| list.arguments().iter().collect::<Vec<_>>()).collect();
            self.end = last_argument_bytes(&arguments).map(|(_, end)| end);
            return;
        }
        ruby_prism::visit_call_node(self, call);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::elements;
    use TemplateLang::{Erb, Haml, Slim};

    /// `text` with `name=""` added to its first element called `tag`.
    fn with_attribute(lang: TemplateLang, text: &str, tag: &str, name: &str) -> Option<String> {
        let found = elements(lang, text, &[], ruby_prism::Version::Latest);
        let element = found.iter().find(|element| element.name == tag)?;
        let (offset, added) = empty_attribute(lang, text, element, name)?;
        Some(format!("{}{added}{}", &text[..offset], &text[offset..]))
    }

    fn img(lang: TemplateLang, text: &str) -> String {
        with_attribute(lang, text, "img", "alt").unwrap_or_else(|| panic!("no edit for {text}"))
    }

    #[test]
    fn erb_markup_gets_the_attribute_after_the_tag_name() {
        assert_eq!(img(Erb, "<img src=\"a.png\">\n"), "<img alt=\"\" src=\"a.png\">\n");
        assert_eq!(img(Erb, "<p><img\n  src=\"a\"\n  class=\"b\"/></p>"), "<p><img alt=\"\"\n  src=\"a\"\n  class=\"b\"/></p>");
        assert_eq!(img(Erb, "<IMG src=x>"), "<IMG alt=\"\" src=x>");
    }

    #[test]
    fn helpers_get_a_keyword_argument_after_their_last_argument() {
        assert_eq!(img(Erb, "<%= image_tag \"a.png\" %>\n"), "<%= image_tag \"a.png\", alt: \"\" %>\n");
        assert_eq!(img(Erb, "<%= image_tag(\"a.png\", class: \"b\") %>"), "<%= image_tag(\"a.png\", class: \"b\", alt: \"\") %>");
        assert_eq!(img(Erb, "<%= image_tag \"a.png\", { class: \"b\" } %>"), "<%= image_tag \"a.png\", { class: \"b\", alt: \"\" } %>");
        assert_eq!(
            img(Erb, "<%= image_tag user.avatar_url, class: \"b\" if user %>"),
            "<%= image_tag user.avatar_url, class: \"b\", alt: \"\" if user %>"
        );
        assert_eq!(img(Haml, "= image_tag \"a.png\"\n"), "= image_tag \"a.png\", alt: \"\"\n");
        assert_eq!(img(Slim, "= image_tag(\"a.png\", size: \"2x2\")\n"), "= image_tag(\"a.png\", size: \"2x2\", alt: \"\")\n");
        assert_eq!(
            with_attribute(Erb, "<%= tag.div data: { turbo_permanent: true } %>", "div", "id").unwrap(),
            "<%= tag.div data: { turbo_permanent: true }, id: \"\" %>"
        );
        assert_eq!(with_attribute(Erb, "<%= image_tag %>", "img", "alt"), None);
        assert_eq!(with_attribute(Erb, "<%= image_tag \"a\", {} %>", "img", "alt"), None);
    }

    #[test]
    fn image_tag_in_erb_is_read_from_its_code() {
        assert_eq!(image_tag_arguments_end("image_tag \"a.png\""), Some(17));
        assert_eq!(image_tag_arguments_end("image_tag(\"a.png\", class: \"b\") if x"), Some(29));
        assert_eq!(image_tag_arguments_end("image_tag \"a.png\" do"), None);
        assert_eq!(image_tag_arguments_end("image_tag"), None);
        assert_eq!(image_tag_arguments_end("image_tags \"a.png\""), None);
    }

    #[test]
    fn haml_tags_get_the_attribute_in_the_wrapper_they_have() {
        assert_eq!(img(Haml, "%img{src: \"a.png\"}\n"), "%img{src: \"a.png\", alt: \"\"}\n");
        assert_eq!(img(Haml, "%img.big#logo{:src => \"a\", class: \"b\"}\n"), "%img.big#logo{:src => \"a\", class: \"b\", alt: \"\"}\n");
        assert_eq!(img(Haml, "%img{}\n"), "%img{alt: \"\"}\n");
        assert_eq!(img(Haml, "%img{ src: \"a\",\n      class: \"b\" }\n"), "%img{ src: \"a\",\n      class: \"b\", alt: \"\" }\n");
        assert_eq!(img(Haml, "%img(src=\"a.png\")\n"), "%img(src=\"a.png\" alt=\"\")\n");
        assert_eq!(img(Haml, "%img.big(src=\"a.png\" class=\"b\")\n"), "%img.big(src=\"a.png\" class=\"b\" alt=\"\")\n");
        assert_eq!(img(Haml, "%img{src: \"}\"}\n"), "%img{src: \"}\", alt: \"\"}\n");
        assert_eq!(img(Haml, "%img\n"), "%img(alt=\"\")\n");
        assert_eq!(img(Haml, "%img.big#logo\n"), "%img.big#logo(alt=\"\")\n");
        assert_eq!(img(Haml, "%img/\n"), "%img(alt=\"\")/\n");
        assert_eq!(img(Haml, "%img[@user]\n"), "%img[@user](alt=\"\")\n");
        assert_eq!(img(Haml, "%p\n  %img{src: \"a\"}\n"), "%p\n  %img{src: \"a\", alt: \"\"}\n");
    }

    #[test]
    fn haml_tags_without_a_written_name_get_an_id_too() {
        assert_eq!(
            with_attribute(Haml, ".card{data: {turbo_permanent: true}}\n", "div", "id").unwrap(),
            ".card{data: {turbo_permanent: true}, id: \"\"}\n"
        );
        assert_eq!(with_attribute(Haml, "%div(data-turbo-permanent)\n", "div", "id").unwrap(), "%div(data-turbo-permanent id=\"\")\n");
    }

    #[test]
    fn slim_tags_get_the_attribute_after_their_head() {
        assert_eq!(img(Slim, "img src=\"a.png\"\n"), "img alt=\"\" src=\"a.png\"\n");
        assert_eq!(img(Slim, "img.big#logo src=\"a.png\"\n"), "img.big#logo alt=\"\" src=\"a.png\"\n");
        assert_eq!(img(Slim, "img\n"), "img alt=\"\"\n");
        assert_eq!(img(Slim, "img/\n"), "img alt=\"\"/\n");
        assert_eq!(img(Slim, "img(src=\"a.png\")\n"), "img(src=\"a.png\" alt=\"\")\n");
        assert_eq!(img(Slim, "img[src=\"a.png\" class=\"b\"]\n"), "img[src=\"a.png\" class=\"b\" alt=\"\"]\n");
        assert_eq!(img(Slim, "img{src=\"a.png\"}\n"), "img{src=\"a.png\" alt=\"\"}\n");
        assert_eq!(img(Slim, "p\n  img.x src=\"a\"\n"), "p\n  img.x alt=\"\" src=\"a\"\n");
        assert_eq!(with_attribute(Slim, "div[data-turbo-permanent]\n", "div", "id").unwrap(), "div[data-turbo-permanent id=\"\"]\n");
        assert_eq!(
            with_attribute(Slim, ".card data-turbo-permanent=true\n", "div", "id").unwrap(),
            ".card id=\"\" data-turbo-permanent=true\n"
        );
    }
}
