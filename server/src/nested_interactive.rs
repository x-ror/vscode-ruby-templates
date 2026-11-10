//! Interactive content where HTML forbids it: a link or button inside a link or button (HTML 4.5.1), a form inside a form
//! (4.10.3), written as markup or made by `link_to`, `button_to` and the form helpers. Only nesting the template spells out is
//! reported: a partial rendered in an element, or the layout's `yield`, is not followed.

use crate::attributes::{Element, Value};
use crate::erb::ruby::ErbTag;
use crate::erb::tags;
use crate::lang::TemplateLang;
use crate::ruby::BlockEffect;
use crate::util::{coded, lines_with_offsets, warning, LineIndex};
use std::ops::Range;
use tower_lsp_server::ls_types::Diagnostic;

pub(crate) const NESTED_INTERACTIVE: &str = "Html/NestedInteractive";

/// Elements that don't allow interactive content, or forms, inside.
const RESTRICTING: [&str; 3] = ["a", "button", "form"];
/// Elements whose content isn't plain HTML: an `<a>` in an `<svg>` is another element, a `<template>`'s content is inert.
const OPAQUE: [&str; 3] = ["svg", "math", "template"];
/// Always interactive content (`a` and `input` depend on their attributes).
const INTERACTIVE: [&str; 7] = ["button", "select", "textarea", "iframe", "details", "embed", "label"];

/// An element and the range of the template its content is written in.
type Content<'a> = (&'a Element, Range<usize>);

/// The findings in a template's `markup` (its tags) and `helpers` (the elements Rails tag helpers make), on the inner element.
pub(crate) fn diagnostics(lang: TemplateLang, text: &str, markup: &[Element], helpers: &[Element]) -> Vec<Diagnostic> {
    let elements: Vec<&Element> = markup.iter().chain(helpers).filter(|element| !element.at.is_empty()).collect();
    let holders: Vec<&Element> =
        elements.iter().copied().filter(|e| RESTRICTING.contains(&e.name.as_str()) || OPAQUE.contains(&e.name.as_str())).collect();
    let contents = if lang == TemplateLang::Erb { erb_contents(text, &holders) } else { indented_contents(text, &holders) };
    let index = LineIndex::new(text);
    let report = |inner: &Element| {
        let around: Vec<&Content> = contents.iter().filter(|(_, content)| content.contains(&inner.at.start)).collect();
        if around.iter().any(|(holder, _)| OPAQUE.contains(&holder.name.as_str())) {
            return None;
        }
        let nearest =
            around.iter().filter_map(|(outer, content)| Some((content.len(), forbidden(text, outer, inner)?))).min_by_key(|found| found.0);
        let (_, message) = nearest?;
        Some(coded(warning(index.range(inner.at.start, inner.at.end), message), NESTED_INTERACTIVE))
    };
    elements.into_iter().filter_map(report).collect()
}

/// ERB: the content of a matched `<a>…</a>` pair, and of a helper's `do … end` block, which spans ERB tags.
fn erb_contents<'a>(text: &str, holders: &[&'a Element]) -> Vec<Content<'a>> {
    let analyzed = tags::analyzed(text);
    let pairs = tags::elements_of(text, &analyzed);
    let content = |holder: &Element| {
        if holder.block {
            return block_content(&analyzed, holder.at.start);
        }
        let pair = pairs.iter().find(|pair| pair.start + "<".len() == holder.at.start && pair.name == holder.name)?;
        let content = pair.open_end..pair.close_start;
        closes_its_blocks(&analyzed, &content).then_some(content)
    };
    holders.iter().filter_map(|&holder| Some((holder, content(holder)?))).collect()
}

/// Whether the ERB tags in `content` open and end their blocks among themselves. If not, the element's end tag may be in
/// another branch than its start tag, and it contains what the branches don't agree on.
fn closes_its_blocks(analyzed: &[(ErbTag, BlockEffect)], content: &Range<usize>) -> bool {
    let mut depth = 0;
    for (_, effect) in analyzed.iter().filter(|(tag, _)| content.contains(&tag.start)) {
        match *effect {
            BlockEffect::Open(opened) => depth += opened,
            BlockEffect::Close(closed) if closed <= depth => depth -= closed,
            BlockEffect::Continue if depth > 0 => {}
            BlockEffect::Neutral => {}
            _ => return false,
        }
    }
    depth == 0
}

/// What lies between the ERB tag holding `at`, which opens a block, and the tag that ends it.
fn block_content(analyzed: &[(ErbTag, BlockEffect)], at: usize) -> Option<Range<usize>> {
    let open = analyzed.iter().position(|(tag, _)| tag.start <= at && at < tag.end)?;
    if analyzed[open].1 != BlockEffect::Open(1) {
        return None;
    }
    let mut depth = 1;
    for (tag, effect) in &analyzed[open + 1..] {
        match *effect {
            BlockEffect::Open(opened) => depth += opened,
            BlockEffect::Close(closed) if closed == depth => return Some(analyzed[open].0.end..tag.start),
            BlockEffect::Close(closed) if closed > depth => return None,
            BlockEffect::Close(closed) => depth -= closed,
            BlockEffect::Continue | BlockEffect::Neutral => {}
        }
    }
    None
}

/// Haml and Slim: the lines below the element's, indented deeper than it.
fn indented_contents<'a>(text: &str, holders: &[&'a Element]) -> Vec<Content<'a>> {
    let lines: Vec<(usize, &str)> = lines_with_offsets(text).collect();
    let index = LineIndex::new(text);
    let indentation = |line: &str| line.len() - line.trim_start().len();
    let content = |holder: &Element| {
        let line = index.line_of(holder.at.start);
        let indent = indentation(lines.get(line)?.1);
        let below = lines.get(line + 1..)?;
        let start = below.first().map_or(text.len(), |(offset, _)| *offset);
        let end = below.iter().find(|(_, line)| !line.trim().is_empty() && indentation(line) <= indent);
        Some(start..end.map_or(text.len(), |(offset, _)| *offset))
    };
    holders.iter().filter_map(|&holder| Some((holder, content(holder)?))).collect()
}

/// Why `inner` can't be in `outer`.
fn forbidden(text: &str, outer: &Element, inner: &Element) -> Option<String> {
    if is_form(text, outer) && is_form(text, inner) {
        return Some("`<form>` inside `<form>`: a form can't contain another form (HTML 4.10.3)".into());
    }
    let restricts = matches!(outer.name.as_str(), "a" | "button");
    (restricts && is_interactive(text, outer, inner))
        .then(|| format!("`<{}>` inside `<{}>`: interactive content can't be nested (HTML 4.5.1)", inner.name, outer.name))
}

/// A `<form>`, or `button_to`, which renders a form around its button.
fn is_form(text: &str, element: &Element) -> bool {
    element.name == "form" || helper(text, element) == Some("button_to")
}

/// The Rails helper an element was made by, or its tag name written in markup.
fn helper<'a>(text: &'a str, element: &Element) -> Option<&'a str> {
    text.get(element.at.clone())
}

/// Whether `inner` is interactive content, as far as its attributes tell; one that may be set by code unseen is not.
fn is_interactive(text: &str, outer: &Element, inner: &Element) -> bool {
    match inner.name.as_str() {
        // A link without `href` is not interactive, but may not hold another link either.
        "a" => outer.name == "a" || inner.attribute("href").is_some() || helper(text, inner) == Some("link_to"),
        "input" => match inner.attribute("type").map(|attribute| &attribute.value) {
            Some(Value::Text(range)) => !text[range.clone()].eq_ignore_ascii_case("hidden"),
            Some(value) => *value == Value::Empty,
            None => !inner.open,
        },
        "audio" | "video" => inner.attribute("controls").is_some(),
        "img" => inner.attribute("usemap").is_some(),
        name => INTERACTIVE.contains(&name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::{helper_elements, markup_elements};
    use crate::code_context;
    use crate::lang::TemplateLang::{Erb, Haml, Slim};

    /// The messages, each with the text it is reported on.
    fn run(lang: TemplateLang, text: &str) -> Vec<(String, String)> {
        let version = ruby_prism::Version::Latest;
        let markup = markup_elements(lang, text, &[], version);
        let compiled = code_context::compiled(lang, text, None, &[]).unwrap();
        let options = ruby_prism::Options::default().version(version).build();
        let parsed = compiled.parse(&options).unwrap();
        let helpers = helper_elements(&compiled, &parsed.node());
        let index = LineIndex::new(text);
        let at = |position| (0..=text.len()).find(|&offset| index.position(offset) == position).unwrap();
        let found = diagnostics(lang, text, &markup, &helpers);
        found.into_iter().map(|d| (d.message, text[at(d.range.start)..at(d.range.end)].to_string())).collect()
    }

    fn flagged(lang: TemplateLang, text: &str) -> Vec<String> {
        run(lang, text).into_iter().map(|(_, flagged)| flagged).collect()
    }

    #[test]
    fn interactive_content_in_a_link_or_button_is_flagged_on_the_inner_element() {
        let found = run(Erb, "<a href=\"/x\">\n  <a href=\"/y\">y</a>\n</a>");
        assert_eq!(found, [("`<a>` inside `<a>`: interactive content can't be nested (HTML 4.5.1)".to_string(), "a".to_string())]);
        for (lang, text, inner) in [
            (Erb, "<a href=\"/x\"><span><button>Go</button></span></a>", "button"),
            (Erb, "<button><a href=\"/y\">y</a></button>", "a"),
            (Erb, "<button><button>x</button></button>", "button"),
            (Erb, "<a href=\"/x\"><input type=\"text\"></a>", "input"),
            (Erb, "<a href=\"/x\"><input></a>", "input"),
            (Erb, "<a href=\"/x\"><select></select></a>", "select"),
            (Erb, "<a href=\"/x\"><textarea></textarea></a>", "textarea"),
            (Erb, "<a href=\"/x\"><label>x</label></a>", "label"),
            (Erb, "<a href=\"/x\"><details><summary>x</summary></details></a>", "details"),
            (Erb, "<a href=\"/x\"><iframe title=\"t\"></iframe></a>", "iframe"),
            (Erb, "<a href=\"/x\"><video controls></video></a>", "video"),
            (Erb, "<a href=\"/x\"><img src=\"m.png\" usemap=\"#m\" alt=\"\"></a>", "img"),
            (Erb, "<a href=\"/x\"><% if y %><a href=\"/z\">z</a><% end %></a>", "a"),
            (Erb, "<a href=\"/x\">\n<%= link_to \"Edit\", edit_path %>\n</a>", "link_to"),
            (Erb, "<a href=\"/x\"><%= button_to \"Del\", path %></a>", "button_to"),
            (Erb, "<%= link_to path do %>\n  <%= link_to \"Edit\", edit_path %>\n<% end %>", "link_to"),
            (Erb, "<%= link_to path do %>\n  <button>x</button>\n<% end %>", "button"),
            (Erb, "<%= tag.a href: \"/x\" do %><%= tag.button \"x\" %><% end %>", "button"),
            (Erb, "<%= content_tag :button do %><a href=\"/x\">x</a><% end %>", "a"),
            (Erb, "<%= button_to path do %><%= link_to \"x\", y %><% end %>", "link_to"),
            (Haml, "%a{href: \"/users\"}\n  %a{href: \"/users/new\"} New\n", "a"),
            (Haml, "%a{href: \"/users\"}\n  .row\n    %button Go\n", "button"),
            (Haml, "= link_to path do\n  = link_to \"Edit\", edit_path\n", "link_to"),
            (Haml, "%button\n  %input{type: \"checkbox\"}\n", "input"),
            (Slim, "a href=\"/users\"\n  a href=\"/users/new\" New\n", "a"),
            (Slim, "= link_to path do\n  button Go\n", "button"),
            (Slim, "button\n  - if x\n    a href=\"/y\" y\n", "a"),
        ] {
            assert_eq!(flagged(lang, text), [inner], "{text}");
        }
    }

    #[test]
    fn a_form_in_a_form_is_flagged_including_button_to() {
        let message = "`<form>` inside `<form>`: a form can't contain another form (HTML 4.10.3)";
        assert_eq!(run(Erb, "<form>\n  <form></form>\n</form>"), [(message.to_string(), "form".to_string())]);
        for (lang, text, inner) in [
            (Erb, "<%= form_with model: x do |f| %>\n  <%= button_to \"Del\", path %>\n<% end %>", "button_to"),
            (Erb, "<%= form_with model: x do |f| %><%= form_with model: y %><% end %>", "form_with"),
            (Erb, "<form action=\"/x\"><%= form_tag path do %><% end %></form>", "form_tag"),
            (Erb, "<%= form_for @x do |f| %><form></form><% end %>", "form"),
            (Haml, "%form\n  = button_to \"Del\", path\n", "button_to"),
            (Haml, "= form_with model: x do |f|\n  = f.fields_for :y do |g|\n    %form\n", "form"),
            (Slim, "form\n  form\n", "form"),
        ] {
            let found = run(lang, text);
            assert_eq!(found.iter().map(|(_, flagged)| flagged.as_str()).collect::<Vec<_>>(), [inner], "{text}");
            assert_eq!(found[0].0, message, "{text}");
        }
        let found = run(Erb, "<a href=\"/x\"><%= button_to \"Del\", path %></a>");
        assert_eq!(found[0].0, "`<button>` inside `<a>`: interactive content can't be nested (HTML 4.5.1)");
    }

    #[test]
    fn the_nearest_outer_element_is_named_and_each_inner_one_is_reported_once() {
        let found = run(Erb, "<a href=\"/a\"><button><a href=\"/b\">b</a></button></a>");
        assert_eq!(found.len(), 2);
        assert!(found[0].0.starts_with("`<button>` inside `<a>`"));
        assert!(found[1].0.starts_with("`<a>` inside `<button>`"));
    }

    #[test]
    fn allowed_and_uncertain_nesting_is_left_alone() {
        for (lang, text) in [
            (Erb, "<label>Name <input type=\"text\"></label>"),
            (Erb, "<details><summary>x</summary><a href=\"/y\">y</a></details>"),
            (Erb, "<form><button>Go</button><input></form>"),
            (Erb, "<button form=\"f\">Go</button><form id=\"f\"></form>"),
            (Erb, "<a href=\"/x\"><input type=\"hidden\" name=\"a\"></a>"),
            (Erb, "<a href=\"/x\"><input type=\"HIDDEN\"></a>"),
            (Erb, "<a href=\"/x\"><input type=\"<%= kind %>\"></a>"),
            (Erb, "<a href=\"/x\"><input <%= attrs %>></a>"),
            (Erb, "<button><a>placeholder</a></button>"),
            (Erb, "<button><a name=\"top\">x</a></button>"),
            (Erb, "<a href=\"/x\"><video></video><audio></audio><img src=\"a.png\" alt=\"\"></a>"),
            (Erb, "<a href=\"/x\"><span>x</span></a><a href=\"/y\">y</a>"),
            (Erb, "<a href=\"/x\">x</a>\n<button>y</button>\n<form></form><form></form>"),
            (Erb, "<a href=\"/x\"><svg><a href=\"/y\"><text>y</text></a></svg></a>"),
            (Erb, "<a href=\"/x\"><template><button>x</button></template></a>"),
            (Erb, "<a href=\"/x\"><%= render \"shared/button\" %></a>"),
            (Erb, "<a href=\"/x\"><%= yield %></a>"),
            (Erb, "<% if x %><a href=\"/x\"><% else %><div><% end %><a href=\"/y\">y</a><% if x %></a><% else %></div><% end %>"),
            (Erb, "<%= link_to \"x\", path %><%= link_to \"y\", other %>"),
            (Erb, "<%= link_to path do %>x<% end %><%= link_to other do %>y<% end %>"),
            (Erb, "<%= form_with model: x do |f| %><%= f.submit %><% end %><%= button_to \"x\", y %>"),
            (Erb, "<%= form_with model: x do |f| %><%= f.text_field :a %><button>x</button><% end %>"),
            (Erb, "<%= tag.a \"x\", href: y %><%= tag.button \"z\" %>"),
            (Erb, "<%= link_to path do %><%= image_tag \"a.png\", alt: \"\" %><% end %>"),
            (Erb, "<script>var a = '<a><a>'</script><!-- <a><a> --><a href=\"/x\">x</a>"),
            (Haml, "%label\n  %input{type: \"checkbox\"}\n"),
            (Haml, "%a{href: \"/x\"}\n  %input{type: \"hidden\"}\n"),
            (Haml, "%a{href: \"/x\"} x\n%a{href: \"/y\"} y\n"),
            (Haml, "%a{href: \"/x\"}\n  %svg\n    %a{href: \"/y\"}\n"),
            (Haml, "%ul\n  %li\n    %a{href: \"/x\"} x\n  %li\n    %a{href: \"/y\"} y\n"),
            (Haml, "= link_to \"x\", y\n= link_to \"z\", w\n"),
            (Haml, "= form_with model: x do |f|\n  = f.submit\n= button_to \"x\", y\n"),
            (Haml, "%a{href: \"/x\"}\n  = render \"shared/button\"\n"),
            (Slim, "a href=\"/x\"\n  span x\na href=\"/y\" y\n"),
            (Slim, "form\n  button Go\nform\n  input\n"),
            (Slim, "button\n  img src=\"a.png\" alt=\"\"\n"),
        ] {
            assert_eq!(flagged(lang, text), Vec::<String>::new(), "{text}");
        }
    }
}
