//! Slim structure checks: indentation, bracket balance, embedded engines, branches and misuse of explicit `end`.

use crate::ruby::{analyze, ends_with_block};
use crate::util::{coded, error, find_balanced, leading_ws, line_range, warning};
use tower_lsp::lsp_types::Diagnostic;


/// Stable diagnostic codes; `rubyTemplates.slim.disabledDiagnostics` refers to these.
pub mod code {
    pub const INCONSISTENT_INDENT: &str = "inconsistent-indent";
    pub const BAD_DEDENT: &str = "bad-dedent";
    pub const TOP_LEVEL_INDENT: &str = "top-level-indent";
    pub const DOCTYPE_CHILDREN: &str = "doctype-children";
    pub const VOID_CHILDREN: &str = "void-children";
    pub const UNKNOWN_ENGINE: &str = "unknown-engine";
    pub const UNCLOSED_ATTRIBUTE: &str = "unclosed-attribute";
    pub const UNCLOSED_RUBY: &str = "unclosed-ruby";
    pub const EXPLICIT_END: &str = "explicit-end";
    pub const UNMATCHED_BRANCH: &str = "unmatched-branch";
    pub const SILENT_OUTPUT_HELPER: &str = "silent-output-helper";
    /// Checks that parse the `-`/`=` code as Ruby.
    pub const RUBY: &[&str] = &[UNCLOSED_RUBY, EXPLICIT_END, UNMATCHED_BRANCH, SILENT_OUTPUT_HELPER];
}

// Slim 5 `Slim::Embedded` registrations; apps can register more (see `extra_engines`).
pub(crate) const SLIM_ENGINES: &[&str] = &[
    "ruby",
    "javascript",
    "css",
    "sass",
    "scss",
    "coffee",
    "markdown",
    "textile",
    "rdoc",
    "asciidoc",
];
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];
/// Helpers whose block form returns the HTML, so `-` discards it.
const OUTPUT_BLOCK_HELPERS: &[&str] = &[
    "form_with",
    "form_for",
    "form_tag",
    "fields_for",
    "fields",
    "link_to",
    "button_to",
    "content_tag",
    "turbo_frame_tag",
    "render",
];
/// Slim::Parser default `tabsize`: a tab advances to the next multiple of 4.
const SLIM_TABSIZE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    /// Children are free-form text (filters, comments, text blocks).
    Verbatim,
    /// Ruby control line `- ...`; `keyword` is its leading branch keyword, or `do` for a trailing block.
    Control {
        keyword: Option<&'static str>,
    },
    /// Ruby output `= ...`.
    Output,
    Tag {
        void: bool,
    },
    Text,
    Doctype,
}

struct Frame {
    indent: usize,
    kind: LineKind,
    line: usize,
    /// Keyword that opened the `if`/`case`/`begin`… chain this control line belongs to.
    chain: Option<&'static str>,
    has_child: bool,
}

pub fn diagnostics(text: &str, extra_engines: &[String]) -> Vec<Diagnostic> {
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
        let ws_range = line_range(text, line_no, 0, ws.len());        // Tabs expand (see SLIM_TABSIZE), so mixing them with spaces is fine.
        let indent = indent_width(ws);

        // Inside a verbatim block? Everything deeper is skipped.
        if let Some(top) = stack.last() {
            if top.kind == LineKind::Verbatim && indent > top.indent {
                i += 1;
                continue;
            }
        }

        // Dedent to a level that never existed (checked before popping).
        let deepest = stack.last().map_or(0, |f| f.indent);
        let bad_dedent =
            indent < deepest && indent != 0 && !stack.iter().any(|f| f.indent == indent);
        if bad_dedent {
            out.push(coded(
                error(ws_range, "Indentation does not match any outer level"),
                code::BAD_DEDENT,
            ));
        }

        // Pop to the parent level; the last frame popped at this indent is the previous sibling.
        let mut prev_sibling = None;
        while let Some(top) = stack.last() {
            if top.indent < indent {
                break;
            }
            let top = stack.pop();
            if top.as_ref().is_some_and(|f| f.indent == indent) {
                prev_sibling = top;
            }
        }
        let parent_indent = stack.last().map_or(0, |f| f.indent);
        let is_child = !stack.is_empty() && indent > parent_indent;

        if is_child && !bad_dedent {
            let step = indent - parent_indent;
            if let Some(u) = unit.filter(|&u| u != step) {
                let message = format!(
                    "Inconsistent indentation: expected {} spaces, found {}",
                    u, step
                );
                out.push(coded(warning(ws_range, message), code::INCONSISTENT_INDENT));
            }
        } else if stack.is_empty() && indent > 0 {
            out.push(coded(
                error(ws_range, "Unexpected indentation at top level"),
                code::TOP_LEVEL_INDENT,
            ));
        }

        // Illegal nesting checks against the parent.
        // `/` is a Slim code comment (renders nothing); `/!` is an HTML comment.
        let content = raw.trim_start();
        let code_comment = content.starts_with('/') && !content.starts_with("/!");
        if let Some(parent) = stack.last_mut().filter(|_| is_child) {
            let line_content = line_range(text, line_no, ws.len(), raw.len());
            let first_child = !parent.has_child && !code_comment;
            parent.has_child |= !code_comment;
            match parent.kind {
                LineKind::Doctype => {
                    out.push(coded(
                        error(line_content, "Nothing can be nested under a doctype"),
                        code::DOCTYPE_CHILDREN,
                    ));
                }
                LineKind::Tag { void: true, .. } if first_child => {
                    let name = tag_name(lines[parent.line]);
                    let message = if VOID_ELEMENTS.contains(&name) {
                        format!("`{name}` is a void element and can't have children")
                    } else {
                        format!("`{name}` is explicitly closed with `/` and can't have children")
                    };
                    out.push(coded(warning(line_content, message), code::VOID_CHILDREN));
                }
                _ => {}
            }
        }

        // Classify the line and gather continuation lines.
        let (kind, consumed) = classify(&lines, i, text, extra_engines, &mut out);
        // A code comment between branches is transparent: `- if` / `/ note` / `- else`.
        let mut chain = prev_sibling
            .as_ref()
            .filter(|_| code_comment)
            .and_then(|f| f.chain);
        if let LineKind::Control {
            keyword: Some(keyword),
        } = kind
        {
            chain = Some(keyword);
            if let Some(openers) = branch_openers(keyword) {
                chain = match &prev_sibling {
                    Some(f) => f.chain.filter(|c| openers.contains(c)),
                    // `- case x` with its `- when`s nested one level deeper.
                    None => stack
                        .last()
                        .filter(|_| is_child)
                        .and_then(|f| f.chain)
                        .filter(|c| nested_branch_openers(keyword).contains(c)),
                };
                if chain.is_none() {
                    let named: Vec<&str> = openers.iter().copied().filter(|o| *o != "do").collect();
                    let block = if openers.contains(&"do") {
                        " or a block (`do`)"
                    } else {
                        ""
                    };
                    let message = format!(
                        "`- {}` without a matching `- {}`{} at the same indentation",
                        keyword,
                        named.join("`/`- "),
                        block
                    );
                    out.push(coded(
                        error(line_range(text, line_no, ws.len(), raw.len()), message),
                        code::UNMATCHED_BRANCH,
                    ));
                }
            }
        }
        stack.push(Frame {
            indent,
            kind,
            line: line_no,
            chain,
            has_child: false,
        });
        i += consumed.max(1);
    }
    out
}

fn indent_width(ws: &str) -> usize {
    ws.chars().fold(0, |col, c| {
        if c == '\t' {
            (col / SLIM_TABSIZE + 1) * SLIM_TABSIZE
        } else {
            col + 1
        }
    })
}

/// Keywords a branch line (`else`, `when`, …) may continue; `None` when `keyword` opens a chain.
fn branch_openers(keyword: &str) -> Option<&'static [&'static str]> {
    match keyword {
        "else" => Some(&["if", "unless", "case", "begin", "do"]),
        "elsif" => Some(&["if"]),
        "when" | "in" => Some(&["case"]),
        "rescue" | "ensure" => Some(&["begin", "do"]),
        _ => None,
    }
}

/// Openers a branch line may also continue from one level up, as the opener's first child.
fn nested_branch_openers(keyword: &str) -> &'static [&'static str] {
    match keyword {
        "when" | "in" => &["case"],
        "rescue" | "ensure" => &["begin"],
        _ => &[],
    }
}

/// Most common positive indentation step between a line and its parent: the file's indent unit.
fn detect_unit(lines: &[&str]) -> Option<usize> {
    let mut levels: Vec<usize> = vec![0];
    let mut counts: std::collections::BTreeMap<usize, usize> = Default::default();
    for raw in lines {
        if raw.trim().is_empty() {
            continue;
        }
        let indent = indent_width(leading_ws(raw));
        while levels.last().is_some_and(|l| *l > indent) {
            levels.pop();
        }
        let parent = *levels.last().unwrap_or(&0);
        if indent > parent {
            let step = indent - parent;
            *counts.entry(step).or_insert(0) += 1;
            levels.push(indent);
        }
    }
    // Highest count wins; on a tie the smaller step (BTreeMap iterates ascending, so `>` keeps the first).
    counts
        .into_iter()
        .fold(None, |best: Option<(usize, usize)>, (step, n)| match best {
            Some((_, bn)) if bn >= n => best,
            _ => Some((step, n)),
        })
        .map(|(step, _)| step)
}

/// Returns the line kind and how many physical lines it spans.
fn classify(
    lines: &[&str],
    i: usize,
    text: &str,
    extra_engines: &[String],
    out: &mut Vec<Diagnostic>,
) -> (LineKind, usize) {
    let raw = lines[i];
    let ws_len = leading_ws(raw).len();
    let content_range = line_range(text, i, ws_len, raw.len());
    let c = raw.trim_start();

    // Doctype
    if c.starts_with("doctype") {
        return (LineKind::Doctype, 1);
    }
    // Comments
    if c.starts_with('/') {
        return (LineKind::Verbatim, 1);
    }
    // Embedded engines
    if let Some(name) = c.strip_suffix(':') {
        if !name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
            if !SLIM_ENGINES.contains(&name) && !extra_engines.iter().any(|e| e == name) {
                out.push(coded(
                    warning(
                        content_range,
                        format!("Unknown embedded engine `{}:`", name),
                    ),
                    code::UNKNOWN_ENGINE,
                ));
            }
            return (LineKind::Verbatim, 1);
        }
    }
    if c.starts_with('|') || c.starts_with('\'') {
        return (LineKind::Verbatim, 1);
    }
    // Ruby lines
    let ruby_prefix: Option<(&str, bool)> = if let Some(rest) = c.strip_prefix('-') {
        Some((rest, true))
    } else {
        strip_output_prefix(c).map(|rest| (rest, false))
    };
    if let Some((code, is_control)) = ruby_prefix {
        let (joined, consumed) = join_continuation(lines, i, code);
        check_explicit_end(&joined, content_range, out);
        check_ruby_balance(&joined, content_range, out);
        if !is_control {
            return (LineKind::Output, consumed);
        }
        check_silent_output_helper(
            &joined,
            text,
            i,
            ws_len + 1 + (code.len() - code.trim_start().len()),
            out,
        );
        return (
            LineKind::Control {
                keyword: control_keyword(&joined),
            },
            consumed,
        );
    }
    // Tags
    let is_tag = c.starts_with('.')
        || c.starts_with('#')
        || c.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic());
    if is_tag {
        let (rest, consumed) = skip_tag_head(lines, i, text, out);
        let rest_t = rest.trim();
        let void = rest_t == "/" || VOID_ELEMENTS.contains(&tag_name(raw));
        if let Some(code) = strip_output_prefix(rest_t) {
            let (joined, _) = join_continuation(lines, i, code);
            check_explicit_end(&joined, content_range, out);
            check_ruby_balance(&joined, content_range, out);
        }
        return (LineKind::Tag { void }, consumed);
    }
    (LineKind::Text, 1)
}

/// Slim tag name at the start of the line (`img.photo src=x` → `img`); empty for `.class`/`#id` shortcuts.
fn tag_name(raw: &str) -> &str {
    let c = raw.trim_start();
    let end = c
        .find(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '-' | ':')))
        .unwrap_or(c.len());
    c[..end].trim_end_matches(':')
}

/// Leading branch keyword of a control line (`if`, `else`, …), or `do` when it ends with a block.
fn control_keyword(code: &str) -> Option<&'static str> {
    const KEYWORDS: &[&str] = &[
        "if", "unless", "case", "begin", "else", "elsif", "when", "in", "rescue", "ensure",
    ];
    let first = strip_assignment(code)
        .trim_start()
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .next()
        .unwrap_or("");
    KEYWORDS
        .iter()
        .copied()
        .find(|k| *k == first)
        .or_else(|| ends_with_block(code).then_some("do"))
}

/// The right-hand side of `x = …`, `@x ||= …`, `x &&= …`; otherwise `code` itself.
fn strip_assignment(code: &str) -> &str {
    let t = code.trim_start();
    let name_len = t
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '@'))
        .unwrap_or(t.len());
    if name_len == 0 {
        return t;
    }
    let rest = t[name_len..].trim_start();
    ["||=", "&&=", "="]
        .iter()
        .find_map(|op| rest.strip_prefix(op))
        .filter(|rhs| !rhs.starts_with(['=', '~', '>']))
        .unwrap_or(t)
}

/// `- form_with … do` builds HTML and returns it; with `-` that result is thrown away.
fn check_silent_output_helper(
    code: &str,
    text: &str,
    line: usize,
    code_col: usize,
    out: &mut Vec<Diagnostic>,
) {
    if !ends_with_block(code) {
        return;
    }
    let trimmed = code.trim_start();
    let call = trimmed.strip_prefix("tag.").map_or(trimmed, |_| "tag");
    let name_len = call
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(call.len());
    let name = &call[..name_len];
    if name != "tag" && !OUTPUT_BLOCK_HELPERS.contains(&name) {
        return;
    }
    let range = line_range(text, line, code_col, code_col + name_len);
    out.push(coded(
        warning(
            range,
            format!("`- {name} … do` discards the HTML it builds; use `= {name}`"),
        ),
        code::SILENT_OUTPUT_HELPER,
    ));
}

fn check_ruby_balance(code: &str, range: tower_lsp::lsp_types::Range, out: &mut Vec<Diagnostic>) {
    let Some(open) = unclosed_delimiter(code) else {
        return;
    };
    let what = if matches!(open, '"' | '\'' | '`') {
        "string"
    } else {
        "bracket"
    };
    // A warning: the scan is a heuristic, not a Ruby parser.
    out.push(coded(
        warning(range, format!("Unclosed {what} `{open}` in Ruby code")),
        code::UNCLOSED_RUBY,
    ));
}

/// Innermost bracket or string quote left open in a line of Ruby, ignoring a trailing `#` comment.
/// Returns `None` on a closer that doesn't match, or on a regex, `%` literal or `?x` character
/// literal: their contents aren't balanced, and telling them from operators needs a real parser.
fn unclosed_delimiter(code: &str) -> Option<char> {
    // Interpolation `#{` inside a string is pushed as `#` so its `}` returns to the string.
    let mut stack: Vec<char> = Vec::new();
    let mut chars = code.chars().peekable();
    let mut prev = ' ';
    let mut prev_solid = ' ';
    while let Some(ch) = chars.next() {
        let next = chars.peek().copied().unwrap_or(' ');
        let after_operand =
            prev_solid.is_alphanumeric() || matches!(prev_solid, '_' | ')' | ']' | '}');
        let in_code = !matches!(stack.last(), Some('"' | '\'' | '`'));
        if in_code {
            match ch {
                // `/re/` where an operand is expected; `foo /re/` too, but not `a / b`.
                '/' if !after_operand || (prev == ' ' && next != ' ') => return None,
                '%' if !after_operand || (prev == ' ' && next != ' ') => return None,
                '?' if !after_operand && next != ' ' => return None,
                // `$'`, `$"`, `$(`…: special globals.
                '$' => {
                    chars.next();
                    prev = ch;
                    prev_solid = 'x';
                    continue;
                }
                _ => {}
            }
        }
        if !ch.is_whitespace() {
            prev_solid = ch;
        }
        prev = ch;
        if let Some(&quote @ ('"' | '\'' | '`')) = stack.last() {
            match ch {
                '\\' => {
                    chars.next();
                }
                '#' if quote != '\'' && chars.peek() == Some(&'{') => {
                    chars.next();
                    stack.push('#');
                }
                c if c == quote => {
                    stack.pop();
                }
                _ => {}
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' | '(' | '[' | '{' => stack.push(ch),
            ')' | ']' | '}' => {
                let expected = match ch {
                    ')' => '(',
                    ']' => '[',
                    _ if stack.last() == Some(&'#') => '#',
                    _ => '{',
                };
                if stack.pop() != Some(expected) {
                    return None;
                }
            }
            '#' => break,
            _ => {}
        }
    }
    stack
        .last()
        .map(|&open| if open == '#' { '{' } else { open })
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

fn check_explicit_end(
    code: &str,
    range: tower_lsp::lsp_types::Range,
    out: &mut Vec<Diagnostic>,
) {
    let (_, _, closes) = analyze(code);
    if closes > 0 {
        out.push(coded(
            error(range, "`end` is not allowed: Slim closes blocks by indentation"),
            code::EXPLICIT_END,
        ));
    }
}

/// Skip over `tag.class#id attr=val (...)`, validating bracket balance (possibly across lines).
/// Returns the remainder and lines consumed.
fn skip_tag_head(
    lines: &[&str],
    i: usize,
    text: &str,
    out: &mut Vec<Diagnostic>,
) -> (String, usize) {
    let raw = lines[i];
    let head_range = line_range(text, i, leading_ws(raw).len(), raw.len());
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
                    Err(open_ch) => {
                        out.push(coded(
                            error(
                                head_range,
                                format!("Unclosed `{}` in attribute value", open_ch),
                            ),
                            code::UNCLOSED_ATTRIBUTE,
                        ));
                        return (String::new(), consumed);
                    }
                }
                continue;
            }
            break;
        }
        if !opener {
            break;
        }
        if ch == '*' {
            pos += 1;
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
            out.push(coded(
                error(head_range, format!("Unclosed `{}` in tag attributes", ch)),
                code::UNCLOSED_ATTRIBUTE,
            ));
            return (String::new(), consumed);
        };
        pos += len;
    }
    (s[pos..].to_string(), consumed)
}

/// Byte length of the attribute name at the start of `s`.
fn attr_name_len(s: &str) -> usize {
    s.find(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '@' | '.')))
        .unwrap_or(s.len())
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

    /// (code, 0-based line, 0-based start column, severity) for each Slim diagnostic.
    fn slim(text: &str) -> Vec<(String, u32, u32, DiagnosticSeverity)> {
        slim_with(text, &[])
    }

    fn slim_with(
        text: &str,
        extra_engines: &[String],
    ) -> Vec<(String, u32, u32, DiagnosticSeverity)> {
        diagnostics(text, extra_engines)
            .into_iter()
            .map(|d| {
                let Some(NumberOrString::String(code)) = d.code else {
                    panic!("diagnostic without code: {}", d.message)
                };
                (
                    code,
                    d.range.start.line,
                    d.range.start.character,
                    d.severity.unwrap(),
                )
            })
            .collect()
    }

    const ERROR: DiagnosticSeverity = DiagnosticSeverity::ERROR;
    const WARNING: DiagnosticSeverity = DiagnosticSeverity::WARNING;

    fn one(
        code: &str,
        line: u32,
        col: u32,
        sev: DiagnosticSeverity,
    ) -> Vec<(String, u32, u32, DiagnosticSeverity)> {
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
    fn inconsistent_indent() {
        assert_eq!(
            slim("div\n  p\n  p\n    a\n  ul\n   li\n"),
            one(code::INCONSISTENT_INDENT, 5, 0, WARNING)
        );
    }

    #[test]
    fn bad_dedent() {
        assert_eq!(
            slim("div\n  p\n    a\n   span\n"),
            one(code::BAD_DEDENT, 3, 0, ERROR)
        );
    }

    #[test]
    fn top_level_indent() {
        assert_eq!(slim("  p\n"), one(code::TOP_LEVEL_INDENT, 0, 0, ERROR));
    }

    #[test]
    fn doctype_children() {
        assert_eq!(
            slim("doctype html\n  p\n"),
            one(code::DOCTYPE_CHILDREN, 1, 2, ERROR)
        );
    }

    #[test]
    fn unclosed_attribute() {
        assert_eq!(
            slim("div\n  a href=(foo\n"),
            one(code::UNCLOSED_ATTRIBUTE, 1, 2, ERROR)
        );
    }

    #[test]
    fn explicit_end() {
        assert_eq!(
            slim("- items.each do\n  p\n- end\n"),
            one(code::EXPLICIT_END, 2, 0, ERROR)
        );
    }

    #[test]
    fn unknown_engine_and_extra_engines() {
        assert_eq!(
            slim("mjml:\n  x\n"),
            one(code::UNKNOWN_ENGINE, 0, 0, WARNING)
        );
        assert!(slim_with("mjml:\n  x\n", &["mjml".into()]).is_empty());
        assert!(slim("javascript:\n  x\n").is_empty());
    }

    #[test]
    fn unmatched_branch() {
        assert!(slim("- if a\n  p\n- elsif b\n  p\n- else\n  p\n").is_empty());
        assert!(slim("- case x\n- when 1\n  p\n- else\n  p\n").is_empty());
        assert!(slim("- begin\n  p\n- rescue => e\n  p\n- ensure\n  p\n").is_empty());
        assert_eq!(
            slim("p\n- else\n  p\n"),
            one(code::UNMATCHED_BRANCH, 1, 0, ERROR)
        );
        assert_eq!(
            slim("- if a\n  p\n  - else\n    q\n"),
            one(code::UNMATCHED_BRANCH, 2, 2, ERROR)
        );
        assert_eq!(
            slim("- if a\n  p\n- when 1\n"),
            one(code::UNMATCHED_BRANCH, 2, 0, ERROR)
        );
        assert_eq!(
            slim("- unless a\n  p\n- elsif b\n"),
            one(code::UNMATCHED_BRANCH, 2, 0, ERROR)
        );
        let message = &diagnostics("p\n- else\n", &[])[0].message;
        assert!(message.contains("or a block (`do`)"), "{message}");
    }

    #[test]
    fn code_comment_between_branches() {
        assert!(slim("- if x\n  p a\n/ note\n- else\n  p b\n").is_empty());
        assert!(slim("- case x\n- when 1\n  p 1\n/ two\n- when 2\n  p 2\n").is_empty());
        // `/!` renders an HTML comment, which ends the `if` in Slim's output.
        assert_eq!(
            slim("- if x\n  p a\n/! note\n- else\n"),
            one(code::UNMATCHED_BRANCH, 3, 0, ERROR)
        );
    }

    #[test]
    fn branches_nested_under_opener() {
        assert!(slim("- case x\n  - when 1\n    p one\n  - else\n    p other\n").is_empty());
        assert!(slim("- begin\n  - rescue\n    p\n").is_empty());
        assert_eq!(
            slim("- if a\n  - elsif b\n"),
            one(code::UNMATCHED_BRANCH, 1, 2, ERROR)
        );
    }

    #[test]
    fn branch_opener_after_assignment() {
        assert!(slim("- r = case x\n- when 1\n  p 1\n").is_empty());
        assert!(slim("- @r ||= case x\n- when 1\n").is_empty());
        assert_eq!(
            slim("- x == case y\n- when 1\n"),
            one(code::UNMATCHED_BRANCH, 1, 0, ERROR)
        );
    }

    #[test]
    fn void_children() {
        assert_eq!(
            slim("br\n  p x\n  p y\n"),
            one(code::VOID_CHILDREN, 1, 2, WARNING)
        );
        assert_eq!(
            slim("div\n  img/\n    p\n"),
            one(code::VOID_CHILDREN, 2, 4, WARNING)
        );
        assert_eq!(
            slim("input.big type=\"x\"\n  p\n"),
            one(code::VOID_CHILDREN, 1, 2, WARNING)
        );
        assert!(slim("br\n  / old stuff\n").is_empty());
        assert!(slim("p = x\n  span y\n").is_empty());
    }

    #[test]
    fn closed_tag_children() {
        let d = diagnostics("div/\n  p x\n", &[]);
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "`div` is explicitly closed with `/` and can't have children"
        );
        assert_eq!(d[0].severity, Some(WARNING));
    }

    #[test]
    fn non_ascii_text_after_tag() {
        assert!(slim("p Größe\np ü\nspan.x ä=1 b\n").is_empty());
    }

    #[test]
    fn silent_output_helper() {
        assert_eq!(
            slim("- form_with model: @u do |f|\n  p\n"),
            one(code::SILENT_OUTPUT_HELPER, 0, 2, WARNING)
        );
        assert_eq!(
            slim("-  tag.div do\n  p\n"),
            one(code::SILENT_OUTPUT_HELPER, 0, 3, WARNING)
        );
        assert!(slim("= form_with model: @u do |f|\n  p\n").is_empty());
        assert!(slim("- content_for :x do\n  p\n").is_empty());
    }

    #[test]
    fn unclosed_ruby() {
        assert_eq!(slim("= foo(\n"), one(code::UNCLOSED_RUBY, 0, 0, WARNING));
        assert_eq!(
            slim("p = link_to \"x, y\n"),
            one(code::UNCLOSED_RUBY, 0, 0, WARNING)
        );
        assert_eq!(
            slim("- x = [1,\n  2\n"),
            one(code::UNCLOSED_RUBY, 0, 0, WARNING)
        );
        assert!(
            slim("- x = [1,\n  2]\n= \"#{a ? \"(\" : '['}\" # (\n= t('.x') if ok?\n").is_empty()
        );
        assert_eq!(unclosed_delimiter("foo(\"#{bar"), Some('{'));
        assert_eq!(unclosed_delimiter("a)"), None);
        assert_eq!(unclosed_delimiter("total / count + (x"), Some('('));
        assert_eq!(unclosed_delimiter("a % b + (x"), Some('('));
    }

    #[test]
    fn unclosed_ruby_skips_literals_it_cannot_scan() {
        for code in [
            "= name.gsub(/'/, \"\")\n",
            "- if path =~ /\\(/\n  p x\n",
            "= t(\"x\", default: %q(it's))\n",
            "= foo(?()\n",
            "= $'\n",
            "= foo /(/\n",
        ] {
            assert!(slim(code).is_empty(), "{code}");
        }
    }

    #[test]
    fn broken_fixture() {
        let text = include_str!("../../test/broken/bad.html.slim");
        assert_eq!(
            slim(text),
            vec![
                (code::DOCTYPE_CHILDREN.to_string(), 1, 2, ERROR),
                (code::INCONSISTENT_INDENT.to_string(), 3, 0, WARNING),
                (code::BAD_DEDENT.to_string(), 4, 0, ERROR),
                (code::UNCLOSED_ATTRIBUTE.to_string(), 4, 2, ERROR),
                (code::EXPLICIT_END.to_string(), 7, 2, ERROR),
                (code::UNKNOWN_ENGINE.to_string(), 8, 2, WARNING),
                (code::BAD_DEDENT.to_string(), 10, 0, ERROR),
            ]
        );
    }
}
