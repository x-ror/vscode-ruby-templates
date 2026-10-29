//! Folding ranges and document symbols for Slim (and Skim): indentation blocks over logical lines.

use crate::outline::{erb_symbol, indent_level, nest, symbol, ID_ATTR, LANDMARKS};
use crate::partials::render_refs;
use crate::slim_parser::SLIM_ENGINES;
use crate::util::LineIndex;
use regex::Regex;
use std::sync::LazyLock;
use tower_lsp::lsp_types::{DocumentSymbol, FoldingRange, FoldingRangeKind, SymbolKind};

static BRANCH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:if|unless|elsif|else|case|when)\b").unwrap());

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verbatim {
    Text,
    Comment,
    Engine,
}

/// One Slim line, or several when wrapped attributes or a `,`/`\` Ruby continuation span lines.
struct Entry {
    first: usize,
    last: usize,
    indent: i64,
    /// Byte offset of the first non-blank character.
    start: usize,
    /// Set for headers whose deeper lines are text, comment or engine content rather than Slim.
    verbatim: Option<Verbatim>,
    /// Byte ranges of `tag.class#id`, more than one for inline tags (`li: a.x`).
    heads: Vec<(usize, usize)>,
    /// Byte offset where Ruby code (`- x`, `= x`, `tag = x`) starts.
    code: Option<usize>,
    control: bool,
}

impl Entry {
    fn plain(line: usize, indent: i64, start: usize) -> Self {
        Entry { first: line, last: line, indent, start, verbatim: None, heads: Vec::new(), code: None, control: false }
    }
}

/// What follows a tag's head and attributes.
enum Tail {
    Nothing,
    Code(usize),
    Text,
}

/// (byte offset, content without `\r`) per line.
fn line_spans(text: &str) -> Vec<(usize, &str)> {
    let mut offset = 0;
    text.split('\n')
        .map(|l| {
            let span = (offset, l.strip_suffix('\r').unwrap_or(l));
            offset += l.len() + 1;
            span
        })
        .collect()
}

fn line_end(lines: &[(usize, &str)], i: usize) -> usize {
    lines[i].0 + lines[i].1.len()
}

/// Characters outside string literals with the bracket depth after each.
fn depths(s: &str) -> impl Iterator<Item = (usize, char, i32)> + '_ {
    let (mut depth, mut quote, mut escaped) = (0, None, false);
    s.char_indices().filter_map(move |(i, c)| {
        if let Some(q) = quote {
            if escaped {
                escaped = false
            } else if c == '\\' {
                escaped = true
            } else if c == q {
                quote = None
            }
            return None;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                return None;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        Some((i, c, depth))
    })
}

/// Length of the bracket group `s` starts with, `None` while it is unclosed.
fn group_len(s: &str) -> Option<usize> {
    depths(s).find(|&(_, _, d)| d == 0).map(|(i, c, _)| i + c.len_utf8())
}

/// Length of an attribute value: up to the first space outside brackets and strings.
fn value_len(s: &str) -> usize {
    depths(s).find(|&(_, c, d)| c.is_whitespace() && d <= 0).map_or(s.len(), |(i, ..)| i)
}

/// `tag`, `.class`, `#id`, `svg:use` and combinations; 0 when `c` doesn't start with a tag.
fn tag_head_len(c: &str) -> usize {
    let word = |s: &str| s.find(|ch: char| !(ch.is_alphanumeric() || ch == '-' || ch == '_')).unwrap_or(s.len());
    if !c.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '.' || ch == '#') {
        return 0;
    }
    let mut pos = word(c);
    while let Some(sep) = c[pos..].chars().next().filter(|ch| matches!(ch, '.' | '#' | ':')) {
        let n = word(&c[pos + 1..]);
        if n == 0 {
            break;
        }
        pos += sep.len_utf8() + n;
    }
    pos
}

fn attr_name_len(s: &str) -> usize {
    let n = s.find(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '@' | '.'))).unwrap_or(s.len());
    if n > 0 && s[n..].starts_with('=') {
        n
    } else {
        0
    }
}

/// The engine of an embedded engine header; Slim's `@embedded_re` is the engine, optional attributes,
/// `:`, then anything.
fn engine_name<'a>(c: &'a str, extra_engines: &[String]) -> Option<&'a str> {
    let len = c.find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-')).unwrap_or(c.len());
    let (word, rest) = c.split_at(len);
    let known = SLIM_ENGINES.contains(&word) || extra_engines.iter().any(|e| e == word);
    let header = rest.starts_with(':') || (rest.starts_with(char::is_whitespace) && rest.contains(':'));
    (known && header).then_some(word)
}

fn verbatim_kind(c: &str, extra_engines: &[String]) -> Option<Verbatim> {
    // `/[if IE]` conditional comments wrap markup.
    if c.starts_with("/[") {
        return None;
    }
    if c.starts_with('/') {
        return Some(Verbatim::Comment);
    }
    if c.starts_with(['|', '\'', '>']) {
        return Some(Verbatim::Text);
    }
    engine_name(c, extra_engines).map(|_| Verbatim::Engine)
}

/// Offset after `=`/`==` and their whitespace modifiers, when `c` (at `offset`) is an output line.
fn output_code(c: &str, offset: usize) -> Option<usize> {
    let rest = c.strip_prefix('=')?;
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start_matches(['<', '>', '\'']);
    Some(offset + c.len() - rest.len())
}

fn indent_at(lines: &[(usize, &str)], i: usize) -> Option<i64> {
    lines.get(i).and_then(|l| indent_level(l.1))
}

/// Skip a tag's head, attribute wrappers (across lines) and `name=value` attributes, following `tag: tag`
/// nesting, and classify what is left.
fn tag_tail(text: &str, lines: &[(usize, &str)], e: &mut Entry, mut pos: usize) -> Tail {
    loop {
        let rest = &text[pos..line_end(lines, e.last)];
        let Some(first) = rest.chars().next() else {
            return Tail::Nothing;
        };
        match first {
            '(' | '[' | '{' => {
                // An unclosed wrapper would swallow the rest of the file; leave it on its own line instead.
                let mut last = e.last;
                let len = loop {
                    if let Some(n) = group_len(&text[pos..line_end(lines, last)]) {
                        break n;
                    }
                    if !indent_at(lines, last + 1).is_some_and(|d| d >= e.indent) {
                        return Tail::Nothing;
                    }
                    last += 1;
                };
                e.last = last;
                pos += len;
            }
            '<' | '>' => pos += 1,
            '*' => pos += 1 + value_len(&rest[1..]),
            ':' => {
                let inner = rest[1..].trim_start();
                let at = pos + rest.len() - inner.len();
                let head = tag_head_len(inner);
                if head == 0 {
                    return output_code(inner, at).map_or(Tail::Nothing, Tail::Code);
                }
                e.heads.push((at, at + head));
                pos = at + head;
            }
            c if c.is_whitespace() => {
                let t = rest.trim_start();
                let at = pos + rest.len() - t.len();
                let name = attr_name_len(t);
                if name > 0 {
                    let value = t[name..].trim_start_matches('=');
                    pos = at + t.len() - value.len() + value_len(value);
                    continue;
                }
                return match output_code(t, at) {
                    Some(code) => Tail::Code(code),
                    None if t.is_empty() => Tail::Nothing,
                    None => Tail::Text,
                };
            }
            '=' => return output_code(rest, pos).map_or(Tail::Nothing, Tail::Code),
            _ => return Tail::Nothing,
        }
    }
}

fn parse(text: &str, lines: &[(usize, &str)], i: usize, indent: i64, start: usize, extra_engines: &[String]) -> Entry {
    let mut e = Entry::plain(i, indent, start);
    let c = &text[start..line_end(lines, i)];
    e.verbatim = verbatim_kind(c, extra_engines);
    if e.verbatim.is_some() || c.starts_with('<') || c.starts_with("doctype") {
        return e;
    }
    if c.starts_with('-') {
        e.code = Some(start + 1);
        e.control = true;
    } else if let Some(code) = output_code(c, start) {
        e.code = Some(code);
    } else {
        let head = tag_head_len(c);
        if head == 0 {
            return e;
        }
        e.heads.push((start, start + head));
        match tag_tail(text, lines, &mut e, start + head) {
            Tail::Code(code) => e.code = Some(code),
            // Lines nested under a tag with inline text continue that text.
            Tail::Text => e.verbatim = Some(Verbatim::Text),
            Tail::Nothing => {}
        }
    }
    // Slim's broken lines: a trailing `,` or `\` continues the code on the next, deeper line.
    if let Some(code) = e.code {
        loop {
            let so_far = text[code..line_end(lines, e.last)].trim_end();
            let open = so_far.ends_with(',') || so_far.ends_with('\\');
            if !open || !indent_at(lines, e.last + 1).is_some_and(|d| d > indent) {
                break;
            }
            e.last += 1;
        }
    }
    e
}

fn entries(text: &str, lines: &[(usize, &str)], extra_engines: &[String]) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut verbatim_indent: Option<i64> = None;
    let mut i = 0;
    while i < lines.len() {
        let (offset, line) = lines[i];
        let Some(indent) = indent_level(line) else {
            i += 1;
            continue;
        };
        let start = offset + line.len() - line.trim_start().len();
        if verbatim_indent.is_some_and(|v| indent > v) {
            out.push(Entry::plain(i, indent, start));
            i += 1;
            continue;
        }
        let entry = parse(text, lines, i, indent, start, extra_engines);
        verbatim_indent = entry.verbatim.map(|_| indent);
        i = entry.last + 1;
        out.push(entry);
    }
    out
}

/// Byte ranges of Ruby code (`- x`, `= x`, `tag = x`, broken lines included); text, comments and engine
/// content are left out.
pub fn code_spans(text: &str, extra_engines: &[String]) -> Vec<(usize, usize)> {
    let lines = line_spans(text);
    entries(text, &lines, extra_engines).iter().filter_map(|e| Some((e.code?, line_end(&lines, e.last)))).collect()
}

/// Last line of each entry's block: itself plus every following deeper entry.
fn block_ends(entries: &[Entry]) -> Vec<usize> {
    let mut ends: Vec<usize> = entries.iter().map(|e| e.last).collect();
    let mut open: Vec<usize> = Vec::new();
    for (k, e) in entries.iter().enumerate() {
        while open.last().is_some_and(|&top| entries[top].indent >= e.indent) {
            open.pop();
        }
        for &parent in &open {
            ends[parent] = e.last;
        }
        open.push(k);
    }
    ends
}

/// Offside indentation folds (trailing blank lines stay outside), with each logical line folded as one unit.
pub fn folding_ranges(text: &str, extra_engines: &[String]) -> Vec<FoldingRange> {
    let lines = line_spans(text);
    let entries = entries(text, &lines, extra_engines);
    let ends = block_ends(&entries);
    entries
        .iter()
        .zip(ends)
        .filter(|(e, end)| *end > e.first)
        .map(|(e, end)| FoldingRange {
            start_line: e.first as u32,
            end_line: end as u32,
            kind: (e.verbatim == Some(Verbatim::Comment)).then_some(FoldingRangeKind::Comment),
            ..Default::default()
        })
        .collect()
}

/// Outline: tags with `#id`/`.class` or landmark names, `content_for`, forms, Turbo Frames, `render`,
/// `- if`/`case` branches and embedded engines, nested by indentation.
pub fn document_symbols(text: &str, extra_engines: &[String]) -> Vec<DocumentSymbol> {
    let lines = line_spans(text);
    let entries = entries(text, &lines, extra_engines);
    let ends = block_ends(&entries);
    let mut items = Vec::new();
    let index = LineIndex::new(text);
    let mut add = |name: String, kind, range: (usize, usize), selection: (usize, usize)| {
        items.push((range.0, range.1, symbol(&index, name, kind, range, selection)));
    };
    for (e, end) in entries.iter().zip(ends) {
        let block = (e.start, line_end(&lines, end));
        let first_line = (e.start, line_end(&lines, e.first));
        if e.verbatim == Some(Verbatim::Engine) {
            let engine = engine_name(&text[e.start..], extra_engines).unwrap_or_default();
            add(format!("{engine}:"), SymbolKind::PACKAGE, block, first_line);
        }
        let tail_end = e.code.unwrap_or_else(|| line_end(&lines, e.last));
        let attr_ends = e.heads.iter().skip(1).map(|h| h.0).chain(std::iter::once(tail_end));
        for (&(head_start, head_end), attrs_end) in e.heads.iter().zip(attr_ends) {
            let head = &text[head_start..head_end];
            let id = ID_ATTR
                .captures(&text[head_end..attrs_end.max(head_end)])
                .and_then(|c| c.iter().skip(1).flatten().next())
                .map(|m| m.as_str());
            let name = match id {
                _ if head.contains(['.', '#']) => Some(head.to_string()),
                Some(id) => Some(format!("{head}#{id}")),
                None => LANDMARKS.contains(&head).then(|| head.to_string()),
            };
            if let Some(name) = name {
                add(name, SymbolKind::FIELD, block, (head_start, head_end));
            }
        }
        let Some(code_start) = e.code else { continue };
        let code = &text[code_start..line_end(&lines, e.last)];
        for r in render_refs(code) {
            let name_range = (code_start + r.start, code_start + r.end);
            add(format!("render \"{}\"", r.name), SymbolKind::FILE, name_range, name_range);
        }
        let branch =
            || (e.control && BRANCH.is_match(code)).then(|| (code.split_whitespace().collect::<Vec<_>>().join(" "), SymbolKind::OPERATOR));
        if let Some((name, kind)) = erb_symbol(code).or_else(branch) {
            add(name, kind, block, first_line);
        }
    }
    nest(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::{Position, Range};

    fn folds(text: &str) -> Vec<(u32, u32)> {
        folding_ranges(text, &[]).into_iter().map(|r| (r.start_line, r.end_line)).collect()
    }

    fn outline(syms: &[DocumentSymbol]) -> Vec<String> {
        syms.iter()
            .map(|s| match &s.children {
                Some(c) => format!("{}[{}]", s.name, outline(c).join(",")),
                None => s.name.clone(),
            })
            .collect()
    }

    fn symbols(text: &str) -> Vec<String> {
        outline(&document_symbols(text, &[]))
    }

    #[test]
    fn wrapped_attributes_and_continuations_fold_as_one_line() {
        let text = concat!(
            "div(\n",                      // 0
            "class=\"x\")\n",              // 1
            "  p a\n",                     // 2
            "= form_with model: @user,\n", // 3
            "    class: \"f\" do |f|\n",   // 4
            "  = f.text_field :name\n",    // 5
            "\n",                          // 6
            "input[type=\"x\"\n",          // 7
            "  name=\"y\"]\n",             // 8
            "- items = [1,\n",             // 9
            "    2]\n",                    // 10
            "= link_to \"x\", \\\n",       // 11
            "  y\n",                       // 12
        );
        assert_eq!(folds(text), [(0, 2), (3, 5), (7, 8), (9, 10), (11, 12)]);
    }

    #[test]
    fn only_trailing_comma_or_backslash_continues_code() {
        let text = concat!(
            "- # deprecated (see below\n", // 0
            "header\n",                    // 1
            "  p\n",                       // 2
            "= f(a,\n",                    // 3
            "b)\n",                        // 4
            "  p\n",                       // 5
        );
        assert_eq!(folds(text), [(1, 2), (4, 5)]);
        assert_eq!(symbols(text), ["header"]);
    }

    #[test]
    fn unclosed_wrapper_stays_on_its_line() {
        assert_eq!(folds("a(href=\"/\"\n  span#x\nnav#n\n  p"), [(0, 1), (2, 3)]);
        assert_eq!(symbols("a(href=\"/\"\n  span#x\nnav#n\n  p"), ["span#x", "nav#n"]);
    }

    #[test]
    fn verbatim_content_is_not_parsed_as_slim() {
        let text = concat!(
            "javascript:\n", // 0
            "  f(a,\n",      // 1
            "    b);\n",     // 2
            "  g(\n",        // 3
            "/ note (\n",    // 4
            "  more\n",      // 5
            "p\n",           // 6
            "  | x\n",       // 7
        );
        let ranges = folding_ranges(text, &[]);
        let got: Vec<_> = ranges.iter().map(|r| (r.start_line, r.end_line, r.kind.clone())).collect();
        assert_eq!(got, [(0, 3, None), (1, 2, None), (4, 5, Some(FoldingRangeKind::Comment)), (6, 7, None)]);
    }

    #[test]
    fn engine_headers_follow_slim_embedded_re() {
        assert_eq!(symbols("css: \n  .btn { color: red }\nnav\n  a"), ["css:", "nav"]);
        assert_eq!(symbols("javascript: init()\n  .x\n"), ["javascript:"]);
        assert_eq!(symbols("javascript nonce=n:\n  .x\n"), ["javascript:"]);
        assert_eq!(symbols("mjml:\n  .x\n"), [".x"]);
        let extra = ["mjml".to_string()];
        assert_eq!(outline(&document_symbols("mjml:\n  .x\n", &extra)), ["mjml:"]);
        assert_eq!(symbols("css.theme\n  .x\n"), ["css.theme[.x]"]);
    }

    #[test]
    fn lines_under_inline_text_are_text() {
        assert_eq!(symbols("p Ranked list\n  #1 pick\n  .gitignore matters\n"), Vec::<String>::new());
        assert_eq!(symbols("li: a.y Text\n  .z\n"), ["a.y"]);
        assert_eq!(symbols("nav = x\n  .z\n"), ["nav[.z]"]);
    }

    #[test]
    fn conditional_comment_children_are_markup() {
        assert_eq!(symbols("/[if IE]\n  nav#old\n"), ["nav#old"]);
        assert_eq!(symbols("/! html\n  nav#old\n"), Vec::<String>::new());
    }

    #[test]
    fn inline_tags_each_get_a_symbol() {
        let syms = document_symbols("ul\n  li: a#x href=\"/\" = render \"a\"\n  li: nav: a id=\"y\"\n", &[]);
        assert_eq!(outline(&syms), ["a#x[render \"a\"]", "nav[a#y]"]);
        assert_eq!(syms[0].selection_range, Range::new(Position::new(1, 6), Position::new(1, 9)));
    }

    #[test]
    fn every_indentation_block_of_the_sample_folds() {
        let text = include_str!("../../test/sample.html.slim");
        let lines = line_spans(text);
        let continuation: Vec<usize> = entries(text, &lines, &[]).iter().flat_map(|e| e.first + 1..=e.last).collect();
        let starts: Vec<u32> = folds(text).into_iter().map(|f| f.0).collect();
        let indents: Vec<(usize, i64)> = lines.iter().enumerate().filter_map(|(i, l)| Some((i, indent_level(l.1)?))).collect();
        for pair in indents.windows(2) {
            let ((line, indent), (_, next)) = (pair[0], pair[1]);
            if next > indent && !continuation.contains(&line) {
                assert!(starts.contains(&(line as u32)), "line {line} doesn't fold");
            }
        }
    }

    #[test]
    fn symbols_nest_by_indentation() {
        let text = concat!(
            "doctype html\n",
            "/ header (\n",
            "  .ignored\n",
            "body\n",
            "  main#page.wide\n",
            "    = content_for :sidebar do\n",
            "      nav: a.link href=\"/\" = render \"shared/menu\"\n",
            "    - if current_user\n",
            "      = turbo_frame_tag \"cart\" do\n",
            "        = form_with(model: @cart,\n",
            "            url: x) do |f|\n",
            "          section id=\"f\" = f.submit\n",
            "    - else\n",
            "      p Sign in = render \"no\"\n",
            "      div data-x=\"(\" = render partial: \"users/login\"\n",
            "  javascript:\n",
            "    render \"js\"\n",
            "  | text render \"t\"\n",
        );
        assert_eq!(symbols(text), [
            "main#page.wide[content_for :sidebar[nav[a.link[render \"shared/menu\"]]],if current_user[turbo_frame_tag \"cart\"[form_with(model: @cart, url: x)[section#f]]],else[render \"users/login\"]]",
            "javascript:",
        ]);
    }

    #[test]
    fn code_spans_skip_text_comments_and_engines() {
        let text = "= a\n/ = b\np c = d\njavascript:\n  = e\np = f,\n    g\n";
        let spans: Vec<&str> = code_spans(text, &[]).into_iter().map(|(s, e)| &text[s..e]).collect();
        assert_eq!(spans, [" a", " f,\n    g"]);
    }

    #[test]
    fn symbol_ranges_are_utf16() {
        let syms = document_symbols("😀\n.é#a\n  p x\n\n", &[]);
        assert_eq!(syms[0].range, Range::new(Position::new(1, 0), Position::new(2, 5)));
        assert_eq!(syms[0].selection_range, Range::new(Position::new(1, 0), Position::new(1, 4)));
    }
}
