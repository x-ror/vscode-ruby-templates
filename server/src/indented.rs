//! Line-based analysis shared by Slim and Haml: indentation structure, bracket
//! balance, embedded engines, and misuse of explicit `end`.

use crate::ruby::{analyze, ends_with_block, BlockEffect};
use crate::util::{error, leading_ws, line_range, warning};
use once_cell::sync::Lazy;
use regex::Regex;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang { Slim, Haml }

const SLIM_ENGINES: &[&str] = &["ruby", "javascript", "css", "sass", "scss", "less", "coffee", "markdown", "textile", "rdoc", "erb", "builder", "nokogiri", "wiki", "creole", "mediawiki", "asciidoc", "org", "styl", "stylus", "livescript", "babel", "typescript", "haml"];
/// The filters Haml 6 and 7 register (`Haml::Filters`).
const HAML_FILTERS: &[&str] = &["plain", "escaped", "preserve", "cdata", "javascript", "css", "ruby", "erb", "markdown", "sass", "scss", "less", "coffee", "coffeescript"];
/// Haml's `MID_BLOCK_KEYWORDS`: a `-` line starting with one continues the block above instead of closing it.
const MID_BLOCK_KEYWORDS: &[&str] = &["else", "elsif", "rescue", "ensure", "end", "when", "in"];
/// Haml's `BLOCK_KEYWORD_REGEX`, applied to the code after the `-`/`=` prefix.
static BLOCK_KEYWORD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*(?:(else|elsif|rescue|ensure|end|when|in)|(?:\w+(?:,\s*\w+)*\s*=\s*)?(if|begin|case|unless))\b").unwrap()
});

static BLOCK_WITH_SPACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"do\s*\|\s*[^|]*\s+\|$").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    /// Children are free-form text (filters, comments, text blocks).
    Verbatim,
    /// Ruby line `- ...` / `= ...`; children need it to open a block.
    Ruby { opens_block: bool },
    /// `inline_content`: Haml content on the tag's line, which rules out children.
    Tag { inline_content: bool },
    /// Haml `%br/`.
    SelfClosing,
    /// Haml `/ text`: an HTML comment with inline content.
    TextComment,
    Text,
    Doctype,
    Other,
}

/// A line the structure checks look at: not blank, not inside a verbatim block, not a continuation line.
struct Visit { line: usize, indent: usize, kind: LineKind }

struct Frame { indent: usize, kind: LineKind, line: usize, has_children: bool }

/// An open Haml `if`/`case`/`unless`, for matching `else`/`elsif`/`when`.
struct Opener { indent: usize, level: usize, seen_when: bool }

pub fn diagnostics(text: &str, lang: Lang) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let visits = visit_lines(lang, &lines, text, &mut out);
    check_structure(lang, &lines, &visits, text, &mut out);
    out.sort_by_key(|d| (d.range.start.line, d.range.start.character));
    out
}

/// Classifies each structural line, skipping verbatim bodies and the lines a continuation consumed.
fn visit_lines(lang: Lang, lines: &[&str], text: &str, out: &mut Vec<Diagnostic>) -> Vec<Visit> {
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
        let (kind, consumed) = classify(lang, lines, i, text, out);
        visits.push(Visit { line: i, indent, kind });
        i += consumed.max(1);
    }
    visits
}

/// Tabs count as two columns.
fn indent_width(ws: &str) -> usize {
    ws.chars().map(|c| if c == '\t' { 2 } else { 1 }).sum()
}

/// Indentation consistency, dedents, illegal nesting and Haml's `else`/`when` placement.
fn check_structure(lang: Lang, lines: &[&str], visits: &[Visit], text: &str, out: &mut Vec<Diagnostic>) {
    let mut stack: Vec<Frame> = Vec::new();
    let mut openers: Vec<Opener> = Vec::new();
    // Haml measures every line against the first indented one.
    let doc_indentation = visits.iter().map(|v| leading_ws(lines[v.line])).find(|ws| !ws.is_empty())
        .filter(|ws| lang == Lang::Haml && !(ws.contains('\t') && ws.contains(' ')));
    let unit = detect_unit(lines, visits, doc_indentation);

    for &Visit { line: line_no, indent, kind } in visits {
        let raw = lines[line_no];
        let ws = leading_ws(raw);
        let mut tab_space_mismatch = false;
        if ws.contains('\t') && ws.contains(' ') {
            out.push(error(line_range(text, line_no, 0, ws.len()), "Mixed tabs and spaces in indentation"));
        } else if let Some(doc_ws) = doc_indentation.filter(|d| !ws.is_empty() && ws.contains('\t') != d.contains('\t')) {
            tab_space_mismatch = true;
            out.push(error(line_range(text, line_no, 0, ws.len()), format!(
                "Inconsistent indentation: {} used for indentation, but the rest of the document was indented using {}.",
                human_indentation(ws), human_indentation(doc_ws))));
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

        if is_child && !bad_dedent && !tab_space_mismatch {
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

        // Illegal nesting checks against the parent, reported once on the parent line.
        if let Some(parent) = stack.last_mut().filter(|_| is_child) {
            let parent_raw = lines[parent.line];
            let nesting_error = match parent.kind {
                _ if parent.has_children => None,
                LineKind::Tag { inline_content: true } if lang == Lang::Haml => Some(format!(
                    "Illegal nesting: content can't be both given on the same line as %{} and nested within it.",
                    haml_tag_name(&parent_raw[leading_ws(parent_raw).len()..]))),
                LineKind::Text if lang == Lang::Haml => Some("Illegal nesting: nesting within plain text is illegal.".into()),
                LineKind::Ruby { opens_block: false } if lang == Lang::Haml =>
                    Some("Nested content under Ruby code that doesn't open a block (`do`, `if`, ...).".into()),
                LineKind::SelfClosing => Some("Illegal nesting: nesting within a self-closing tag is illegal.".into()),
                LineKind::TextComment => Some("Illegal nesting: nesting within a tag that already has content is illegal.".into()),
                _ => None,
            };
            if let Some(message) = nesting_error {
                out.push(error(line_range(text, parent.line, 0, parent_raw.len()), message));
            }
            if parent.kind == LineKind::Doctype {
                out.push(error(line_range(text, line_no, 0, raw.len()), "Nothing can be nested under a doctype"));
            }
            parent.has_children = true;
        }

        if lang == Lang::Haml {
            if let Some(message) = track_block_keyword(&mut openers, &raw[ws.len()..], indent, stack.len()) {
                out.push(error(line_range(text, line_no, ws.len(), raw.len()), message));
            }
        }

        stack.push(Frame { indent, kind, line: line_no, has_children: false });
    }
}

/// Element name in Haml's messages: the `%name`, or `div` for `.class` / `#id`.
fn haml_tag_name(content: &str) -> &str {
    let Some(rest) = content.strip_prefix('%') else { return "div" };
    &rest[..rest.find(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '-' | '_' | ':'))).unwrap_or(rest.len())]
}

/// Haml's wording for an indentation string: "2 spaces", "1 tab".
fn human_indentation(ws: &str) -> String {
    let noun = if ws.contains('\t') { "tab" } else { "space" };
    let plural = if ws.len() == 1 { "" } else { "s" };
    format!("{} {noun}{plural}", ws.len())
}

/// Haml's `if`/`case`/`unless` stack: pops the blocks this line closes, pushes the one it opens,
/// and returns Haml's error for an `else`/`elsif`/`when` with no opener at its level.
fn track_block_keyword(openers: &mut Vec<Opener>, content: &str, indent: usize, level: usize) -> Option<String> {
    let silent_code = content.strip_prefix('-').filter(|code| !code.starts_with('#'));
    let keyword = silent_code.or_else(|| strip_output_prefix(Lang::Haml, content)).and_then(block_keyword);
    let continues = silent_code.is_some() && keyword.is_some_and(|k| MID_BLOCK_KEYWORDS.contains(&k));
    while openers.last().is_some_and(|o| o.indent > indent || (o.indent == indent && !continues)) {
        openers.pop();
    }
    match keyword? {
        "if" | "case" | "unless" => {
            openers.push(Opener { indent, level, seen_when: false });
            None
        }
        k @ ("else" | "elsif" | "when") if continues => {
            let Some(top) = openers.last_mut() else { return Some(format!("Got \"{k}\" with no preceding \"if\"")) };
            // Haml lets the first `when` sit one level deeper than its `case`.
            if k == "when" && !top.seen_when {
                if top.level + 1 == level { top.level = level; }
                top.seen_when = true;
            }
            (top.level != level).then(|| format!("\"{k}\" is indented at wrong level: expected {}, but was at {level}.", top.level))
        }
        _ => None,
    }
}

fn block_keyword(code: &str) -> Option<&str> {
    let caps = BLOCK_KEYWORD.captures(code)?;
    caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str())
}

/// Most common positive indentation step between a line and its parent: the file's indent unit.
/// Lines indented with the other character than `doc_indentation` are left out; they get their own error.
fn detect_unit(lines: &[&str], visits: &[Visit], doc_indentation: Option<&str>) -> Option<usize> {
    let mut levels: Vec<usize> = vec![0];
    let mut counts: std::collections::BTreeMap<usize, usize> = Default::default();
    for &Visit { line, indent, .. } in visits {
        let ws = leading_ws(lines[line]);
        if doc_indentation.is_some_and(|d| !ws.is_empty() && ws.contains('\t') != d.contains('\t')) { continue; }
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
    let c = &raw[ws_len..];

    // Doctype
    if (lang == Lang::Haml && c.starts_with("!!!")) || (lang == Lang::Slim && c.starts_with("doctype")) {
        return (LineKind::Doctype, 1);
    }
    // Comments
    if lang == Lang::Haml && c.starts_with("-#") {
        return (LineKind::Verbatim, 1);
    }
    if lang == Lang::Slim && c.starts_with('/') {
        return (LineKind::Verbatim, 1);
    }
    if lang == Lang::Haml {
        if let Some(comment) = c.strip_prefix('/') {
            // Conditional comments `/[if IE]` and `/![if !IE]` may have text after the `]`.
            let conditional = comment.strip_prefix('!').unwrap_or(comment).strip_prefix('[');
            let text_after = conditional.map_or(comment, |cond| cond.split_once(']').map_or("", |(_, rest)| rest));
            return (if text_after.trim().is_empty() { LineKind::Other } else { LineKind::TextComment }, 1);
        }
        if (c.starts_with('.') || (c.starts_with('#') && !c.starts_with("#{")))
            && !c[1..].starts_with(|ch: char| ch.is_alphanumeric() || matches!(ch, '-' | '_'))
        {
            out.push(error(line_range(text, i, ws_len, raw.len()), "Illegal element: classes and ids must have values."));
            return (LineKind::Other, 1);
        }
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
    if let Some(code) = c.strip_prefix('-').or_else(|| strip_output_prefix(lang, c)) {
        let (joined, consumed) = join_continuation(lang, lines, i, code);
        check_explicit_end(lang, &joined, text, i, raw, out);
        return (LineKind::Ruby { opens_block: opens_block(&joined) }, consumed);
    }
    // Tags
    let is_tag = match lang {
        Lang::Haml => c.starts_with(['%', '.']) || (c.starts_with('#') && !c.starts_with("#{")),
        Lang::Slim => c.starts_with('.') || c.starts_with('#') || c.chars().next().map(|ch| ch.is_ascii_alphabetic()).unwrap_or(false),
    };
    if is_tag {
        let (rest, head_lines) = skip_tag_head(lang, lines, i, text, out);
        if lang == Lang::Haml {
            if let Some(content) = rest.trim_start_matches(['<', '>']).strip_prefix('/') {
                if !content.trim().is_empty() {
                    out.push(error(line_range(text, i, ws_len, raw.len()), "Self-closing tags can't have content."));
                }
                return (LineKind::SelfClosing, head_lines);
            }
        }
        let rest_t = rest.trim();
        let last_head_line = i + head_lines - 1;
        // Haml allows children after content only when the line's value ends in `,` (`is_ruby_multiline?`).
        let value = rest.trim_start_matches(['<', '>']).trim();
        let inline_content = lang == Lang::Haml && !value.is_empty() && !is_ruby_multiline(value);
        let tail_lines = match strip_output_prefix(lang, rest_t) {
            Some(code) => {
                let (joined, code_lines) = join_continuation(lang, lines, last_head_line, code);
                check_explicit_end(lang, &joined, text, i, raw, out);
                code_lines
            }
            None => multiline_len(lang, lines, last_head_line),
        };
        return (LineKind::Tag { inline_content }, head_lines - 1 + tail_lines);
    }
    (LineKind::Text, multiline_len(lang, lines, i))
}

/// Haml's `is_ruby_multiline?`: ends in `,`, except the character literals `?,` and `?\,`.
fn is_ruby_multiline(text: &str) -> bool {
    let b = text.as_bytes();
    let n = b.len();
    if n < 2 || b[n - 1] != b',' {
        return false;
    }
    let char_literal = n >= 3 && ((b[n - 2] == b'?' && !(b[n - 3].is_ascii_alphanumeric() || b[n - 3] == b'_')) || (b[n - 3] == b'?' && b[n - 2] == b'\\'));
    !char_literal
}

/// Can nested template lines follow? Either the code opens a block or continues one (`else`, `when`, ...).
fn opens_block(code: &str) -> bool {
    ends_with_block(code) || matches!(analyze(code).0, BlockEffect::Open(_) | BlockEffect::Continue)
}

fn strip_output_prefix(lang: Lang, s: &str) -> Option<&str> {
    match lang {
        // `==`, `&==`, `!==` are interpolated plain text, not Ruby.
        Lang::Haml if ["==", "&==", "!=="].iter().any(|p| s.starts_with(p)) => None,
        Lang::Haml => ["!=", "&=", "!~", "&~", "=", "~"].iter().find_map(|p| s.strip_prefix(p)),
        Lang::Slim => {
            let t = s.strip_prefix("==").or_else(|| s.strip_prefix('='))?;
            Some(t.trim_start_matches(['<', '>']))
        }
    }
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
    let is_tag = content.starts_with(['%', '.']) || (content.starts_with('#') && !content.starts_with("#{"));
    let code_line = if is_tag { content[tag_head_len(content)?..].trim_start_matches(['<', '>']) } else { content };
    strip_output_prefix(Lang::Haml, code_line).map(|code| line.len() - code.len())
}

/// Length of `%tag.class#id{...}(...)[...]` on one line; `None` when a bracket isn't closed on it.
fn tag_head_len(s: &str) -> Option<usize> {
    let mut pos = s.find(|ch: char| !is_tag_name_char(ch)).unwrap_or(s.len());
    loop {
        let open = s[pos..].chars().next();
        let close = match open {
            Some('{') => '}',
            Some('(') => ')',
            Some('[') => ']',
            _ => return Some(pos),
        };
        pos += find_balanced(&s[pos..], open?, close)?;
    }
}

fn is_tag_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '%' | '.' | '#' | '-' | '_' | ':')
}

/// Haml's `is_multiline?`: ends in ` |`, unless it's a block argument list (`do | x |`).
fn is_haml_multiline(line: &str) -> bool {
    let t = line.trim_end();
    t.ends_with(" |") && !BLOCK_WITH_SPACES.is_match(t)
}

/// Physical lines in the Haml `|` multiline group starting at line `i`, blank lines inside included; 1 if none.
fn multiline_len(lang: Lang, lines: &[&str], i: usize) -> usize {
    if lang != Lang::Haml || !is_haml_multiline(lines[i]) {
        return 1;
    }
    let mut len = 1;
    for (offset, next) in lines.iter().enumerate().skip(i + 1) {
        if next.trim().is_empty() { continue; }
        if !is_haml_multiline(next) { break; }
        len = offset - i + 1;
    }
    len
}

/// Join Ruby continuation lines (`,` or `\` at end, Haml `|` multiline).
fn join_continuation(lang: Lang, lines: &[&str], i: usize, first: &str) -> (String, usize) {
    let group = multiline_len(lang, lines, i);
    if group > 1 {
        let strip_pipe = |s: &str| s.trim().trim_end_matches('|').trim_end().to_string();
        let rest = lines[i + 1..i + group].iter().map(|l| strip_pipe(l));
        let code = std::iter::once(strip_pipe(first)).chain(rest).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
        return (code, group);
    }
    let mut consumed = 1;
    let mut code = first.trim_end().to_string();
    while is_ruby_multiline(&code) || code.ends_with('\\') {
        let Some(next) = lines.get(i + consumed) else { break };
        code = format!("{} {}", code.trim_end_matches('\\'), next.trim());
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
fn skip_tag_head(lang: Lang, lines: &[&str], i: usize, text: &str, out: &mut Vec<Diagnostic>) -> (String, usize) {
    let raw = lines[i];
    let mut consumed = 1;
    let mut s: String = raw[leading_ws(raw).len()..].to_string();
    let mut pos = 0usize;
    let bytes = |s: &String| s.as_bytes().to_vec();
    // tag name / classes / ids
    let b = bytes(&s);
    while pos < b.len() {
        let ch = b[pos] as char;
        if is_tag_name_char(ch) { pos += 1; } else { break; }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// (line, start char, end char, message) for each Haml diagnostic.
    fn haml(text: &str) -> Vec<(u32, u32, u32, String)> {
        diagnostics(text, Lang::Haml).into_iter()
            .map(|d| (d.range.start.line, d.range.start.character, d.range.end.character, d.message))
            .collect()
    }

    fn at(line: u32, start: u32, end: u32, message: &str) -> (u32, u32, u32, String) {
        (line, start, end, message.to_string())
    }

    #[test]
    fn broken_fixture() {
        let got = haml(include_str!("../../test/broken/bad.html.haml"));
        assert_eq!(got, vec![
            at(2, 0, 3, "Inconsistent indentation: expected 2 spaces, found 3"),
            at(3, 0, 2, "Indentation does not match any outer level"),
            at(3, 0, 12, "Illegal nesting: content can't be both given on the same line as %span and nested within it."),
            at(7, 2, 7, "`end` is not allowed: Haml closes blocks by indentation"),
            at(8, 2, 16, "Unknown Haml filter `:unknownfilter`"),
            at(10, 2, 8, "Got \"else\" with no preceding \"if\""),
            at(12, 2, 11, "Self-closing tags can't have content."),
            at(13, 0, 8, "Illegal nesting: nesting within a tag that already has content is illegal."),
            at(15, 0, 13, "Nested content under Ruby code that doesn't open a block (`do`, `if`, ...)."),
            at(17, 2, 14, "Unclosed `{` in tag attributes"),
        ]);
    }

    #[test]
    fn messages_and_ranges() {
        let cases: &[(&str, (u32, u32, u32, &str))] = &[
            ("%div\n  %p\n    %b\n   %i", (3, 0, 3, "Indentation does not match any outer level")),
            (" %p", (0, 0, 1, "Unexpected indentation at top level")),
            ("%p\n \t%b", (1, 0, 2, "Mixed tabs and spaces in indentation")),
            ("%div\n\t%p x\n%div\n  %p y", (3, 0, 2, "Inconsistent indentation: 2 spaces used for indentation, but the rest of the document was indented using 1 tab.")),
            ("%div\n    %p x\n%div\n\t%p y", (3, 0, 1, "Inconsistent indentation: 1 tab used for indentation, but the rest of the document was indented using 4 spaces.")),
            ("!!! 5\n  %p", (1, 0, 4, "Nothing can be nested under a doctype")),
            ("hello\n  %b", (0, 0, 5, "Illegal nesting: nesting within plain text is illegal.")),
            ("#{x} text\n  %b", (0, 0, 9, "Illegal nesting: nesting within plain text is illegal.")),
            ("#{x}\n  %b", (0, 0, 4, "Illegal nesting: nesting within plain text is illegal.")),
            ("== hi #{x}\n  %b", (0, 0, 10, "Illegal nesting: nesting within plain text is illegal.")),
            ("%p== hi #{x}\n  %b", (0, 0, 12, "Illegal nesting: content can't be both given on the same line as %p and nested within it.")),
            ("%p= form_with model: @u do |f|\n  = f.text_field :name", (0, 0, 30, "Illegal nesting: content can't be both given on the same line as %p and nested within it.")),
            (".user= x\n  %p", (0, 0, 8, "Illegal nesting: content can't be both given on the same line as %div and nested within it.")),
            ("%p= x ?,\n  %b", (0, 0, 8, "Illegal nesting: content can't be both given on the same line as %p and nested within it.")),
            ("/[if IE] text\n  %p", (0, 0, 13, "Illegal nesting: nesting within a tag that already has content is illegal.")),
            ("# heading", (0, 0, 9, "Illegal element: classes and ids must have values.")),
            ("...and more", (0, 0, 11, "Illegal element: classes and ids must have values.")),
            ("- x = 1\n  %p", (0, 0, 7, "Nested content under Ruby code that doesn't open a block (`do`, `if`, ...).")),
            ("&~ x\n  %p", (0, 0, 4, "Nested content under Ruby code that doesn't open a block (`do`, `if`, ...).")),
            ("~ x\n  %p", (0, 0, 3, "Nested content under Ruby code that doesn't open a block (`do`, `if`, ...).")),
            ("%br/ text", (0, 0, 9, "Self-closing tags can't have content.")),
            ("%br/\n  %p", (0, 0, 4, "Illegal nesting: nesting within a self-closing tag is illegal.")),
            ("/ note\n  %p", (0, 0, 6, "Illegal nesting: nesting within a tag that already has content is illegal.")),
            ("%div\n  - elsif x", (1, 2, 11, "Got \"elsif\" with no preceding \"if\"")),
            ("- when 1", (0, 0, 8, "Got \"when\" with no preceding \"if\"")),
            ("- if a\n  %p\n-# note\n- else", (3, 0, 6, "Got \"else\" with no preceding \"if\"")),
            ("%p= if a\n- else", (1, 0, 6, "Got \"else\" with no preceding \"if\"")),
            ("- if a\n  - else", (1, 2, 8, "\"else\" is indented at wrong level: expected 0, but was at 1.")),
            (":textile\n  x", (0, 0, 8, "Unknown Haml filter `:textile`")),
        ];
        for (src, (line, start, end, message)) in cases {
            assert_eq!(haml(src), vec![at(*line, *start, *end, message)], "{src:?}");
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
            "%br\n  %p x",
            "/\n  %p x",
            "/[if IE]\n  %p x",
            "%p Dear Sir,\n  %b x",
            "%p= foo(1,\n  2) do |x|\n  %b",
            "/![if !IE]\n  %p x",
            "!~ capture do\n  %p",
            "%p&~ \"x\"",
            "#{a} b",
            ".9a x",
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
