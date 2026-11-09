//! `Layout/LineLength`: lines longer than a limit of characters (Unicode scalar values, a tab counting as one).

use crate::util::{coded, lines_with_offsets, warning, LineIndex};
use tower_lsp_server::ls_types::Diagnostic;

pub(crate) const LINE_LENGTH: &str = "Layout/LineLength";

/// Rails reads the strict locals comment from one line, so it can't be wrapped.
const LOCALS_COMMENT: &str = "# locals:";

/// Each line longer than `max` characters, reported from the first character past the limit. Trailing whitespace doesn't
/// count, and neither does a line that can't be made shorter: one unbroken word (a URL) or the strict locals comment.
pub(crate) fn diagnostics(text: &str, max: usize) -> Vec<Diagnostic> {
    let index = LineIndex::new(text);
    lines_with_offsets(text)
        .filter_map(|(start, line)| {
            let line = line.trim_end();
            let width = line.chars().count();
            let content = line.trim_start();
            let unbreakable = !content.contains(char::is_whitespace) || content.contains(LOCALS_COMMENT);
            if width <= max || unbreakable {
                return None;
            }
            let past_limit = line.char_indices().nth(max).map_or(line.len(), |(at, _)| at);
            let message = format!("line is {width} characters long; the limit is {max}");
            Some(coded(warning(index.range(start + past_limit, start + line.len()), message), LINE_LENGTH))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flagged(text: &str, max: usize) -> Vec<(u32, u32, u32)> {
        diagnostics(text, max).iter().map(|d| (d.range.start.line, d.range.start.character, d.range.end.character)).collect()
    }

    #[test]
    fn reports_the_part_past_the_limit() {
        assert_eq!(flagged("<p>short</p>\n<p>a long line here</p>\n", 10), [(1, 10, 23)]);
        let diagnostic = &diagnostics("aaa bbb ccc\n", 5)[0];
        assert_eq!(diagnostic.message, "line is 11 characters long; the limit is 5");
        assert_eq!(crate::util::code(diagnostic), Some(LINE_LENGTH));
        assert_eq!(diagnostic.severity, Some(tower_lsp_server::ls_types::DiagnosticSeverity::WARNING));
    }

    #[test]
    fn a_line_at_the_limit_is_fine_and_trailing_whitespace_is_not_counted() {
        assert!(flagged("aaaa bbbb\n", 9).is_empty());
        assert!(flagged("aaaa bbbb   \t\n", 9).is_empty());
        assert_eq!(flagged("aaaa bbbbb\n", 9), [(0, 9, 10)]);
    }

    #[test]
    fn counts_characters_and_reports_utf16_columns() {
        assert!(flagged("ééé éé\n", 6).is_empty());
        assert_eq!(flagged("ééé ééé\n", 6), [(0, 6, 7)]);
        // U+1F600 is one character and two UTF-16 units.
        assert_eq!(flagged("😀😀 😀😀\n", 4), [(0, 7, 9)]);
        assert_eq!(flagged("\t\ta b\n", 4), [(0, 4, 5)]);
    }

    #[test]
    fn what_cannot_be_shortened_is_not_reported() {
        assert!(flagged("    https://example.com/a/very/long/url/that/goes/on\n", 20).is_empty());
        assert!(flagged("<%# locals: (user:, size: :large, colour: nil) %>\n", 20).is_empty());
        assert!(flagged("-# locals: (user:, size: :large, colour: nil)\n", 20).is_empty());
        assert_eq!(flagged("  https://example.com/a/very/long/url x\n", 20), [(0, 20, 39)]);
    }

    #[test]
    fn line_endings_do_not_count() {
        assert!(flagged("aaaa bbbb\r\naaaa bbbb\r\n", 9).is_empty());
    }
}
