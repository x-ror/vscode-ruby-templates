use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range, TextDocumentContentChangeEvent,
};

/// Convert a byte offset within `text` to an LSP position (line, UTF-16 column); for many offsets, use one `LineIndex`.
pub fn offset_to_position(text: &str, offset: usize) -> Position {
    let offset = char_floor(text, offset);
    let before = &text[..offset];
    column_position(text, before.matches('\n').count(), before.rfind('\n').map_or(0, |i| i + 1), offset)
}

/// `offset` clamped to `text` and moved back to a character boundary.
fn char_floor(text: &str, offset: usize) -> usize {
    (0..=offset.min(text.len())).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0)
}

/// The position of `offset` on `line`, which starts at byte `line_start`: its column counts UTF-16 units.
fn column_position(text: &str, line: usize, line_start: usize, offset: usize) -> Position {
    Position::new(line as u32, text[line_start..offset].encode_utf16().count() as u32)
}

/// The line starts of a text, so that many offsets or lines can be looked up without scanning from the start each time.
pub struct LineIndex<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(text: &'a str) -> Self {
        LineIndex { text, starts: std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect() }
    }

    /// The line and UTF-16 column of `offset`, moved back to a character boundary and clamped to the text.
    pub fn position(&self, offset: usize) -> Position {
        let offset = char_floor(self.text, offset);
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        column_position(self.text, line, self.starts[line], offset)
    }

    /// Line `n` as `str::lines` gives it; empty past the end.
    pub fn line(&self, n: usize) -> &'a str {
        let Some(&start) = self.starts.get(n) else { return "" };
        let end = self.starts.get(n + 1).copied().unwrap_or(self.text.len());
        without_line_ending(&self.text[start..end])
    }

    /// `line_text_range` of line `n`.
    pub fn text_range(&self, n: usize) -> Range {
        let raw = self.line(n);
        let start = leading_ws(raw).len();
        span_range(n, raw, start, raw.trim_end().len().max(start))
    }

    /// `str::lines().count()`.
    pub fn line_count(&self) -> usize {
        // A final line ending opens no line of its own.
        self.starts.len() - usize::from(self.text.is_empty() || self.text.ends_with('\n'))
    }
}

/// A line as `str::lines` yields it: without its `\n` or `\r\n`; a lone `\r` at the very end stays.
fn without_line_ending(raw: &str) -> &str {
    raw.strip_suffix('\n').map_or(raw, |l| l.strip_suffix('\r').unwrap_or(l))
}

pub fn line_range(text: &str, line: usize, start_col: usize, end_col: usize) -> Range {
    span_range(line, text.lines().nth(line).unwrap_or(""), start_col, end_col)
}

/// The text of `line` without its leading and trailing whitespace.
pub fn line_text_range(text: &str, line: usize) -> Range {
    LineIndex::new(text).text_range(line)
}

/// Byte columns of `line_text` (line number `line`) as an LSP range.
pub fn span_range(line: usize, line_text: &str, start_col: usize, end_col: usize) -> Range {
    let s = line_text.get(..start_col.min(line_text.len())).unwrap_or("").encode_utf16().count() as u32;
    let e = line_text.get(..end_col.min(line_text.len())).unwrap_or("").encode_utf16().count() as u32;
    Range::new(Position::new(line as u32, s), Position::new(line as u32, e))
}

pub fn diag(range: Range, severity: DiagnosticSeverity, message: impl Into<String>) -> Diagnostic {
    Diagnostic { range, severity: Some(severity), message: message.into(), ..Default::default() }
}

pub fn error(range: Range, message: impl Into<String>) -> Diagnostic {
    diag(range, DiagnosticSeverity::ERROR, message)
}

pub fn warning(range: Range, message: impl Into<String>) -> Diagnostic {
    diag(range, DiagnosticSeverity::WARNING, message)
}

/// `%XX` escapes decoded; malformed escapes are kept as they are.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Do `query`'s characters appear in `name` in order (`uscard` in `users/card`)?
pub fn fuzzy_match(name: &str, query: &str) -> bool {
    let mut chars = name.chars();
    query.chars().all(|q| chars.any(|c| c == q))
}

/// Attach a stable `code` so users can filter or disable the diagnostic.
pub fn coded(mut d: Diagnostic, code: &str) -> Diagnostic {
    d.code = Some(NumberOrString::String(code.into()));
    d
}

/// The string `code` of `d`, as set by `coded`.
pub fn code(d: &Diagnostic) -> Option<&str> {
    match &d.code {
        Some(NumberOrString::String(code)) => Some(code),
        _ => None,
    }
}

pub fn completion_item(label: impl Into<String>, kind: CompletionItemKind, detail: impl Into<String>) -> CompletionItem {
    CompletionItem { label: label.into(), kind: Some(kind), detail: Some(detail.into()), ..Default::default() }
}

/// Template indentation: spaces and tabs only (Haml and Slim don't indent with other Unicode whitespace).
pub fn leading_ws(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// Length of a balanced `open ... close` group starting at s[0], honoring strings.
pub fn find_balanced(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    let mut esc = false;
    for (idx, ch) in s.char_indices() {
        if let Some(q) = in_str {
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == q {
                in_str = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => in_str = Some(ch),
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

/// `text.lines()` paired with each line's byte offset in `text`.
pub fn lines_with_offsets(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split_inclusive('\n').scan(0, |start, raw| {
        let offset = *start;
        *start += raw.len();
        Some((offset, without_line_ending(raw)))
    })
}

/// Byte index in `line` of UTF-16 column `character`, clamped to the line's end.
pub fn byte_col(line: &str, character: u32) -> usize {
    let mut units = 0u32;
    for (idx, ch) in line.char_indices() {
        if units >= character {
            return idx;
        }
        units += ch.len_utf16() as u32;
    }
    line.len()
}

/// The byte offset in `text` and content of `pos`'s line, and `pos`'s byte column in it; past the end, an empty line.
pub fn line_at(text: &str, pos: Position) -> (usize, &str, usize) {
    let (line_start, line) = lines_with_offsets(text).nth(pos.line as usize).unwrap_or((text.len(), ""));
    (line_start, line, byte_col(line, pos.character))
}

/// The identifier (`?`/`!` included) around byte `col` of `line`.
pub fn word_at(line: &str, col: usize) -> &str {
    let is_word = |c: char| c.is_alphanumeric() || matches!(c, '_' | '?' | '!');
    let (before, after) = line.split_at(col.min(line.len()));
    let start = before.trim_end_matches(is_word).len();
    let end = line.len() - after.trim_start_matches(is_word).len();
    &line[start..end]
}

/// Byte offset of an LSP position; a column past the line end clamps to it, a line past the end is `None`.
pub fn position_to_offset(text: &str, pos: Position) -> Option<usize> {
    let mut line_start = 0;
    for _ in 0..pos.line {
        line_start += text[line_start..].find('\n')? + 1;
    }
    let line = lines_with_offsets(&text[line_start..]).next().map_or("", |(_, line)| line);
    Some(line_start + byte_col(line, pos.character))
}

/// Apply one `didChange` content change: a ranged edit, or the whole text when `range` is absent.
pub fn apply_change(text: &mut String, change: TextDocumentContentChangeEvent) {
    let Some(range) = change.range else {
        *text = change.text;
        return;
    };
    let start = position_to_offset(text, range.start).unwrap_or(text.len());
    let end = position_to_offset(text, range.end).unwrap_or(text.len()).max(start);
    text.replace_range(start..end, &change.text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_agrees_with_scanning_the_text() {
        for text in ["", "a", "a\n", "é😀b\r\nc\n\nd", "x\r", "a\r\n\r\nb\r"] {
            let index = LineIndex::new(text);
            for offset in 0..=text.len() + 1 {
                let at = (0..=offset.min(text.len())).rev().find(|&i| text.is_char_boundary(i)).unwrap();
                let before = &text[..at];
                let column = before[before.rfind('\n').map_or(0, |i| i + 1)..].encode_utf16().count();
                let scanned = Position::new(before.matches('\n').count() as u32, column as u32);
                assert_eq!(index.position(offset), scanned, "{text:?} at {offset}");
            }
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(index.line_count(), lines.len(), "{text:?}");
            for n in 0..lines.len() + 2 {
                assert_eq!(index.line(n), lines.get(n).copied().unwrap_or(""), "{text:?} line {n}");
            }
        }
    }

    fn edit(range: Option<((u32, u32), (u32, u32))>, text: &str) -> TextDocumentContentChangeEvent {
        let range = range.map(|((sl, sc), (el, ec))| Range::new(Position::new(sl, sc), Position::new(el, ec)));
        TextDocumentContentChangeEvent { range, range_length: None, text: text.into() }
    }

    #[test]
    fn positions_count_utf16_and_stay_on_char_boundaries() {
        assert_eq!(offset_to_position("a\n😀x", 6), Position::new(1, 2));
        assert_eq!(offset_to_position("a\n😀x", 4), Position::new(1, 0));
        assert_eq!(offset_to_position("é", 99), Position::new(0, 1));
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("show.html%2Bphone.erb"), "show.html+phone.erb");
        assert_eq!(percent_decode("%C3%A9%2"), "é%2");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn fuzzy_matches_in_order() {
        assert!(fuzzy_match("users/card", "uscard"));
        assert!(fuzzy_match("users/card", ""));
        assert!(!fuzzy_match("users/card", "cardus"));
    }

    #[test]
    fn applies_ranged_and_full_changes() {
        // 😀 is two UTF-16 units, é one; both are multibyte in UTF-8.
        let mut text = "é😀<%= a %>\r\nnext\n".to_string();
        apply_change(&mut text, edit(Some(((0, 7), (0, 8))), "user"));
        assert_eq!(text, "é😀<%= user %>\r\nnext\n");
        for change in [edit(Some(((0, 1), (0, 3))), ""), edit(Some(((1, 4), (1, 4))), "!"), edit(Some(((0, 12), (1, 1))), "\nN")] {
            apply_change(&mut text, change);
        }
        assert_eq!(text, "é<%= user %>\nNext!\n");
        apply_change(&mut text, edit(Some(((9, 0), (9, 0))), "end"));
        assert_eq!(text, "é<%= user %>\nNext!\nend");
        apply_change(&mut text, edit(None, "fresh"));
        assert_eq!(text, "fresh");
    }

    #[test]
    fn word_under_cursor() {
        let cases = [
            ("a user? b", 4, "user?"),
            ("a user? b", 2, "user?"),
            ("a user? b", 7, "user?"),
            ("a user? b", 8, "b"),
            ("a  b", 2, ""),
            ("t(’greeting’)", 8, "greeting"),
            ("— ok!", 5, "ok!"),
            ("😀名前", 4, "名前"),
            ("abc", 99, "abc"),
            ("", 0, ""),
        ];
        for (line, col, word) in cases {
            assert_eq!(word_at(line, col), word, "{line:?} at {col}");
        }
    }

    #[test]
    fn find_balanced_ignores_brackets_inside_strings() {
        let len = |s: &str| find_balanced(s, '(', ')');
        assert_eq!(len("(a (b) c) tail"), Some(9));
        assert_eq!(len(r#"("a)" + 1) x"#), Some(10));
        assert_eq!(len(r#"("it's)") x"#), Some(9));
        assert_eq!(len(r#"('say "hi)') x"#), Some(12));
        assert_eq!(len(r#"("a\")") x"#), Some(8));
        assert_eq!(len(r#"("é)")"#), Some(r#"("é)")"#.len()));
        assert_eq!(len(r#"("a)""#), None);
        assert_eq!(len("(a"), None);
    }
}
