//! `Layout/EmptyLines` and `Layout/TrailingEmptyLines`, run on the re-indented text. The formatter says which lines
//! may lose their blank line; the rest (a filter body, a text block, a `<pre>`) keeps every line.

use super::Settings;
use crate::util::BOM;

const SPACE: [char; 4] = [' ', '\t', '\r', '\n'];

/// `text` with runs of blank lines cut to `settings.max_empty_lines`, where `removable[i]` allows line `i` to go, and the
/// end of the file cut to one line ending. `original` sets the line ending to add.
pub(crate) fn apply(text: &str, removable: &[bool], settings: &Settings, original: &str) -> String {
    let body = text.strip_prefix(BOM).unwrap_or(text);
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..text.len() - body.len()]);
    let body_start = out.len();
    let mut run = 0;
    for (i, line) in body.split_inclusive('\n').enumerate() {
        run = if removable.get(i) == Some(&true) && is_blank(line) { run + 1 } else { 0 };
        if settings.max_empty_lines.is_none_or(|max| run <= max) {
            out.push_str(line);
        }
    }
    if settings.trailing_empty_lines {
        end_with_one_line_ending(&mut out, body_start, original);
    }
    out
}

fn is_blank(line: &str) -> bool {
    line.trim_matches(SPACE).is_empty()
}

/// Drops the blank lines after the last line with content, which then ends with a line ending; `from` is where the text starts.
fn end_with_one_line_ending(text: &mut String, from: usize, original: &str) {
    let mut at = from;
    let mut end = from;
    for line in text[from..].split_inclusive('\n') {
        at += line.len();
        if !is_blank(line) {
            end = at;
        }
    }
    text.truncate(end);
    if end > from && !text.ends_with(['\n', '\r']) {
        text.push_str(dominant_ending(original));
    }
}

fn dominant_ending(text: &str) -> &'static str {
    let lf = text.matches('\n').count();
    let crlf = text.matches("\r\n").count();
    if crlf * 2 > lf {
        "\r\n"
    } else {
        "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, max: Option<usize>, trailing: bool) -> String {
        let removable = vec![true; text.split('\n').count()];
        let settings = Settings { max_empty_lines: max, trailing_empty_lines: trailing, ..Settings::default() };
        apply(text, &removable, &settings, text)
    }

    #[test]
    fn runs_are_cut_to_max_at_the_start_in_the_middle_and_at_the_end() {
        let text = "\n\n\na\n\n\n\nb\n\n\n";
        assert_eq!(run(text, Some(0), false), "a\nb\n");
        assert_eq!(run(text, Some(1), false), "\na\n\nb\n\n");
        assert_eq!(run(text, Some(2), false), "\n\na\n\n\nb\n\n\n");
        assert_eq!(run(text, None, false), text);
        assert_eq!(run(text, Some(1), true), "\na\n\nb\n");
    }

    #[test]
    fn only_removable_lines_are_cut() {
        let settings = Settings { max_empty_lines: Some(0), trailing_empty_lines: false, ..Settings::default() };
        assert_eq!(apply("a\n\n\nb\n\n", &[true, false, true, true, true, true], &settings, ""), "a\n\nb\n");
        assert_eq!(apply("a\n\n", &[], &settings, ""), "a\n\n");
    }

    #[test]
    fn whitespace_only_lines_count_as_blank_and_endings_stay() {
        assert_eq!(run("a\r\n\r\n \t\r\n\r\nb\r\n", Some(1), false), "a\r\n\r\nb\r\n");
        assert_eq!(run("\u{feff}\n\n\na\n", Some(1), false), "\u{feff}\na\n");
    }

    #[test]
    fn the_end_of_the_file_is_one_line_ending() {
        assert_eq!(run("a\n\n\n", None, true), "a\n");
        assert_eq!(run("a", None, true), "a\n");
        assert_eq!(run("a\r\nb", None, true), "a\r\nb\r\n");
        assert_eq!(run("a  \n \n  ", None, true), "a  \n");
        assert_eq!(run("a\r", None, true), "a\r");
        assert_eq!(run("\n\n", None, true), "");
        assert_eq!(run("\u{feff}\n", None, true), "\u{feff}");
        assert_eq!(run("a\n\n\n", None, false), "a\n\n\n");
    }
}
