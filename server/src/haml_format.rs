//! Haml formatting that can't change the compiled template: each structural line is indented `width` spaces per
//! level, filter and `-#` bodies move as a block, continuation lines move with their line, and trailing whitespace
//! goes where Haml ignores it. Empty lines at the end become one line ending.

use crate::haml_parser::{lines_with_endings, parse, Role, BOM};

const BLANK_SPACE: [char; 2] = [' ', '\t'];

/// The formatted text, or `None` when Haml would reject the text (or `width` is 0).
pub(crate) fn format(text: &str, width: usize) -> Option<String> {
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
        let content = line.trim_start_matches(BLANK_SPACE);
        let old = line.len() - content.len();
        match role {
            Role::Blank => {}
            Role::Structural { depth } => {
                columns[i] = (columns_of(&line[..old], width), depth * width);
                unit_known |= depth > 0;
                out.push_str(&" ".repeat(depth * width));
                out.push_str(content.trim_end_matches(BLANK_SPACE));
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
                out.push_str(content.trim_end_matches(BLANK_SPACE));
            }
            Role::FlatBody { owner, prefix_len } => match prefix_len {
                // Before Haml knows its indentation unit a body has no prefix to strip: the whole line is text.
                0 if content.is_empty() && unit_known => {}
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
    Some(out)
}

/// A tab counts as one indent level.
fn columns_of(whitespace: &str, width: usize) -> usize {
    whitespace.chars().map(|c| if c == '\t' { width } else { 1 }).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(text: &str) -> String {
        let once = format(text, 2).expect("formattable");
        assert_eq!(format(&once, 2).as_deref(), Some(once.as_str()), "idempotent");
        once
    }

    #[test]
    fn reindents_structure() {
        assert_eq!(fmt("%div\n    %p\n        %b x  \n\t\n    %i\n"), "%div\n  %p\n    %b x\n\n  %i\n");
        assert_eq!(fmt("%div\n\t%p\n\t\t%b\n"), "%div\n  %p\n    %b\n");
        assert_eq!(format("%ul\n  %li\n", 4).as_deref(), Some("%ul\n    %li\n"));
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
        assert_eq!(format(text, 4).as_deref(), Some("%div\n    %p= link_to \"Home\",\n        root_path\n"));
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
    fn empty_lines_at_the_end_collapse_into_the_final_line_ending() {
        assert_eq!(fmt("%p\n\n\n"), "%p\n");
        assert_eq!(fmt("%p\n  \n \t\n"), "%p\n");
        assert_eq!(fmt("%p\r\n\r\n"), "%p\r\n");
        assert_eq!(fmt("%p"), "%p");
        assert_eq!(fmt(":plain\n  a\n\n  \n"), ":plain\n  a\n");
        assert_eq!(fmt("%p\n\n%i\n\n"), "%p\n\n%i\n");
        assert_eq!(fmt("\n\n"), "");
    }

    #[test]
    fn files_haml_rejects_come_back_as_none() {
        assert_eq!(format("%p= x\n  %b", 2), None, "illegal nesting");
        assert_eq!(format("%div\n    %p\n  %i odd", 2), None, "inconsistent indentation");
        assert_eq!(format("  %p", 2), None, "indented at the start");
        assert_eq!(format("%p", 0), None);
    }

    #[test]
    fn many_attribute_groups_stay_linear() {
        let text = format!("%div\n  %p{} x,\n  %i", "{}".repeat(20_000));
        let started = std::time::Instant::now();
        format(&text, 2);
        assert!(started.elapsed().as_millis() < 500);
    }
}
