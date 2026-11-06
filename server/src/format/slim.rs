//! Slim formatting that can't change the compiled template: each Slim line is re-indented to
//! `tab_size` spaces per nesting level, text and engine blocks and continuations move with their
//! owner line, and trailing whitespace goes where Slim ignores it. Every line stays; the blank ones that may go are reported.

use crate::format::Indented;
use crate::slim::parser::{indent_width, parse, Role};
use crate::util::{leading_ws, raw_lines, BOM};

/// The formatted text, or `None` when Slim would reject the text (or `tab_size` is 0).
pub(crate) fn format(text: &str, tab_size: usize, extra_engines: &[String]) -> Option<Indented> {
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
    let lines: Vec<(usize, &str, &str)> = raw_lines(body).collect();
    let mut shift = vec![0isize; parsed.lines.len()];
    for (i, (&(_, line, cr), info)) in lines.iter().zip(&parsed.lines).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let ws = leading_ws(line);
        let content = &line[ws.len()..];
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
    let removable = parsed.lines.iter().map(|info| info.role == Role::Blank && !info.inside_block).collect();
    Some(Indented { text: out, removable, notes: vec![] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reindent(text: &str, width: usize, extra_engines: &[String]) -> Option<String> {
        format(text, width, extra_engines).map(|indented| indented.text)
    }

    fn fmt(text: &str) -> String {
        let once = reindent(text, 2, &[]).expect("formattable");
        assert_eq!(reindent(&once, 2, &[]).as_deref(), Some(once.as_str()), "idempotent");
        once
    }

    #[test]
    fn reindents_structure() {
        assert_eq!(fmt("html\n    body\n        p x  \n    \n\tfooter"), "html\n  body\n    p x  \n\n  footer");
        assert_eq!(reindent("ul\n  li\n", 4, &[]).as_deref(), Some("ul\n    li\n"));
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
    fn keeps_byte_order_mark_and_inner_blank_lines() {
        assert_eq!(fmt("\u{feff}div\n    p\n"), "\u{feff}div\n  p\n");
        assert_eq!(fmt("p\n   \nq"), "p\n\nq");
    }

    #[test]
    fn every_line_stays_and_the_blank_ones_that_may_go_are_reported() {
        assert_eq!(fmt("p  \n\n   \n \n"), "p\n\n\n\n");
        assert_eq!(fmt("div\n    | text\n\n\n        "), "div\n  | text\n\n\n");
        assert_eq!(fmt("p\r\n\r\n"), "p\r\n\r\n");
        assert_eq!(fmt("\u{feff}\n"), "\u{feff}\n");
        let removable = |text: &str| format(text, 2, &[]).unwrap().removable;
        assert_eq!(removable("p\n\nq\n\n"), [false, true, false, true, true]);
        // Between lines of a text block or an engine body a blank line is text; after the block it is not.
        assert_eq!(removable("| a\n\n  b\n\np\n"), [false, false, false, true, false, true]);
        assert_eq!(removable("javascript:\n  x\n\n  y\n"), [false, false, false, false, true]);
        assert_eq!(removable("/ c\n\n  d\n"), [false, true, false, true]);
    }

    #[test]
    fn refuses_what_it_cannot_keep() {
        assert_eq!(reindent("p\n    a\n  b", 2, &[]), None);
        assert_eq!(reindent("\tp Text\n\t\t  more", 2, &[]), None, "text column counts the tab as one");
        assert!(reindent("\tp Text\n\tp", 2, &[]).is_some());
        assert_eq!(reindent("p", 0, &[]), None);
    }
}
