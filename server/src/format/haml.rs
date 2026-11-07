//! Haml formatting that can't change the compiled template: each structural line is indented `width` spaces per
//! level, filter and `-#` bodies move as a block, continuation lines move with their line, and trailing whitespace
//! goes where Haml ignores it. Every line stays; the blank ones that may go are reported.

use crate::format::blank_lines::{code_role, drop_childless_openers, Body, LineInfo, LineRole};
use crate::format::{Indented, Settings};
use crate::haml::parser::{lines_with_endings, parse, Role};
use crate::util::{leading_ws, BOM};

const BLANK_SPACE: [char; 2] = [' ', '\t'];

/// The formatted text, or `None` when Haml would reject the text (or `width` is 0). Trailing whitespace stays unless `trim_trailing`.
pub(crate) fn format(text: &str, settings: &Settings) -> Option<Indented> {
    let (width, trim_trailing) = (settings.indent_size, settings.trailing_whitespace);
    if width == 0 {
        return None;
    }
    let body = text.strip_prefix(BOM).unwrap_or(text);
    let parsed = parse(text);
    if parsed.error.is_some() {
        return None;
    }
    let lines = lines_with_endings(body);
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..text.len() - body.len()]);
    // The old and the new indentation columns of each structural line, for the lines that move with it.
    let mut columns = vec![(0usize, 0usize); parsed.roles.len()];
    // Haml takes its indentation unit from the first indented line it reads, a continuation line included.
    let mut unit_known = false;
    for (i, (&role, &(line, ending))) in parsed.roles.iter().zip(&lines).enumerate() {
        let old = leading_ws(line).len();
        let content = &line[old..];
        match role {
            Role::Blank if trim_trailing => {}
            Role::Blank => out.push_str(line),
            Role::Structural { depth } => {
                columns[i] = (columns_of(&line[..old], width), depth * width);
                unit_known |= depth > 0;
                out.push_str(&" ".repeat(depth * width));
                out.push_str(line_end(content, trim_trailing));
            }
            Role::Inner { owner, counted } => {
                let (owner_old, owner_new) = columns[owner];
                let mut column = (columns_of(&line[..old], width) + owner_new).saturating_sub(owner_old);
                if counted {
                    // Haml reads this line's depth, so the column has to be a whole number of units.
                    column = (column + width / 2) / width * width;
                }
                if !unit_known && column > 0 {
                    column = width;
                    unit_known = true;
                }
                out.push_str(&" ".repeat(column));
                out.push_str(line_end(content, trim_trailing));
            }
            Role::FlatBody { owner, prefix_len } => match prefix_len {
                // Before Haml knows its indentation unit a body has no prefix to strip: the whole line is text.
                0 if content.is_empty() && unit_known && trim_trailing => {}
                0 => out.push_str(line),
                _ => {
                    unit_known = true;
                    out.push_str(&" ".repeat(columns[owner].1 + width));
                    out.push_str(&line[prefix_len..]);
                }
            },
        }
        out.push_str(ending);
    }
    // Haml reads nothing past its last line with content.
    for &(line, ending) in &lines[parsed.roles.len()..] {
        out.push_str(if trim_trailing { line.trim_matches(BLANK_SPACE) } else { line });
        out.push_str(ending);
    }
    // Haml also ends a line at a lone `\r`, which the blank-line pass doesn't.
    let lone_cr = lines.iter().any(|&(_, ending)| ending == "\r");
    let mut infos: Vec<LineInfo> = (0..lines.len())
        .map(|i| {
            let role = parsed.roles.get(i).copied();
            let removable = !lone_cr && role.is_none_or(|role| role == Role::Blank);
            match role {
                Some(Role::Structural { depth }) if !lone_cr => {
                    let tail =
                        parsed.roles[i + 1..].iter().take_while(|&&next| matches!(next, Role::Inner { owner, .. } if owner == i)).count();
                    LineInfo { removable, depth: Some(depth), role: node_role(lines[i].0.trim_start()), tail, ..LineInfo::default() }
                }
                _ => LineInfo { removable, ..LineInfo::default() },
            }
        })
        .collect();
    drop_childless_openers(&mut infos);
    Some(Indented { text: out, lines: infos, notes: vec![] })
}

/// What a node does if it has children: Ruby opens a block, a tag an element; a filter or text opens nothing.
fn node_role(content: &str) -> LineRole {
    if let Some(code) = content.strip_prefix('-') {
        return if code.starts_with('#') { LineRole::Other } else { code_role(code) };
    }
    let output = ["=", "!=", "&=", "~"].iter().any(|prefix| content.starts_with(prefix));
    let tag = content.starts_with(['%', '.']) || (content.starts_with('#') && !content.starts_with("#{"));
    match (output, tag) {
        (true, _) => LineRole::Opens(Body::Block),
        (_, true) => LineRole::Opens(Body::Element),
        _ => LineRole::Other,
    }
}

fn line_end(content: &str, trim_trailing: bool) -> &str {
    if trim_trailing {
        content.trim_end_matches(BLANK_SPACE)
    } else {
        content
    }
}

/// A tab counts as one indent level.
fn columns_of(whitespace: &str, width: usize) -> usize {
    whitespace.chars().map(|c| if c == '\t' { width } else { 1 }).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reindent(text: &str, width: usize) -> Option<String> {
        format(text, &Settings { indent_size: width, ..Settings::default() }).map(|indented| indented.text)
    }

    fn info(text: &str) -> Vec<(Option<usize>, LineRole)> {
        format(text, &Settings::default()).unwrap().lines.iter().map(|line| (line.depth, line.role)).collect()
    }

    fn fmt(text: &str) -> String {
        let once = reindent(text, 2).expect("formattable");
        assert_eq!(reindent(&once, 2).as_deref(), Some(once.as_str()), "idempotent");
        once
    }

    #[test]
    fn reindents_structure() {
        assert_eq!(fmt("%div\n    %p\n        %b x  \n\t\n    %i\n"), "%div\n  %p\n    %b x\n\n  %i\n");
        assert_eq!(fmt("%div\n\t%p\n\t\t%b\n"), "%div\n  %p\n    %b\n");
        assert_eq!(reindent("%ul\n  %li\n", 4).as_deref(), Some("%ul\n    %li\n"));
        let each = "%ul\n    - each do |x|\n        %li\n            -# deep\n";
        assert_eq!(fmt(each), "%ul\n  - each do |x|\n    %li\n      -# deep\n");
    }

    #[test]
    fn filter_bodies_move_as_a_block() {
        let css = "%div\n    %section\n        :css\n            a { b: c }\n              d\n        %p\n";
        assert_eq!(fmt(css), "%div\n  %section\n    :css\n      a { b: c }\n        d\n    %p\n");
        assert_eq!(
            fmt("%div\n\t:preserve\n\t\tif (a) {\n\t\t    b();  \n\t\t}\n"),
            "%div\n  :preserve\n    if (a) {\n        b();  \n    }\n"
        );
        assert_eq!(fmt("%div\n  :plain\n    a\n    \tb\n"), "%div\n  :plain\n    a\n    \tb\n", "tabs past the prefix are content");
    }

    #[test]
    fn blank_lines_in_filter_bodies_keep_their_extra() {
        assert_eq!(fmt("%div\n    :plain\n        a\n          \n  \n        b\n"), "%div\n  :plain\n    a\n      \n\n    b\n");
    }

    #[test]
    fn continuations_move_with_their_line() {
        let text = "%div\n    %a{href: x,\n         title: y} link\n    = link_to 'x',\n          class: 'y'\n    %p a |\n    b |\n";
        assert_eq!(fmt(text), "%div\n  %a{href: x,\n       title: y} link\n  = link_to 'x',\n        class: 'y'\n  %p a |\n  b |\n");
        assert_eq!(fmt("%div\n    = f(1,\n  2)\n"), "%div\n  = f(1,\n2)\n", "clamped at the left edge");
    }

    #[test]
    fn a_continuation_line_that_sets_the_indentation_unit_is_one_unit() {
        assert_eq!(fmt("%p= x,\n     y\n-#\n     c\n"), "%p= x,\n  y\n-#\n  c\n");
    }

    #[test]
    fn the_line_after_a_tag_with_code_ending_in_a_comma_stays_a_whole_number_of_units() {
        let text = "%div\n  %p= link_to \"Home\",\n    root_path\n";
        assert_eq!(reindent(text, 4).as_deref(), Some("%div\n    %p= link_to \"Home\",\n        root_path\n"));
        assert_eq!(fmt("%div\n   %p= link_to \"x\",\n      y\n"), "%div\n  %p= link_to \"x\",\n      y\n");
    }

    #[test]
    fn trailing_whitespace_only_where_haml_ignores_it() {
        assert_eq!(
            fmt("%p  \n  text  \n  :plain\n    a  \n  = f x,  \n    y  \n%i \r\n"),
            "%p\n  text\n  :plain\n    a  \n  = f x,\n    y\n%i\r\n"
        );
    }

    #[test]
    fn keeps_line_endings_and_the_byte_order_mark() {
        assert_eq!(fmt("\u{feff}%div\r\n    %p\r\n"), "\u{feff}%div\r\n  %p\r\n");
        assert_eq!(fmt("%div\r    %p\r"), "%div\r  %p\r");
        assert_eq!(fmt("%div\n    %p\r\n    %i"), "%div\n  %p\r\n  %i");
    }

    #[test]
    fn every_line_stays_and_the_blank_ones_that_may_go_are_reported() {
        assert_eq!(fmt("%p\n  \n \t\n"), "%p\n\n\n");
        assert_eq!(fmt("%p\r\n\r\n"), "%p\r\n\r\n");
        assert_eq!(fmt("\n\n"), "\n\n");
        let removable =
            |text: &str| format(text, &Settings::default()).unwrap().lines.iter().map(|line| line.removable).collect::<Vec<_>>();
        assert_eq!(removable("%p\n\n%i\n"), [false, true, false, true]);
        // A blank line in a filter or `-#` comment body is content; Haml skips the ones in multi-line Ruby.
        assert_eq!(removable(":plain\n  a\n\n  b\n%p\n"), [false, false, false, false, false, true]);
        assert_eq!(removable("-#\n  a\n\n  b\n"), [false, false, false, false, true]);
        assert_eq!(removable("%p= f(1,\n\n  2)\n"), [false, true, false, true]);
        assert_eq!(removable("%p\r\n\r\n%i\r\n\r"), [false; 5], "a lone CR ends a line only for Haml");
    }

    #[test]
    fn lines_report_their_depth_and_what_they_open() {
        use LineRole::{Branch, Opens, Other};
        let text = "- if a\n  %div\n    = f do\n      x\n- else\n  :plain\n    y\n%p\n  \\- b\n.c\n#{d}\n";
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
                (Some(0), Other),
                (None, Other),
            ]
        );
        assert_eq!(
            info("- case 1\n- when 1\n  a\n- elsewhere\n  b\n- in 2\n").iter().map(|l| l.1).collect::<Vec<_>>(),
            [Opens(Body::Block), Branch, Other, Opens(Body::Block), Other, Branch, Other]
        );
        assert_eq!(info("%p\r%i\r")[0], (None, Other), "a lone CR: no rule applies");
    }

    #[test]
    fn files_haml_rejects_come_back_as_none() {
        assert_eq!(reindent("%p= x\n  %b", 2), None, "illegal nesting");
        assert_eq!(reindent("%div\n    %p\n  %i odd", 2), None, "inconsistent indentation");
        assert_eq!(reindent("  %p", 2), None, "indented at the start");
        assert_eq!(reindent("%p", 0), None);
    }

    #[test]
    fn many_attribute_groups_stay_linear() {
        let text = format!("%div\n  %p{} x,\n  %i", "{}".repeat(20_000));
        let started = std::time::Instant::now();
        reindent(&text, 2);
        assert!(started.elapsed().as_millis() < 500);
    }
}
