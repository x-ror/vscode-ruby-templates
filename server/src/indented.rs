//! Line-based analysis shared by Slim and Haml: indentation structure, bracket
//! balance, embedded engines, and misuse of explicit `end`.

use crate::ruby::{analyze, ends_with_block, BlockEffect};
use crate::util::{error, leading_ws, line_range, warning};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang { Slim, Haml }

pub(crate) const SLIM_ENGINES: &[&str] = &["ruby", "javascript", "css", "sass", "scss", "less", "coffee", "markdown", "textile", "rdoc", "erb", "builder", "nokogiri", "wiki", "creole", "mediawiki", "asciidoc", "org", "styl", "stylus", "livescript", "babel", "typescript", "haml"];
const HAML_FILTERS: &[&str] = &["plain", "escaped", "preserve", "cdata", "javascript", "css", "ruby", "erb", "markdown", "sass", "scss", "less", "coffee", "textile", "maruku", "styl", "stylus", "builder", "nokogiri", "wiki", "creole", "mediawiki"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    /// Children are free-form text (filters, comments, text blocks).
    Verbatim,
    /// Ruby control line `- ...`.
    Control,
    /// Ruby output `= ...`.
    Output,
    Tag { inline_content: bool, inline_code_opens_block: bool },
    Text,
    Doctype,
    Other,
}

struct Frame { indent: usize, kind: LineKind, line: usize }

pub fn diagnostics(text: &str, lang: Lang) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut stack: Vec<Frame> = Vec::new();
    let unit: Option<usize> = detect_unit(&lines);
    let mut i = 0usize;

    while i < lines.len() {
        let raw = lines[i];
        let line_no = i;
        if raw.trim().is_empty() {
            i += 1;
            continue;
        }
        let ws = leading_ws(raw);
        if ws.contains('\t') {
            if ws.contains(' ') {
                out.push(error(line_range(text, line_no, 0, ws.len()), "Mixed tabs and spaces in indentation"));
            } else if lang == Lang::Haml {
                out.push(warning(line_range(text, line_no, 0, ws.len()), "Tabs used for indentation; Haml expects consistent spaces"));
            }
        }
        let indent = ws.chars().map(|c| if c == '\t' { 2 } else { 1 }).sum::<usize>();
        let _content = raw.trim_start();

        // Inside a verbatim block? Everything deeper is skipped.
        if let Some(top) = stack.last() {
            if top.kind == LineKind::Verbatim && indent > top.indent {
                i += 1;
                continue;
            }
        }

        // Dedent to a level that never existed (checked before popping).
        let known_levels: Vec<usize> = std::iter::once(0).chain(stack.iter().map(|f| f.indent)).collect();
        let bad_dedent = indent < *known_levels.last().unwrap_or(&0) && !known_levels.contains(&indent);
        if bad_dedent {
            out.push(error(line_range(text, line_no, 0, ws.len()), "Indentation does not match any outer level"));
        }

        // Pop to the parent level.
        while let Some(top) = stack.last() {
            if top.indent >= indent { stack.pop(); } else { break; }
        }
        let parent_indent = stack.last().map(|f| f.indent).unwrap_or(0);
        let is_child = stack.last().is_some() && indent > parent_indent;

        if is_child && !bad_dedent {
            let step = indent - parent_indent;
            match unit {
                None => {}
                Some(u) if step != u => {
                    let sev = if lang == Lang::Haml { DiagnosticSeverity::ERROR } else { DiagnosticSeverity::WARNING };
                    out.push(crate::util::diag(line_range(text, line_no, 0, ws.len()), sev,
                        format!("Inconsistent indentation: expected {} spaces, found {}", u, step)));
                }
                _ => {}
            }
        } else if stack.is_empty() && indent > 0 {
            out.push(error(line_range(text, line_no, 0, ws.len()), "Unexpected indentation at top level"));
        }

        // Illegal nesting checks against the parent.
        if is_child {
            if let Some(parent) = stack.last() {
                match parent.kind {
                    LineKind::Tag { inline_content: true, inline_code_opens_block: false } if lang == Lang::Haml => {
                        out.push(error(line_range(text, parent.line, 0, lines[parent.line].len()),
                            "Illegal nesting: content can't be both given on the same line and nested within it"));
                    }
                    LineKind::Text if lang == Lang::Haml => {
                        out.push(error(line_range(text, parent.line, 0, lines[parent.line].len()),
                            "Illegal nesting: nesting within plain text is illegal"));
                    }
                    LineKind::Doctype => {
                        out.push(error(line_range(text, line_no, 0, raw.len()), "Nothing can be nested under a doctype"));
                    }
                    _ => {}
                }
            }
        }

        // Classify the line and gather continuation lines.
        let (kind, consumed) = classify(lang, &lines, i, text, &mut out);
        stack.push(Frame { indent, kind, line: line_no });
        i += consumed.max(1);
    }
    out
}

/// Most common positive indentation step between a line and its parent: the file's indent unit.
fn detect_unit(lines: &[&str]) -> Option<usize> {
    let mut levels: Vec<usize> = vec![0];
    let mut counts: std::collections::BTreeMap<usize, usize> = Default::default();
    for raw in lines {
        if raw.trim().is_empty() { continue; }
        let indent = leading_ws(raw).chars().map(|c| if c == '\t' { 2 } else { 1 }).sum::<usize>();
        while levels.last().map(|l| *l > indent).unwrap_or(false) { levels.pop(); }
        let parent = *levels.last().unwrap_or(&0);
        if indent > parent {
            let step = indent - parent;
            *counts.entry(step).or_insert(0) += 1;
            levels.push(indent);
        }
    }
    // Highest count wins; on a tie the smaller step (BTreeMap iterates ascending, so `>` keeps the first).
    counts.into_iter().fold(None, |best: Option<(usize, usize)>, (step, n)| match best {
        Some((_, bn)) if bn >= n => best,
        _ => Some((step, n)),
    }).map(|(step, _)| step)
}

/// Returns the line kind and how many physical lines it spans.
fn classify(lang: Lang, lines: &[&str], i: usize, text: &str, out: &mut Vec<Diagnostic>) -> (LineKind, usize) {
    let raw = lines[i];
    let ws_len = leading_ws(raw).len();
    let c = raw.trim_start();

    // Doctype
    if (lang == Lang::Haml && c.starts_with("!!!")) || (lang == Lang::Slim && c.starts_with("doctype")) {
        return (LineKind::Doctype, 1);
    }
    // Comments
    if lang == Lang::Haml && (c.starts_with("-#") ) {
        return (LineKind::Verbatim, 1);
    }
    if lang == Lang::Slim && c.starts_with('/') {
        return (LineKind::Verbatim, 1);
    }
    if lang == Lang::Haml && c.starts_with('/') {
        return (LineKind::Other, 1);
    }
    // Filters / embedded engines
    if lang == Lang::Haml {
        if let Some(name) = c.strip_prefix(':') {
            if !name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
                if !HAML_FILTERS.contains(&name) {
                    out.push(warning(line_range(text, i, ws_len, raw.len()), format!("Unknown Haml filter `:{}`", name)));
                }
                return (LineKind::Verbatim, 1);
            }
        }
    }
    if lang == Lang::Slim {
        if let Some(name) = c.strip_suffix(':') {
            if !name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
                if !SLIM_ENGINES.contains(&name) {
                    out.push(warning(line_range(text, i, ws_len, raw.len()), format!("Unknown embedded engine `{}:`", name)));
                }
                return (LineKind::Verbatim, 1);
            }
        }
        if c.starts_with('|') || c.starts_with('\'') {
            return (LineKind::Verbatim, 1);
        }
    }
    // Ruby lines
    let ruby_prefix: Option<(&str, bool)> = if c.starts_with("-") {
        Some((&c[1..], true))
    } else if let Some(rest) = strip_output_prefix(lang, c) {
        Some((rest, false))
    } else { None };
    if let Some((code, is_control)) = ruby_prefix {
        let (joined, consumed) = join_continuation(lang, lines, i, code);
        check_explicit_end(lang, &joined, text, i, raw, out);
        if lang == Lang::Haml && is_control && joined.trim().starts_with('#') && !joined.trim().starts_with("#{") {
            // `-# comment` already handled; `- # foo` is a ruby comment: fine
        }
        return (if is_control { LineKind::Control } else { LineKind::Output }, consumed);
    }
    // Tags
    let is_tag = match lang {
        Lang::Haml => c.starts_with('%') || c.starts_with('.') || c.starts_with('#'),
        Lang::Slim => c.starts_with('.') || c.starts_with('#') || c.chars().next().map(|ch| ch.is_ascii_alphabetic()).unwrap_or(false),
    };
    if is_tag {
        let (rest, consumed) = skip_tag_head(lang, lines, i, text, out);
        let rest_t = rest.trim();
        let mut inline_content = !rest_t.is_empty();
        let mut opens_block = false;
        if let Some(code) = strip_output_prefix(lang, rest_t).or_else(|| if lang == Lang::Haml { rest_t.strip_prefix('~') } else { None }) {
            let (joined, _) = join_continuation(lang, lines, i, code);
            check_explicit_end(lang, &joined, text, i, raw, out);
            opens_block = ends_with_block(&joined) || matches!(analyze(&joined).0, BlockEffect::Open(_));
        } else if lang == Lang::Haml && (rest_t == "/" || rest_t.is_empty() || rest_t == "<" || rest_t == ">" || rest_t == "<>" || rest_t == "><") {
            inline_content = false;
        } else if lang == Lang::Slim && rest_t.starts_with(':') {
            // `li: a href="/"` inline tag — children allowed
            inline_content = false;
        }
        return (LineKind::Tag { inline_content, inline_code_opens_block: opens_block }, consumed);
    }
    (LineKind::Text, 1)
}

fn strip_output_prefix<'a>(lang: Lang, s: &'a str) -> Option<&'a str> {
    match lang {
        Lang::Haml => s.strip_prefix("!=").or_else(|| s.strip_prefix("&=")).or_else(|| s.strip_prefix('=')),
        Lang::Slim => {
            let t = s.strip_prefix("==").or_else(|| s.strip_prefix('='))?;
            Some(t.trim_start_matches(['<', '>']))
        }
    }
}

/// Join Ruby continuation lines (`,` or `\` at end, Haml `|` multiline).
fn join_continuation(lang: Lang, lines: &[&str], i: usize, first: &str) -> (String, usize) {
    let mut code = first.to_string();
    let mut consumed = 1;
    loop {
        let t = code.trim_end();
        let continues = t.ends_with(',') || t.ends_with('\\') || (lang == Lang::Haml && t.ends_with(" |"))
            || t.ends_with("do") && false;
        if !continues { break; }
        let Some(next) = lines.get(i + consumed) else { break };
        let next_t = next.trim();
        if lang == Lang::Haml && t.ends_with(" |") {
            code = format!("{} {}", t.trim_end_matches('|').trim_end(), next_t.trim_end_matches('|').trim_end());
            consumed += 1;
            if !next_t.ends_with('|') { break; }
            continue;
        }
        code = format!("{} {}", t.trim_end_matches('\\'), next_t);
        consumed += 1;
    }
    (code, consumed)
}

fn check_explicit_end(lang: Lang, code: &str, text: &str, line: usize, raw: &str, out: &mut Vec<Diagnostic>) {
    let (_, _, closes) = analyze(code);
    if closes > 0 {
        let name = if lang == Lang::Haml { "Haml" } else { "Slim" };
        out.push(error(line_range(text, line, leading_ws(raw).len(), raw.len()),
            format!("`end` is not allowed: {} closes blocks by indentation", name)));
    }
}

/// Skip over `%tag.class#id{...}(...)[...]` (Haml) or `tag.class#id attr=val (...)` (Slim),
/// validating bracket balance (possibly across lines). Returns the remainder and lines consumed.
fn skip_tag_head<'a>(lang: Lang, lines: &[&'a str], i: usize, text: &str, out: &mut Vec<Diagnostic>) -> (String, usize) {
    let raw = lines[i];
    let mut consumed = 1;
    let mut s: String = raw.trim_start().to_string();
    let mut pos = 0usize;
    let bytes = |s: &String| s.as_bytes().to_vec();
    // tag name / classes / ids
    let b = bytes(&s);
    while pos < b.len() {
        let ch = b[pos] as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '%' | '.' | '#' | '-' | '_' | ':') { pos += 1; } else { break; }
    }
    // Slim: `li: a ...` — the colon ends the head; treat the rest as content
    loop {
        let b = s.as_bytes();
        if pos >= b.len() { break; }
        let ch = b[pos] as char;
        let opener = match lang {
            Lang::Haml => matches!(ch, '{' | '(' | '['),
            Lang::Slim => matches!(ch, '(' | '[' | '{') || ch == '*',
        };
        if ch == ' ' && lang == Lang::Slim {
            // inline attributes `a=b` may follow; stop at the first token that is not `name=`
            let rest = &s[pos..];
            let trimmed = rest.trim_start();
            if looks_like_attr(trimmed) {
                pos += rest.len() - trimmed.len();
                match consume_attr(&s[pos..]) {
                    Ok(n) => pos += n,
                    Err(open_ch) => {
                        out.push(error(line_range(text, i, leading_ws(raw).len(), raw.len()),
                            format!("Unclosed `{}` in attribute value", open_ch)));
                        return (String::new(), consumed);
                    }
                }
                continue;
            }
            break;
        }
        if !opener { break; }
        if ch == '*' { pos += 1; continue; }
        let close = match ch { '{' => '}', '(' => ')', '[' => ']', _ => unreachable!() };
        match find_balanced(&s[pos..], ch, close) {
            Some(len) => pos += len,
            None => {
                // Try to extend across following lines (multiline attributes).
                let mut extended = false;
                while let Some(next) = lines.get(i + consumed) {
                    consumed += 1;
                    s.push('\n');
                    s.push_str(next);
                    if find_balanced(&s[pos..], ch, close).is_some() { extended = true; break; }
                    if consumed > 20 { break; }
                }
                if extended {
                    pos += find_balanced(&s[pos..], ch, close).unwrap();
                } else {
                    out.push(error(line_range(text, i, leading_ws(raw).len(), raw.len()),
                        format!("Unclosed `{}` in tag attributes", ch)));
                    return (String::new(), consumed);
                }
            }
        }
    }
    (s[pos..].to_string(), consumed)
}

fn looks_like_attr(s: &str) -> bool {
    let name_len = s.chars().take_while(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '@' | '.')).count();
    name_len > 0 && s[name_len..].starts_with('=') && !s[name_len..].starts_with("==") || (name_len > 0 && s[name_len..].starts_with("=="))
}

/// Length of `name=value` where value is a quoted string, a balanced bracket group, or a bare word.
/// Err(open) when a bracket in the value is never closed.
fn consume_attr(s: &str) -> std::result::Result<usize, char> {
    let name_len = s.chars().take_while(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '@' | '.')).count();
    let mut pos = name_len;
    while s[pos..].starts_with('=') { pos += 1; }
    while s[pos..].starts_with(' ') { pos += 1; }
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
                    '(' | '[' | '{' => { depth += 1; last_open = ch; }
                    ')' | ']' | '}' => depth -= 1,
                    ' ' if depth <= 0 => { n = idx; break; }
                    _ => {}
                }
                n = idx + ch.len_utf8();
            }
            if depth > 0 { return Err(last_open); }
            n
        }
    };
    Ok(pos + len)
}

fn find_quoted(s: &str, q: char) -> Option<usize> {
    let mut esc = false;
    for (idx, ch) in s.char_indices().skip(1) {
        if esc { esc = false; continue; }
        if ch == '\\' { esc = true; continue; }
        if ch == q { return Some(idx + 1); }
    }
    None
}

/// Length of a balanced `open ... close` group starting at s[0], honoring strings.
fn find_balanced(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    let mut esc = false;
    for (idx, ch) in s.char_indices() {
        if let Some(q) = in_str {
            if esc { esc = false; } else if ch == '\\' { esc = true; } else if ch == q { in_str = None; }
            continue;
        }
        match ch {
            '"' | '\'' => in_str = Some(ch),
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 { return Some(idx + ch.len_utf8()); }
            }
            _ => {}
        }
    }
    None
}
