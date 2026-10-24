//! Slim formatting that can't change the compiled template: each Slim line is re-indented to
//! `tab_size` spaces per nesting level, text and engine blocks and continuations move with their
//! owner line, and trailing whitespace goes where Slim ignores it.

use crate::slim_parser::{indent_width, parse, Role};

/// The formatted text, or `None` when Slim would reject the text (or `tab_size` is 0).
pub fn format(text: &str, tab_size: usize, extra_engines: &[String]) -> Option<String> {
    if tab_size == 0 {
        return None;
    }
    let parsed = parse(text, extra_engines);
    if !parsed.valid {
        return None;
    }
    let mut shift = vec![0isize; parsed.lines.len()];
    let mut out = String::with_capacity(text.len());
    for (i, (raw, info)) in text.split('\n').zip(&parsed.lines).enumerate() {
        if i > 0 {
            out.push('\n');
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
        if new_width < 0 || (info.raw_column && ws.contains('\t')) {
            return None;
        }
        // A trailing `,` or `\` before the whitespace would turn into a continuation once stripped.
        let stripped = content.trim_end_matches(|c: char| c.is_ascii_whitespace());
        let content = if info.keeps_trailing || stripped.ends_with([',', '\\']) {
            content
        } else {
            stripped
        };
        if !content.is_empty() {
            out.push_str(&" ".repeat(new_width as usize));
            out.push_str(content);
        }
        out.push_str(cr);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(text: &str) -> String {
        let once = format(text, 2, &[]).expect("formattable");
        assert_eq!(
            format(&once, 2, &[]).as_deref(),
            Some(once.as_str()),
            "idempotent"
        );
        once
    }

    #[test]
    fn reindents_structure() {
        assert_eq!(
            fmt("html\n    body\n        p x  \n    \n\tfooter"),
            "html\n  body\n    p x  \n\n  footer"
        );
        assert_eq!(
            format("ul\n  li\n", 4, &[]).as_deref(),
            Some("ul\n    li\n")
        );
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
        assert_eq!(
            fmt(text),
            "div\n  = link_to a,\n        b\n  a(href=\"/\"\n    class=\"x\") t\n"
        );
    }

    #[test]
    fn trailing_whitespace_only_where_ignored() {
        assert_eq!(
            fmt("p  \n| t  \n' u  \na href=x  \n/ c  \n<b>  \np Text  "),
            "p\n| t  \n' u  \na href=x\n/ c\n<b>  \np Text  "
        );
        assert_eq!(
            fmt("a href=x,  \n  y"),
            "a href=x,  \n  y",
            "`x,` + space ends the value"
        );
        assert_eq!(fmt("p title=\"a  \n  b\""), "p title=\"a  \n  b\"");
    }

    #[test]
    fn refuses_what_it_cannot_keep() {
        assert_eq!(format("p\n    a\n  b", 2, &[]), None);
        assert_eq!(
            format("\tp Text\n\t\t  more", 2, &[]),
            None,
            "text column counts the tab as one"
        );
        assert!(format("\tp Text\n\tp", 2, &[]).is_some());
        assert_eq!(format("p", 0, &[]), None);
    }
}
