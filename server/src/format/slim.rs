//! Slim formatting that can't change the compiled template: each Slim line is re-indented to
//! `tab_size` spaces per nesting level, text and engine blocks and continuations move with their
//! owner line, and trailing whitespace goes where Slim ignores it. Every line stays; the blank ones that may go are reported.

use crate::format::blank_lines::{code_role, drop_childless_openers, Body, LineInfo, LineRole};
use crate::format::{Indented, Settings};
use crate::slim::parser::{indent_width, parse, Role};
use crate::util::{leading_ws, raw_lines, BOM};

/// The formatted text, or `None` when Slim would reject the text (or `tab_size` is 0). Trailing whitespace stays unless `trim_trailing`.
pub(crate) fn format(text: &str, settings: &Settings, extra_engines: &[String]) -> Option<Indented> {
    let (tab_size, trim_trailing) = (settings.indent_size, settings.trailing_whitespace);
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
        let content = if !trim_trailing || info.keeps_trailing || stripped.ends_with([',', '\\']) { content } else { stripped };
        if !content.is_empty() {
            out.push_str(&" ".repeat(new_width));
            out.push_str(content);
        } else if !trim_trailing {
            out.push_str(ws);
        }
        out.push_str(cr);
    }
    let mut infos: Vec<LineInfo> = parsed
        .lines
        .iter()
        .zip(&lines)
        .enumerate()
        .map(|(i, (info, &(_, line, _)))| LineInfo {
            removable: info.role == Role::Blank && !info.inside_block,
            depth: match info.role {
                Role::Structural { depth } => Some(depth),
                _ => None,
            },
            role: if matches!(info.role, Role::Structural { .. }) { node_role(line.trim_start()) } else { LineRole::Other },
            tail: parsed.lines[i + 1..].iter().take_while(|next| next.role == Role::Inner { owner: i }).count(),
            ..LineInfo::default()
        })
        .collect();
    drop_childless_openers(&mut infos);
    Some(Indented { text: out, lines: infos, notes: vec![] })
}

/// What a node does if it has children: Ruby opens a block, a tag an element; text and comments open nothing.
fn node_role(content: &str) -> LineRole {
    match content.chars().next() {
        Some('-') => code_role(&content[1..]),
        Some('=') => LineRole::Opens(Body::Block),
        Some('|' | '\'' | '/' | '<' | '\\') | None => LineRole::Other,
        Some(_) => LineRole::Opens(Body::Element),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reindent(text: &str, width: usize, extra_engines: &[String]) -> Option<String> {
        format(text, &Settings { indent_size: width, ..Settings::default() }, extra_engines).map(|indented| indented.text)
    }

    fn info(text: &str) -> Vec<(Option<usize>, LineRole)> {
        format(text, &Settings::default(), &[]).unwrap().lines.iter().map(|line| (line.depth, line.role)).collect()
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
        let removable =
            |text: &str| format(text, &Settings::default(), &[]).unwrap().lines.iter().map(|line| line.removable).collect::<Vec<_>>();
        assert_eq!(removable("p\n\nq\n\n"), [false, true, false, true, true]);
        // Between lines of a text block or an engine body a blank line is text; after the block it is not.
        assert_eq!(removable("| a\n\n  b\n\np\n"), [false, false, false, true, false, true]);
        assert_eq!(removable("javascript:\n  x\n\n  y\n"), [false, false, false, false, true]);
        assert_eq!(removable("/ c\n\n  d\n"), [false, true, false, true]);
    }

    #[test]
    fn lines_report_their_depth_and_what_they_open() {
        use LineRole::{Branch, Opens, Other};
        let text = "- if a\n  div\n    = f do\n      | x\n- else\n  javascript:\n    y\nul\n  li\n/ c\n  d\np\n";
        assert_eq!(
            info(text),
            [
                (Some(0), Opens(Body::Block)),
                (Some(1), Opens(Body::Element)),
                (Some(2), Opens(Body::Block)),
                (Some(3), Other),
                (Some(0), Branch),
                (Some(1), Other),
                (None, Other),
                (Some(0), Opens(Body::Element)),
                (Some(1), Other),
                (Some(0), Other),
                (None, Other),
                (Some(0), Other),
                (None, Other),
            ]
        );
    }

    #[test]
    fn refuses_what_it_cannot_keep() {
        assert_eq!(reindent("p\n    a\n  b", 2, &[]), None);
        assert_eq!(reindent("\tp Text\n\t\t  more", 2, &[]), None, "text column counts the tab as one");
        assert!(reindent("\tp Text\n\tp", 2, &[]).is_some());
        assert_eq!(reindent("p", 0, &[]), None);
    }
}
