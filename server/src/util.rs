use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

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
    let line_text = text.lines().nth(line).unwrap_or("");
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

pub fn leading_ws(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}
