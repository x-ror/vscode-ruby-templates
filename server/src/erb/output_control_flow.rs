//! `<%= if x %>`: control flow in an output tag, where `<% if x %>` is meant. Erubi wraps the code of `<%=` in an
//! expression, so a keyword that opens a construct the tag doesn't close is, almost always, a syntax error that never
//! names the `=`. `<%= for x in y do %>` compiles (as a block opener) but prints the loop's value, so it is flagged too.

use super::ruby::tags_with;
use crate::ruby::prism;
use crate::util::{coded, warning, LineIndex};
use std::borrow::Cow;
use std::ops::Range as ByteRange;
use tower_lsp_server::ls_types::Diagnostic;

pub const OUTPUT_CONTROL_FLOW: &str = "Rails/OutputControlFlow";

/// Fine as an expression when the same tag closes them: `<%= if a then b else c end %>`.
const OPENING: [&str; 7] = ["if", "unless", "while", "until", "case", "for", "begin"];
/// Never the start of an expression.
const CONTINUING: [&str; 6] = ["else", "elsif", "when", "end", "rescue", "ensure"];
/// Keywords that stand alone in a tag; the others take a condition or a pattern.
const BARE: [&str; 3] = ["else", "end", "ensure"];

pub struct Misuse {
    /// The `=` or `==` after `<%`.
    pub marker: ByteRange<usize>,
    pub keyword: &'static str,
}

/// Where the tag's code is parsed, as the compiled template has it: in a method and a block, so `yield` and `next` are valid.
fn is_complete(code: &str, ruby: ruby_prism::Version) -> bool {
    prism::syntax_errors(&format!("def _\n_ {{\n{code}\n}}\nend"), ruby).is_empty()
}

/// The output tags of `text` that start with a keyword they can't use; `percent_lines` as the Ruby compile reads them.
pub fn find(text: &str, ruby: ruby_prism::Version, percent_lines: bool) -> Vec<Misuse> {
    tags_with(text, percent_lines)
        .into_iter()
        .filter_map(|tag| {
            let indicator = tag.indicator.filter(|indicator| indicator.starts_with('='))?;
            let code = &text[tag.code.clone()];
            let code = code.trim_start_matches([' ', '\t', '\r', '\n']);
            let word_end =
                code.find(|c: char| !(c.is_alphanumeric() || !c.is_ascii() || matches!(c, '_' | '?' | '!'))).unwrap_or(code.len());
            let keyword = *OPENING.iter().chain(&CONTINUING).find(|keyword| **keyword == &code[..word_end])?;
            let closed = OPENING.contains(&keyword) && is_complete(code, ruby);
            let marker = tag.start + 2..tag.start + 2 + indicator.len();
            (!closed).then_some(Misuse { marker, keyword })
        })
        .collect()
}

/// `text` with the output markers of `misuses` blanked, so the Ruby the rest of the template compiles to is checked
/// without the errors they cause; every offset stays.
pub fn without_markers<'a>(text: &'a str, misuses: &[Misuse]) -> Cow<'a, str> {
    if misuses.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut blanked = text.to_string();
    for misuse in misuses {
        blanked.replace_range(misuse.marker.clone(), &" ".repeat(misuse.marker.len()));
    }
    Cow::Owned(blanked)
}

pub fn diagnostics(text: &str, misuses: &[Misuse]) -> Vec<Diagnostic> {
    let index = LineIndex::new(text);
    misuses
        .iter()
        .map(|Misuse { marker, keyword }| {
            let output = &text[marker.clone()];
            let rest = if BARE.contains(keyword) { "" } else { " …" };
            let message = format!("`{keyword}` in an output tag `<%{output}`: use `<% {keyword}{rest} %>`");
            coded(warning(index.range(marker.start, marker.end), message), OUTPUT_CONTROL_FLOW)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keywords(text: &str) -> Vec<&'static str> {
        find(text, ruby_prism::Version::Latest, false).into_iter().map(|misuse| misuse.keyword).collect()
    }

    #[test]
    fn keywords_that_open_a_construct_the_tag_leaves_open() {
        for keyword in ["if", "unless", "while", "until", "case", "for", "begin"] {
            assert_eq!(keywords(&format!("<%= {keyword} x %>")), [keyword]);
        }
        assert_eq!(keywords("<%= if @user %>\n<p>hi</p>\n<% end %>"), ["if"]);
        assert_eq!(keywords("<%==if x%>"), ["if"]);
        assert_eq!(keywords("<%= for x in items do %>"), ["for"]);
        assert_eq!(keywords("<%=\n  unless x -%>"), ["unless"]);
    }

    #[test]
    fn percent_lines_are_tags_only_when_asked() {
        let text = "% if x\n<%= end %>\n";
        assert_eq!(find(text, ruby_prism::Version::Latest, true).len(), 1);
        assert_eq!(find("%= if x\n", ruby_prism::Version::Latest, true).len(), 0);
    }

    #[test]
    fn continuation_keywords_are_always_flagged() {
        assert_eq!(
            keywords("<%= else %><%= elsif x %><%= when 1 %><%= end -%><%= rescue %><%= ensure %>"),
            ["else", "elsif", "when", "end", "rescue", "ensure"]
        );
    }

    #[test]
    fn complete_expressions_and_other_code_are_not_flagged() {
        for text in [
            "<%= if x then a else b end %>",
            "<%= unless x then a end %>",
            "<%= case x when 1 then \"a\" end %>",
            "<%= begin; x; rescue; y; end %>",
            "<%= x if y %>",
            "<%= if content_for?(:title) then yield(:title) else \"App\" end %>",
            "<%= begin; yield; rescue; \"\" end %>",
            "<% items.each do |i| %><%= if i.hidden? then next end %><% end %>",
            "<%= endé %><%= whenß %>",
            "<%= iffy %><%= end_date %><%= else_value %><%= for_each x %><%= x.end %>",
            "<%= form_with model: @user do |f| %>",
            "<% if x %><% end %>",
            "<%# if x %>",
            "<%%= if x %>",
        ] {
            assert!(keywords(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn the_marker_is_blanked_without_moving_anything() {
        let text = "<%= if a %>\n<%== end %>";
        let misuses = find(text, ruby_prism::Version::Latest, false);
        assert_eq!(without_markers(text, &misuses), "<%  if a %>\n<%   end %>");
        assert!(matches!(without_markers("<% x %>", &[]), Cow::Borrowed(_)));
    }

    #[test]
    fn the_message_names_the_keyword_and_the_marker() {
        let text = "<%= if a %><%== end %>";
        let found = diagnostics(text, &find(text, ruby_prism::Version::Latest, false));
        let messages: Vec<_> = found.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(messages, ["`if` in an output tag `<%=`: use `<% if … %>`", "`end` in an output tag `<%==`: use `<% end %>`"]);
        assert_eq!((found[0].range.start.character, found[0].range.end.character), (2, 3));
        assert_eq!((found[1].range.start.character, found[1].range.end.character), (13, 15));
    }
}
