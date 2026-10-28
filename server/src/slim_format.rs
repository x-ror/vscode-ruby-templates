//! Slim formatting that can't change the compiled template: each Slim line is re-indented to
//! `tab_size` spaces per nesting level, text and engine blocks and continuations move with their
//! owner line, trailing whitespace goes where Slim ignores it, and empty lines at the end become one line ending.

use crate::slim_parser::{indent_width, parse, Role, BOM};

/// The formatted text, or `None` when Slim would reject the text (or `tab_size` is 0).
pub fn format(text: &str, tab_size: usize, extra_engines: &[String]) -> Option<String> {
    if tab_size == 0 {
        return None;
    }
    let parsed = parse(text, extra_engines);
    if !parsed.valid {
        return None;
    }
    let body = text.strip_prefix(BOM).unwrap_or(text);
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..text.len() - body.len()]);
    // Slim keeps whitespace-only lines at the end (as newlines in the compiled code) but drops empty ones.
    let lines: Vec<&str> = body.split('\n').collect();
    let tail = lines.iter().rposition(|l| !l.trim().is_empty()).map_or(0, |i| i + 1);
    let end = lines.iter().rposition(|l| !l.trim_end_matches('\r').is_empty()).map_or(0, |i| i + 1);
    let mut shift = vec![0isize; parsed.lines.len()];
    for (i, (&raw, info)) in lines.iter().zip(&parsed.lines).take(end).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if i >= tail {
            out.push_str(raw);
            continue;
        }
        let (line, cr) = raw.strip_suffix('\r').map_or((raw, ""), |l| (l, "\r"));
        let (ws, content) = line.split_at(line.len() - line.trim_start_matches([' ', '\t']).len());
        let width = indent_width(ws) as isize;
        let new_width = match info.role {
            Role::Blank => 0,
            Role::Structural { depth } => {
                shift[i] = (depth * tab_size) as isize - width;
                (depth * tab_size) as isize
            }
            Role::Inner { owner } => width + shift[owner],
        };
        // A text block measured from a raw column counts a tab as one character: spaces would move it.
        if info.raw_column && ws.contains('\t') {
            return None;
        }
        let new_width = match new_width {
            w if w >= 0 => w as usize,
            _ if info.free_indent || content.trim().is_empty() => 0,
            _ => return None,
        };
        // A trailing `,` or `\` before the whitespace would turn into a continuation once stripped.
        let stripped = content.trim_end_matches(|c: char| c.is_ascii_whitespace());
        let content = if info.keeps_trailing || stripped.ends_with([',', '\\']) { content } else { stripped };
        if !content.is_empty() {
            out.push_str(&" ".repeat(new_width));
            out.push_str(content);
        }
        out.push_str(cr);
    }
    // The empty lines after the last one Slim compiles collapse into one line ending.
    if end > 0 && lines.len() > end {
        out.push('\n');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(text: &str) -> String {
        let once = format(text, 2, &[]).expect("formattable");
        assert_eq!(format(&once, 2, &[]).as_deref(), Some(once.as_str()), "idempotent");
        once
    }

    #[test]
    fn reindents_structure() {
        assert_eq!(fmt("html\n    body\n        p x  \n    \n\tfooter"), "html\n  body\n    p x  \n\n  footer");
        assert_eq!(format("ul\n  li\n", 4, &[]).as_deref(), Some("ul\n    li\n"));
        assert_eq!(fmt("- if a   \r\n    = b \r\n"), "- if a\r\n  = b\r\n");
    }

    #[test]
    fn blocks_keep_relative_indentation() {
        let text = "div\n     p Text\n         more\n       less\n     |  a  \n        b\n     javascript:\n         if (x) {\n           y();\n         }\n";
        let want = "div\n  p Text\n      more\n    less\n  |  a  \n     b\n  javascript:\n      if (x) {\n        y();\n      }\n";
        assert_eq!(fmt(text), want);
    }

    #[test]
    fn continuations_move_with_their_line() {
        let text = "div\n    = link_to a,\n          b\n    a(href=\"/\"\n      class=\"x\") t\n";
        assert_eq!(fmt(text), "div\n  = link_to a,\n        b\n  a(href=\"/\"\n    class=\"x\") t\n");
    }

    #[test]
    fn trailing_whitespace_only_where_ignored() {
        assert_eq!(fmt("p  \n| t  \n' u  \na href=x  \n/ c  \n<b>  \np Text  "), "p\n| t  \n' u  \na href=x\n/ c\n<b>  \np Text  ");
        assert_eq!(fmt("a href=x,  \n  y"), "a href=x,  \n  y", "`x,` + space ends the value");
        assert_eq!(fmt("p title=\"a  \n  b\""), "p title=\"a  \n  b\"");
    }

    #[test]
    fn continuations_left_of_zero_clamp() {
        let text = "div\n    = f(1,\n  2)\n    a(href=x\n\nclass=y) t\n";
        assert_eq!(fmt(text), "div\n  = f(1,\n2)\n  a(href=x\n\nclass=y) t\n");
    }

    #[test]
    fn keeps_byte_order_mark_and_trailing_blank_lines() {
        assert_eq!(fmt("\u{feff}div\n    p\n"), "\u{feff}div\n  p\n");
        assert_eq!(fmt("p  \n\n   \n \n"), "p\n\n   \n \n");
        assert_eq!(fmt("p\n   \nq"), "p\n\nq");
    }

    #[test]
    fn empty_lines_at_the_end_collapse_into_the_final_newline() {
        // Slim splits on newlines and drops trailing empty strings; a whitespace-only line is one it compiles.
        assert_eq!(fmt("p\n\n\n"), "p\n");
        assert_eq!(fmt("p\n  \n\n"), "p\n  \n");
        assert_eq!(fmt("p\n\n  \n\n\n"), "p\n\n  \n");
        assert_eq!(fmt("p\r\n\r\n"), "p\r\n");
        assert_eq!(fmt("p"), "p");
        assert_eq!(fmt("\n\n\n"), "");
        assert_eq!(fmt("\u{feff}\n"), "\u{feff}");
        assert_eq!(fmt("javascript:\n  x\n\n\n"), "javascript:\n  x\n");
        assert_eq!(fmt("| a\n  b\n\n"), "| a\n  b\n");
        assert_eq!(fmt("p\n\nq\n\n"), "p\n\nq\n");
    }

    #[test]
    fn refuses_what_it_cannot_keep() {
        assert_eq!(format("p\n    a\n  b", 2, &[]), None);
        assert_eq!(format("\tp Text\n\t\t  more", 2, &[]), None, "text column counts the tab as one");
        assert!(format("\tp Text\n\tp", 2, &[]).is_some());
        assert_eq!(format("p", 0, &[]), None);
    }
}
