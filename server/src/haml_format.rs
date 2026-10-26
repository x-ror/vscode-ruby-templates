//! Haml re-indentation: each nesting level becomes one indent unit, the whitespace of the first indented line, as
//! Haml itself detects it. Filter and `-#` bodies and continuation lines (`|` multiline, Ruby after a trailing `,`,
//! unclosed attribute groups) keep their whitespace past their head's indent, so their content is unchanged.

use crate::format::{skip, Indented, SkipKind, Skipped};
use crate::haml_indent;
use regex::Regex;
use std::sync::LazyLock;
use tower_lsp::lsp_types::DiagnosticSeverity;

// Word characters are ASCII, as in Haml's Ruby regexps. Lines nested under these aren't Haml structure: filter bodies and `-#` comment bodies.
static VERBATIM_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(:[A-Za-z0-9_]+\s*$|-#)").unwrap());
static TAG_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[%.#][A-Za-z0-9_:.#-]*").unwrap());
static RUBY_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([-=~]|[!&][=~])").unwrap());
static MULTILINE_PIPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s\|\s*$").unwrap());
static BLOCK_PARAMS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[^A-Za-z0-9_])do\s*\|[^|]*\|\s*$").unwrap());
static TRAILING_COMMA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r",\s*$").unwrap());
static CONTINUED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(,|\s\|)\s*$").unwrap());
static OUTPUT_AFTER_GROUPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[<>&!]*=").unwrap());

/// Haml's own errors this re-indenter is there to fix; any other error leaves the file alone.
const INDENTATION_CODES: &[&str] = &[haml_indent::code::INCONSISTENT_INDENT, haml_indent::code::TOP_LEVEL_INDENT];

/// `text` re-indented; `tab_width` measures tabs. Trailing whitespace is dropped (with `trim`) on structural lines
/// only, where Haml strips it too.
pub(crate) fn indent(text: &str, tab_width: usize, trim: bool) -> Result<Indented, Skipped> {
    let doubt = haml_indent::diagnostics(text)
        .into_iter()
        .find(|d| d.severity == Some(DiagnosticSeverity::ERROR) && !crate::util::code(d).is_some_and(|c| INDENTATION_CODES.contains(&c)));
    if let Some(d) = doubt {
        return skip(SkipKind::Doubt, format!("line {}: {}", d.range.start.line + 1, d.message));
    }
    let mut out = String::with_capacity(text.len());
    let mut fixed = Vec::new();
    let mut indenter = Reindenter { tab_width, unit: None, levels: vec![0], verbatim: None, follow: None };
    for (i, raw) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let (line, cr) = raw.strip_suffix('\r').map_or((raw, ""), |l| (l, "\r"));
        let (new, structural) = indenter.line(line);
        let new = if trim && structural { new.trim_end_matches([' ', '\t']).to_string() } else { new };
        out.push_str(&new);
        out.push_str(cr);
        fixed.push(!structural);
    }
    Ok(Indented { text: out, fixed, notes: vec![] })
}

/// A line whose nested or continued lines move with it.
#[derive(Clone)]
struct Head {
    ws: String,
    indent: String,
    width: usize,
}

struct Reindenter {
    tab_width: usize,
    unit: Option<String>,
    levels: Vec<usize>,
    verbatim: Option<Head>,
    /// The head the previous line continues onto this one from, and its open bracket depth.
    follow: Option<(Head, usize)>,
}

impl Reindenter {
    fn width(&self, ws: &str) -> usize {
        ws.chars().map(|c| if c == '\t' { self.tab_width } else { 1 }).sum()
    }

    /// The re-indented line, and whether it is a structural line (not a body or a continuation).
    fn line(&mut self, line: &str) -> (String, bool) {
        let body = line.trim_start_matches([' ', '\t']);
        let ws = &line[..line.len() - body.len()];
        if let Some((head, depth)) = self.follow.take() {
            if body.is_empty() {
                self.follow = Some((head, depth));
                return (line.to_string(), false);
            }
            let after = scan_brackets(body, depth as isize, 0, false).0;
            let continued = after > 0 || (depth == 0 && CONTINUED.is_match(body));
            let moved = moved(line, &head);
            if continued {
                self.follow = Some((head, after.max(0) as usize));
            }
            return (moved, false);
        }
        let width = self.width(ws);
        if let Some(head) = &self.verbatim {
            if body.is_empty() {
                return (line.to_string(), false);
            }
            if width > head.width {
                return (moved(line, head), false);
            }
        }
        self.verbatim = None;
        if body.is_empty() {
            return (String::new(), true);
        }
        while self.levels.last().is_some_and(|&l| l > width) {
            self.levels.pop();
        }
        if self.levels.last().is_some_and(|&l| l < width) {
            self.levels.push(width);
        }
        if width > 0 && self.unit.is_none() {
            self.unit = Some(ws.to_string());
        }
        let indent = self.unit.as_deref().unwrap_or("").repeat(self.levels.len() - 1);
        let head = Head { ws: ws.to_string(), indent: indent.clone(), width };
        if VERBATIM_HEAD.is_match(body) {
            self.verbatim = Some(head.clone());
        }
        let (depth, _) = attribute_groups(body);
        if depth > 0 || continues_on_next_line(body) {
            self.follow = Some((head, depth));
        }
        (format!("{indent}{body}"), true)
    }
}

/// `line` with `head`'s old indentation swapped for its new one, or unchanged when it doesn't start with it.
fn moved(line: &str, head: &Head) -> String {
    line.strip_prefix(head.ws.as_str()).map_or_else(|| line.to_string(), |rest| format!("{}{rest}", head.indent))
}

/// Bracket depth after scanning `text` from `from`, skipping quoted strings, and where the scan stopped. With
/// `until_closed`, stops right after the bracket that brings the depth back to 0.
fn scan_brackets(text: &str, mut depth: isize, from: usize, until_closed: bool) -> (isize, usize) {
    let bytes = text.as_bytes();
    let mut quote = None;
    let mut i = from;
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(_) if c == b'\\' => i += 1,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'"' || c == b'\'' => quote = Some(c),
            None if b"{([".contains(&c) => depth += 1,
            None if b")}]".contains(&c) => {
                depth -= 1;
                if until_closed && depth == 0 {
                    return (depth, i + 1);
                }
            }
            None => {}
        }
        i += 1;
    }
    (depth, text.len())
}

/// A tag line's `{}` / `()` / `[]` groups: the depth an unclosed one leaves open, and where they end.
fn attribute_groups(body: &str) -> (usize, Option<usize>) {
    let Some(head) = TAG_HEAD.find(body) else { return (0, None) };
    let mut end = head.end();
    while body.as_bytes().get(end).is_some_and(|b| b"{([".contains(b)) {
        let (depth, group_end) = scan_brackets(body, 0, end, true);
        if depth > 0 {
            return (depth as usize, None);
        }
        end = group_end;
    }
    (0, Some(end))
}

/// Whether the next line belongs to this one: Haml `|` multiline, or Ruby continued after a trailing `,`.
fn continues_on_next_line(body: &str) -> bool {
    if MULTILINE_PIPE.is_match(body) && !BLOCK_PARAMS.is_match(body) {
        return true;
    }
    if !TRAILING_COMMA.is_match(body) {
        return false;
    }
    if RUBY_LINE.is_match(body) {
        return true;
    }
    attribute_groups(body).1.is_some_and(|end| OUTPUT_AFTER_GROUPS.is_match(&body[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reindent(lines: &[&str]) -> String {
        let text = lines.join("\n");
        let once = indent(&text, 2, false).unwrap_or_else(|e| panic!("{e}")).text;
        assert_eq!(indent(&once, 2, false).unwrap().text, once, "idempotent");
        once
    }

    fn same(lines: &[&str]) {
        assert_eq!(reindent(lines), lines.join("\n"));
    }

    #[test]
    fn levels_follow_the_first_indented_line() {
        let text = ["%div", "    %p", "        %b x", "    %p y", "  %i odd", "", "%footer"];
        assert_eq!(reindent(&text), ["%div", "    %p", "        %b x", "    %p y", "    %i odd", "", "%footer"].join("\n"));
        let each = ["%ul", "  - each do |x|", "      %li", "        -# deep"];
        assert_eq!(reindent(&each), ["%ul", "  - each do |x|", "    %li", "      -# deep"].join("\n"));
        same(&["%div", "\t%p", "\t\t%b"]);
        assert_eq!(reindent(&["%p", "    ", "  %b"]), "%p\n\n  %b", "blank structural lines become empty");
    }

    #[test]
    fn filter_bodies_move_with_their_head() {
        same(&["%div", "    :javascript", "      if (a) {", "          b();", "      }", "    %p"]);
        let css = ["%div", "  %section", "      :css", "        a { b: c }", "      %p"];
        assert_eq!(reindent(&css), ["%div", "  %section", "    :css", "      a { b: c }", "    %p"].join("\n"));
        for unchanged in [
            &["%div", "\t:preserve", "\t\tif (a) {", "\t\t    b();", "\t\t}"][..],
            &["%div", "  :preserve", "    a", "    \tb"],
            &["%div", "  :preserve", "    a", "        ", "    b"],
            &["%pre", "  :plain", "    a", "          ", "      b"],
        ] {
            same(unchanged);
        }
        let deep = ["%div", "  %p", "      :preserve", "        a", "        \tb", "            ", "  %i"];
        assert_eq!(reindent(&deep), ["%div", "  %p", "    :preserve", "      a", "      \tb", "            ", "  %i"].join("\n"));
    }

    #[test]
    fn continuations_move_with_their_line() {
        let text = [
            "%div",
            "  %div",
            "      %a{href: x,",
            "         title: y} link",
            "      = link_to 'x',",
            "            class: 'y'",
            "      %p",
        ];
        let want = ["%div", "  %div", "    %a{href: x,", "       title: y} link", "    = link_to 'x',", "          class: 'y'", "    %p"];
        assert_eq!(reindent(&text), want.join("\n"));
        // Plain text ending in `,` doesn't pull the next line along; a bracket in a string opens nothing.
        assert_eq!(reindent(&["%div", "    %p", "      Dear Sir,", "  %p"]), ["%div", "    %p", "        Dear Sir,", "    %p"].join("\n"));
        same(&["%div", "    %a{title: \"(\"} x", "    %p", "        %b y"]);
    }

    #[test]
    fn trailing_whitespace_only_on_structural_lines() {
        let text = "%p  \n  :plain\n    a  \n  = f x,  \n    y  \n%i \r\n";
        assert_eq!(indent(text, 2, true).unwrap().text, "%p\n  :plain\n    a  \n  = f x,\n    y  \n%i\r\n");
    }

    #[test]
    fn many_attribute_groups_stay_linear() {
        let text = format!("%div\n  %p{} x,\n  %i", "{}".repeat(20_000));
        let started = std::time::Instant::now();
        indent(&text, 2, true).unwrap();
        assert!(started.elapsed().as_millis() < 500);
    }

    #[test]
    fn other_errors_leave_the_file_alone() {
        assert!(indent("%p= x\n  %b", 2, true).is_err(), "illegal nesting");
        assert!(indent("%div\n    %p\n  %i odd", 2, true).is_ok(), "the indentation errors are what it fixes");
    }
}
