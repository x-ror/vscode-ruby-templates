//! Slim structure checks: indentation, bracket balance, embedded engines, branches and misuse of explicit `end`.

use crate::slim_parser::{self, Piece, BOM, VOID_ELEMENTS};
use crate::slim_ruby;
use crate::util::{coded, error, find_balanced, leading_ws, span_range, warning};
use tower_lsp::lsp_types::Diagnostic;

/// Stable diagnostic codes; `rubyTemplates.slim.disabledDiagnostics` refers to these.
pub mod code {
    pub const BAD_DEDENT: &str = "bad-dedent";
    pub const DOCTYPE_CHILDREN: &str = "doctype-children";
    pub const VOID_CHILDREN: &str = "void-children";
    pub const UNKNOWN_ENGINE: &str = "unknown-engine";
    /// Slim parser errors that belong to no other code.
    pub const SYNTAX_ERROR: &str = "syntax-error";
    pub const UNCLOSED_ATTRIBUTE: &str = "unclosed-attribute";
    pub const RUBY_SYNTAX: &str = "ruby-syntax";
    pub const EXPLICIT_END: &str = "explicit-end";
    /// Checks that parse the `-`/`=` code as Ruby.
    pub const RUBY: &[&str] = &[RUBY_SYNTAX, EXPLICIT_END];
}

/// Slim::Parser default `tabsize`: a tab advances to the next multiple of 4.
const SLIM_TABSIZE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    /// Children are free-form text (filters, comments, text blocks).
    Verbatim,
    Tag {
        void: bool,
        /// Ends in `/`: Slim accepts no children at all.
        closed: bool,
    },
    /// Ruby lines and text.
    Other,
    Doctype,
}

struct Frame {
    indent: usize,
    kind: LineKind,
    line: usize,
    has_child: bool,
}

pub fn diagnostics(text: &str, extra_engines: &[String], ruby: ruby_prism::Version) -> Vec<Diagnostic> {
    let Some(body) = text.strip_prefix(BOM) else {
        return check_structure(text, extra_engines, ruby);
    };
    // Slim drops the byte order mark, but the editor still counts it on the first line.
    let mut out = check_structure(body, extra_engines, ruby);
    let bom_units = BOM.encode_utf16().count() as u32;
    for position in out.iter_mut().flat_map(|d| [&mut d.range.start, &mut d.range.end]).filter(|p| p.line == 0) {
        position.character += bom_units;
    }
    out
}

fn check_structure(text: &str, extra_engines: &[String], ruby: ruby_prism::Version) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut stack: Vec<Frame> = Vec::new();
    let parsed = slim_parser::parse(text, extra_engines);
    if let Some(syntax) = &parsed.syntax_error {
        out.push(syntax_diagnostic(&lines, syntax));
    }
    if let Some((line, name)) = parsed.repeated_attribute.as_ref().filter(|_| parsed.valid) {
        let raw = lines[*line];
        let range = span_range(*line, raw, leading_ws(raw).len(), raw.len());
        // Temple raises this while compiling, so it carries no line; the tag's line stands in.
        out.push(coded(error(range, format!("Multiple {name} attributes specified")), code::SYNTAX_ERROR));
    }
    // Temple raises this after `EndInserter` and the attribute merger, so it only shows when they stay quiet.
    let explicit_end = parsed.pieces.iter().any(|p| matches!(p, Piece::Control { code, .. } if is_explicit_end(code)));
    let filters_pass = parsed.valid && parsed.repeated_attribute.is_none() && !explicit_end;
    let invalid_doctype = parsed.invalid_doctype.as_ref().filter(|_| filters_pass);
    if let Some((line, kind)) = invalid_doctype {
        let raw = lines[*line];
        let range = span_range(*line, raw, leading_ws(raw).len(), raw.len());
        out.push(coded(error(range, format!("Invalid doctype {kind}")), code::SYNTAX_ERROR));
    }
    // Slim compiles only a template it parsed, and stops at the errors its filters raise.
    if filters_pass && invalid_doctype.is_none() {
        out.extend(slim_ruby::diagnostics(text, &parsed, ruby).into_iter().map(|d| coded(d, code::RUBY_SYNTAX)));
    }
    // A file the parser rejects has every line after the unclosed construct as its continuation; those still count.
    let roles = if parsed.valid { parsed.lines } else { vec![] };
    let mut i = 0usize;

    while i < lines.len() {
        let raw = lines[i];
        let line_no = i;
        // Continuations the parser knows and this loop doesn't (`a [href=x` + newline + `class=y]`) belong to their line.
        if raw.trim().is_empty() || roles.get(i).is_some_and(|l| matches!(l.role, slim_parser::Role::Inner { .. })) {
            i += 1;
            continue;
        }
        let ws = leading_ws(raw);
        // Tabs expand (see SLIM_TABSIZE), so mixing them with spaces is fine.
        let indent = indent_width(ws);

        // Inside a verbatim block? Everything deeper is skipped.
        if let Some(top) = stack.last() {
            if top.kind == LineKind::Verbatim && indent > top.indent {
                i += 1;
                continue;
            }
        }

        // Slim's "Malformed indentation": a dedent to a level that never existed, below the first line's too (checked before popping).
        let deepest = stack.last().map_or(0, |f| f.indent);
        let bad_dedent =
            parsed.stop_line.is_none_or(|stop| line_no < stop) && indent < deepest && !stack.iter().any(|f| f.indent == indent);
        if bad_dedent {
            out.push(coded(error(span_range(line_no, raw, ws.len(), raw.len()), "Malformed indentation"), code::BAD_DEDENT));
        }

        // Pop to the parent level.
        while stack.last().is_some_and(|top| top.indent >= indent) {
            stack.pop();
        }
        let parent_indent = stack.last().map_or(0, |f| f.indent);
        let is_child = !stack.is_empty() && indent > parent_indent;

        // Illegal nesting checks against the parent.
        // `/` is a Slim code comment (renders nothing); `/!` is an HTML comment.
        let content = raw.trim_start();
        let code_comment = content.starts_with('/') && !content.starts_with("/!");
        if let Some(parent) = stack.last_mut().filter(|_| is_child) {
            let line_content = span_range(line_no, raw, ws.len(), raw.len());
            let first_child = !parent.has_child && !code_comment;
            parent.has_child |= !code_comment;
            match parent.kind {
                LineKind::Doctype => {
                    out.push(coded(error(line_content, "Unexpected indentation"), code::DOCTYPE_CHILDREN));
                }
                LineKind::Tag { closed: true, .. } => {
                    out.push(coded(error(line_content, "Unexpected indentation"), code::VOID_CHILDREN));
                }
                LineKind::Tag { void: true, .. } if first_child => {
                    let name = tag_name(lines[parent.line]);
                    let message = format!("`{name}` is a void element and can't have children");
                    out.push(coded(warning(line_content, message), code::VOID_CHILDREN));
                }
                _ => {}
            }
        }

        // Classify the line and gather continuation lines.
        let (kind, consumed) = classify(&lines, i, &mut out);
        stack.push(Frame { indent, kind, line: line_no, has_child: false });
        i += consumed.max(1);
    }
    out.sort_by_key(|d| (d.range.start.line, d.range.start.character));
    out
}

fn is_lone_word_with_colon(content: &str) -> bool {
    content.strip_suffix(':').is_some_and(|name| !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
}

/// Slim's first syntax error, filed under the code of the construct it is about.
fn syntax_diagnostic(lines: &[&str], syntax: &slim_parser::SyntaxError) -> Diagnostic {
    let raw = lines[syntax.line];
    let start = raw.char_indices().nth(syntax.column).map_or(raw.len(), |(at, _)| at);
    let code = match syntax.message.as_str() {
        "Unexpected end of file" => code::UNCLOSED_ATTRIBUTE,
        message if message.starts_with("Expected closing delimiter") => code::UNCLOSED_ATTRIBUTE,
        // A lone `name:` is an unregistered engine; `ul: ` or `html: body:` is a block expansion missing its tag.
        "Expected tag" if is_lone_word_with_colon(raw.trim_start()) => code::UNKNOWN_ENGINE,
        _ => code::SYNTAX_ERROR,
    };
    coded(error(span_range(syntax.line, raw, start, raw.len()), syntax.message.clone()), code)
}

fn indent_width(ws: &str) -> usize {
    ws.chars().fold(0, |col, c| if c == '\t' { (col / SLIM_TABSIZE + 1) * SLIM_TABSIZE } else { col + 1 })
}

/// Returns the line kind and how many physical lines it spans.
fn classify(lines: &[&str], i: usize, out: &mut Vec<Diagnostic>) -> (LineKind, usize) {
    let raw = lines[i];
    let ws_len = leading_ws(raw).len();
    let content_range = span_range(i, raw, ws_len, raw.len());
    let c = raw.trim_start();

    // Doctype
    if c.starts_with("doctype") {
        return (LineKind::Doctype, 1);
    }
    // Comments
    if is_conditional_comment(c) {
        return (LineKind::Tag { void: false, closed: false }, 1);
    }
    if c.starts_with('/') {
        return (LineKind::Verbatim, 1);
    }
    // Embedded engines
    if let Some(name) = c.strip_suffix(':') {
        if !name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
            return (LineKind::Verbatim, 1);
        }
    }
    if c.starts_with('|') || c.starts_with('\'') {
        return (LineKind::Verbatim, 1);
    }
    // Ruby lines
    let ruby_prefix: Option<(&str, bool)> =
        if let Some(rest) = c.strip_prefix('-') { Some((rest, true)) } else { strip_output_prefix(c).map(|rest| (rest, false)) };
    if let Some((code, is_control)) = ruby_prefix {
        let (_, consumed) = join_continuation(lines, i, code);
        if is_control {
            check_explicit_end(code, content_range, out);
        }
        return (LineKind::Other, consumed);
    }
    // Tags
    let is_tag = c.starts_with('.') || c.starts_with('#') || c.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic());
    if is_tag {
        let (rest, consumed) = skip_tag_head(lines, i);
        let rest_t = rest.trim();
        let closed = rest_t == "/";
        let void = closed || VOID_ELEMENTS.contains(&tag_name(raw));
        // Slim's parse_text_block takes every deeper line after inline text as text.
        if !void && !rest_t.is_empty() && !rest_t.starts_with(['=', '/', ':']) {
            return (LineKind::Verbatim, consumed);
        }
        return (LineKind::Tag { void, closed }, consumed);
    }
    (LineKind::Other, 1)
}

/// `/[if IE]`: Slim nests its children like a tag's.
fn is_conditional_comment(content: &str) -> bool {
    content.len() > 2 && content.starts_with("/[") && content.trim_end().ends_with(']')
}

/// Slim tag name at the start of the line (`img.photo src=x` → `img`); empty for `.class`/`#id` shortcuts.
fn tag_name(raw: &str) -> &str {
    let c = raw.trim_start();
    let end = c.find(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '-' | ':'))).unwrap_or(c.len());
    c[..end].trim_end_matches(':')
}

fn strip_output_prefix(s: &str) -> Option<&str> {
    let t = s.strip_prefix("==").or_else(|| s.strip_prefix('='))?;
    Some(t.trim_start_matches(['<', '>', '\'']))
}

/// Join Ruby continuation lines (`,` or `\` at end).
fn join_continuation(lines: &[&str], i: usize, first: &str) -> (String, usize) {
    let mut code = first.to_string();
    let mut consumed = 1;
    loop {
        let t = code.trim_end();
        if !(t.ends_with(',') || t.ends_with('\\')) {
            break;
        }
        let Some(next) = lines.get(i + consumed) else {
            break;
        };
        let next_t = next.trim();
        code = format!("{} {}", t.trim_end_matches('\\'), next_t);
        consumed += 1;
    }
    (code, consumed)
}

/// Slim::EndInserter's `/\Aend\b/` on a `-` line.
fn is_explicit_end(code: &str) -> bool {
    code.trim_start().strip_prefix("end").is_some_and(|after| after.chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_')))
}

/// Slim reports no line for an explicit `end`, so the diagnostic sits on its line.
fn check_explicit_end(code: &str, range: tower_lsp::lsp_types::Range, out: &mut Vec<Diagnostic>) {
    if is_explicit_end(code) {
        out.push(coded(error(range, "Explicit end statements are forbidden"), code::EXPLICIT_END));
    }
}

/// Skip over `tag.class#id attr=val (...)`, validating bracket balance (possibly across lines).
/// Returns the remainder and lines consumed.
fn skip_tag_head(lines: &[&str], i: usize) -> (String, usize) {
    let raw = lines[i];
    let mut consumed = 1;
    let mut s: String = raw.trim_start().to_string();
    let mut pos = 0usize;
    // tag name / classes / ids
    let b = s.as_bytes();
    while pos < b.len() {
        let ch = b[pos] as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '%' | '.' | '#' | '-' | '_' | ':') {
            pos += 1;
        } else {
            break;
        }
    }
    // Slim: `li: a ...` — the colon ends the head; treat the rest as content
    loop {
        let b = s.as_bytes();
        if pos >= b.len() {
            break;
        }
        let ch = b[pos] as char;
        let opener = matches!(ch, '(' | '[' | '{' | '*');
        if ch == ' ' {
            // inline attributes `a=b` may follow; stop at the first token that is not `name=`
            let rest = &s[pos..];
            let trimmed = rest.trim_start();
            if looks_like_attr(trimmed) {
                pos += rest.len() - trimmed.len();
                match consume_attr(&s[pos..]) {
                    Ok(n) => pos += n,
                    Err(_) => return (String::new(), consumed),
                }
                continue;
            }
            if trimmed.starts_with('*') && trimmed[1..].starts_with(|c: char| !c.is_whitespace()) {
                pos += rest.len() - trimmed.len();
                continue;
            }
            break;
        }
        if matches!(ch, '<' | '>') {
            pos += 1;
            continue;
        }
        if !opener {
            break;
        }
        if ch == '*' {
            pos += 1;
            if !s[pos..].starts_with(['{', '(', '[']) {
                pos += s[pos..].find(' ').unwrap_or(s.len() - pos);
            }
            continue;
        }
        let close = match ch {
            '{' => '}',
            '(' => ')',
            _ => ']',
        };
        if let Some(len) = find_balanced(&s[pos..], ch, close) {
            pos += len;
            continue;
        }
        // Try to extend across following lines (multiline attributes).
        let mut balanced = None;
        while let Some(next) = lines.get(i + consumed) {
            consumed += 1;
            s.push('\n');
            s.push_str(next);
            balanced = find_balanced(&s[pos..], ch, close);
            if balanced.is_some() || consumed > 20 {
                break;
            }
        }
        let Some(len) = balanced else {
            return (String::new(), consumed);
        };
        pos += len;
    }
    (s[pos..].to_string(), consumed)
}

/// Byte length of the attribute name at the start of `s`.
fn attr_name_len(s: &str) -> usize {
    s.find(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '@' | '.'))).unwrap_or(s.len())
}

fn looks_like_attr(s: &str) -> bool {
    let name_len = attr_name_len(s);
    name_len > 0 && s[name_len..].starts_with('=')
}

/// Length of `name=value` where value is a quoted string, a balanced bracket group, or a bare word.
/// Err(open) when a bracket in the value is never closed.
fn consume_attr(s: &str) -> std::result::Result<usize, char> {
    let mut pos = attr_name_len(s);
    while s[pos..].starts_with('=') {
        pos += 1;
    }
    while s[pos..].starts_with(' ') {
        pos += 1;
    }
    let rest = &s[pos..];
    let first = rest.chars().next().unwrap_or(' ');
    let len = match first {
        '"' | '\'' => find_quoted(rest, first).ok_or(first)?,
        _ => {
            // bare ruby expression: stop at whitespace unless inside brackets
            let mut depth = 0i32;
            let mut n = 0;
            let mut last_open = '(';
            for (idx, ch) in rest.char_indices() {
                match ch {
                    '(' | '[' | '{' => {
                        depth += 1;
                        last_open = ch;
                    }
                    ')' | ']' | '}' => depth -= 1,
                    ' ' if depth <= 0 => {
                        n = idx;
                        break;
                    }
                    _ => {}
                }
                n = idx + ch.len_utf8();
            }
            if depth > 0 {
                return Err(last_open);
            }
            n
        }
    };
    Ok(pos + len)
}

fn find_quoted(s: &str, q: char) -> Option<usize> {
    let mut esc = false;
    for (idx, ch) in s.char_indices().skip(1) {
        if esc {
            esc = false;
            continue;
        }
        if ch == '\\' {
            esc = true;
            continue;
        }
        if ch == q {
            return Some(idx + 1);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::{DiagnosticSeverity, NumberOrString};

    fn diagnostics(text: &str, extra_engines: &[String]) -> Vec<Diagnostic> {
        super::diagnostics(text, extra_engines, ruby_prism::Version::Latest)
    }

    /// (code, 0-based line, 0-based start column, severity) for each Slim diagnostic.
    fn slim(text: &str) -> Vec<(String, u32, u32, DiagnosticSeverity)> {
        slim_with(text, &[])
    }

    fn slim_with(text: &str, extra_engines: &[String]) -> Vec<(String, u32, u32, DiagnosticSeverity)> {
        diagnostics(text, extra_engines)
            .into_iter()
            .map(|d| {
                let Some(NumberOrString::String(code)) = d.code else { panic!("diagnostic without code: {}", d.message) };
                (code, d.range.start.line, d.range.start.character, d.severity.unwrap())
            })
            .collect()
    }

    const ERROR: DiagnosticSeverity = DiagnosticSeverity::ERROR;
    const WARNING: DiagnosticSeverity = DiagnosticSeverity::WARNING;

    fn one(code: &str, line: u32, col: u32, sev: DiagnosticSeverity) -> Vec<(String, u32, u32, DiagnosticSeverity)> {
        vec![(code.to_string(), line, col, sev)]
    }

    #[test]
    fn slim_tabs_advance_to_next_multiple_of_four() {
        assert_eq!(indent_width("\t"), 4);
        assert_eq!(indent_width("  \t"), 4);
        assert_eq!(indent_width("\t \t"), 8);
        assert_eq!(indent_width("     \t"), 8);
        // A tab and four spaces are the same level, so mixing them is fine in Slim.
        assert!(slim("div\n    p\n\tspan\n").is_empty());
        assert!(slim("div\n    p\n  \t    span\n").is_empty());
    }

    #[test]
    fn indentation_depth_is_free() {
        // README: "the indentation depth can be chosen as you like"; Slim compiles the first input.
        assert!(slim("div\n  p\n   span\n  x\n").is_empty());
        assert!(slim("div\n  p\n  p\n    a\n  ul\n   li\n").is_empty());
        assert!(slim("ruby:\n  a =\n    b ||\n      c\n= f x,\n  y\ndiv\n    p\n    span\n        b\n").is_empty());
    }

    #[test]
    fn bad_dedent() {
        assert_eq!(slim("div\n  p\n    a\n   span\n"), one(code::BAD_DEDENT, 3, 3, ERROR));
    }

    #[test]
    fn malformed_indentation_uses_slim_message() {
        // Slim's test_malformed_indentation, test_malformed_indentation2, and a dedent below the first line's level.
        for (text, line, column) in [
            ("\np\n  div Valid\n div Invalid\n", 3, 1),
            ("\n  div Valid\n div Invalid\n", 2, 1),
            ("  p\nb\n", 1, 0),
            ("\u{feff}  p\nb\n", 1, 0),
            ("div\n  /[if IE]\n      p\n    a\n", 3, 4),
        ] {
            let found: Vec<_> = diagnostics(text, &[]).into_iter().filter(|d| d.severity == Some(ERROR)).collect();
            assert_eq!(found.len(), 1, "{text:?}");
            assert_eq!(found[0].message, "Malformed indentation", "{text:?}");
            assert_eq!((found[0].range.start.line, found[0].range.start.character), (line, column), "{text:?}");
        }
    }

    #[test]
    fn dedent_check_stops_at_slim_syntax_error() {
        let found = diagnostics("  a href=(foo\n) t\n", &[]);
        assert!(found.iter().all(|d| d.message != "Malformed indentation"), "{found:?}");
    }

    #[test]
    fn bom_counts_in_columns_of_the_first_line() {
        let found = diagnostics("\u{feff}?x\n", &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].range.start.line, found[0].range.start.character), (0, 1));
    }

    #[test]
    fn ruby_is_checked_only_when_slim_would_compile_it() {
        // A parser error, an explicit `end` and a repeated attribute come before Slim generates any Ruby.
        for text in ["?bad\n= foo(\n", "- end\n= foo(\n", "a id=1 id=2\n= foo(\n"] {
            let found = diagnostics(text, &[]);
            assert!(found.iter().all(|d| d.code != Some(NumberOrString::String(code::RUBY_SYNTAX.into()))), "{text:?}: {found:?}");
        }
        let found = diagnostics("= foo(\n", &[]);
        assert!(found.iter().all(|d| d.code == Some(NumberOrString::String(code::RUBY_SYNTAX.into()))) && found.len() == 2, "{found:?}");
    }

    #[test]
    fn removed_syntax_is_what_slim_5_2_2_makes_of_it() {
        // `=' x` and `-' x` start a Ruby string now; the tag forms (`p=' x`, `p' text`, `p<' x`) still mean trailing space,
        // and `|'`, `''`, `!!!` and the backtick are text or unknown indicators. Checked against Slim 5.2.2.
        for (text, first) in [
            ("=' x\n", "unterminated string meets end of file"),
            ("==' x\n", "unterminated string meets end of file"),
            ("-' x\n", "unterminated string meets end of file"),
            ("p\n  =' y\n", "unterminated string meets end of file"),
            ("`text`\n", "Unknown line indicator"),
            ("! html5\n", "Unknown line indicator"),
            ("!!! html\n", "Unknown line indicator"),
            (":javascript\n  x\n", "Unknown line indicator"),
            ("p x=1' text\n", "unterminated string meets end of file"),
        ] {
            let found = diagnostics(text, &[]);
            assert_eq!(found.first().map(|d| d.message.as_str()), Some(first), "{text:?}");
        }
        for text in ["p=' x\n", "p==' x\n", "p' text\n", "p'\n  | x\n", "p<' x\n", "|' x\n", "'' x\n", "=> x\n", "a href=\"x\"' text\n"] {
            assert_eq!(diagnostics(text, &[]), vec![], "{text:?}");
        }
    }

    #[test]
    fn removed_engines_are_tags_without_a_tag_after_the_colon() {
        // Builder, Creole, Erb, Nokogiri, Opal, Org-Ruby, Styl, Wikicloth (4.0.0), Less (5.2.2), Haml, Liquid, Radius, Markaby.
        for name in
            ["builder", "creole", "erb", "nokogiri", "opal", "org", "styl", "wikicloth", "less", "haml", "liquid", "radius", "markaby"]
        {
            let found = diagnostics(&format!("{name}:\n  x\n"), &[]);
            assert_eq!(found.len(), 1, "{name}");
            assert_eq!(found[0].message, "Expected tag", "{name}");
            assert_eq!(found[0].range.start.character as usize, name.len() + 1, "{name}");
            assert_eq!(found[0].code, Some(NumberOrString::String(code::UNKNOWN_ENGINE.into())), "{name}");
        }
        let found = diagnostics("erb(x=\"y\"):\n  x\n", &[]);
        assert_eq!((found[0].message.as_str(), found[0].range.start.character), ("Expected tag", 11));
        // The engines 5.2.2 still has.
        for name in ["ruby", "javascript", "css", "sass", "scss", "coffee", "markdown", "textile", "rdoc", "asciidoc"] {
            assert_eq!(diagnostics(&format!("{name}:\n  x\n"), &[]).iter().filter(|d| d.message == "Expected tag").count(), 0, "{name}");
        }
    }

    #[test]
    fn doctypes_temple_does_not_know() {
        // Temple::HTML::Fast, format xhtml: the table's keys and `xml [encoding]`, any case.
        for (text, kind, line) in [
            ("doctype\n", "", 0),
            ("p\ndoctype foo\n", "foo", 1),
            ("doctype html5\n", "html5", 0),
            ("doctype  HTML Extra \n", "html extra", 0),
            ("doctype xmlfoo\n", "xmlfoo", 0),
            ("doctype foo\n= bar(\n", "foo", 0),
        ] {
            let found = diagnostics(text, &[]);
            assert_eq!(found.len(), 1, "{text:?}: {found:?}");
            assert_eq!(found[0].message, format!("Invalid doctype {kind}"), "{text:?}");
            assert_eq!(found[0].range.start.line, line, "{text:?}");
            assert_eq!(found[0].code, Some(NumberOrString::String(code::SYNTAX_ERROR.into())), "{text:?}");
        }
        for kind in [
            "html",
            "HTML",
            "5",
            "1.1",
            "Strict",
            "frameset",
            "mobile",
            "basic",
            "transitional",
            "svg",
            "xml",
            "xml ISO-8859-1",
            "XML utf-8",
        ] {
            assert_eq!(diagnostics(&format!("doctype {kind}\n"), &[]), vec![], "{kind}");
        }
        assert_eq!(diagnostics("p\n  doctype foo\n", &[]).len(), 1);
        // Slim stops at its parser, `EndInserter` and the attribute merger first.
        for text in ["doctype foo\n?bad\n", "doctype foo\n- end\n", "doctype foo\na id=1 id=2\n"] {
            let found = diagnostics(text, &[]);
            assert!(found.iter().all(|d| !d.message.starts_with("Invalid doctype")), "{text:?}");
        }
    }

    #[test]
    fn top_level_indent() {
        assert_eq!(slim("  p a\n  p b\n"), vec![]);
    }

    #[test]
    fn inline_text_makes_deeper_lines_text() {
        // Slim 5 parses all of these without error: every deeper line after inline text is text.
        for ok in [
            "p Hello\n   world\n  more\n",
            "p Hello\n  span\n    a\n   b\n",
            "a> Link\n  b\n     c\n",
            "p #{x}\n  more\n",
            "p(class=\"x\") text\n  span x\n   y\n",
            "p class=\"x\" text\n  more\n",
            "div *{a: 1} text\n  span x\n   y\n",
        ] {
            assert_eq!(slim(ok), vec![], "{ok:?}");
        }
    }

    #[test]
    fn tag_without_inline_text_keeps_structure_checks() {
        // Slim 5 raises "Malformed indentation" for the `   b` line after each of these heads.
        for head in ["p(class=\"x\")", "p class=\"x\"", "p= x", "p x=1", "div"] {
            let text = format!("{head}\n  span\n    a\n   b\n");
            assert_eq!(slim(&text), one(code::BAD_DEDENT, 3, 3, ERROR), "{text:?}");
        }
    }

    #[test]
    fn whitespace_modifier_tag_keeps_structure_checks() {
        for head in ["p<", "p>", "p<>", "p.a.b<"] {
            let text = format!("{head}\n  span\n    a\n   b\n");
            assert_eq!(slim(&text), one(code::BAD_DEDENT, 3, 3, ERROR), "{text:?}");
        }
    }

    #[test]
    fn splat_attributes_tag_keeps_structure_checks() {
        for head in ["div *attrs", "p.a *{a: 1}"] {
            let text = format!("{head}\n  span\n    a\n   b\n");
            assert_eq!(slim(&text), one(code::BAD_DEDENT, 3, 3, ERROR), "{text:?}");
        }
    }

    #[test]
    fn doctype_children() {
        assert_eq!(slim("doctype html\n  p\n"), one(code::DOCTYPE_CHILDREN, 1, 2, ERROR));
    }

    #[test]
    fn doctype_children_use_slim_message() {
        // Slim's test_unexpected_indentation (and test_correct_filename), plus a nested doctype and a bare one.
        for (text, line, column) in
            [("\ndoctype 5\n  div Invalid\n", 2, 2), ("html\n  doctype 5\n    p\n", 2, 4), ("doctype\n    x\n", 1, 4)]
        {
            let found = diagnostics(text, &[]);
            assert_eq!(found.len(), 1, "{text:?}");
            assert_eq!(found[0].message, "Unexpected indentation", "{text:?}");
            assert_eq!((found[0].range.start.line, found[0].range.start.character), (line, column), "{text:?}");
        }
    }

    #[test]
    fn unclosed_attribute() {
        assert_eq!(slim("div\n  a href=(foo\n"), one(code::UNCLOSED_ATTRIBUTE, 1, 13, ERROR));
    }

    #[test]
    fn attribute_errors_use_slim_messages() {
        // Slim's test_expected_closing_attribute_delimiter, test_expected_closing_delimiter, test_missing_quote_unexpected_end,
        // then wrappers across lines, a trailing blank line, and a non-ASCII prefix.
        for (text, message, line, column) in [
            ("\np\n  img src=[hash[1] + hash[2]\n", "Expected closing delimiter ]", 2, 28),
            ("\np\n  img(src=\"img.jpg\" title={title}\n", "Expected closing delimiter )", 2, 33),
            ("\np\n  img(src=\"img.jpg\n", "Unexpected end of file", 2, 0),
            ("a href=(foo\n", "Expected closing delimiter )", 0, 11),
            ("a(href=\"x\"\n  class=\"y\"\n", "Expected closing delimiter )", 1, 11),
            ("a(href=\"x\"\n\n\n", "Expected closing delimiter )", 0, 10),
            ("a href=x(\"é\" y=[1\n", "Expected closing delimiter )", 0, 17),
            ("a href=[1,\n", "Unexpected end of file", 0, 0),
        ] {
            let found = diagnostics(text, &[]);
            assert_eq!(found.len(), 1, "{text:?}: {found:?}");
            assert_eq!(found[0].message, message, "{text:?}");
            assert_eq!((found[0].range.start.line, found[0].range.start.character), (line, column), "{text:?}");
        }
    }

    #[test]
    fn parser_errors_use_slim_messages() {
        // Slim's test_parser_errors.rb and probes: (template, message, 0-based line, character, code).
        for (text, message, line, column, code) in [
            ("\n.#test\n", "Illegal shortcut", 1, 0, code::SYNTAX_ERROR),
            ("\ndiv.#test\n", "Illegal shortcut", 1, 3, code::SYNTAX_ERROR),
            ("\np\n  img{src= }\n", "Invalid empty attribute", 2, 11, code::SYNTAX_ERROR),
            ("\np\n  img{src=}\n", "Invalid empty attribute", 2, 10, code::SYNTAX_ERROR),
            ("\np\n  img src=\n", "Invalid empty attribute", 2, 10, code::SYNTAX_ERROR),
            ("\nhtml: body: /comment\n", "Expected tag", 1, 12, code::SYNTAX_ERROR),
            ("\nhtml: body:/comment\n", "Expected tag", 1, 11, code::SYNTAX_ERROR),
            ("\nhtml: body:\n", "Expected tag", 1, 11, code::SYNTAX_ERROR),
            ("ul: \n", "Expected tag", 0, 4, code::SYNTAX_ERROR),
            ("p:\n", "Expected tag", 0, 2, code::UNKNOWN_ENGINE),
            ("mjml:\n  x\n", "Expected tag", 0, 5, code::UNKNOWN_ENGINE),
            ("\nimg / text\n", "Unexpected text after closed tag", 1, 6, code::SYNTAX_ERROR),
            ("\np\n  div Valid\n  .valid\n  #valid\n  ?invalid\n", "Unknown line indicator", 5, 2, code::SYNTAX_ERROR),
            ("a(=x)\n", "Expected attribute", 0, 2, code::SYNTAX_ERROR),
            ("a(\"x\")\n", "Expected attribute", 0, 2, code::SYNTAX_ERROR),
            // Slim stops at its first error.
            ("p\n  ?bad\na href=(foo\n", "Unknown line indicator", 1, 2, code::SYNTAX_ERROR),
        ] {
            let found = diagnostics(text, &[]);
            assert_eq!(found.len(), 1, "{text:?}: {found:?}");
            assert_eq!(found[0].message, message, "{text:?}");
            assert_eq!((found[0].range.start.line, found[0].range.start.character), (line, column), "{text:?}");
            assert_eq!(found[0].code, Some(NumberOrString::String(code.to_string())), "{text:?}");
        }
    }

    #[test]
    fn registered_engines_are_not_expected_tag_errors() {
        assert!(slim_with("mjml:\n  x\n", &["mjml".into()]).is_empty());
    }

    #[test]
    fn brackets_inside_ruby_strings_in_attributes_are_not_unclosed() {
        // Slim counts only the delimiter that opened the value, so the `[` in the string is plain text.
        assert!(slim("\ndiv crazy=action_path('[') id=\"crazy_delimiters\"\n").is_empty());
    }

    #[test]
    fn explicit_end() {
        assert_eq!(slim("- items.each do\n  p\n- end\n"), one(code::EXPLICIT_END, 2, 0, ERROR));
    }

    fn explicit_end_lines(text: &str) -> Vec<u32> {
        let found = diagnostics(text, &[]);
        found.iter().filter(|d| d.message == "Explicit end statements are forbidden").map(|d| d.range.start.line).collect()
    }

    #[test]
    fn explicit_end_uses_slim_message_and_condition() {
        // Slim's test_explicit_end; EndInserter rejects a `-` line whose code starts with `end\b`.
        assert_eq!(explicit_end_lines("\ndiv\n  - if show_first?\n      p The first paragraph\n  - end\n"), [4]);
        for text in ["-end\n", "- end # note\n", "- end.foo\n", "-   end\n"] {
            assert_eq!(explicit_end_lines(text), [0], "{text:?}");
        }
        for text in ["- endx\n", "= end\n", "p= end\n", "- foo; end\n", "- ensure\n", "p end\n"] {
            assert!(explicit_end_lines(text).is_empty(), "{text:?}");
        }
    }

    #[test]
    fn repeated_attributes_use_temple_message() {
        // Temple's AttributeMerger merges only `class`; any other repeated name in a tag fails, on the tag's line.
        for (text, message, line) in [
            ("\n#alpha id=\"beta\" Test it\n", "Multiple id attributes specified", 1),
            ("a href=\"x\" href=\"y\"\n", "Multiple href attributes specified", 0),
            ("p\n  a href=x href=y\n", "Multiple href attributes specified", 1),
            ("a(href=\"x\" href)\n", "Multiple href attributes specified", 0),
            ("#a#b\n", "Multiple id attributes specified", 0),
            ("a data-x=1 data-x=2\n", "Multiple data-x attributes specified", 0),
            ("a id=\"x\" class=\"y\" id=\"z\"\n", "Multiple id attributes specified", 0),
            ("a(href=\"x\"\n  href=\"y\")\n", "Multiple href attributes specified", 0),
        ] {
            let found = diagnostics(text, &[]);
            assert_eq!(found.len(), 1, "{text:?}: {found:?}");
            assert_eq!(found[0].message, message, "{text:?}");
            assert_eq!(found[0].range.start.line, line, "{text:?}");
            assert_eq!(found[0].code, Some(NumberOrString::String(code::SYNTAX_ERROR.to_string())), "{text:?}");
        }
        for text in [".a class=\"b\"\n", "a href=1\n  b href=2\n", "a X=1 x=2\n", "#alpha *{id: \"beta\"} t\n", "a b c b\n"] {
            let found: Vec<_> = diagnostics(text, &[]).into_iter().filter(|d| d.message.starts_with("Multiple")).collect();
            assert!(found.is_empty(), "{text:?}: {found:?}");
        }
        // A parser error comes first: Slim never reaches the compile step.
        let found = diagnostics("a id=1 id=2\n?x\n", &[]);
        assert_eq!(found.iter().map(|d| d.message.as_str()).collect::<Vec<_>>(), ["Unknown line indicator"]);
    }

    #[test]
    fn unknown_engine_and_extra_engines() {
        assert_eq!(slim("mjml:\n  x\n"), one(code::UNKNOWN_ENGINE, 0, 5, ERROR));
        assert!(slim_with("mjml:\n  x\n", &["mjml".into()]).is_empty());
        assert!(slim("javascript:\n  x\n").is_empty());
    }

    #[test]
    fn void_children() {
        assert_eq!(slim("br\n  p x\n  p y\n"), one(code::VOID_CHILDREN, 1, 2, WARNING));
        assert_eq!(slim("input.big type=\"x\"\n  p\n"), one(code::VOID_CHILDREN, 1, 2, WARNING));
        assert!(slim("br\n  / old stuff\n").is_empty());
        assert!(slim("p = x\n  span y\n").is_empty());
    }

    #[test]
    fn closed_tag_children() {
        // Slim rejects a child under a tag closed with `/`, void element or not.
        for (text, line, column) in [
            ("div/\n  p x\n", 1, 2),
            ("br/ \n  p\n", 1, 2),
            ("img/\n  p\n", 1, 2),
            ("img src=\"x\"/\n  p\n", 1, 2),
            ("br /\n  p\n", 1, 2),
            ("div\n  a.b#c(href=\"x\")/\n\n    p\n      q\n", 3, 4),
        ] {
            let found = diagnostics(text, &[]);
            assert_eq!(found.len(), 1, "{text:?}: {found:?}");
            assert_eq!(found[0].message, "Unexpected indentation", "{text:?}");
            assert_eq!(found[0].severity, Some(ERROR), "{text:?}");
            assert_eq!((found[0].range.start.line, found[0].range.start.character), (line, column), "{text:?}");
        }
    }

    #[test]
    fn non_ascii_text_after_tag() {
        assert!(slim("p Größe\np ü\nspan.x ä=1 b\n").is_empty());
    }

    #[test]
    fn block_helpers_under_a_control_line_are_valid() {
        // Valid Slim and Ruby: `-` just discards the helper's HTML.
        assert!(slim("- form_with model: @u do |f|\n  p\n").is_empty());
        assert!(slim("-  tag.div do\n  p\n").is_empty());
        assert!(slim("= form_with model: @u do |f|\n  p\n").is_empty());
    }

    #[test]
    fn broken_fixture() {
        let text = include_str!("../../test/broken/bad.html.slim");
        assert_eq!(
            slim(text),
            vec![
                (code::DOCTYPE_CHILDREN.to_string(), 1, 2, ERROR),
                (code::UNCLOSED_ATTRIBUTE.to_string(), 4, 23, ERROR),
                (code::EXPLICIT_END.to_string(), 7, 2, ERROR),
            ]
        );
    }
}
