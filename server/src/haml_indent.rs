//! Haml's diagnostics: the first error `haml_parser` finds, or the void elements that hold content.

use crate::haml_parser::{self, is_haml_multiline, is_ruby_multiline, strip_output_prefix};
use crate::util::{coded, error, find_balanced, leading_ws, span_range, warning};
use tower_lsp::lsp_types::Diagnostic;

/// Stable diagnostic codes; `rubyTemplates.haml.disabledDiagnostics` refers to these.
pub mod code {
    pub const INCONSISTENT_INDENT: &str = "inconsistent-indent";
    pub const TOP_LEVEL_INDENT: &str = "top-level-indent";
    pub const ILLEGAL_NESTING: &str = "illegal-nesting";
    pub const DOCTYPE_CHILDREN: &str = "doctype-children";
    pub const UNMATCHED_BRANCH: &str = "unmatched-branch";
    pub const VOID_CHILDREN: &str = "void-children";
    pub const EXPLICIT_END: &str = "explicit-end";
    /// Haml's parser errors that belong to no other code (`Invalid tag`, `There's no Ruby code…`).
    pub const SYNTAX_ERROR: &str = "syntax-error";
    pub const ILLEGAL_ELEMENT: &str = "illegal-element";
    pub const UNKNOWN_FILTER: &str = "unknown-filter";
    pub const SELF_CLOSING_CONTENT: &str = "self-closing-content";
    pub const UNCLOSED_ATTRIBUTE: &str = "unclosed-attribute";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    /// Children are free-form text (filters, comments, text blocks).
    Verbatim,
    /// Ruby line `- ...` / `= ...`.
    Ruby,
    Tag,
    Other,
}

/// A line the structure checks look at: not blank, not inside a verbatim block, not a continuation line.
struct Visit {
    line: usize,
    indent: usize,
    kind: LineKind,
    span: usize,
}

pub fn diagnostics(text: &str, parsed: &haml_parser::Parsed) -> Vec<Diagnostic> {
    if parsed.error.is_none() && parsed.void_content.is_empty() {
        return vec![];
    }
    let lines = haml_parser::physical_lines(text);
    let mut out: Vec<Diagnostic> = parsed.error.iter().map(|parse_error| parse_error_diagnostic(&lines, parse_error)).collect();
    out.extend(parsed.void_content.iter().map(|void| void_content_diagnostic(&lines, void)));
    out
}

/// Classifies each structural line, skipping verbatim bodies and the lines a continuation consumed.
fn visit_lines(lines: &[&str]) -> Vec<Visit> {
    let mut visits: Vec<Visit> = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        let indent = indent_width(leading_ws(lines[i]));
        let in_verbatim = visits.last().is_some_and(|v| v.kind == LineKind::Verbatim && indent > v.indent);
        if in_verbatim {
            i += 1;
            continue;
        }
        let (kind, consumed) = classify(lines, i);
        let span = consumed.max(1);
        visits.push(Visit { line: i, indent, kind, span });
        i += span;
    }
    visits
}

/// Haml lines outside filter and comment bodies, each with the physical lines it spans (`|`, `,`, multi-line `{}`).
pub fn haml_structure(text: &str) -> Vec<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    visit_lines(&lines).into_iter().map(|v| (v.line, v.span)).collect()
}

/// What a Haml line is, for features that only make sense at a line's start or in Ruby.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineRole {
    /// A structural line: `%tag`, `- code`, `:filter`, plain text, … (blank lines too).
    Start,
    /// Ruby that doesn't start the line: a continuation of `-`/`=` code or a multi-line `{}` hash, or a `:ruby` body.
    Ruby,
    /// A filter or `-#` body, or a `|` plain-text continuation.
    Text,
}

/// The `LineRole` of each line of a Haml document.
pub fn haml_line_roles(text: &str) -> Vec<LineRole> {
    let lines: Vec<&str> = text.lines().collect();
    let mut roles = vec![LineRole::Start; lines.len()];
    let visits = visit_lines(&lines);
    for (n, v) in visits.iter().enumerate() {
        let content = lines[v.line].trim_start_matches([' ', '\t']);
        let continuation = match v.kind {
            LineKind::Ruby => LineRole::Ruby,
            LineKind::Tag if tag_head_len(content).is_none() || haml_code_start(lines[v.line]).is_some() => LineRole::Ruby,
            _ => LineRole::Text,
        };
        roles[v.line + 1..v.line + v.span].fill(continuation);
        if v.kind == LineKind::Verbatim {
            let body = if content.trim_end() == ":ruby" { LineRole::Ruby } else { LineRole::Text };
            let next = visits.get(n + 1).map_or(lines.len(), |next| next.line);
            // Blank lines after the body belong to whatever comes next.
            let last = (v.line + 1..next).rev().find(|&i| !lines[i].trim().is_empty()).unwrap_or(v.line);
            roles[v.line + 1..=last].fill(body);
        }
    }
    roles
}

/// Tabs count as two columns.
fn indent_width(ws: &str) -> usize {
    ws.chars().map(|c| if c == '\t' { 2 } else { 1 }).sum()
}

/// `<br>x</br>`: the end tag of a void element is a parse error in HTML (WHATWG 13.2.6.4.7).
fn void_content_diagnostic(lines: &[&str], void: &haml_parser::VoidContent) -> Diagnostic {
    let raw = lines.get(void.line).copied().unwrap_or("");
    let start = leading_ws(raw).len();
    let what = if void.nested { "children" } else { "content" };
    let message = format!("`{}` is a void element and can't have {what}", void.name);
    coded(warning(span_range(void.line, raw, start, raw.len()), message), code::VOID_CHILDREN)
}

/// The first error Haml's parser raises: indentation errors are shown over the indentation, the others over the line.
fn parse_error_diagnostic(lines: &[&str], parse_error: &haml_parser::Error) -> Diagnostic {
    let raw = lines.get(parse_error.line).copied().unwrap_or("");
    let body = raw.strip_prefix(haml_parser::BOM).filter(|_| parse_error.line == 0).unwrap_or(raw);
    let indent = leading_ws(body).len();
    let bom_len = raw.len() - body.len();
    let (code, start, end) = match parse_error.message.as_str() {
        "Indenting at the beginning of the document is illegal." => (code::TOP_LEVEL_INDENT, 0, bom_len + indent),
        "Illegal nesting: nesting within a header command is illegal." => (code::DOCTYPE_CHILDREN, bom_len + indent, raw.len()),
        message if message.starts_with("You don't need to use") => (code::EXPLICIT_END, bom_len + indent, raw.len()),
        message if message.starts_with("Got \"") || message.contains("is indented at wrong level") => {
            (code::UNMATCHED_BRANCH, bom_len + indent, raw.len())
        }
        message if message.starts_with("FilterCompiler for") => (code::UNKNOWN_FILTER, bom_len + indent, raw.len()),
        message
            if message.starts_with("Invalid tag")
                || message.starts_with("Invalid filter name")
                || message.starts_with("There's no Ruby code") =>
        {
            (code::SYNTAX_ERROR, bom_len + indent, raw.len())
        }
        "Self-closing tags can't have content." => (code::SELF_CLOSING_CONTENT, bom_len + indent, raw.len()),
        "Unbalanced brackets." => (code::UNCLOSED_ATTRIBUTE, bom_len + indent, raw.len()),
        message if message.starts_with("Invalid attribute list") => (code::UNCLOSED_ATTRIBUTE, bom_len + indent, raw.len()),
        "Illegal element: classes and ids must have values." => (code::ILLEGAL_ELEMENT, bom_len + indent, raw.len()),
        message if message.starts_with("Illegal nesting") => (code::ILLEGAL_NESTING, bom_len + indent, raw.len()),
        _ => (code::INCONSISTENT_INDENT, 0, bom_len + if indent > 0 { indent } else { body.len() }),
    };
    coded(error(span_range(parse_error.line, raw, start, end), parse_error.message.clone()), code)
}

/// Returns the line kind and how many physical lines it spans.
fn classify(lines: &[&str], i: usize) -> (LineKind, usize) {
    let raw = lines[i];
    let ws_len = leading_ws(raw).len();
    let c = &raw[ws_len..];

    // Comments
    if c.starts_with("-#") {
        return (LineKind::Verbatim, 1);
    }
    if c.starts_with('/') {
        return (LineKind::Other, 1);
    }
    // Filters
    if let Some(name) = c.strip_prefix(':') {
        if !name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
            return (LineKind::Verbatim, 1);
        }
    }
    // Ruby lines
    if let Some(code) = c.strip_prefix('-').or_else(|| strip_output_prefix(c)) {
        return (LineKind::Ruby, continuation_len(lines, i, code));
    }
    // Tags
    if is_haml_tag(c) {
        let (rest, head_lines) = skip_tag_head(lines, i);
        if rest.trim_start_matches(['<', '>']).starts_with('/') {
            return (LineKind::Tag, head_lines);
        }
        let rest_t = rest.trim();
        let last_head_line = i + head_lines - 1;
        let tail_lines = match strip_output_prefix(rest_t) {
            Some(code) => continuation_len(lines, last_head_line, code),
            None => multiline_len(lines, last_head_line),
        };
        return (LineKind::Tag, head_lines - 1 + tail_lines);
    }
    (LineKind::Other, multiline_len(lines, i))
}

/// Byte offset where Ruby code starts on a Haml line, after a line-start `-`/`=`/`~`/`!=`/`&=`
/// or the same prefix after a single-line tag head (`%p{a: 1}= code`).
pub fn haml_code_start(line: &str) -> Option<usize> {
    let content = &line[leading_ws(line).len()..];
    if content.starts_with("-#") {
        return None;
    }
    if let Some(code) = content.strip_prefix('-') {
        return Some(line.len() - code.len());
    }
    let code_line = if is_haml_tag(content) { content[tag_head_len(content)?..].trim_start_matches(['<', '>']) } else { content };
    strip_output_prefix(code_line).map(|code| line.len() - code.len())
}

/// `%tag`, `.class` or `#id` (not `#{interpolation}`) at the start of a line's content.
pub fn is_haml_tag(content: &str) -> bool {
    content.starts_with(['%', '.']) || (content.starts_with('#') && !content.starts_with("#{"))
}

/// `%tag.class#id` without its attribute groups.
pub fn tag_name_len(s: &str) -> usize {
    s.find(|ch: char| !is_tag_name_char(ch)).unwrap_or(s.len())
}

/// A `{…}`, `(…)` or `[…]` group of a tag head: byte offset of the opening bracket, and just past the
/// closing one (`None` when the line ends first).
pub struct AttrGroup {
    pub open: char,
    pub start: usize,
    pub end: Option<usize>,
}

/// The attribute groups following `%tag.class#id` on one line; an unclosed group is the last one.
pub fn tag_attr_groups(s: &str) -> Vec<AttrGroup> {
    let mut pos = tag_name_len(s);
    let mut groups = Vec::new();
    while let Some(open @ ('{' | '(' | '[')) = s[pos..].chars().next() {
        let close = match open {
            '{' => '}',
            '(' => ')',
            _ => ']',
        };
        let end = find_balanced(&s[pos..], open, close).map(|len| pos + len);
        groups.push(AttrGroup { open, start: pos, end });
        let Some(end) = end else { break };
        pos = end;
    }
    groups
}

/// Length of `%tag.class#id{...}(...)[...]` on one line; `None` when a bracket isn't closed on it.
pub fn tag_head_len(s: &str) -> Option<usize> {
    tag_attr_groups(s).last().map_or(Some(tag_name_len(s)), |g| g.end)
}

fn is_tag_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '%' | '.' | '#' | '-' | '_' | ':' | '@')
}

/// Physical lines in the Haml `|` multiline group starting at line `i`, blank lines inside included; 1 if none.
fn multiline_len(lines: &[&str], i: usize) -> usize {
    if !is_haml_multiline(lines[i]) {
        return 1;
    }
    let mut len = 1;
    for (offset, next) in lines.iter().enumerate().skip(i + 1) {
        if next.trim().is_empty() {
            continue;
        }
        if !is_haml_multiline(next) {
            break;
        }
        len = offset - i + 1;
    }
    len
}

/// Physical lines of a Ruby line with its continuations (`,` or `\` at end, Haml `|` multiline).
fn continuation_len(lines: &[&str], i: usize, first: &str) -> usize {
    let group = multiline_len(lines, i);
    if group > 1 {
        return group;
    }
    let mut consumed = 1;
    let mut code = first.trim_end().to_string();
    while is_ruby_multiline(&code) || code.ends_with('\\') {
        let Some(next) = lines.get(i + consumed) else { break };
        code = format!("{} {}", code.trim_end_matches('\\'), next.trim());
        consumed += 1;
    }
    consumed
}

/// Skip over `%tag.class#id{...}(...)[...]` (possibly across lines).
/// Returns the remainder and lines consumed.
fn skip_tag_head(lines: &[&str], i: usize) -> (String, usize) {
    let raw = lines[i];
    let mut consumed = 1;
    let mut s: String = raw[leading_ws(raw).len()..].to_string();
    let mut pos = tag_name_len(&s);
    while let Some(ch @ ('{' | '(' | '[')) = s[pos..].chars().next() {
        let close = match ch {
            '{' => '}',
            '(' => ')',
            _ => ']',
        };
        match find_balanced(&s[pos..], ch, close) {
            Some(len) => pos += len,
            None => {
                // Try to extend across following lines (multiline attributes).
                let mut extended = false;
                while let Some(next) = lines.get(i + consumed) {
                    consumed += 1;
                    s.push('\n');
                    s.push_str(next);
                    if find_balanced(&s[pos..], ch, close).is_some() {
                        extended = true;
                        break;
                    }
                    if consumed > 20 {
                        break;
                    }
                }
                if extended {
                    pos += find_balanced(&s[pos..], ch, close).unwrap();
                } else {
                    return (String::new(), consumed);
                }
            }
        }
    }
    (s[pos..].to_string(), consumed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::DiagnosticSeverity;

    /// (line, start char, end char, message) for each Haml diagnostic.
    fn haml(text: &str) -> Vec<(u32, u32, u32, String)> {
        diagnostics(text, &haml_parser::parse(text))
            .into_iter()
            .map(|d| (d.range.start.line, d.range.start.character, d.range.end.character, d.message))
            .collect()
    }

    fn at(line: u32, start: u32, end: u32, message: &str) -> (u32, u32, u32, String) {
        (line, start, end, message.to_string())
    }

    #[test]
    fn broken_fixture() {
        let got = haml(include_str!("../../test/broken/bad.html.haml"));
        assert_eq!(
            got,
            vec![at(4, 4, 31, "Illegal nesting: content can't be both given on the same line as %span and nested within it."),]
        );
    }

    #[test]
    fn messages_and_ranges() {
        type Expected<'a> = (u32, u32, u32, &'a str);
        const TOP_INDENT: &str = "Indenting at the beginning of the document is illegal.";
        let cases: &[(&str, Expected)] = &[
            (
                "%div\n  %p\n    %b\n   %i",
                (
                    3,
                    0,
                    3,
                    "Inconsistent indentation: 3 spaces used for indentation, but the rest of the document was indented using 2 spaces.",
                ),
            ),
            (
                "- if x\n  %p\n   %b\n",
                (
                    2,
                    0,
                    3,
                    "Inconsistent indentation: 3 spaces used for indentation, but the rest of the document was indented using 2 spaces.",
                ),
            ),
            ("%div\n  %p\n      %b", (2, 0, 6, "The line was indented 2 levels deeper than the previous line.")),
            (" %p", (0, 0, 1, TOP_INDENT)),
            ("  %p foo", (0, 0, 2, TOP_INDENT)),
            (" \n\t\n %p foo", (2, 0, 1, TOP_INDENT)),
            ("\n\n %p foo", (2, 0, 1, TOP_INDENT)),
            ("  %a", (0, 0, 2, TOP_INDENT)),
            ("\n\n  %a", (2, 0, 2, TOP_INDENT)),
            ("%p\n \t%b", (1, 0, 2, "Indentation can't use both tabs and spaces.")),
            (
                "%div\n\t%p x\n%div\n  %p y",
                (
                    3,
                    0,
                    2,
                    "Inconsistent indentation: 2 spaces used for indentation, but the rest of the document was indented using 1 tab.",
                ),
            ),
            (
                "%div\n    %p x\n%div\n\t%p y",
                (
                    3,
                    0,
                    1,
                    "Inconsistent indentation: 1 tab used for indentation, but the rest of the document was indented using 4 spaces.",
                ),
            ),
            ("!!! 5\n  %p", (1, 2, 4, "Illegal nesting: nesting within a header command is illegal.")),
            ("hello\n  %b", (1, 2, 4, "Illegal nesting: nesting within plain text is illegal.")),
            ("#{x} text\n  %b", (1, 2, 4, "Illegal nesting: nesting within plain text is illegal.")),
            ("#{x}\n  %b", (1, 2, 4, "Illegal nesting: nesting within plain text is illegal.")),
            ("== hi #{x}\n  %b", (1, 2, 4, "Illegal nesting: nesting within plain text is illegal.")),
            ("%p== hi #{x}\n  %b", (1, 2, 4, "Illegal nesting: content can't be both given on the same line as %p and nested within it.")),
            (
                "%p= form_with model: @u do |f|\n  = f.text_field :name",
                (1, 2, 22, "Illegal nesting: content can't be both given on the same line as %p and nested within it."),
            ),
            (".user= x\n  %p", (1, 2, 4, "Illegal nesting: content can't be both given on the same line as %div and nested within it.")),
            ("%p= x ?,\n  %b", (1, 2, 4, "Illegal nesting: content can't be both given on the same line as %p and nested within it.")),
            ("/[if IE] text\n  %p", (1, 2, 4, "Illegal nesting: nesting within a tag that already has content is illegal.")),
            ("# heading", (0, 0, 9, "Illegal element: classes and ids must have values.")),
            ("...and more", (0, 0, 11, "Illegal element: classes and ids must have values.")),
            ("%br/ text", (0, 0, 9, "Self-closing tags can't have content.")),
            ("%br/\n  %p", (1, 2, 4, "Illegal nesting: nesting within a self-closing tag is illegal.")),
            ("/ note\n  %p", (1, 2, 4, "Illegal nesting: nesting within a tag that already has content is illegal.")),
            ("%div\n  - elsif x", (1, 2, 11, "Got \"elsif\" with no preceding \"if\"")),
            ("- when 1", (0, 0, 8, "Got \"when\" with no preceding \"if\"")),
            ("- if a\n  %p\n-# note\n- else", (3, 0, 6, "Got \"else\" with no preceding \"if\"")),
            ("%p= if a\n- else", (1, 0, 6, "Got \"else\" with no preceding \"if\"")),
            ("- if a\n  - else", (1, 2, 8, "\"else\" is indented at wrong level: expected 0, but was at 1.")),
        ];
        for (src, (line, start, end, message)) in cases {
            assert_eq!(haml(src), vec![at(*line, *start, *end, message)], "{src:?}");
        }
    }

    #[test]
    fn void_elements_with_content_warn_on_the_content() {
        let text = "%p\n  %br\n    x\n  %img{src: 1} y\n";
        let diagnostics = diagnostics(text, &haml_parser::parse(text));
        let got: Vec<_> =
            diagnostics.iter().map(|d| (d.range.start.line, d.range.start.character, d.range.end.character, &d.message)).collect();
        assert_eq!(
            got,
            [
                (2, 4, 5, &"`br` is a void element and can't have children".to_string()),
                (3, 2, 16, &"`img` is a void element and can't have content".to_string()),
            ]
        );
        assert!(diagnostics.iter().all(|d| d.severity == Some(DiagnosticSeverity::WARNING)));
        assert!(haml("%br\n  x\n% a").iter().all(|d| d.3 != "`br` is a void element and can't have children"));
    }

    #[test]
    fn ranges_cover_the_indentation_on_the_parsers_lines() {
        let top = "Indenting at the beginning of the document is illegal.";
        assert_eq!(haml("\u{feff}  %p"), vec![at(0, 0, 3, top)]);
        let inconsistent =
            "Inconsistent indentation: 3 spaces used for indentation, but the rest of the document was indented using 2 spaces.";
        assert_eq!(haml("%p\r  a\r   b"), vec![at(2, 0, 3, inconsistent)]);
    }

    #[test]
    fn indent_unit_is_the_first_indented_line() {
        let four = "Inconsistent indentation: 6 spaces used for indentation, but the rest of the document was indented using 4 spaces.";
        // Haml 6: a 4-space first indent makes a 2-space child step a non-multiple.
        assert_eq!(haml("%div\n    %p\n      %b\n"), vec![at(2, 0, 6, four)]);
        assert_eq!(haml("- if x\n    %p\n      %b\n    %i\n"), vec![at(2, 0, 6, four)]);
        assert_eq!(haml("%div\n    %p\n        %b\n    %i\n"), vec![]);
        assert_eq!(haml("%div\n\t%p\n\t\t%b\n\t%i\n"), vec![]);
        assert_eq!(haml("%div\n\t%p\n\t\t\t%b\n"), vec![at(2, 0, 3, "The line was indented 2 levels deeper than the previous line.")]);
    }

    #[test]
    fn off_unit_dedent_uses_haml_message() {
        let cases = [
            (
                "%div\n  %p\n    %b\n   %i\n",
                at(
                    3,
                    0,
                    3,
                    "Inconsistent indentation: 3 spaces used for indentation, but the rest of the document was indented using 2 spaces.",
                ),
            ),
            (
                "%div\n    %p\n  %b\n",
                at(
                    2,
                    0,
                    2,
                    "Inconsistent indentation: 2 spaces used for indentation, but the rest of the document was indented using 4 spaces.",
                ),
            ),
        ];
        for (src, expected) in cases {
            assert_eq!(haml(src), vec![expected], "{src:?}");
        }
    }

    #[test]
    fn duplicate_children_report_once() {
        assert_eq!(haml("%p text\n  %a\n  %b").len(), 1);
    }

    #[test]
    fn no_false_positives() {
        for src in [
            "~ capture do\n  %p x",
            "- x = 1\n  %p",
            "&~ x\n  %p",
            "~ x\n  %p",
            "= x\n  %p",
            "%p= link_to |\n    \"a\", |\n      root_path",
            "/[[if IE]]\n  %span hello\n  world",
            "%p= link_to \"x\",\n  class: \"y\"\n%b ok",
            "%p= \"a\" + |\n  \"b\" + |\n  \"c\" |\n%hr",
            "%div\n  Plain text that |\n    spans lines |\n  %hr",
            "- x = [1, |\n  2] |\n%p",
            "%div\n\t%p\n\t\t%b x",
            "- if a\n  %p\n- elsif b\n  %p\n- else\n  %p",
            "- if a\n- else\n  %p",
            "- if a\n  %p\n\n- else\n  %p",
            "- unless a\n  %p\n- else\n  %p",
            "= if a\n  %p\n- else\n  %p",
            "- x = if a\n  - 1\n- else\n  - 2",
            "- if a\n  - if b\n    %p\n  - else\n    %p\n- elsif c\n  %p",
            "- case x\n- when 1\n  %p\n- else\n  %p",
            "- case x\n  - when 1\n    %p\n  - when 2\n    %p\n  - else\n    %p",
            "- begin\n  %p\n- rescue => e\n  %p\n- ensure\n  %p",
            "- [1].each do |x|\n  %p= x",
            "- [1].each do | x |\n  %p= x",
            "- f = lambda do\n  %p",
            "- while x\n  %p",
            "/\n  %p x",
            "/[if IE]\n  %p x",
            "%p Dear Sir,\n  %b x",
            "%p= foo(1,\n  2) do |x|\n  %b",
            "/![if !IE]\n  %p x",
            "!~ capture do\n  %p",
            "%p&~ \"x\"",
            "#{a} b",
            ".9a x",
            ".@container",
            "#$x t",
            "#@x t",
            "%div\n  .@container.flex= render \"card\",\n    item: x",
            "%div\n  %p x\n\u{3000}y",
            "%div\n  :javascript\n    if (a) {\n    \tb();\n    }\n  :plain\n    \t x",
            "%a{href: \"x\",\n   title: \"y\"} z\n%div\n\t%p x",
            "%div\n\t%a{href: \"x\",\n\t   title: \"y\"} z",
            "= link_to \"x\",\n    \"/y\"\n%div\n\t%p x",
            "%p a |\n    b |\n%div\n\t%p x",
            "%p\n  ==   hi #{x}",
            ":coffeescript\n  x = 1",
        ] {
            assert_eq!(haml(src), vec![], "{src:?}");
        }
    }

    #[test]
    fn line_roles() {
        use LineRole::*;
        let text = concat!(
            "%a{href: x,\n",      // 0
            "   title: y}\n",     // 1
            "= link_to \"a\",\n", // 2
            "  b\n",              // 3
            ":css\n",             // 4
            "  .btn{\n",          // 5
            "\n",                 // 6
            ":ruby\n",            // 7
            "  x = link_\n",      // 8
            "-# note\n",          // 9
            "  = hidden\n",       // 10
            "%p a |\n",           // 11
            "  b |\n",            // 12
            "%hr\n",              // 13
            "%p= \"a\" + |\n",    // 14
            "  \"b\" |\n",        // 15
        );
        assert_eq!(
            haml_line_roles(text),
            [Start, Ruby, Start, Ruby, Start, Text, Start, Start, Ruby, Start, Text, Start, Text, Start, Start, Ruby]
        );
    }

    #[test]
    fn code_start() {
        for (line, start) in [
            ("= render 'x'", Some(1)),
            ("  - x = 1", Some(3)),
            ("%p= x", Some(3)),
            (".a#b{c: {d: 1}}(e=\"f\")!= x", Some(24)),
            ("%p<>&= x", Some(6)),
            ("#main~ x", Some(6)),
            ("&~ x", Some(2)),
            ("%p!~ x", Some(4)),
            ("%p text = x", None),
            ("%p == x", None),
            ("%p== #{x}", None),
            ("#{x} = y", None),
            ("-# = x", None),
            ("%p{a: 1", None),
        ] {
            assert_eq!(haml_code_start(line), start, "{line:?}");
        }
    }
}
