//! Whitespace-only formatting of HTML ERB, Haml and Slim: indentation, trailing whitespace, blank lines, the final
//! newline and line endings. Anything else stays byte for byte; a file whose structure is in doubt is left unchanged,
//! with the reason.

use crate::lang::{html_host, slim_html_host, TemplateLang};
use crate::{erb, haml_indent, slim_format, slim_indent, tags, util};
use std::fmt;
use tower_lsp::lsp_types::{Diagnostic, Url};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentStyle {
    Space,
    Tab,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub indent_style: IndentStyle,
    /// Columns per nesting level; with tabs, how wide a tab counts when measuring indentation.
    pub indent_size: usize,
    /// `None` keeps each line's ending.
    pub end_of_line: Option<LineEnding>,
    /// `Some(true)` ends the file with a line ending, `Some(false)` drops it, `None` leaves it.
    pub final_newline: Option<bool>,
    pub trim_trailing_whitespace: bool,
    /// At most one blank line in a row.
    pub collapse_blank_lines: bool,
    pub trim_trailing_blank_lines: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            indent_style: IndentStyle::Space,
            indent_size: 2,
            end_of_line: None,
            final_newline: Some(true),
            trim_trailing_whitespace: true,
            collapse_blank_lines: false,
            trim_trailing_blank_lines: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipKind {
    /// Not a file this formatter handles (`.js.erb`, Jbuilder, generator templates).
    NotHandled,
    /// Its structure is broken or ambiguous.
    Doubt,
    /// A formatter bug, caught by an invariant check.
    Internal,
}

/// Why a file came back unchanged.
#[derive(Debug, PartialEq, Eq)]
pub struct Skipped {
    pub kind: SkipKind,
    pub reason: String,
}

impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

pub(crate) fn skip<T>(kind: SkipKind, reason: impl Into<String>) -> Result<T, Skipped> {
    Err(Skipped { kind, reason: reason.into() })
}

/// A language's re-indented text, plus which lines hold content whitespace (a `<pre>` body, Ruby code, a filter)
/// that the blank-line steps must keep, and whether the file's last line ending is content too.
pub(crate) struct Indented {
    pub text: String,
    pub fixed: Vec<bool>,
    pub fixed_end: bool,
}

/// `text` formatted, or why it is left as it is.
pub fn format(uri: &Url, lang: TemplateLang, text: &str, settings: &Settings, extra_engines: &[String]) -> Result<String, Skipped> {
    if uri.path().to_ascii_lowercase().ends_with(".tt") {
        return skip(SkipKind::NotHandled, "generator template: the generator's ERB is mixed in");
    }
    let formatted = format_once(uri, lang, text, settings, extra_engines)?;
    if formatted == text {
        return Ok(formatted);
    }
    if !same_non_whitespace(text, &formatted) {
        return skip(SkipKind::Internal, "formatting would change more than whitespace");
    }
    if format_once(uri, lang, &formatted, settings, extra_engines).as_deref() != Ok(formatted.as_str()) {
        return skip(SkipKind::Internal, "formatting is not stable");
    }
    let before = structure_diagnostics(uri, lang, text, extra_engines);
    if structure_diagnostics(uri, lang, &formatted, extra_engines).iter().any(|d| !before.contains(d)) {
        return skip(SkipKind::Internal, "formatting would add a diagnostic");
    }
    Ok(formatted)
}

fn format_once(uri: &Url, lang: TemplateLang, text: &str, settings: &Settings, extra_engines: &[String]) -> Result<String, Skipped> {
    let indented = match lang {
        TemplateLang::Slim | TemplateLang::Skim => indent_slim(text, settings, extra_engines)?,
        TemplateLang::Erb if !html_host(uri) => return skip(SkipKind::NotHandled, "not an HTML ERB template"),
        TemplateLang::Erb | TemplateLang::Haml => return skip(SkipKind::NotHandled, "not supported yet"),
        TemplateLang::Ruby => return skip(SkipKind::NotHandled, "Ruby views are formatted by Ruby tools"),
    };
    let mut lines = split_lines(&indented.text, &indented.fixed);
    if settings.collapse_blank_lines {
        collapse_blank_lines(&mut lines);
    }
    if settings.trim_trailing_blank_lines {
        trim_trailing_blank_lines(&mut lines);
    }
    let final_newline = settings.final_newline.filter(|_| !indented.fixed_end);
    Ok(join_lines(&lines, dominant_ending(text), settings.end_of_line, final_newline))
}

fn indent_slim(text: &str, settings: &Settings, extra_engines: &[String]) -> Result<Indented, Skipped> {
    if settings.indent_style == IndentStyle::Tab {
        return skip(SkipKind::NotHandled, "Slim is only indented with spaces");
    }
    let Some(text) = slim_format::format(text, settings.indent_size, extra_engines) else {
        return skip(SkipKind::Doubt, "Slim would reject the file, or its indentation can't move safely");
    };
    // Slim compiles every line, blank ones included, so line counts stay as they are.
    let fixed = vec![true; text.split('\n').count()];
    Ok(Indented { text, fixed, fixed_end: false })
}

struct Line<'a> {
    text: &'a str,
    /// `"\n"`, `"\r\n"`, or `""` on a last line without one.
    eol: &'a str,
    fixed: bool,
}

impl Line<'_> {
    fn blank(&self) -> bool {
        !self.fixed && self.text.trim().is_empty()
    }
}

fn split_lines<'a>(text: &'a str, fixed: &[bool]) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    let mut rest = text;
    for i in 0.. {
        if rest.is_empty() {
            break;
        }
        let (line, eol, next) = match rest.find('\n') {
            Some(n) if rest[..n].ends_with('\r') => (&rest[..n - 1], "\r\n", &rest[n + 1..]),
            Some(n) => (&rest[..n], "\n", &rest[n + 1..]),
            None => (rest, "", ""),
        };
        lines.push(Line { text: line, eol, fixed: fixed.get(i).copied().unwrap_or(true) });
        rest = next;
    }
    lines
}

fn collapse_blank_lines(lines: &mut Vec<Line>) {
    let mut previous_blank = false;
    lines.retain(|line| {
        let blank = line.blank();
        let keep = !(blank && previous_blank);
        previous_blank = blank;
        keep
    });
}

fn trim_trailing_blank_lines(lines: &mut Vec<Line>) {
    while lines.last().is_some_and(Line::blank) {
        lines.pop();
    }
}

fn dominant_ending(text: &str) -> LineEnding {
    let lf = text.matches('\n').count();
    let crlf = text.matches("\r\n").count();
    if crlf * 2 > lf {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

fn join_lines(lines: &[Line], dominant: LineEnding, end_of_line: Option<LineEnding>, final_newline: Option<bool>) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        out.push_str(line.text);
        let last = i + 1 == lines.len();
        let eol = match (end_of_line, line.eol) {
            _ if last && final_newline == Some(false) => "",
            (_, "") if final_newline != Some(true) => "",
            (Some(ending), _) => ending.as_str(),
            (None, "") => dominant.as_str(),
            (None, eol) => eol,
        };
        out.push_str(eol);
    }
    out
}

// ASCII only: a no-break space is content.
fn same_non_whitespace(a: &str, b: &str) -> bool {
    a.chars().filter(|c| !c.is_ascii_whitespace()).eq(b.chars().filter(|c| !c.is_ascii_whitespace()))
}

/// The language's own structure checks, as `(code, message)` with line numbers dropped, since blank-line
/// steps move lines.
fn structure_diagnostics(uri: &Url, lang: TemplateLang, text: &str, extra_engines: &[String]) -> Vec<(String, String)> {
    let found: Vec<Diagnostic> = match lang {
        TemplateLang::Erb => {
            let partial = uri.path().rsplit('/').next().is_some_and(|f| f.starts_with('_'));
            let mut found = erb::diagnostics(text);
            if html_host(uri) {
                found.extend(tags::diagnostics(text, partial));
            }
            found
        }
        TemplateLang::Haml => haml_indent::diagnostics(text),
        TemplateLang::Slim | TemplateLang::Skim => {
            let mut found = slim_indent::diagnostics(text, extra_engines);
            if !slim_html_host(uri) {
                found.retain(|d| util::code(d) != Some(slim_indent::code::VOID_CHILDREN));
            }
            found
        }
        TemplateLang::Ruby => vec![],
    };
    let key = |d: &Diagnostic| {
        let code = util::code(d).unwrap_or_default().to_string();
        (code, d.message.chars().filter(|c| !c.is_ascii_digit()).collect())
    };
    found.iter().map(key).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(name: &str) -> Url {
        Url::parse(&format!("file:///app/views/users/{name}")).unwrap()
    }

    fn fmt(name: &str, text: &str, settings: &Settings) -> Result<String, Skipped> {
        let lang = TemplateLang::from_uri_or_id(&uri(name), None).unwrap();
        format(&uri(name), lang, text, settings, &[])
    }

    #[test]
    fn slim_goes_through_its_formatter() {
        assert_eq!(fmt("a.html.slim", "div\n    p  \n", &Settings::default()).as_deref(), Ok("div\n  p\n"));
        let four = Settings { indent_size: 4, ..Settings::default() };
        assert_eq!(fmt("a.html.slim", "div\n  p\n", &four).as_deref(), Ok("div\n    p\n"));
        let tabs = Settings { indent_style: IndentStyle::Tab, ..Settings::default() };
        assert!(fmt("a.html.slim", "div\n  p\n", &tabs).is_err());
        assert!(fmt("a.html.slim", "p\n    a\n  b", &Settings::default()).is_err());
    }

    #[test]
    fn final_newline_and_line_endings() {
        let s = Settings::default();
        assert_eq!(fmt("a.slim", "p", &s).as_deref(), Ok("p\n"));
        assert_eq!(fmt("a.slim", "p\r\nq", &s).as_deref(), Ok("p\r\nq\r\n"));
        let keep = Settings { final_newline: None, ..Settings::default() };
        assert_eq!(fmt("a.slim", "p", &keep).as_deref(), Ok("p"));
        let drop = Settings { final_newline: Some(false), ..Settings::default() };
        assert_eq!(fmt("a.slim", "p\n", &drop).as_deref(), Ok("p"));
        let lf = Settings { end_of_line: Some(LineEnding::Lf), ..Settings::default() };
        assert_eq!(fmt("a.slim", "p\r\nq\r\n", &lf).as_deref(), Ok("p\nq\n"));
        let crlf = Settings { end_of_line: Some(LineEnding::CrLf), ..Settings::default() };
        assert_eq!(fmt("a.slim", "p\nq", &crlf).as_deref(), Ok("p\r\nq\r\n"));
    }

    #[test]
    fn leaves_other_files_alone() {
        let s = Settings::default();
        assert_eq!(fmt("a.js.erb", "x", &s).map_err(|e| e.kind), Err(SkipKind::NotHandled));
        assert!(fmt("a.json.jbuilder", "json.x 1", &s).is_err());
        let template = Url::parse("file:///lib/templates/slim/scaffold/index.html.slim.tt").unwrap();
        assert!(format(&template, TemplateLang::Slim, "p", &s, &[]).is_err());
    }
}
