//! `Style/AttributeQuotes`, `Style/BooleanAttributes`, `Style/VoidTagClosing` and `Style/TagNameCase`: rewrites of the tags that
//! leave the page as it is. They run before the other rules (like the Turbo renames), whose checks then guard the rest. Here
//! every tag is read again after the rewrite, and the rewrite is kept only when nothing but the intended change differs.

use super::inline::replaced;
use super::{Quote, Settings, VoidClosing};
use crate::erb::ruby::{self, ErbTag};
use crate::erb::tag_spacing::{name_range, raw_end_names, read, tag_spans, Parsed, Written};
use crate::erb::Edit;
use crate::html::{canonical_tag_name, BOOLEAN_ATTRIBUTE_ELEMENTS, VOID_ELEMENTS};
use crate::lang::TemplateLang;
use crate::slim::parser::{self, Piece, SLIM_ENGINES};
use std::ops::Range;

const REFUSED: &str = "tag style kept: rewriting it would change more than quotes, boolean attributes, void tag closing and tag name case";
const SLASH_ENDING: &str = " /";

pub(super) fn is_on(settings: &Settings) -> bool {
    settings.attribute_quotes.is_some() || settings.boolean_attributes || settings.void_tag_closing.is_some() || settings.tag_name_case
}

/// `text` rewritten; `Ok(None)` when nothing changes, and a note when the rewrite is refused. `html` is false for a template
/// that outputs something else, such as XML, where case counts.
pub(super) fn apply(
    lang: TemplateLang,
    text: &str,
    settings: &Settings,
    extra_engines: &[String],
    html: bool,
) -> Result<Option<String>, String> {
    let styled = match lang {
        TemplateLang::Erb if html => erb_styled(text, settings),
        TemplateLang::Slim | TemplateLang::Skim if settings.tag_name_case && html => rewritten(text, slim_edits(text, extra_engines)),
        _ => return Ok(None),
    };
    if styled == text {
        return Ok(None);
    }
    let kept =
        if lang == TemplateLang::Erb { same_erb_tags(text, &styled, settings) } else { same_slim_pieces(text, &styled, extra_engines) };
    if kept {
        Ok(Some(styled))
    } else {
        Err(REFUSED.to_string())
    }
}

/// `text` with `edits` made. Stable sorting keeps an insertion before an edit that starts where it does.
fn rewritten(text: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|(range, _)| range.start);
    replaced(text, &edits)
}

/// The void tags are closed in a second pass over the first one's text: whether an ERB tag ends a tag's attributes can change
/// when a value gets its quotes.
fn erb_styled(text: &str, settings: &Settings) -> String {
    let first = Settings { void_tag_closing: None, ..settings.clone() };
    let once = rewritten(text, erb_edits(text, &first));
    let second = Settings { attribute_quotes: None, boolean_attributes: false, tag_name_case: false, ..settings.clone() };
    rewritten(&once, erb_edits(&once, &second))
}

fn erb_edits(text: &str, settings: &Settings) -> Vec<Edit> {
    let Some(spans) = tag_spans(text) else { return Vec::new() };
    let erb = ruby::tags(text);
    let mut edits = Vec::new();
    if settings.tag_name_case {
        edits.extend(raw_end_names(text).into_iter().filter_map(|at| renamed(text, at)));
    }
    for (span, foreign) in spans.iter().zip(foreign_tags(text, &spans)) {
        let name_at = name_range(text, span);
        if settings.tag_name_case {
            edits.extend(renamed(text, name_at.clone()));
        }
        let closing = text[span.start + 1..].starts_with('/');
        let name = text[name_at.clone()].to_ascii_lowercase();
        let Some((_, parsed)) = read(text, span, &erb).filter(|_| !closing) else { continue };
        for attribute in &parsed.written {
            edits.extend(attribute_edits(text, &erb, &name, foreign, attribute, settings));
        }
        let void = !foreign && VOID_ELEMENTS.contains(&name.as_str());
        let closing_edit =
            settings.void_tag_closing.filter(|_| void).and_then(|style| void_edit(text, &erb, &parsed, &name_at, span, style));
        edits.extend(closing_edit);
    }
    edits
}

/// The edit that spells the tag name written at `at` as HTML does.
fn renamed(text: &str, at: Range<usize>) -> Option<Edit> {
    let canonical = canonical_tag_name(&text[at.clone()]);
    (canonical != text[at.clone()]).then_some((at, canonical))
}

/// For each tag, whether it is `<svg>`, `<math>` or inside one, where HTML's rules for attributes and void elements don't hold.
fn foreign_tags(text: &str, spans: &[Range<usize>]) -> Vec<bool> {
    let mut depth = 0usize;
    spans
        .iter()
        .map(|span| {
            let name = text[name_range(text, span)].to_ascii_lowercase();
            if !matches!(name.as_str(), "svg" | "math") {
                return depth > 0;
            }
            if text[span.start + 1..].starts_with('/') {
                depth = depth.saturating_sub(1);
            } else if !text[..span.end - 1].ends_with('/') {
                depth += 1;
            }
            true
        })
        .collect()
}

fn attribute_edits(text: &str, erb: &[ErbTag], element: &str, foreign: bool, attribute: &Written, settings: &Settings) -> Vec<Edit> {
    let Some(value) = attribute.value.clone() else { return Vec::new() };
    let name = &text[attribute.name.clone()];
    if name.contains("<%") {
        return Vec::new();
    }
    let quoted = text[value.start..].starts_with(['"', '\'']);
    let inner = if quoted { value.start + 1..value.end - 1 } else { value.clone() };
    if settings.boolean_attributes && !foreign && sets_by_presence(element, name, &text[inner.clone()]) {
        let removed = attribute.name.end..value.end;
        return if text[removed.clone()].contains(['\n', '\r']) { Vec::new() } else { vec![(removed, String::new())] };
    }
    let Some(style) = settings.attribute_quotes else { return Vec::new() };
    let target = if style == Quote::Double { '"' } else { '\'' };
    if quoted && text[value.start..].starts_with(target) {
        return Vec::new();
    }
    // Helpers return output with quotes of their own, and a bare value's output may be empty or hold blanks.
    let has_output =
        erb.iter().any(|tag| tag.start >= inner.start && tag.end <= inner.end && tag.indicator.is_some_and(|i| i.contains('=')));
    let breaks_lines = text[value.clone()].contains(['\n', '\r']);
    if text[inner.clone()].contains(target) || breaks_lines || (quoted && has_output) || (!quoted && text[inner].contains("<%")) {
        return Vec::new();
    }
    let target = target.to_string();
    if quoted {
        vec![(value.start..value.start + 1, target.clone()), (value.end - 1..value.end, target)]
    } else {
        vec![(value.start..value.start, target.clone()), (value.end..value.end, target)]
    }
}

/// Whether `<element name="value">` is on exactly when `<element name>` is: a boolean attribute of that element, with an
/// empty value or its own name.
fn sets_by_presence(element: &str, name: &str, value: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let belongs = BOOLEAN_ATTRIBUTE_ELEMENTS
        .iter()
        .any(|(boolean, elements)| *boolean == name && (elements.is_empty() || elements.contains(&element)));
    // A custom element may read its attribute's value.
    belongs && !element.contains('-') && (value.is_empty() || value.eq_ignore_ascii_case(&name))
}

/// The edit that ends a void element's start tag the way `style` says.
fn void_edit(text: &str, erb: &[ErbTag], parsed: &Parsed, name_at: &Range<usize>, span: &Range<usize>, style: VoidClosing) -> Option<Edit> {
    let body_end = span.end - 1;
    let (range, with) = match (style, &parsed.solidus) {
        (VoidClosing::NoSlash, None) => return None,
        // Nothing but the `>` follows, but the blank before an ERB tag's output is kept like Layout/SpaceInHtmlTag does.
        (VoidClosing::NoSlash, Some(solidus)) => (solidus.clone(), ""),
        (VoidClosing::Slash, Some(solidus)) => (solidus.clone(), SLASH_ENDING),
        (VoidClosing::Slash, None) => {
            let content_end = parsed.content_end.unwrap_or(name_at.end);
            // A value that ends in `/` isn't a self-closing slash, and a bare value ending in an ERB tag may print nothing, which
            // the ` /` would then become.
            let bare_value_with_erb = parsed
                .written
                .last()
                .and_then(|a| a.value.as_ref())
                .is_some_and(|value| !text[value.start..].starts_with(['"', '\'']) && erb.iter().any(|tag| tag.end == value.end));
            if text[..content_end].ends_with('/') || bare_value_with_erb {
                return None;
            }
            (content_end..body_end, SLASH_ENDING)
        }
    };
    let ends_erb_tag = erb.iter().any(|tag| tag.end == range.start);
    let stays = text[range.clone()] == *with || text[range.clone()].contains(['\n', '\r']);
    (!stays && !(ends_erb_tag && with.is_empty())).then_some((range, with.to_string()))
}

/// A tag as the guard compares it.
#[derive(PartialEq)]
enum Shape {
    Read {
        name: String,
        attributes: Vec<(String, Option<String>)>,
        solidus: bool,
    },
    /// A tag the reader gave up on: its name and the rest as text.
    Unread(String, String),
}

/// Whether `after` has the tags of `before` but for the changes the rules in `settings` make: the same names but for case, the
/// same attributes with the same values, whatever the quotes, a boolean attribute with or without its value, a void element
/// with or without its slash, and the same text between the tags.
fn same_erb_tags(before: &str, after: &str, settings: &Settings) -> bool {
    shapes(before, settings) == shapes(after, settings)
}

fn shapes(text: &str, settings: &Settings) -> Option<(Vec<Shape>, String)> {
    let spans = tag_spans(text)?;
    let erb = ruby::tags(text);
    let shapes = spans
        .iter()
        .zip(foreign_tags(text, &spans))
        .map(|(span, foreign)| {
            let name_at = name_range(text, span);
            let tag = text[name_at.clone()].to_ascii_lowercase();
            let Some((_, parsed)) = read(text, span, &erb) else { return Shape::Unread(tag, text[name_at.end..span.end].to_string()) };
            let attribute = |written: &Written| {
                let name = text[written.name.clone()].to_ascii_lowercase();
                let value = written.value.clone().map(|value| match text[value.clone()].strip_prefix(['"', '\'']) {
                    Some(inside) => inside[..inside.len() - 1].to_string(),
                    None => text[value].to_string(),
                });
                let unset = settings.boolean_attributes && !foreign && value.as_deref().is_some_and(|v| sets_by_presence(&tag, &name, v));
                (name, value.filter(|_| !unset))
            };
            let attributes = parsed.written.iter().map(attribute).collect();
            let rewrites_slash = !foreign && VOID_ELEMENTS.contains(&tag.as_str()) && settings.void_tag_closing.is_some();
            let closing = text[span.start + 1..].starts_with('/');
            let name = if closing { format!("/{tag}") } else { tag };
            Shape::Read { name, attributes, solidus: parsed.solidus.is_some() && !rewrites_slash }
        })
        .collect();
    // The end tags of text elements are between the spans; their name's case is what Style/TagNameCase changes.
    let mut between = text.to_string();
    if settings.tag_name_case {
        raw_end_names(text).into_iter().for_each(|at| between[at].make_ascii_lowercase());
    }
    let mut outside = String::new();
    let mut done = 0;
    for span in &spans {
        outside.push_str(&between[done..span.start]);
        done = span.end;
    }
    outside.push_str(&between[done..]);
    Some((shapes, outside))
}

/// The tag names of Slim elements: written in a case, spelled as HTML does.
fn slim_edits(text: &str, extra_engines: &[String]) -> Vec<Edit> {
    let parsed = parser::parse(text, extra_engines);
    if !parsed.valid {
        return Vec::new();
    }
    // `Javascript:` is a tag and `javascript:` an embedded engine, `Doctype` a tag and `doctype` a declaration.
    let keyword = |name: &str| name == "doctype" || SLIM_ENGINES.contains(&name) || extra_engines.iter().any(|engine| engine == name);
    parsed
        .elements
        .iter()
        .filter(|element| !element.at.is_empty())
        .filter_map(|element| {
            let canonical = canonical_tag_name(&text[element.at.clone()]);
            (canonical != text[element.at.clone()] && !keyword(&canonical)).then(|| (element.at.clone(), canonical))
        })
        .collect()
}

/// Whether Slim reads `after` as it does `before`, but for the case of the markup it writes.
fn same_slim_pieces(before: &str, after: &str, extra_engines: &[String]) -> bool {
    let (before, after) = (parser::parse(before, extra_engines), parser::parse(after, extra_engines));
    let same_piece = |a: &Piece, b: &Piece| match (a, b) {
        (Piece::Static(a), Piece::Static(b)) | (Piece::Close(a), Piece::Close(b)) => a.eq_ignore_ascii_case(b),
        _ => a == b,
    };
    before.valid
        && after.valid
        && before.elements == after.elements
        && before.markers == after.markers
        && before.pieces.len() == after.pieces.len()
        && before.pieces.iter().zip(&after.pieces).all(|(a, b)| same_piece(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styled(lang: TemplateLang, text: &str, settings: &Settings) -> Result<String, String> {
        let once = apply(lang, text, settings, &[], true)?.unwrap_or_else(|| text.to_string());
        let twice = apply(lang, &once, settings, &[], true)?;
        assert_eq!(twice, None, "stable: {text}");
        Ok(once)
    }

    fn erb(text: &str, settings: &Settings) -> String {
        styled(TemplateLang::Erb, text, settings).unwrap()
    }

    fn quotes(text: &str, style: Quote) -> String {
        erb(text, &Settings { attribute_quotes: Some(style), ..Settings::default() })
    }

    fn booleans(text: &str) -> String {
        erb(text, &Settings { boolean_attributes: true, ..Settings::default() })
    }

    fn voids(text: &str, style: VoidClosing) -> String {
        erb(text, &Settings { void_tag_closing: Some(style), ..Settings::default() })
    }

    fn case(text: &str) -> String {
        erb(text, &Settings { tag_name_case: true, ..Settings::default() })
    }

    #[test]
    fn nothing_changes_while_the_rules_are_off() {
        assert!(!is_on(&Settings::default()));
        assert_eq!(apply(TemplateLang::Erb, "<DIV class='a' disabled=\"\"><br/></DIV>", &Settings::default(), &[], true), Ok(None));
    }

    #[test]
    fn single_quotes_and_bare_values_get_double_quotes() {
        assert_eq!(
            quotes("<div class='card' data-controller=toggle id=\"u\">\n  <%= user.name %>\n</div>\n", Quote::Double),
            "<div class=\"card\" data-controller=\"toggle\" id=\"u\">\n  <%= user.name %>\n</div>\n"
        );
        assert_eq!(quotes("<a href=/x?a=b title = 'T'>", Quote::Double), "<a href=\"/x?a=b\" title = \"T\">");
        assert_eq!(quotes("<p class=\"a\" id=b>", Quote::Single), "<p class='a' id='b'>");
        assert_eq!(quotes("<p title=\"say &quot;hi&quot;\">", Quote::Single), "<p title='say &quot;hi&quot;'>");
    }

    #[test]
    fn values_with_the_quote_are_left() {
        assert_eq!(quotes("<p title='say \"hi\"' alt=a\"b>", Quote::Double), "<p title='say \"hi\"' alt=a\"b>");
        assert_eq!(quotes("<p title=\"it's\" alt=it's>", Quote::Single), "<p title=\"it's\" alt=it's>");
        let ruby_quote = "<p class='<%= a ? \"x\" : \"y\" %>'>";
        assert_eq!(quotes(ruby_quote, Quote::Double), ruby_quote);
        assert_eq!(quotes("<p class='<%= a ? 'x' : 'y' %>'>", Quote::Single), "<p class='<%= a ? 'x' : 'y' %>'>");
    }

    #[test]
    fn values_with_erb_keep_their_quotes_unless_it_is_control_flow() {
        assert_eq!(quotes("<a class='<% if a %>b<% end %>'>", Quote::Double), "<a class=\"<% if a %>b<% end %>\">");
        for (text, style) in [
            ("<a title='<%= link_to \"x\", y %>' data-content='<%= render @x %>'>", Quote::Double),
            ("<a title=\"<%= t(:help_html) %>\">", Quote::Single),
            ("<a href='<%== url %>'>", Quote::Double),
            ("<input value=<%= q %> class=<%= cls %> id=u<%= id %>>", Quote::Double),
            ("<a class=<% if a %>b<% end %>>", Quote::Double),
            ("<a href=<%= x %>\"y\">", Quote::Double),
            ("<a class='a\n  b'>", Quote::Double),
            ("<a class=\"<% if a %>b\n<% end %>\">", Quote::Single),
        ] {
            assert_eq!(quotes(text, style), text, "{text}");
        }
    }

    #[test]
    fn quotes_stay_where_the_tag_is_not_plain() {
        for text in [
            "<script>var a = '<p class=b>';</script>",
            "<style>p::after { content: '<a b=c>' }</style>",
            "<!-- <p class='a'> -->",
            "<textarea><p class='a'></textarea>",
            "<%= tag.p class: 'a' %>",
            "<p <%= attrs %>=a>",
            "<p data-<%= name %>='a'>",
            "<p class='a",
            "<p title=it's>",
        ] {
            assert_eq!(quotes(text, Quote::Double), text, "{text}");
        }
        assert_eq!(quotes("<pre><code class='ruby'>x</code></pre>", Quote::Double), "<pre><code class=\"ruby\">x</code></pre>");
    }

    #[test]
    fn boolean_attributes_lose_their_value() {
        assert_eq!(
            booleans("<input type=\"checkbox\" checked=\"checked\" disabled=\"disabled\">"),
            "<input type=\"checkbox\" checked disabled>"
        );
        assert_eq!(
            booleans("<input disabled=\"\" required='' readonly=readonly DISABLED=\"Disabled\">"),
            "<input disabled required readonly DISABLED>"
        );
        assert_eq!(booleans("<button disabled = \"disabled\"/>"), "<button disabled/>");
        assert_eq!(
            booleans("<option selected=\"selected\">a</option><details open=\"\"></details><div hidden=\"hidden\">"),
            "<option selected>a</option><details open></details><div hidden>"
        );
    }

    #[test]
    fn boolean_attributes_with_another_meaning_stay() {
        for text in [
            "<input disabled=\"false\" checked=\"true\" required=\"required \">",
            "<div hidden=\"until-found\">",
            "<input disabled=\"<%= x %>\" checked=<%= y %>>",
            "<input disabled=\"<% if a %>disabled<% end %>\">",
            "<svg><a hidden=\"hidden\"></a><foreignObject><input disabled=\"disabled\"></foreignObject></svg>",
            "<math><input disabled=\"\"></math>",
            "<my-input disabled=\"disabled\" hidden=\"\">",
            "<div disabled=\"disabled\" checked=\"\" selected=\"selected\">",
            "<script>var s = '<input disabled=\"\">'</script>",
            "<input disabled =\n\"disabled\">",
            "<input <%= 'disabled' if x %> value=\"\" class=\"checked\">",
        ] {
            assert_eq!(booleans(text), text, "{text}");
        }
        assert_eq!(booleans("<svg></svg><input disabled=\"\">"), "<svg></svg><input disabled>");
    }

    #[test]
    fn void_tags_lose_or_gain_the_slash() {
        let messy = "<br/><br /><hr class=\"a\" /><img src=\"<%= u %>\" alt=\"\"/><input disabled/><wbr  /><link rel=x />";
        assert_eq!(
            voids(messy, VoidClosing::NoSlash),
            "<br><br><hr class=\"a\"><img src=\"<%= u %>\" alt=\"\"><input disabled><wbr><link rel=x>"
        );
        let plain = "<br><br ><hr class=\"a\"><img src=\"<%= u %>\" alt=\"\"><input disabled><wbr><meta name=x>";
        assert_eq!(
            voids(plain, VoidClosing::Slash),
            "<br /><br /><hr class=\"a\" /><img src=\"<%= u %>\" alt=\"\" /><input disabled /><wbr /><meta name=x />"
        );
        assert_eq!(
            voids(messy, VoidClosing::Slash),
            "<br /><br /><hr class=\"a\" /><img src=\"<%= u %>\" alt=\"\" /><input disabled /><wbr /><link rel=x />"
        );
    }

    #[test]
    fn void_tags_stay_where_a_slash_means_something_else() {
        for (text, style) in [
            ("<div/><span /><p/>", VoidClosing::NoSlash),
            ("<div></div><p>", VoidClosing::Slash),
            ("<svg><path d=\"M0\"/><image href=\"a\"/></svg>", VoidClosing::NoSlash),
            ("<svg><source/></svg><math><br/></math>", VoidClosing::NoSlash),
            ("<img src=a.png/>", VoidClosing::NoSlash),
            ("<img src=a.png/>", VoidClosing::Slash),
            ("<img src=<%= u %>/>", VoidClosing::Slash),
            ("<input value=<%= x %>>", VoidClosing::Slash),
            ("<img <%= attrs %>/>", VoidClosing::NoSlash),
            ("<img <%= attrs %> />", VoidClosing::NoSlash),
            ("<img\n  src=\"a\"\n/>", VoidClosing::NoSlash),
            ("<br\n>", VoidClosing::Slash),
            ("<br / >", VoidClosing::NoSlash),
            ("<script>document.write('<br/>')</script><!-- <br/> -->", VoidClosing::NoSlash),
            ("</br/>", VoidClosing::NoSlash),
            ("<%= '<br/>' %>", VoidClosing::NoSlash),
        ] {
            assert_eq!(voids(text, style), text, "{text}");
        }
        assert_eq!(voids("<svg></svg><br/>", VoidClosing::NoSlash), "<svg></svg><br>");
    }

    #[test]
    fn tag_names_are_lower_case_but_for_svg_elements() {
        assert_eq!(case("<DIV class=\"Icon\">\n  <Span>x</SPAN>\n</DIV>\n"), "<div class=\"Icon\">\n  <span>x</span>\n</div>\n");
        assert_eq!(
            case("<svg viewBox=\"0 0 8 8\"><lineargradient id=\"f\"></LINEARGRADIENT><foreignobject><P>x</P></foreignobject><CLIPPATH/><textpath/></svg>"),
            "<svg viewBox=\"0 0 8 8\"><linearGradient id=\"f\"></linearGradient><foreignObject><p>x</p></foreignObject><clipPath/><textPath/></svg>"
        );
        assert_eq!(
            case("<linearGradient></linearGradient><My-Element></My-Element><BR/>"),
            "<linearGradient></linearGradient><my-element></my-element><br/>"
        );
        for text in ["<!-- <DIV> -->", "<%= '<DIV>' %>", "<p>a < B and c</p>"] {
            assert_eq!(case(text), text, "{text}");
        }
        assert_eq!(
            case("<SCRIPT>a</SCRIPT><Title>t</TITLE><STYLE></Style><TEXTAREA></TextArea>"),
            "<script>a</script><title>t</title><style></style><textarea></textarea>"
        );
        let on = Settings { tag_name_case: true, space_in_html_tag: true, ..Settings::default() };
        assert_eq!(erb("<SCRIPT  >a</SCRIPT>", &on), "<script  >a</script>");
    }

    #[test]
    fn rules_combine_and_stay_stable() {
        let all = Settings {
            attribute_quotes: Some(Quote::Double),
            boolean_attributes: true,
            void_tag_closing: Some(VoidClosing::NoSlash),
            tag_name_case: true,
            ..Settings::default()
        };
        assert_eq!(
            erb("<INPUT type='checkbox' checked='checked' disabled=disabled name=<%= n %> />\n<BR/>\n", &all),
            "<input type=\"checkbox\" checked disabled name=<%= n %> />\n<br>\n"
        );
        let slash = Settings { void_tag_closing: Some(VoidClosing::Slash), ..all };
        assert_eq!(erb("<input value=a disabled=''>", &slash), "<input value=\"a\" disabled />");
        assert_eq!(erb("<input value=a>", &slash), "<input value=\"a\" />");
    }

    #[test]
    fn the_guard_sees_what_the_rules_do_not_change() {
        let settings =
            Settings { attribute_quotes: Some(Quote::Double), boolean_attributes: true, tag_name_case: true, ..Settings::default() };
        let same = |a: &str, b: &str| same_erb_tags(a, b, &settings);
        assert!(same("<BUTTON a='b' disabled=''>x</BUTTON>", "<button a=\"b\" disabled>x</button>"));
        assert!(!same("<div a='b'>x</div>", "<div a=\"c\">x</div>"));
        assert!(!same("<div a='b'>x</div>", "<div a='b' c>x</div>"));
        assert!(!same("<div>x</div>", "<div>y</div>"));
        assert!(!same("<div>x</div>", "<span>x</span>"));
        assert!(!same("<div>x</div>", "<div>x</div><p>"));
        assert!(!same("<input disabled=\"false\">", "<input disabled>"));
        assert!(!same("<a href=<%= x %>>", "<a href=\"<%= y %>\">"));
        assert!(!same("<br/>", "<br>"), "the void rule is off");
        let voids = Settings { void_tag_closing: Some(VoidClosing::NoSlash), ..settings };
        assert!(same_erb_tags("<br/><div/>", "<br><div/>", &voids));
        assert!(!same_erb_tags("<br/><div/>", "<br><div>", &voids));
    }

    #[test]
    fn combination_with_the_other_rules_of_the_formatter() {
        use crate::format::format;
        let uri = "file:///app/views/users/index.html.erb".parse().unwrap();
        let settings = Settings {
            attribute_quotes: Some(Quote::Double),
            boolean_attributes: true,
            void_tag_closing: Some(VoidClosing::NoSlash),
            tag_name_case: true,
            space_in_html_tag: true,
            prefer_turbo_attributes: true,
            ..Settings::default()
        };
        let text = "<DIV  class = 'a' >\n  <A  href=/x  data-method=delete  data-confirm='Sure?'  >x</A>\n<INPUT  checked = 'checked' />\n</DIV>\n";
        let formatted = format(&uri, TemplateLang::Erb, false, text, &settings, &[]).unwrap();
        assert_eq!(
            formatted.text,
            "<div class=\"a\">\n  <a href=\"/x\" data-turbo-method=\"delete\" data-turbo-confirm=\"Sure?\">x</a>\n  <input checked>\n</div>\n"
        );
        assert_eq!(formatted.notes, Vec::<String>::new());
        let again = format(&uri, TemplateLang::Erb, false, &formatted.text, &settings, &[]).unwrap();
        assert_eq!(again.text, formatted.text);
    }

    #[test]
    fn slim_tag_names_are_lower_case() {
        let slim = |text: &str| styled(TemplateLang::Slim, text, &Settings { tag_name_case: true, ..Settings::default() });
        assert_eq!(slim("DIV.icon\n  SPAN Hi\n  A href=x = y\n"), Ok("div.icon\n  span Hi\n  a href=x = y\n".to_string()));
        assert_eq!(slim("LinearGradient#a\nforeignObject\n  P text\n"), Ok("linearGradient#a\nforeignObject\n  p text\n".to_string()));
        for text in [
            "div.icon\n  span Hi\n",
            ".card\n  = Foo\n  | <DIV> Text\n",
            "Ruby:\n  X = 1\n",
            "Javascript:\n  var X;\n",
            "Doctype html\n",
            "P\n  Javascript: span x\n",
        ] {
            let styled = slim(text).unwrap_or_else(|note| note);
            assert!(styled == text || !styled.contains("javascript:") && !styled.contains("doctype"), "{text}: {styled}");
        }
        assert_eq!(slim("Doctype html\n"), Ok("Doctype html\n".to_string()));
        assert_eq!(slim("Javascript:\n  var X;\n"), Ok("Javascript:\n  var X;\n".to_string()));
        assert_eq!(slim("DIV\n   SPAN\n  B\n"), Ok("DIV\n   SPAN\n  B\n".to_string()), "a file Slim rejects");
        let settings = Settings { tag_name_case: true, ..Settings::default() };
        assert_eq!(apply(TemplateLang::Slim, "DIV\n", &settings, &[], false), Ok(None), "XML output");
        assert_eq!(apply(TemplateLang::Slim, "DIV\n", &Settings::default(), &[], true), Ok(None));
        assert_eq!(apply(TemplateLang::Erb, "<DIV>\n", &settings, &[], false), Ok(None), "not HTML");
        assert_eq!(apply(TemplateLang::Haml, "%DIV\n", &settings, &[], true), Ok(None));
    }
}
