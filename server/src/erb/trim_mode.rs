//! `%>` and `-%>` mixed in one template: the closers of the less used kind are reported.

use super::ruby::tags;
use crate::util::{coded, warning, LineIndex};
use tower_lsp_server::ls_types::Diagnostic;

pub const ERB_TRIM_MODE: &str = "Style/ErbTrimMode";

const TRIMMING: &str = "-%>";
const PLAIN: &str = "%>";

/// The closers of the kind used less often in `text`; with as many of each, the `-%>` ones. Comments, `<%%` tags and
/// closers with more on their line are not counted: `-%>` only trims a line break.
pub fn diagnostics(text: &str) -> Vec<Diagnostic> {
    // Each closer's byte range, and whether it trims.
    let closers: Vec<(usize, usize, bool)> = tags(text)
        .into_iter()
        .filter(|tag| !tag.is_comment() && !tag.is_literal() && ends_line(text, tag.end))
        .map(|tag| (tag.code.end, tag.end, text[tag.code.end..tag.end].starts_with('-')))
        .collect();
    let trimming_count = closers.iter().filter(|closer| closer.2).count();
    let plain_count = closers.len() - trimming_count;
    if trimming_count == 0 || plain_count == 0 {
        return Vec::new();
    }
    let minority_trims = trimming_count <= plain_count;
    let (minority, majority) = if minority_trims { (TRIMMING, PLAIN) } else { (PLAIN, TRIMMING) };
    let message = format!("`{minority}` here, but most closers in this template are `{majority}`");
    let index = LineIndex::new(text);
    closers
        .into_iter()
        .filter(|&(_, _, trims)| trims == minority_trims)
        .map(|(from, to, _)| coded(warning(index.range(from, to), message.clone()), ERB_TRIM_MODE))
        .collect()
}

/// Whether only blanks follow `offset` on its line.
fn ends_line(text: &str, offset: usize) -> bool {
    text[offset..].split('\n').next().is_some_and(|rest| rest.trim_matches([' ', '\t', '\r']).is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flagged(text: &str) -> Vec<&str> {
        let index = LineIndex::new(text);
        let at = |position| (0..=text.len()).find(|&offset| index.position(offset) == position).unwrap();
        diagnostics(text).iter().map(|d| &text[at(d.range.start)..at(d.range.end)]).collect()
    }

    #[test]
    fn reports_the_closers_of_the_less_used_kind() {
        let one_trimming = "<% a.each do |u| -%>\n<%= u %>\n<% end %>\n";
        assert_eq!(flagged(one_trimming), ["-%>"]);
        let one_plain = "<% a.each do |u| -%>\n<%= u -%>\n<% end %>\n";
        assert_eq!(flagged(one_plain), ["%>"]);
        let diagnostic = &diagnostics(one_trimming)[0];
        assert_eq!(diagnostic.message, "`-%>` here, but most closers in this template are `%>`");
        assert_eq!((diagnostic.range.start.line, diagnostic.range.start.character), (0, 17));
    }

    #[test]
    fn a_tie_reports_the_trimming_closers() {
        assert_eq!(flagged("<% a -%>\n<% b %>\n"), ["-%>"]);
    }

    #[test]
    fn a_template_with_one_kind_is_fine() {
        assert!(flagged("<% a -%>\n<% b -%>\n").is_empty());
        assert!(flagged("<% a %>\n<%= b %>\n").is_empty());
        assert!(flagged("<p>no tags</p>\n").is_empty());
    }

    #[test]
    fn only_closers_at_the_end_of_a_line_count() {
        assert!(flagged("<p><%= a -%></p>\n<% b %>\n<% c %>\n").is_empty());
        assert_eq!(flagged("<%= f.label :a -%>\n<%= f.text_field :a %></p>\n<% x %>  \n<% y %>"), ["-%>"]);
    }

    #[test]
    fn comments_and_literal_tags_are_not_counted() {
        assert!(flagged("<%# note -%>\n<% a %>\n<% b %>\n").is_empty());
        assert!(flagged("<%% raw -%>\n<% a %>\n").is_empty());
    }
}
