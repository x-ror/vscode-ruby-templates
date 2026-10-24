use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range, TextDocumentContentChangeEvent};

/// Convert a byte offset within `text` to an LSP position (line, UTF-16 column).
pub fn offset_to_position(text: &str, offset: usize) -> Position {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col: usize = text[line_start..offset].encode_utf16().count();
    Position::new(line as u32, col as u32)
}

pub fn line_range(text: &str, line: usize, start_col: usize, end_col: usize) -> Range {
    span_range(line, text.lines().nth(line).unwrap_or(""), start_col, end_col)
}

/// Byte columns of `line_text` (line number `line`) as an LSP range.
pub fn span_range(line: usize, line_text: &str, start_col: usize, end_col: usize) -> Range {
    let s = line_text.get(..start_col.min(line_text.len())).unwrap_or("").encode_utf16().count() as u32;
    let e = line_text.get(..end_col.min(line_text.len())).unwrap_or("").encode_utf16().count() as u32;
    Range::new(Position::new(line as u32, s), Position::new(line as u32, e))
}

pub fn diag(range: Range, severity: DiagnosticSeverity, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(severity),
        source: Some("ruby-templates".into()),
        message: message.into(),
        ..Default::default()
    }
}

pub fn error(range: Range, message: impl Into<String>) -> Diagnostic {
    diag(range, DiagnosticSeverity::ERROR, message)
}

pub fn warning(range: Range, message: impl Into<String>) -> Diagnostic {
    diag(range, DiagnosticSeverity::WARNING, message)
}

/// Template indentation: spaces and tabs only (Haml and Slim don't indent with other Unicode whitespace).
pub fn leading_ws(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// `text.lines()` paired with each line's byte offset in `text`.
pub fn lines_with_offsets(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split_inclusive('\n').scan(0, |start, raw| {
        let offset = *start;
        *start += raw.len();
        let line = raw.strip_suffix('\n').map_or(raw, |l| l.strip_suffix('\r').unwrap_or(l));
        Some((offset, line))
    })
}

/// Byte index in `line` of UTF-16 column `character`, clamped to the line's end.
pub fn byte_col(line: &str, character: u32) -> usize {
    let mut units = 0u32;
    for (idx, ch) in line.char_indices() {
        if units >= character { return idx; }
        units += ch.len_utf16() as u32;
    }
    line.len()
}

/// Byte offset of an LSP position in `text`; past-the-end lines map to the end.
pub fn position_to_offset(text: &str, pos: Position) -> usize {
    let (start, line) = lines_with_offsets(text).nth(pos.line as usize).unwrap_or((text.len(), ""));
    start + byte_col(line, pos.character)
}

/// Apply one `didChange` content change: a ranged edit, or the whole text when `range` is absent.
pub fn apply_change(text: &mut String, change: TextDocumentContentChangeEvent) {
    let Some(range) = change.range else {
        *text = change.text;
        return;
    };
    let start = position_to_offset(text, range.start);
    let end = position_to_offset(text, range.end).max(start);
    text.replace_range(start..end, &change.text);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(range: Option<((u32, u32), (u32, u32))>, text: &str) -> TextDocumentContentChangeEvent {
        let range = range.map(|((sl, sc), (el, ec))| Range::new(Position::new(sl, sc), Position::new(el, ec)));
        TextDocumentContentChangeEvent { range, range_length: None, text: text.into() }
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
}
