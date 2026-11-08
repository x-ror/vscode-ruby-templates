//! Lints on the elements of ERB, Haml and Slim templates, as `attributes` reads them: missing `alt`, `href`, `title`,
//! repeated ids and attributes, and link text that says nothing. ERB's `alt` and id checks stay in `erb::html_lint`, which
//! also knows the ERB branches.

use crate::attributes::{Attribute, Element, Value};
use crate::erb::html_lint::{self, ALT_RULE, NO_ALT_AVAILABLE};
use crate::lang::TemplateLang;
use crate::ruby::{analyze, BlockEffect, CONTINUERS};
use crate::util::{coded, lines_with_offsets, warning, LineIndex};
use std::collections::HashMap;
use std::ops::Range;
use tower_lsp_server::ls_types::Diagnostic;

pub(crate) const DUPLICATE_ATTRIBUTE: &str = "Html/DuplicateAttribute";
pub(crate) const ANCHOR_HREF: &str = "Html/AnchorHref";
pub(crate) const IFRAME_TITLE: &str = "A11y/IframeTitle";
pub(crate) const GENERIC_LINK_TEXT: &str = "A11y/GenericLinkText";

/// What a link's accessible name can't be, in lower case with the punctuation left out (WCAG 2.4.4, technique F84).
const GENERIC_LINK_TEXTS: [&str; 10] =
    ["click here", "here", "read more", "more", "learn more", "link", "this link", "click", "more info", "more information"];
/// What names an element besides its content.
const LABELS: [&str; 2] = ["aria-label", "aria-labelledby"];
/// A link without `href` that a keyboard can still reach or that only marks a place (`id` too, when it has no content).
const ANCHOR_EXCEPTIONS: [&str; 2] = ["name", "tabindex"];
/// Haml and Slim merge these from the shorthand, the list and the hash instead of repeating them.
const MERGED: [&str; 2] = ["class", "id"];

/// The findings in a template's `markup` (its tags) and `helpers` (the elements Rails tag helpers make). A repeated attribute
/// in Slim markup is not reported: Slim rejects it itself, and its parser reports that as `Syntax/Template`.
pub(crate) fn diagnostics(lang: TemplateLang, text: &str, markup: &[Element], helpers: &[Element]) -> Vec<Diagnostic> {
    let index = LineIndex::new(text);
    let lines: Vec<(usize, &str)> = lines_with_offsets(text).collect();
    let report = |at: &Range<usize>, message: String, code: &str| coded(warning(index.range(at.start, at.end), message), code);
    let mut out = Vec::new();
    if lang != TemplateLang::Erb {
        for element in markup.iter().chain(helpers).filter(|element| element.name == "img") {
            out.extend(missing_alt(text, element).map(|(at, message)| report(&at, message, html_lint::code::IMG_MISSING_ALT)));
        }
        out.extend(
            duplicate_ids(text, &lines, markup, &index)
                .into_iter()
                .map(|(at, message)| report(&at, message, html_lint::code::DUPLICATE_ID)),
        );
    }
    for element in markup {
        if lang != TemplateLang::Slim {
            out.extend(
                repeated_attributes(element, lang == TemplateLang::Erb)
                    .into_iter()
                    .map(|(at, message)| report(&at, message, DUPLICATE_ATTRIBUTE)),
            );
        }
        out.extend(anchor_without_href(lang, text, &lines, &index, element).map(|at| {
            report(
                &at,
                "`<a>` without `href` is not a link and the keyboard can't reach it; use a `<button>` for an action".into(),
                ANCHOR_HREF,
            )
        }));
    }
    for element in helpers {
        out.extend(repeated_attributes(element, false).into_iter().map(|(at, message)| report(&at, message, DUPLICATE_ATTRIBUTE)));
    }
    for element in markup.iter().chain(helpers) {
        out.extend(iframe_without_title(element).map(|at| {
            report(&at, "`<iframe>` without `title`: screen readers announce it to name the frame (WCAG 4.1.2)".into(), IFRAME_TITLE)
        }));
        out.extend(generic_link_text(text, element).map(|(at, message)| report(&at, message, GENERIC_LINK_TEXT)));
    }
    out
}

fn missing_alt(text: &str, element: &Element) -> Option<(Range<usize>, String)> {
    let declared_unavailable = |attribute: &Attribute| match &attribute.value {
        Value::Empty => true,
        Value::Text(range) => range.is_empty(),
        _ => false,
    };
    let has_alt = element.attribute("alt").is_some() || element.attribute(NO_ALT_AVAILABLE).is_some_and(declared_unavailable);
    if has_alt || element.may_have_more() || element.at.is_empty() {
        return None;
    }
    let message = if &text[element.at.clone()] == "image_tag" {
        format!("`image_tag` without `alt:`: {ALT_RULE}")
    } else {
        format!("`<img>` without `alt`: {ALT_RULE}")
    };
    Some((element.at.clone(), message))
}

/// Static ids used twice that no Ruby control flow separates. Both ids must sit in the same blocks and no `if`, `else`, loop
/// or block may start between them, since one of them may then not be rendered; ids inside a template's shared blocks count.
fn duplicate_ids(text: &str, lines: &[(usize, &str)], elements: &[Element], index: &LineIndex) -> Vec<(Range<usize>, String)> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::new();
    for element in elements.iter().filter(|element| element.name != "template") {
        let mut ids = element.attributes.iter().filter(|attribute| attribute.name == "id");
        // Haml joins an `#id` and an `id:` into one id.
        let (Some(id), None) = (ids.next(), ids.next()) else { continue };
        let Value::Text(range) = &id.value else { continue };
        if range.is_empty() {
            continue;
        }
        // A `<template>`'s content isn't in the document yet.
        if ancestors(lines, index.line_of(range.start)).iter().any(|&line| tag_name(lines[line].1) == "template") {
            continue;
        }
        let name = &text[range.clone()];
        let Some(first) = seen.insert(name, range.start) else { continue };
        if !separated_by_control_flow(lines, index.line_of(first), index.line_of(range.start)) {
            out.push((range.clone(), html_lint::duplicate_id_message(name, index.line_of(first) + 1)));
        }
    }
    out
}

fn separated_by_control_flow(lines: &[(usize, &str)], first: usize, second: usize) -> bool {
    let between = lines.get(first + 1..second).is_some_and(|inner| inner.iter().any(|(_, line)| ruby_of(line).is_some_and(branches)));
    between || enclosing_control_flow(lines, first) != enclosing_control_flow(lines, second)
}

/// The lines of Ruby control flow that a line is nested in, innermost first.
fn enclosing_control_flow(lines: &[(usize, &str)], line: usize) -> Vec<usize> {
    let mut found = ancestors(lines, line);
    found.retain(|&at| ruby_of(lines[at].1).is_some_and(branches));
    found
}

/// The lines a line is nested in by indentation, innermost first.
fn ancestors(lines: &[(usize, &str)], line: usize) -> Vec<usize> {
    let indentation = |text: &str| text.len() - text.trim_start().len();
    let mut depth = indentation(lines[line].1);
    let mut found = Vec::new();
    for (at, (_, text)) in lines[..line].iter().enumerate().rev().filter(|(_, (_, text))| !text.trim().is_empty()) {
        if indentation(text) < depth {
            depth = indentation(text);
            found.push(at);
        }
    }
    found
}

/// The element a Haml or Slim line starts: `svg` for `%svg.icon{…}` and `svg.icon`.
fn tag_name(line: &str) -> &str {
    let line = line.trim_start().trim_start_matches('%');
    &line[..line.find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | ':' | '_'))).unwrap_or(line.len())]
}

/// The Ruby of a Haml or Slim `-` or `=` line.
fn ruby_of(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let code = line.strip_prefix(['-', '='])?;
    (!line.starts_with("-#")).then(|| code.trim_start_matches('='))
}

/// Whether Ruby code opens a block or continues one: what may leave a later element unrendered.
fn branches(code: &str) -> bool {
    !matches!(analyze(code), BlockEffect::Neutral) || code.split_whitespace().next().is_some_and(|word| CONTINUERS.contains(&word))
}

/// The attributes an element repeats. Plain ERB markup repeats them in the page, so the second is reported with the HTML rule.
/// Elsewhere a repeated key renders once (Haml and Ruby keep the last value), so the value dropped is reported; `class` and
/// `id` are left out there, since Haml, Slim and the helpers merge them. A `data: { a: }` and an `"data-a":` are both emitted.
fn repeated_attributes(element: &Element, plain_html: bool) -> Vec<(Range<usize>, String)> {
    // ERB in the tag may pick one of two attributes at render time.
    if plain_html && element.open {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (at, attribute) in element.attributes.iter().enumerate() {
        let merged = !plain_html && MERGED.contains(&attribute.name.as_str());
        let earlier = element.attributes[..at].iter().rev().find(|earlier| earlier.name == attribute.name);
        let (Some(earlier), false) = (earlier, merged || attribute.at.is_empty()) else { continue };
        if plain_html || earlier.in_data != attribute.in_data {
            let message = format!(
                "Duplicate attribute `{}` on `<{}>`: a start tag must not repeat an attribute (HTML 13.1.2.3)",
                attribute.name, element.name
            );
            out.push((attribute.at.clone(), message));
        } else if !earlier.at.is_empty() {
            out.push((earlier.at.clone(), format!("`{}` is given twice; only one value is rendered", attribute.name)));
        }
    }
    out
}

/// An `<a>` that isn't a link: no `href` (also `:href`, `x-bind:href`, `xlink:href`), `name` or `tabindex`; an `id` with no
/// text in it is a target to jump to; and an `<a>` in an `<svg>` is something else.
fn anchor_without_href(
    lang: TemplateLang,
    text: &str,
    lines: &[(usize, &str)],
    index: &LineIndex,
    element: &Element,
) -> Option<Range<usize>> {
    let has = |name: &str| element.attribute(name).is_some();
    let linked = element.attributes.iter().any(|attribute| attribute.name == "href" || attribute.name.ends_with(":href"));
    let target = has("id") && element.text.as_ref().is_none_or(|written| written.is_empty());
    let exempt = linked || target || ANCHOR_EXCEPTIONS.iter().any(|name| has(name));
    if element.name != "a" || exempt || element.may_have_more() || element.at.is_empty() {
        return None;
    }
    let in_svg = match lang {
        TemplateLang::Erb => {
            text[..element.at.start].to_ascii_lowercase().rsplit_once("<svg").is_some_and(|(_, after)| !after.contains("</svg"))
        }
        _ => ancestors(lines, index.line_of(element.at.start)).iter().any(|&line| tag_name(lines[line].1) == "svg"),
    };
    (!in_svg).then(|| element.at.clone())
}

fn iframe_without_title(element: &Element) -> Option<Range<usize>> {
    let named = element.attribute("title").is_some() || LABELS.iter().any(|name| element.attribute(name).is_some());
    (element.name == "iframe" && !named && !element.may_have_more() && !element.at.is_empty()).then(|| element.at.clone())
}

fn generic_link_text(text: &str, element: &Element) -> Option<(Range<usize>, String)> {
    let range = element.text.as_ref().filter(|_| element.name == "a" && !element.may_have_more())?;
    if LABELS.iter().chain(&["aria-describedby"]).any(|name| element.attribute(name).is_some()) {
        return None;
    }
    let written = &text[range.clone()];
    let words: Vec<String> = written.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).map(str::to_lowercase).collect();
    if !GENERIC_LINK_TEXTS.contains(&words.join(" ").as_str()) {
        return None;
    }
    let start = range.start + written.len() - written.trim_start().len();
    let message = format!("Link text `{}` says nothing out of context; name where the link goes (WCAG 2.4.9, F84)", written.trim());
    Some((start..start + written.trim().len(), message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::{helper_elements, markup_elements};
    use crate::code_context;
    use TemplateLang::{Erb, Haml, Slim};

    /// Each finding as its code, the text it underlines and the line it is on.
    fn run(lang: TemplateLang, text: &str) -> Vec<(String, String, u32)> {
        let version = ruby_prism::Version::Latest;
        let markup = markup_elements(lang, text, &[], version);
        let compiled = code_context::compiled(lang, text, None, &[]).unwrap();
        let options = ruby_prism::Options::default().version(version).build();
        let helpers = helper_elements(&compiled, &compiled.parse(&options).unwrap().node());
        let index = LineIndex::new(text);
        let at = |position| (0..=text.len()).find(|&offset| index.position(offset) == position).unwrap();
        diagnostics(lang, text, &markup, &helpers)
            .into_iter()
            .map(|d| (crate::util::code(&d).unwrap().to_string(), text[at(d.range.start)..at(d.range.end)].to_string(), d.range.start.line))
            .collect()
    }

    fn flagged(lang: TemplateLang, code: &str, text: &str) -> Vec<String> {
        run(lang, text).into_iter().filter(|(found, _, _)| found == code).map(|(_, at, _)| at).collect()
    }

    #[test]
    fn images_without_alt_in_haml_and_slim() {
        let alt = html_lint::code::IMG_MISSING_ALT;
        assert_eq!(flagged(Haml, alt, "%img{src: x}\n"), ["img"]);
        assert_eq!(flagged(Haml, alt, "%img(src=\"a\")\n"), ["img"]);
        assert_eq!(flagged(Slim, alt, "img src=x\n"), ["img"]);
        assert_eq!(flagged(Haml, alt, "= image_tag \"a.png\", class: \"x\"\n"), ["image_tag"]);
        assert_eq!(flagged(Slim, alt, "= image_tag \"a.png\"\n"), ["image_tag"]);
        assert_eq!(flagged(Slim, alt, "= tag.img src: x\n"), ["img"]);
        for (lang, text) in [
            (Haml, "%img{src: x, alt: \"\"}\n"),
            // Haml renders a `nil` or `false` attribute value as nothing, which Haml's own `alt: ""` is not; stay silent for any value.
            (Haml, "%img{src: x, alt: nil}\n"),
            (Haml, "%img(src=x alt=y)\n"),
            (Haml, "%img{src: x, **attrs}\n"),
            (Haml, "%img{\"alt\" => name}\n"),
            (Haml, "%img(src=x generator-unable-to-provide-required-alt)\n"),
            (Haml, "= image_tag \"a.png\", alt: \"A\"\n"),
            (Haml, "= image_tag \"a.png\", **opts\n"),
            (Haml, "= image_tag \"a.png\", opts\n"),
            (Slim, "img src=x alt=\"\"\n"),
            (Slim, "img src=x *attrs\n"),
            (Slim, "img[src=x alt=y]\n"),
            (Slim, "= image_tag(x, alt: y)\n"),
        ] {
            assert!(flagged(lang, html_lint::code::IMG_MISSING_ALT, text).is_empty(), "{text}");
        }
        assert!(flagged(Haml, alt, "%img{src: x, generator_unable_to_provide_required_alt: 'x'}\n").len() == 1);
    }

    #[test]
    fn duplicate_ids_in_haml_and_slim() {
        let id = html_lint::code::DUPLICATE_ID;
        assert_eq!(flagged(Haml, id, "#a\n%p#a\n"), ["a"]);
        assert_eq!(flagged(Haml, id, "%div{id: \"a\"}\n%p(id=\"a\")\n"), ["a"]);
        assert_eq!(flagged(Slim, id, "#a\np id=\"a\"\n"), ["a"]);
        assert_eq!(flagged(Slim, id, "div#a\n  p#a\n"), ["a"]);
        assert_eq!(flagged(Haml, id, "= form_with model: x do |f|\n  #a\n  #a\n"), ["a"], "a shared block does not separate them");
        let message = run(Haml, "#a\n\n#a\n");
        assert_eq!(message, [(id.to_string(), "a".to_string(), 2)]);
        for (lang, text) in [
            (Haml, "- if x\n  #a\n- else\n  #a\n"),
            (Haml, "- if x\n  #a\n- else\n  %p\n    #a\n"),
            (Haml, "- if x\n  #a\n#a\n"),
            (Haml, "#a\n- if x\n  #a\n"),
            (Haml, "#a\n- items.each do |item|\n  %p\n#a\n"),
            (Haml, "- case x\n- when 1\n  #a\n- when 2\n  #a\n"),
            (Haml, "#a-#{i}\n#a-#{i}\n"),
            (Haml, "#a{id: \"b\"}\n#a{id: \"b\"}\n"),
            (Haml, "#a{id: x}\n#a{id: x}\n"),
            (Haml, "#a\n#b\n"),
            (Slim, "- if x\n  #a\n- else\n  #a\n"),
            (Slim, "- unless x\n  #a\n#a\n"),
            (Slim, "#a id=\"b\"\n#b\n"),
            (Slim, "p id=x\np id=x\n"),
        ] {
            assert!(flagged(lang, id, text).is_empty(), "{text}");
        }
    }

    #[test]
    fn repeated_attributes() {
        let dup = DUPLICATE_ATTRIBUTE;
        assert_eq!(flagged(Haml, dup, "%a{href: \"/a\", href: \"/b\"} Users\n"), ["href"]);
        assert_eq!(flagged(Haml, dup, "%a{href: x}(href=\"y\")\n"), ["href"]);
        assert_eq!(flagged(Haml, dup, "%a(href=x href=y)\n"), ["href"]);
        assert_eq!(flagged(Haml, dup, "%a{data: {a: 1}, \"data-a\" => 2}\n"), ["data-a"]);
        assert_eq!(flagged(Erb, dup, "<div class=\"card\" CLASS=\"wide\" hidden hidden></div>"), ["CLASS", "hidden"]);
        assert_eq!(flagged(Erb, dup, "<input id=\"a\" id=\"b\">"), ["id"]);
        assert_eq!(flagged(Erb, dup, "<%= link_to \"x\", y, href: \"a\", \"href\": \"b\" %>"), ["href"]);
        assert_eq!(flagged(Slim, dup, "= tag.a href: \"a\", \"href\": \"b\"\n").len(), 1);
        for (lang, text) in [
            (Haml, "%a.b{class: \"c\"}(class=\"d\")\n"),
            (Haml, "#a{id: \"b\"}\n"),
            (Haml, "%a{class: [\"x\"], \"class\" => \"y\"}\n"),
            (Haml, "%a{href: x, title: y}\n"),
            (Haml, "%a{data: {a: 1}, title: 1}\n"),
            (Haml, "%a{data: {a: 1, b: 2}}\n"),
            (Slim, "a.b class=\"c\"\n"),
            (Slim, "a href=x title=y\n"),
            (Erb, "<div class=\"a\" id=\"b\"></div>"),
            (Erb, "<div <% if x %>class=\"a\"<% else %>class=\"b\"<% end %>></div>"),
            (Erb, "<div <%= attrs %> class=\"a\" class=\"b\"></div>"),
            (Erb, "<div data-a=\"1\" data-b=\"2\"></div>"),
            (Erb, "<%= tag.div class: \"a\", \"class\": \"b\" %>"),
            (Erb, "<%= content_tag :div, class: \"a\", id: \"b\" %>"),
        ] {
            assert!(flagged(lang, dup, text).is_empty(), "{text}");
        }
    }

    #[test]
    fn anchors_without_href() {
        let anchor = ANCHOR_HREF;
        assert_eq!(flagged(Erb, anchor, "<a class=\"btn\" data-action=\"click->menu#open\">Menu</a>"), ["a"]);
        assert_eq!(flagged(Haml, anchor, "%a.btn{data: {action: \"click->menu#open\"}} Menu\n"), ["a"]);
        assert_eq!(flagged(Slim, anchor, "a.btn data-action=\"click->menu#open\" Menu\n"), ["a"]);
        assert_eq!(flagged(Haml, anchor, "%a\n"), ["a"]);
        for (lang, text) in [
            (Erb, "<a href=\"<%= x %>\">x</a>"),
            (Erb, "<a href>x</a>"),
            (Erb, "<a <%= attrs %>>x</a>"),
            (Erb, "<a name=\"top\"></a>"),
            (Erb, "<a role=\"button\" tabindex=\"0\">x</a>"),
            (Erb, "<%= link_to \"x\", y %><%= link_to y do %>x<% end %>"),
            (Erb, "<%= tag.a \"x\", href: y %>"),
            (Erb, "<abbr>x</abbr><article></article>"),
            (Erb, "<!-- <a>x</a> --><%# <a> %>"),
            (Haml, "%a{href: x} Users\n"),
            (Haml, "%a(href=\"/x\") Users\n"),
            (Haml, "%a{**opts} Users\n"),
            (Haml, "%a{data: opts} Users\n"),
            (Haml, "= link_to \"x\", y\n"),
            (Slim, "a href=x Users\n"),
            (Slim, "a *attrs Users\n"),
            (Slim, "a name=\"top\"\n"),
            (Slim, "= link_to \"x\", y\n"),
        ] {
            assert!(flagged(lang, anchor, text).is_empty(), "{text}");
        }
    }

    #[test]
    fn iframes_without_title() {
        let iframe = IFRAME_TITLE;
        assert_eq!(flagged(Erb, iframe, "<iframe src=\"https://e.com/<%= id %>\"></iframe>"), ["iframe"]);
        assert_eq!(flagged(Haml, iframe, "%iframe{src: \"https://e.com/#{id}\"}\n"), ["iframe"]);
        assert_eq!(flagged(Slim, iframe, "iframe src=\"https://e.com/#{id}\"\n"), ["iframe"]);
        assert_eq!(flagged(Haml, iframe, "= tag.iframe src: x\n"), ["iframe"]);
        assert_eq!(flagged(Erb, iframe, "<%= content_tag :iframe, \"\", src: x %>"), ["iframe"]);
        for (lang, text) in [
            (Erb, "<iframe src=x title=\"Video\"></iframe>"),
            (Erb, "<iframe src=x title=\"<%= v.title %>\"></iframe>"),
            (Erb, "<iframe src=x aria-label=\"Video\"></iframe>"),
            (Erb, "<iframe src=x aria-labelledby=\"h\"></iframe>"),
            (Erb, "<iframe <%= attrs %>></iframe>"),
            (Haml, "%iframe{src: x, title: v.title}\n"),
            (Haml, "%iframe{src: x, **opts}\n"),
            (Haml, "%iframe(src=x title=t)\n"),
            (Slim, "iframe src=x title=v.title\n"),
            (Slim, "iframe src=x *attrs\n"),
            (Erb, "<%= tag.iframe src: x, title: y %>"),
            (Erb, "<%= tag.iframe src: x, **opts %>"),
        ] {
            assert!(flagged(lang, iframe, text).is_empty(), "{text}");
        }
    }

    #[test]
    fn generic_link_text() {
        let generic = GENERIC_LINK_TEXT;
        assert_eq!(flagged(Erb, generic, "<p>Our new pricing. <a href=\"/p\"> Click here! </a></p>"), ["Click here!"]);
        assert_eq!(flagged(Erb, generic, "<a href=\"/p\">READ  more...</a>"), ["READ  more..."]);
        assert_eq!(flagged(Erb, generic, "<%= link_to \"Click here\", pricing_path %>"), ["Click here"]);
        assert_eq!(flagged(Erb, generic, "<%= link_to 'more', pricing_path, class: 'x' %>"), ["more"]);
        assert_eq!(flagged(Erb, generic, "<%= tag.a \"here\", href: y %>"), ["here"]);
        assert_eq!(flagged(Erb, generic, "<%= content_tag :a, \"Learn more\", href: y %>"), ["Learn more"]);
        assert_eq!(flagged(Haml, generic, "%p= link_to \"Click here\", pricing_path\n"), ["Click here"]);
        assert_eq!(flagged(Haml, generic, "%a{href: x} here\n"), ["here"]);
        assert_eq!(flagged(Slim, generic, "p = link_to \"Click here\", pricing_path\n"), ["Click here"]);
        assert_eq!(flagged(Slim, generic, "a href=x Read more\n"), ["Read more"]);
        for (lang, text) in [
            (Erb, "<a href=\"/p\">See the new pricing</a>"),
            (Erb, "<a href=\"/p\">Click here to download the report</a>"),
            (Erb, "<a href=\"/p\" aria-label=\"Pricing\">Click here</a>"),
            (Erb, "<a href=\"/p\" aria-labelledby=\"h\">here</a>"),
            (Erb, "<a href=\"/p\"><%= t(\".more\") %></a>"),
            (Erb, "<a href=\"/p\">here <b>now</b></a>"),
            (Erb, "<a href=\"/p\"><img alt=\"x\"> here</a>"),
            (Erb, "<a href=\"/p\" <%= attrs %>>here</a>"),
            (Erb, "<button>here</button><span>click here</span>"),
            (Erb, "<%= link_to \"Click #{x}\", pricing_path %>"),
            (Erb, "<%= link_to pricing_path do %>here<% end %>"),
            (Erb, "<%= link_to \"here\", y, **opts %>"),
            (Erb, "<%= link_to \"here\", y, \"aria-label\": \"Pricing\" %>"),
            (Erb, "<%= link_to t(\"here\"), y %>"),
            (Erb, "<%= link_to x, y %>"),
            (Haml, "%a{href: x} See pricing\n"),
            (Haml, "%a{href: x}= here\n"),
            (Haml, "%a{href: x} here #{y}\n"),
            (Haml, "%a{href: x, aria: {label: \"Pricing\"}} here\n"),
            (Haml, "%a{href: x, \"aria-label\" => \"Pricing\"} here\n"),
            (Slim, "a href=x See pricing\n"),
            (Slim, "a href=x = here\n"),
            (Slim, "a href=x here #{y}\n"),
            (Slim, "a href=x aria-label=\"Pricing\" here\n"),
        ] {
            assert!(flagged(lang, generic, text).is_empty(), "{text}");
        }
    }

    #[test]
    fn messages() {
        let message = |lang, text: &str| {
            let markup = markup_elements(lang, text, &[], ruby_prism::Version::Latest);
            diagnostics(lang, text, &markup, &[]).remove(0).message
        };
        assert_eq!(message(Haml, "%img{src: x}\n"), "`<img>` without `alt`: the alt attribute must be specified (HTML 4.8.4.4.1)");
        assert_eq!(
            message(Slim, "iframe src=x\n"),
            "`<iframe>` without `title`: screen readers announce it to name the frame (WCAG 4.1.2)"
        );
        assert_eq!(message(Haml, "%a{href: x, href: y}\n"), "`href` is given twice; only one value is rendered");
        assert_eq!(
            message(Erb, "<a href=x href=y>"),
            "Duplicate attribute `href` on `<a>`: a start tag must not repeat an attribute (HTML 13.1.2.3)"
        );
        assert_eq!(
            message(Haml, "%a{data: {a: 1}, \"data-a\" => 2}\n"),
            "Duplicate attribute `data-a` on `<a>`: a start tag must not repeat an attribute (HTML 13.1.2.3)"
        );
        assert_eq!(message(Haml, "%a{href: 1}(href=\"y\") x\n"), "`href` is given twice; only one value is rendered");
        assert!(run(Haml, "%p\n").is_empty());
    }

    #[test]
    fn a_repeated_key_is_reported_at_the_value_that_is_dropped() {
        let columns = |lang, text: &str| {
            let markup = markup_elements(lang, text, &[], ruby_prism::Version::Latest);
            diagnostics(lang, text, &markup, &[]).into_iter().map(|d| d.range.start.character).collect::<Vec<_>>()
        };
        assert_eq!(columns(Haml, "%a{href: 1, href: 2}\n"), [3], "the last in a hash wins");
        assert_eq!(columns(Haml, "%a(href=1){href: 2}\n"), [3], "the hash beats the list");
        assert_eq!(columns(Erb, "<a href=1 href=2>"), [10], "the page has both");
    }

    #[test]
    fn ids_on_one_line_and_in_templates() {
        let id = html_lint::code::DUPLICATE_ID;
        assert_eq!(flagged(Slim, id, "div#a: span#a x\n"), ["a"]);
        assert_eq!(flagged(Slim, id, "#a\ndiv#b: span#b x\n"), ["b"]);
        assert!(flagged(Haml, id, "%template\n  #a\n#a\n").is_empty());
        assert!(flagged(Haml, id, "#a\n%template\n  #a\n").is_empty());
        assert!(flagged(Slim, id, "template\n  #a\n  #a\n").is_empty());
        assert!(flagged(Slim, id, "template#a\n#a\n").is_empty());
    }

    #[test]
    fn anchors_that_are_links_in_other_ways_or_are_not_anchors() {
        let anchor = ANCHOR_HREF;
        for (lang, text) in [
            (Erb, "<a :href=\"url\">x</a>"),
            (Erb, "<a x-bind:href=\"url\">x</a>"),
            (Erb, "<svg><a xlink:href=\"#p\"><text>x</text></a></svg>"),
            (Erb, "<svg viewBox=\"0 0 1 1\">\n<a>x</a></svg>"),
            (Erb, "<a id=\"top\"></a>"),
            (Erb, "<a id=\"top\"><%= x %></a>"),
            (Haml, "%a{\"x-bind:href\" => \"url\"} x\n"),
            (Haml, "%svg\n  %a x\n"),
            (Haml, "%a#top\n"),
            (Slim, "svg\n  a x\n"),
            (Slim, "svg.icon\n  g\n    a x\n"),
            (Slim, "a#top\n"),
        ] {
            assert!(flagged(lang, anchor, text).is_empty(), "{text}");
        }
        assert_eq!(flagged(Erb, anchor, "<svg></svg><a>x</a>"), ["a"]);
        assert_eq!(flagged(Erb, anchor, "<a id=\"menu\" data-action=\"click->m#open\">Menu</a>"), ["a"]);
        assert_eq!(flagged(Haml, anchor, "%svg\n  %g\n%a x\n"), ["a"]);
        assert_eq!(flagged(Slim, anchor, "a#menu data-action=\"x\" Menu\n"), ["a"]);
    }

    #[test]
    fn aria_hashes_are_read_like_data_hashes() {
        let iframe = IFRAME_TITLE;
        assert_eq!(flagged(Haml, iframe, "%iframe{src: x, aria: {hidden: true}}\n"), ["iframe"]);
        assert_eq!(flagged(Erb, iframe, "<%= tag.iframe src: x, aria: { hidden: true } %>"), ["iframe"]);
        for (lang, text) in [
            (Haml, "%iframe{src: x, aria: {label: \"Video\"}}\n"),
            (Haml, "%iframe{src: x, aria: {labelledby: \"h\"}}\n"),
            (Haml, "%iframe{src: x, aria: opts}\n"),
            (Erb, "<%= tag.iframe src: x, aria: { label: \"Video\" } %>"),
            (Slim, "iframe src=x aria={label: \"Video\"}\n"),
        ] {
            assert!(flagged(lang, iframe, text).is_empty(), "{text}");
        }
        assert!(flagged(Haml, GENERIC_LINK_TEXT, "%a{href: x, aria: {describedby: \"d\"}} here\n").is_empty());
        assert_eq!(flagged(Haml, GENERIC_LINK_TEXT, "%a{href: x, aria: {hidden: true}} here\n"), ["here"]);
        assert!(flagged(Erb, GENERIC_LINK_TEXT, "<a href=x aria-describedby=\"d\">here</a>").is_empty());
    }
}
