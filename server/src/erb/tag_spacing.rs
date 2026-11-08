//! `<div  class = "a" >` → `<div class="a">`: the whitespace inside the HTML start and end tags of an ERB template.
//! Only runs without a newline change, and never inside a quoted value or an ERB tag.

use crate::erb::ruby::{self, ErbTag};
use crate::erb::tags::{tokens, Token};
use crate::erb::Edit;
use crate::html::{TEXT_ELEMENTS, TOKENIZED_AS_TEXT};
use std::ops::Range;

/// Whether the content of `name` is text although the tokenizer doesn't skip it: tags inside are not tags.
fn holds_text(name: &str) -> bool {
    name == "plaintext" || (TOKENIZED_AS_TEXT.contains(&name) && !TEXT_ELEMENTS.contains(&name))
}

const CDATA: (&str, &str) = ("<![CDATA[", "]]>");

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Space,
    Eq,
    Quoted,
    Erb,
    Word,
}

struct Piece {
    kind: Kind,
    range: Range<usize>,
}

/// What a tag means: its name and attributes, with an ERB tag's blanks left out.
#[derive(PartialEq)]
enum Shape {
    /// Attributes as `(name, value, no whitespace before it)`, and for each ERB tag whether a blank is before and after it.
    Read(String, Vec<(String, Option<String>, bool)>, Vec<(bool, bool)>),
    /// A tag the reader gave up on, as text.
    Unread(String),
}

/// A tag's whitespace runs and what each becomes, and where its attributes are written.
pub(crate) struct Parsed {
    attributes: Vec<(String, Option<String>, bool)>,
    erb_blanks: Vec<(bool, bool)>,
    spaces: Vec<(Range<usize>, &'static str)>,
    pub written: Vec<Written>,
    /// The `/` right before the `>` (with the blanks before it, when it stands alone), unless it is part of a value or an ERB tag's name.
    pub solidus: Option<Range<usize>>,
    /// Where the last attribute, ERB tag or solidus ends; `None` for a tag with none.
    pub content_end: Option<usize>,
}

/// An attribute as written: its name up to a trailing `/`, and its value with the quotes it has.
pub(crate) struct Written {
    pub name: Range<usize>,
    pub value: Option<Range<usize>>,
}

/// The edits that give the attributes of each tag one space between them and none around `=` or before the `>`.
pub fn edits(text: &str) -> Vec<Edit> {
    let Some(spans) = tag_spans(text) else { return Vec::new() };
    let erb = ruby::tags(text);
    let mut out = Vec::new();
    for span in spans {
        let Some((_, parsed)) = read(text, &span, &erb) else { continue };
        for (range, with) in parsed.spaces {
            let space = &text[range.clone()];
            if space != with && !space.contains(['\n', '\r']) {
                out.push((range, with.to_string()));
            }
        }
    }
    out
}

/// Whether `after` has the tags of `before` with the same names, attributes and values, a blank where an ERB tag had one,
/// and the same text between the tags, but for the blanks inside ERB tags.
pub fn same_tags(before: &str, after: &str) -> bool {
    shapes(before) == shapes(after)
}

fn shapes(text: &str) -> Option<(Vec<Shape>, String)> {
    let spans = tag_spans(text)?;
    let erb = ruby::tags(text);
    let shapes = spans
        .iter()
        .map(|span| match read(text, span, &erb) {
            Some((name, parsed)) => Shape::Read(name, parsed.attributes, parsed.erb_blanks),
            None => Shape::Unread(blanked(text, span.clone(), &erb)),
        })
        .collect();
    let mut outside = String::new();
    let mut done = 0;
    for span in &spans {
        outside.push_str(&blanked(text, done..span.start, &erb));
        done = span.end;
    }
    outside.push_str(&blanked(text, done..text.len(), &erb));
    Some((shapes, outside))
}

/// The byte range of each start and end tag that is markup, `None` when the template's HTML can't be read.
pub(crate) fn tag_spans(text: &str) -> Option<Vec<Range<usize>>> {
    let mut text_until: Option<String> = None;
    let mut spans: Vec<Range<usize>> = Vec::new();
    for token in tokens(text)? {
        let (tag, opens) = match token {
            Token::Open { tag, .. } => (tag, true),
            Token::Close { tag, .. } => (tag, false),
            Token::Raw { tag, .. } => (tag, true),
            _ => continue,
        };
        if let Some(name) = &text_until {
            if opens || tag.name != *name {
                continue;
            }
            text_until = None;
        } else if opens && holds_text(&tag.name) {
            text_until = Some(tag.name.clone());
        }
        spans.push(tag.start..tag.end);
    }
    let cdata = cdata_ranges(text);
    spans.retain(|span| !cdata.iter().any(|c| c.contains(&span.start)));
    Some(spans)
}

fn cdata_ranges(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(open) = text[pos..].find(CDATA.0).map(|at| pos + at) {
        let end = text[open..].find(CDATA.1).map_or(text.len(), |at| open + at + CDATA.1.len());
        out.push(open..end);
        pos = end;
    }
    out
}

/// `text[range]` without the ASCII whitespace of the ERB tags in it, which other rules may change.
fn blanked(text: &str, range: Range<usize>, erb: &[ErbTag]) -> String {
    let mut out = String::new();
    let mut done = range.start;
    let first = erb.partition_point(|t| t.start < range.start);
    for tag in erb[first..].iter().take_while(|t| t.end <= range.end) {
        out.push_str(&text[done..tag.start]);
        out.extend(text[tag.start..tag.end].chars().filter(|c| !c.is_ascii_whitespace()));
        done = tag.end;
    }
    out.push_str(&text[done..range.end]);
    out
}

/// Where the name of the tag at `span` is written.
pub(crate) fn name_range(text: &str, span: &Range<usize>) -> Range<usize> {
    let start = span.start + 1 + usize::from(text[span.start + 1..].starts_with('/'));
    let len = text[start..span.end].bytes().take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.')).count();
    start..start + len
}

/// The tag's name and its attributes with the whitespace runs, or `None` where the tag is not plain enough to be sure of.
pub(crate) fn read(text: &str, span: &Range<usize>, erb: &[ErbTag]) -> Option<(String, Parsed)> {
    let closing = text[span.start + 1..].starts_with('/');
    let Range { start: name_start, end: name_end } = name_range(text, span);
    let name_len = name_end - name_start;
    let body = name_end..span.end.checked_sub(1)?;
    if !text[..span.end].ends_with('>') || text[body.clone()].chars().next().is_some_and(|c| !c.is_ascii_whitespace() && c != '/') {
        return None;
    }
    let pieces = lex(text, body, erb)?;
    let name = format!("{}{}", if closing { "/" } else { "" }, text[name_start..name_start + name_len].to_ascii_lowercase());
    Some((name, parse(text, &pieces, erb)?))
}

fn lex(text: &str, body: Range<usize>, erb: &[ErbTag]) -> Option<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut pos = body.start;
    let mut after_eq = false;
    while pos < body.end {
        let rest = &text[pos..body.end];
        let first = rest.chars().next()?;
        let (kind, len) = if rest.starts_with("<%") {
            let tag = erb_at(erb, pos).filter(|t| !t.is_literal() && t.end <= body.end)?;
            (Kind::Erb, tag.end - pos)
        } else if first.is_ascii_whitespace() {
            (Kind::Space, rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_whitespace()).len())
        } else if first == '=' {
            (Kind::Eq, 1)
        } else if after_eq && matches!(first, '"' | '\'') {
            (Kind::Quoted, quoted_len(text, pos, body.end, erb)?)
        } else {
            let word = rest.find(|c: char| c.is_ascii_whitespace() || c == '=').unwrap_or(rest.len());
            let word = rest[..word].find("<%").filter(|&at| at > 0).unwrap_or(word);
            // A quote that doesn't open a value (after ERB, inside a name) is not worth guessing about.
            if rest[..word].contains(['"', '\'']) {
                return None;
            }
            (Kind::Word, word)
        };
        after_eq = kind == Kind::Eq || (after_eq && kind == Kind::Space);
        pieces.push(Piece { kind, range: pos..pos + len });
        pos += len;
    }
    Some(pieces)
}

fn erb_at<'a>(erb: &'a [ErbTag<'a>], start: usize) -> Option<&'a ErbTag<'a>> {
    erb.binary_search_by_key(&start, |t| t.start).ok().map(|at| &erb[at])
}

/// The length of the quoted value at `start`, ERB tags inside it skipped over.
fn quoted_len(text: &str, start: usize, end: usize, erb: &[ErbTag]) -> Option<usize> {
    let quote = text[start..].chars().next()?;
    let mut pos = start + 1;
    while pos < end {
        if let Some(tag) = erb_at(erb, pos) {
            pos = tag.end;
            continue;
        }
        let c = text[pos..].chars().next()?;
        pos += c.len_utf8();
        if c == quote {
            return Some(pos - start);
        }
    }
    None
}

fn parse(text: &str, pieces: &[Piece], erb: &[ErbTag]) -> Option<Parsed> {
    let kind = |i: usize| pieces.get(i).map(|p| p.kind);
    let joined = |from: usize, to: usize| match (pieces.get(from), pieces.get(to - 1)) {
        (Some(first), Some(last)) => blanked(text, first.range.start..last.range.end, erb),
        _ => String::new(),
    };
    let has_erb = |from: usize, to: usize| pieces[from..to].iter().any(|p| p.kind == Kind::Erb);
    let blank = |i: Option<usize>| i.and_then(kind) == Some(Kind::Space);
    let erb_blanks =
        (0..pieces.len()).filter(|&i| pieces[i].kind == Kind::Erb).map(|i| (blank(i.checked_sub(1)), blank(Some(i + 1)))).collect();
    let content_end = pieces.iter().rfind(|p| p.kind != Kind::Space).map(|p| p.range.end);
    let mut parsed = Parsed { attributes: Vec::new(), erb_blanks, spaces: Vec::new(), written: Vec::new(), solidus: None, content_end };
    let mut i = 0;
    while i < pieces.len() {
        let separator = (kind(i) == Some(Kind::Space)).then_some(i);
        i += usize::from(separator.is_some());
        if i == pieces.len() {
            let space = pieces[i - 1].range.clone();
            let unsafe_to_remove = (i >= 2 && pieces[i - 2].kind == Kind::Erb) || text[..space.start].ends_with('/');
            parsed.spaces.push((space, if unsafe_to_remove { " " } else { "" }));
            break;
        }
        let name_start = i;
        while matches!(kind(i), Some(Kind::Word | Kind::Erb)) {
            i += 1;
        }
        if i == name_start {
            return None;
        }
        if let Some(at) = separator {
            parsed.spaces.push((pieces[at].range.clone(), " "));
        }
        let name = joined(name_start, i);
        let mut name_at = pieces[name_start].range.start..pieces[i - 1].range.end;
        if i == pieces.len() && !has_erb(name_start, i) && name.ends_with('/') {
            let solidus = name_at.end - 1;
            let alone = solidus == name_at.start;
            let blanks = separator.filter(|_| alone).map_or(solidus, |at| pieces[at].range.start);
            parsed.solidus = Some(blanks..solidus + 1);
            name_at.end = solidus;
        }
        let plain_name = !has_erb(name_start, i) && !name.contains('/');
        let space_before = (kind(i) == Some(Kind::Space)).then_some(i);
        let eq = i + usize::from(space_before.is_some());
        if kind(eq) != Some(Kind::Eq) {
            parsed.attributes.push((name, None, separator.is_none()));
            if !name_at.is_empty() {
                parsed.written.push(Written { name: name_at, value: None });
            }
            continue;
        }
        i = eq + 1;
        let space_after = (kind(i) == Some(Kind::Space)).then_some(i);
        i += usize::from(space_after.is_some());
        let value_start = i;
        match kind(i)? {
            Kind::Quoted => i += 1,
            Kind::Word | Kind::Erb | Kind::Eq => {
                while matches!(kind(i), Some(Kind::Word | Kind::Erb | Kind::Eq)) {
                    i += 1;
                }
            }
            Kind::Space => return None,
        }
        if let Some(at) = space_before.filter(|_| plain_name) {
            parsed.spaces.push((pieces[at].range.clone(), ""));
        }
        let plain_value = plain_name && (kind(value_start) == Some(Kind::Quoted) || !has_erb(value_start, i));
        if let Some(at) = space_after.filter(|_| plain_value) {
            parsed.spaces.push((pieces[at].range.clone(), ""));
        }
        let value = pieces[value_start].range.start..pieces[i - 1].range.end;
        parsed.written.push(Written { name: name_at, value: Some(value) });
        parsed.attributes.push((name, Some(joined(value_start, i)), separator.is_none()));
    }
    Some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spaced(text: &str) -> String {
        let mut out = text.to_string();
        for (range, with) in edits(text).into_iter().rev() {
            out.replace_range(range, &with);
        }
        out
    }

    #[test]
    fn runs_between_attributes_become_one_space() {
        assert_eq!(spaced("<div  class=\"a\"   id=\"u\"><%= u.name %></div>"), "<div class=\"a\" id=\"u\"><%= u.name %></div>");
        assert_eq!(spaced("<div\tclass=\"a\" \t id=b>"), "<div class=\"a\" id=b>");
        assert_eq!(spaced("<input  disabled   checked  >"), "<input disabled checked>");
    }

    #[test]
    fn blanks_around_equals_go() {
        assert_eq!(spaced("<img src = \"a\"  alt =\"\" width= 3>"), "<img src=\"a\" alt=\"\" width=3>");
        assert_eq!(spaced("<a href = 'x' >"), "<a href='x'>");
    }

    #[test]
    fn blanks_before_the_closing_bracket_go() {
        assert_eq!(spaced("<p >x</p  >"), "<p>x</p>");
        assert_eq!(spaced("<br  /><br /><br/><hr class=\"a\"  />"), "<br /><br /><br/><hr class=\"a\" />");
        assert_eq!(spaced("<input value=x >"), "<input value=x>");
        assert_eq!(spaced("<br / ><b c/ >"), "<br / ><b c/ >", "a solidus before a blank is not a self-closing one");
    }

    #[test]
    fn end_tags_lose_their_blanks() {
        assert_eq!(spaced("</div  >\n</p\t>"), "</div>\n</p>");
    }

    #[test]
    fn runs_with_a_newline_stay() {
        for text in ["<div\n  class=\"a\"\n  id=\"u\"\n>", "<div class=\"a\"\r\n     id=\"u\"  \r\n>", "<a\n>"] {
            assert_eq!(spaced(text), text, "{text:?}");
        }
        assert_eq!(spaced("<div  class=\"a\"\n     id=\"u\"  >"), "<div class=\"a\"\n     id=\"u\">");
        assert_eq!(spaced("<div\r  class=\"a\"\r  id=\"b\"  >"), "<div\r  class=\"a\"\r  id=\"b\">");
    }

    #[test]
    fn a_newline_after_equals_stays() {
        assert_eq!(spaced("<a href =\n\"x\">"), "<a href=\n\"x\">");
    }

    #[test]
    fn values_are_untouched() {
        let text = "<a title=\"a   b  =  c >\" alt='x  y' href=a/b?c=d>";
        assert_eq!(spaced(text), text);
        assert_eq!(spaced("<a x=y  z=w   v>"), "<a x=y z=w v>");
        assert_eq!(spaced("<a x=y= z>"), "<a x=y= z>");
        assert_eq!(spaced("<a b=c =d  e>"), "<a b=c =d  e>", "a name that starts with =");
    }

    #[test]
    fn erb_tags_in_a_tag_keep_their_neighbours() {
        assert_eq!(spaced("<div <%= x %>  class=\"a\">"), "<div <%= x %> class=\"a\">");
        assert_eq!(spaced("<div  <%=x%>   <%= y  %> >"), "<div <%=x%> <%= y  %> >");
        assert_eq!(spaced("<div <%= x %>  >"), "<div <%= x %> >");
        assert_eq!(spaced("<div class=<%= x %>  >"), "<div class=<%= x %> >");
        assert_eq!(spaced("<div class = <%= x %>>"), "<div class= <%= x %>>");
        for text in [
            "<div <%= x %>class=\"a\">",
            "<div <%= x %> class=\"a\">",
            "<div <%= x %> >",
            "<div class=\"<%= a  b %>  c\">",
            "<div data-<%= k %> = \"v\">",
            "<div<%= x %>  class=\"a\">",
            "<div <%%= x %>  a>",
        ] {
            assert_eq!(spaced(text), text, "{text:?}");
        }
    }

    #[test]
    fn a_quote_after_erb_makes_the_tag_unreadable() {
        for text in [
            "<div class=<% if x %>\"a  b\"<% else %>\"c\"<% end %>>",
            "<div data-x=<%= \"\" %>\"p  q\" id=z>",
            "<div class=<% if true %>'a  b'<% end %>>",
        ] {
            assert_eq!(spaced(text), text, "{text:?}");
        }
    }

    #[test]
    fn text_comments_and_raw_text_are_not_tags() {
        for text in [
            "a < b  and  c > d",
            "<!-- <div  class=\"a\"  > -->",
            "<!--[if IE]><div  class=\"a\"  ><![endif]-->",
            "<script>if (a < b  && c > d) { x = \"<b  c>\" }</script>",
            "<style>a  >  b { }</style>",
            "<textarea>\n<b  c>\n</textarea>",
            "<xmp><b  c></xmp>",
            "<svg><![CDATA[ <b  c> ]]></svg>",
            "<%# <div  a> %>",
            "<% x = \"<div  a>\" %>",
        ] {
            assert_eq!(spaced(text), text, "{text:?}");
        }
        assert_eq!(spaced("<pre  class=\"a\">\n  <b  c>x</b>\n</pre >"), "<pre class=\"a\">\n  <b c>x</b>\n</pre>");
        assert_eq!(
            spaced("<script  src=\"a\" ></script  >"),
            "<script src=\"a\"></script  >",
            "the end tag of a raw text element is part of its body"
        );
    }

    #[test]
    fn byte_order_marks_and_crlf_are_fine() {
        assert_eq!(spaced("\u{feff}<p  a=\"1\"  >\r\n<b  c>\r\n"), "\u{feff}<p a=\"1\">\r\n<b c>\r\n");
    }

    #[test]
    fn edits_are_stable() {
        for text in ["<div  class = \"a\"   id=u  ><br  /></div  >", "<div <%= x %>  a  =b  >"] {
            let once = spaced(text);
            assert_eq!(spaced(&once), once, "{text:?}");
        }
    }

    #[test]
    fn same_tags_sees_a_changed_attribute_and_nothing_else() {
        assert!(same_tags("<div  class = \"a\"   id=u >x\n</div  >", "<div class=\"a\" id=u>x\n</div>"));
        assert!(same_tags("<p <%=x%> a>", "<p <%= x %> a>"), "ERB blanks are another rule's");
        for after in [
            "<div class=\"b\" id=u>",
            "<div class=\"a\"id=u>",
            "<div class=\"a  \" id=u>",
            "<div class=\"a\" id=u x>",
            "<div class=\"a\" id=uu>",
            "<span class=\"a\" id=u>",
            "<div class=\"a\" id=u>x",
        ] {
            assert!(!same_tags("<div class=\"a\" id=u>", after), "{after:?}");
        }
        assert!(!same_tags("<a b=c d>", "<a b=cd>"), "unquoted values stay apart");
        assert!(!same_tags("<a <%= x %> =y>", "<a <%= x %>=y>"));
        assert!(!same_tags("<a b=<%= x %> >", "<a b=<%= x %>>"));
        assert!(!same_tags("<a <%= x %>b>", "<a <%= x %> b>"));
        assert!(!same_tags("<pre>a b</pre>", "<pre>ab</pre>"), "text between tags is exact");
        assert!(!same_tags("<script>a  b</script>", "<script>a b</script>"));
        assert!(same_tags("<p>a  <%=x%></p>", "<p>a  <%= x %></p>"));
        assert!(!same_tags("<a b=<%= x %>\"c  d\">", "<a b=<%= x %>\"c d\">"), "a quote the reader can't place");
        assert!(!same_tags("<a b = c>", "<a b =  d>"));
        assert!(!same_tags("<a <%= x %> b>", "<a <%= x %>b>"), "the space after an ERB tag stays");
        assert!(!same_tags("<a>", "<a><b>"));
    }
}
