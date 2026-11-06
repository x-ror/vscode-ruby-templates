//! The rules that change a line from the inside and no line count: spaces in ERB tags, `<% #` comments and the space after
//! a Haml or Slim script marker. They run before re-indentation, on the text as written.

use crate::erb::ruby::tags;
use crate::erb::{self, Edit};
use crate::format::Settings;
use crate::haml::parser::code_starts;
use crate::lang::TemplateLang;
use crate::slim::parser::{self, Marker};

/// `text` with the rules that are on applied, or `None` when none changes it.
pub(super) fn apply(lang: TemplateLang, text: &str, settings: &Settings, extra_engines: &[String]) -> Option<String> {
    let staged = match lang {
        TemplateLang::Erb => erb_staged(text, settings),
        TemplateLang::Haml if settings.space_after_script_marker => replaced(text, &haml_marker_edits(text)),
        TemplateLang::Slim | TemplateLang::Skim if settings.space_after_script_marker => {
            replaced(text, &slim_marker_edits(text, extra_engines))
        }
        _ => return None,
    };
    (staged != text).then_some(staged)
}

/// Whether `formatted`, the re-indented `staged`, still means what `original` does. Haml and Slim compare what they compile
/// to; ERB has no compiled form here, so the Ruby of every tag is compared instead.
pub(super) fn keeps_meaning(lang: TemplateLang, original: &str, staged: &str, formatted: &str, extra_engines: &[String]) -> bool {
    match lang {
        TemplateLang::Erb => tag_ruby(original) == tag_ruby(formatted),
        TemplateLang::Haml => crate::haml::parser::same_compilation_after_marker_spacing(original, staged),
        TemplateLang::Slim | TemplateLang::Skim => crate::slim::ruby::same_compilation(original, staged, extra_engines),
        _ => true,
    }
}

/// Comments first: a converted tag is a `<%#` comment, which the spacing rule leaves alone.
fn erb_staged(text: &str, settings: &Settings) -> String {
    let mut staged = text.to_string();
    if settings.erb_comment_syntax {
        staged = replaced(&staged, &erb::comment_edits(&staged, settings.space_inside_erb_tag));
    }
    if settings.space_inside_erb_tag {
        staged = replaced(&staged, &erb::spacing_edits(&staged));
    }
    staged
}

/// `text` with `edits`, which are in order and don't overlap, applied.
fn replaced(text: &str, edits: &[Edit]) -> String {
    let mut out = String::with_capacity(text.len() + edits.len());
    let mut done = 0;
    for (range, with) in edits {
        out.push_str(&text[done..range.start]);
        out.push_str(with);
        done = range.end;
    }
    out.push_str(&text[done..]);
    out
}

/// The Ruby of each tag, trimmed. A `<%#` tag, and a plain tag the comment rule may rewrite into one, are compared as
/// comments, their lines trimmed: the rule rewrites the first and `align_comments` moves the rest.
fn tag_ruby(text: &str) -> Vec<(Option<&str>, String)> {
    let comment_body = |code: &str| code.split('\n').map(erb::trim_ascii).collect::<Vec<_>>().join("\n");
    tags(text)
        .iter()
        .map(|tag| {
            let code = erb::trim_ascii(&text[tag.code.clone()]);
            match tag.indicator {
                Some("#") => (Some("#"), comment_body(code)),
                None if erb::is_rewritable_comment(text, tag) => (Some("#"), comment_body(&code[1..])),
                indicator => (indicator, code.to_string()),
            }
        })
        .collect()
}

/// A space after the `=`, `~` or `-` that a Haml Ruby line starts with, found where the parser reads the code: the code
/// starts right after its marker. `==` is text with interpolation in Haml, `-#` a comment, `\=` text; none has Ruby.
fn haml_marker_edits(text: &str) -> Vec<Edit> {
    let starts = code_starts(text);
    let Some(starts) = starts else { return Vec::new() };
    starts
        .into_iter()
        .flatten()
        .filter(|&at| at > 0 && matches!(text.as_bytes()[at - 1], b'=' | b'~' | b'-'))
        .map(|at| (at..at, " ".to_string()))
        .collect()
}

/// A space after the `-`, `=` or `==` (with its `<>` modifiers) that starts Ruby, at a marker the parser found, whether it
/// starts the line or follows a tag. `-#` stays: Slim reads it as code, but it is how Haml writes a comment.
fn slim_marker_edits(text: &str, extra_engines: &[String]) -> Vec<Edit> {
    let parsed = parser::parse(text, extra_engines);
    if !parsed.valid {
        return Vec::new();
    }
    let ruby_marker = parsed.markers.iter().filter(|(_, marker)| matches!(marker, Marker::Control | Marker::Output));
    ruby_marker
        .filter(|(range, marker)| {
            let next = text[range.end..].chars().next();
            next.is_some_and(|c| !c.is_whitespace() && !(*marker == Marker::Control && c == '#'))
        })
        .map(|(range, _)| (range.end..range.end, " ".to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staged(lang: TemplateLang, text: &str, settings: &Settings) -> String {
        apply(lang, text, settings, &[]).unwrap_or_else(|| text.to_string())
    }

    fn erb(text: &str, spacing: bool, comments: bool) -> String {
        let settings = Settings { space_inside_erb_tag: spacing, erb_comment_syntax: comments, ..Settings::default() };
        staged(TemplateLang::Erb, text, &settings)
    }

    fn marker(lang: TemplateLang, text: &str) -> String {
        staged(lang, text, &Settings { space_after_script_marker: true, ..Settings::default() })
    }

    fn haml(text: &str) -> String {
        marker(TemplateLang::Haml, text)
    }

    fn slim(text: &str) -> String {
        marker(TemplateLang::Slim, text)
    }

    #[test]
    fn nothing_changes_while_the_rules_are_off() {
        let off = Settings::default();
        assert_eq!(apply(TemplateLang::Erb, "<%=x%><% # a %>\n", &off, &[]), None);
        assert_eq!(apply(TemplateLang::Haml, "=x\n", &off, &[]), None);
        assert_eq!(apply(TemplateLang::Slim, "=x\n", &off, &[]), None);
        assert_eq!(apply(TemplateLang::Erb, "<%= x %>\n", &Settings { space_inside_erb_tag: true, ..off }, &[]), None);
    }

    #[test]
    fn erb_tags_get_one_space_inside() {
        assert_eq!(erb("<%=x%> <%=  x  %> <%-x-%> <%if a%>\n", true, false), "<%= x %> <%= x %> <%- x -%> <% if a %>\n");
        assert_eq!(erb("\u{feff}<%=é%>\r\n<%==  raw%>\r\n", true, false), "\u{feff}<%= é %>\r\n<%== raw %>\r\n");
    }

    #[test]
    fn erb_spacing_leaves_multi_line_tags_comments_and_literals() {
        let text = "<%=link_to(\n  x)%>\n<%#note%> <%%=lit%> <%= x\n%>\n";
        assert_eq!(erb(text, true, false), text);
    }

    #[test]
    fn erb_comments_lose_the_blank_before_the_hash() {
        assert_eq!(erb("<% # note %>\n", false, true), "<%# note %>\n");
        assert_eq!(erb("  <%   #note -%>\r\n<p>x</p>\n", false, true), "  <%#note -%>\r\n<p>x</p>\n");
        assert_eq!(erb("<% # a\n  # b\n\n  # c %>\n", false, true), "<%# a\n  # b\n\n  # c %>\n");
        assert_eq!(erb("<% # note %>", false, true), "<%# note %>", "the end of the file");
        assert_eq!(erb("<% # a\n\u{a0}# b %>\n", false, true), "<% # a\n\u{a0}# b %>\n", "U+00A0 is code");
        assert_eq!(erb("<%# note %>\n<%#<%# x\n%>\n", false, true), "<%# note %>\n<%#<%# x\n%>\n");
    }

    #[test]
    fn erb_comments_stay_where_the_conversion_could_show_output() {
        for text in [
            // Other content on the line: the `<% #` form comments out the output after it, `<%#` does not.
            "<p><% # note %></p>\n",
            "<% # note %> <p>x</p>\n",
            "<%= a %> <% # note %>\n",
            "<p><% # note %>\n",
            "\u{feff}<% # note %>\n",
            // Ruby after the comment's lines, or an output, trim or literal tag.
            "<% # a\n b %>\n",
            "<% # a\n=begin\n=end\n %>\n",
            "<%\n # a %>\n",
            "<%= # note %>\n",
            "<%- # note -%>\n",
            "<%% # note %>\n",
            "<% x # note %>\n",
            "<% %>\n",
            "<% # coding: utf-8 %>\n",
        ] {
            assert_eq!(erb(text, false, true), text, "{text:?}");
        }
    }

    #[test]
    fn erb_rules_compose_and_are_stable() {
        let both = |text: &str| erb(text, true, true);
        assert_eq!(both("<%  # note  %>\n<%=x%>\n"), "<%# note %>\n<%= x %>\n");
        assert_eq!(both("<%  # note%>\n"), "<%# note %>\n");
        assert_eq!(both("<% # a\n  # b\n%>\n"), "<%# a\n  # b\n%>\n", "a newline before the closing delimiter stays");
        assert_eq!(erb("<%=\u{a0}x%>\n", true, false), "<%= \u{a0}x %>\n");
        assert_eq!(both("<p><%  # note  %></p>\n"), "<p><% # note %></p>\n");
        for text in ["<%  # note  %>\n<%=x%>\n", "<p><%  # note  %></p>\n", "<% # a\n # b %>\n"] {
            let once = both(text);
            assert_eq!(both(&once), once, "{text:?}");
        }
    }

    #[test]
    fn erb_ruby_check_sees_a_changed_tag_and_nothing_else() {
        let same = |a: &str, b: &str| keeps_meaning(TemplateLang::Erb, a, a, b, &[]);
        assert!(same("<%=x%>\n<% # a %>\n", "<%= x %>\n<%# a %>\n"));
        assert!(same("<%# a\n  b %>\n", "<%# a\n      b %>\n"), "comment lines may move");
        assert!(same("<% x\n  y %>\n", "  <%  x\n  y  %>\n"));
        assert!(!same("<%= a %>\n", "<%= b %>\n"));
        assert!(!same("<%= a %>\n", "<% a %>\n"));
        assert!(!same("<% x\n  y %>\n", "<% x\ny %>\n"), "Ruby lines keep their bytes");
        assert!(!same("<% # a\n b %>\n", "<%# a\n b %>\n"), "the second line is Ruby");
        assert!(!same("<%= a %>\n", "<%= a %>\n<%= b %>\n"));
        // A comment rewritten beside other content is not what the rule does.
        assert!(!same("<% # a %> <p>x</p>\n", "<%# a %> <p>x</p>\n"));
        assert!(!same("<p><% # a %>\n", "<p><%# a %>\n"));
        assert!(!same("<%= a %> <% # a %>\n", "<%= a %> <%# a %>\n"));
        assert!(same("<% # a %>\n", "<%# a %>\n"));
    }

    #[test]
    fn haml_markers_get_a_space() {
        assert_eq!(
            haml("-if a\n  =render x\n  ~preserve y\n  !=raw z\n  &=esc w\n"),
            "- if a\n  = render x\n  ~ preserve y\n  != raw z\n  &= esc w\n"
        );
        assert_eq!(haml("\u{feff}=x\r\n-y\r\n"), "\u{feff}= x\r\n- y\r\n");
        assert_eq!(haml("=(1 + 2)\n"), "= (1 + 2)\n");
    }

    #[test]
    fn haml_inline_markers_after_a_tag_get_a_space() {
        assert_eq!(haml("%p=x\n%p.a#b{a: 1}=y\n%p(a=1)<=z\n%p!=raw\n"), "%p= x\n%p.a#b{a: 1}= y\n%p(a=1)<= z\n%p!= raw\n");
    }

    #[test]
    fn haml_leaves_what_is_not_a_script_line() {
        let text = "-# comment\n-#comment\n  body\n==interp #{x}\n\\=text\n%p ==a\n%p{a: \"=x\"} =b\n%p= x\n- y\n:plain\n  =not ruby\n  -nor this\n:ruby\n  z = 1\n%p(a=b) c\n";
        assert_eq!(haml(text), text);
        let multiline = "%p= a, |\n  b |\n";
        assert_eq!(haml(multiline), multiline);
        let broken = "%div\n    =x\n  =y\n";
        assert_eq!(haml(broken), broken, "a file Haml rejects");
    }

    #[test]
    fn haml_ruby_after_a_comma_is_not_touched() {
        let text = "=link_to \"a\",\n  =b\n";
        assert_eq!(haml(text), "= link_to \"a\",\n  =b\n");
    }

    #[test]
    fn haml_marker_check_allows_the_space_and_nothing_else() {
        use crate::haml::parser::same_compilation_after_marker_spacing as same;
        assert!(same("=x\n-y\n%p=z\n%p(a=b)c\n", "= x\n- y\n%p= z\n%p(a=b)c\n"));
        assert!(same("=x\n- y\n", "= x\n- y\n"));
        assert!(same("\u{feff}%p=x\n", "\u{feff}%p= x\n"));
        // A space after a `-` or `=` that is not the marker of Ruby code changes text, a class or an attribute.
        assert!(!same("%p a-b\n", "%p a- b\n"));
        assert!(!same("%p.a-b\n", "%p.a- b\n"));
        assert!(!same("%p(a=\"x-y\")\n", "%p(a=\"x- y\")\n"));
        assert!(!same("%p a=b\n", "%p a= b\n"));
        assert!(!same("%p=x a-b\n", "%p= x a- b\n"), "only the code start gains a space");
        assert!(!same("%p ab\n", "%p a b\n"));
        assert!(!same("=x\n", "= y\n"));
        assert!(!same("=x\n", "=  x\n"));
        assert!(!same("=x\n", "= x\n%p\n"));
    }

    #[test]
    fn slim_markers_get_a_space() {
        assert_eq!(slim("-if a\n  =render x\n  ==raw y\n  =<z\n"), "- if a\n  = render x\n  == raw y\n  =< z\n");
        assert_eq!(slim("\u{feff}=x\r\n-y\r\n"), "\u{feff}= x\r\n- y\r\n");
    }

    #[test]
    fn slim_inline_markers_after_a_tag_get_a_space() {
        assert_eq!(slim("p=x\na href=url =y\nspan.a#b data-x=\"=\"==raw\n"), "p= x\na href=url = y\nspan.a#b data-x=\"=\"== raw\n");
        assert_eq!(slim("p.a=x\np<=x\n"), "p.a= x\np<= x\n");
    }

    #[test]
    fn slim_leaves_what_is_not_a_script_line() {
        let text = "/ comment\n/=x\n-#comment\n|text =x\n  =in text\np\n  | -x\ntextarea\n  | =y\njavascript:\n  =x\nruby:\n  =x\na href=\"=x\" text\np = x\np= x\n";
        assert_eq!(slim(text), text);
        let broken = "p\n   =x\n  -y\n";
        assert_eq!(slim(broken), broken, "a file Slim rejects");
    }

    #[test]
    fn slim_ruby_after_a_comma_is_not_touched() {
        assert_eq!(slim("=link_to \"a\",\n  b\n"), "= link_to \"a\",\n  b\n");
    }

    #[test]
    fn script_marker_edits_are_stable() {
        for text in ["-if a\n  =x\n  p=y\n", "%p=x\n-z\n"] {
            let lang = if text.starts_with('%') { TemplateLang::Haml } else { TemplateLang::Slim };
            let once = marker(lang, text);
            assert_eq!(marker(lang, &once), once);
        }
    }
}
