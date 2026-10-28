//! Whitespace-only formatting of HTML ERB, Haml and Slim: indentation, trailing whitespace, blank lines, the final
//! newline and line endings. Anything else stays byte for byte; a file whose structure is in doubt is left unchanged,
//! with the reason.

use crate::checks::{language_diagnostics, ruby_context, RubyContext};
use crate::lang::{html_host, is_partial, TemplateLang};
use crate::{erb_format, haml_format, haml_parser, slim_format, slim_indent, slim_ruby, util};
use std::fmt;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, FormattingOptions, Url};

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

impl Settings {
    /// These settings under the editor's options: its indentation, and the file options it turns on.
    pub fn with_editor(self, options: &FormattingOptions) -> Self {
        let on = |flag: Option<bool>| flag == Some(true);
        Self {
            indent_style: if options.insert_spaces { IndentStyle::Space } else { IndentStyle::Tab },
            indent_size: options.tab_size as usize,
            trim_trailing_whitespace: self.trim_trailing_whitespace || on(options.trim_trailing_whitespace),
            final_newline: if on(options.insert_final_newline) { Some(true) } else { self.final_newline },
            trim_trailing_blank_lines: self.trim_trailing_blank_lines || on(options.trim_final_newlines),
            ..self
        }
    }
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
/// that the blank-line steps must keep.
pub(crate) struct Indented {
    pub text: String,
    pub fixed: Vec<bool>,
    /// What was left alone inside an otherwise formatted file, for `--verbose`.
    pub notes: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Formatted {
    pub text: String,
    pub notes: Vec<String>,
}

/// `text` formatted, or why it is left as it is.
pub fn format(uri: &Url, lang: TemplateLang, text: &str, settings: &Settings, extra_engines: &[String]) -> Result<Formatted, Skipped> {
    if uri.path().to_ascii_lowercase().ends_with(".tt") {
        return skip(SkipKind::NotHandled, "generator template: the generator's ERB is mixed in");
    }
    let settings = &effective_settings(lang, settings);
    // The formatter refuses on errors, so it must see the ones the server shows: those of the project's Ruby.
    let ruby = ruby_context(uri);
    let formatted = format_once(uri, lang, text, settings, extra_engines, &ruby)?;
    if formatted.text == text {
        return Ok(formatted);
    }
    if let Some(broken) = broken_invariant(uri, lang, text, &formatted.text, settings, extra_engines, &ruby) {
        return skip(SkipKind::Internal, broken);
    }
    Ok(formatted)
}

/// Which invariant turning `text` into `formatted` breaks, if one does.
fn broken_invariant(
    uri: &Url,
    lang: TemplateLang,
    text: &str,
    formatted: &str,
    settings: &Settings,
    extra_engines: &[String],
    ruby: &RubyContext,
) -> Option<&'static str> {
    if !same_non_whitespace(text, formatted) {
        return Some("formatting would change more than whitespace");
    }
    if format_once(uri, lang, formatted, settings, extra_engines, ruby).map(|again| again.text).as_deref() != Ok(formatted) {
        return Some("formatting is not stable");
    }
    let before = structure_diagnostics(uri, lang, text, extra_engines, ruby);
    if structure_diagnostics(uri, lang, formatted, extra_engines, ruby).iter().any(|d| !before.contains(d)) {
        return Some("formatting would add a diagnostic");
    }
    None
}

fn format_once(
    uri: &Url,
    lang: TemplateLang,
    text: &str,
    settings: &Settings,
    extra_engines: &[String],
    ruby: &RubyContext,
) -> Result<Formatted, Skipped> {
    let indented = match lang {
        TemplateLang::Slim | TemplateLang::Skim => indent_slim(text, settings, extra_engines, ruby.grammar)?,
        TemplateLang::Erb if !html_host(uri) => return skip(SkipKind::NotHandled, "not an HTML ERB template"),
        TemplateLang::Erb => erb_format::indent(text, &indent_unit(settings), settings.trim_trailing_whitespace, is_partial(uri), ruby)?,
        TemplateLang::Haml => indent_haml(text, settings)?,
        TemplateLang::Ruby => return skip(SkipKind::NotHandled, "Ruby views are formatted by Ruby tools"),
    };
    let mut lines = split_lines(&indented.text, &indented.fixed);
    if settings.collapse_blank_lines {
        collapse_blank_lines(&mut lines);
    }
    if settings.trim_trailing_blank_lines {
        trim_trailing_blank_lines(&mut lines);
    }
    let final_newline = settings.final_newline;
    // Without a final line ending, an empty last line would only be the previous line's ending.
    if final_newline == Some(false) {
        while lines.last().is_some_and(|l| l.text.is_empty() && !l.fixed) {
            lines.pop();
        }
    }
    let text = join_lines(&lines, dominant_ending(text), settings.end_of_line, final_newline);
    Ok(Formatted { text, notes: indented.notes })
}

/// The settings `format` works with: Slim and Haml take only the indentation width and follow their own rules for the
/// rest, except that a range, which can't add a line, leaves the final newline alone. ERB keeps the end of the file as it is.
pub fn effective_settings(lang: TemplateLang, settings: &Settings) -> Settings {
    if lang == TemplateLang::Erb {
        return Settings { final_newline: None, ..settings.clone() };
    }
    if !matches!(lang, TemplateLang::Slim | TemplateLang::Skim | TemplateLang::Haml) {
        return settings.clone();
    }
    Settings {
        indent_style: IndentStyle::Space,
        end_of_line: None,
        final_newline: settings.final_newline.map(|_| true),
        trim_trailing_whitespace: true,
        collapse_blank_lines: false,
        trim_trailing_blank_lines: false,
        ..settings.clone()
    }
}

fn indent_unit(settings: &Settings) -> String {
    match settings.indent_style {
        IndentStyle::Space => " ".repeat(settings.indent_size),
        IndentStyle::Tab => "\t".into(),
    }
}

fn indent_slim(text: &str, settings: &Settings, extra_engines: &[String], ruby: ruby_prism::Version) -> Result<Indented, Skipped> {
    let rejected = slim_indent::diagnostics(text, extra_engines, ruby).iter().any(|d| d.severity == Some(DiagnosticSeverity::ERROR));
    if rejected {
        return skip(SkipKind::Doubt, "Slim would reject the file");
    }
    let original = text;
    let Some(text) = slim_format::format(text, settings.indent_size, extra_engines, settings.trim_trailing_whitespace) else {
        return skip(SkipKind::Doubt, "Slim would reject the file, or its indentation can't move safely");
    };
    if !slim_ruby::same_compilation(original, &text, extra_engines) {
        return skip(SkipKind::Doubt, "the compiled output would change");
    }
    // Slim compiles every line, blank ones included, so line counts stay as they are.
    let fixed = vec![true; text.split('\n').count()];
    Ok(Indented { text, fixed, notes: vec![] })
}

fn indent_haml(text: &str, settings: &Settings) -> Result<Indented, Skipped> {
    let Some(formatted) = haml_format::format(text, settings.indent_size) else {
        return skip(SkipKind::Doubt, "Haml would reject the file");
    };
    if !haml_parser::same_compilation(text, &formatted) {
        return skip(SkipKind::Doubt, "the compiled output would change");
    }
    let fixed = vec![true; formatted.split('\n').count()];
    Ok(Indented { text: formatted, fixed, notes: vec![] })
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
fn structure_diagnostics(uri: &Url, lang: TemplateLang, text: &str, extra_engines: &[String], ruby: &RubyContext) -> Vec<(String, String)> {
    let key = |d: &Diagnostic| {
        let code = util::code(d).unwrap_or_default().to_string();
        (code, d.message.chars().filter(|c| !c.is_ascii_digit()).collect())
    };
    language_diagnostics(uri, lang, text, extra_engines, ruby).iter().map(key).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(name: &str) -> Url {
        Url::parse(&format!("file:///app/views/users/{name}")).unwrap()
    }

    fn fmt(name: &str, text: &str, settings: &Settings) -> Result<String, Skipped> {
        let lang = TemplateLang::from_uri_or_id(&uri(name), None).unwrap();
        format(&uri(name), lang, text, settings, &[]).map(|f| f.text)
    }

    #[test]
    fn slim_files_with_an_error_are_left_alone() {
        // Misindented, so formatting would change them, each with an error Slim raises while compiling.
        for text in [
            "div\n    p\n        - end\n",
            "div\n    p\n        a id=1 id=2\n",
            "div\n    p\n        doctype foo\n",
            "div\n    p\n        = foo(\n",
            "div\n    p\n        ?bad\n",
            "div\n    p\n        a(href=x\n",
            "div\n    br/\n        p\n",
            "div\n    p\n   q\n",
        ] {
            let skipped = fmt("a.html.slim", text, &Settings::default()).expect_err(text);
            assert_eq!(skipped.reason, "Slim would reject the file", "{text:?}");
        }
        // Warnings don't count.
        assert_eq!(fmt("a.html.slim", "div\n    br\n        p\n", &Settings::default()).as_deref(), Ok("div\n  br\n    p\n"));
    }

    #[test]
    fn erb_formatting_cuts_the_locals_comment_only_where_rails_does() {
        let root = std::env::temp_dir().join(format!("rt-format-locals-{}", std::process::id()));
        let file = root.join("app/views/users/_card.html.erb");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let uri = Url::from_file_path(&file).unwrap();
        // Rails 7.1 cuts `# locals: (a:)` out of the tag; before, the `#` comments out the rest of the line.
        let text = "<div>\n<%= render \"x\" # locals: (a:) %>\n</div>\n";
        let formatted = |actionview: &str| {
            std::fs::write(root.join("Gemfile.lock"), format!("GEM\n  specs:\n    actionview ({actionview})\n")).unwrap();
            format(&uri, TemplateLang::Erb, text, &Settings::default(), &[]).map(|f| f.text)
        };
        assert_eq!(formatted("7.1.0").as_deref(), Ok("<div>\n  <%= render \"x\" # locals: (a:) %>\n</div>\n"));
        assert!(formatted("7.0.8").is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn slim_goes_through_its_formatter() {
        assert_eq!(fmt("a.html.slim", "div\n    p  \n", &Settings::default()).as_deref(), Ok("div\n  p\n"));
        let four = Settings { indent_size: 4, ..Settings::default() };
        assert_eq!(fmt("a.html.slim", "div\n  p\n", &four).as_deref(), Ok("div\n    p\n"));
        assert!(fmt("a.html.slim", "p\n    a\n  b", &Settings::default()).is_err());
    }

    #[test]
    fn slim_takes_only_the_indentation_width_from_the_settings() {
        // A tab-indenting editor still gets spaces, and the width is the only setting that counts.
        let tabs = Settings { indent_style: IndentStyle::Tab, indent_size: 3, ..Settings::default() };
        assert_eq!(fmt("a.html.slim", "div\n  p\n", &tabs).as_deref(), Ok("div\n   p\n"));
        let crlf = Settings { end_of_line: Some(LineEnding::CrLf), ..Settings::default() };
        assert_eq!(fmt("a.slim", "p\nq\n", &crlf).as_deref(), Ok("p\nq\n"));
        let lf = Settings { end_of_line: Some(LineEnding::Lf), ..Settings::default() };
        assert_eq!(fmt("a.slim", "p\r\nq\r\n", &lf).as_deref(), Ok("p\r\nq\r\n"));
        assert_eq!(fmt("a.slim", "p\r\n  q\nr", &lf).as_deref(), Ok("p\r\n  q\nr\n"), "each line keeps its ending");
        let drop = Settings { final_newline: Some(false), ..Settings::default() };
        assert_eq!(fmt("a.slim", "p\n", &drop).as_deref(), Ok("p\n"));
        let keep_trailing = Settings { trim_trailing_whitespace: false, ..Settings::default() };
        assert_eq!(fmt("a.slim", "p  \nq\n", &keep_trailing).as_deref(), Ok("p\nq\n"));
        let more_blank = Settings { collapse_blank_lines: true, trim_trailing_blank_lines: true, ..Settings::default() };
        assert_eq!(fmt("a.slim", "p\n\n\nq\n   \n", &more_blank).as_deref(), Ok("p\n\n\nq\n   \n"));
        // A range leaves the final newline as it is.
        let range = Settings { final_newline: None, ..Settings::default() };
        assert_eq!(fmt("a.slim", "p", &range).as_deref(), Ok("p"));
    }

    #[test]
    fn haml_takes_only_the_indentation_width_from_the_settings() {
        let d = Settings::default;
        assert_eq!(fmt("a.html.haml", "%div\n\t%p  \n", &d()).as_deref(), Ok("%div\n  %p\n"));
        let tabs = Settings { indent_style: IndentStyle::Tab, indent_size: 3, ..d() };
        assert_eq!(fmt("a.haml", "%div\n  %p\n", &tabs).as_deref(), Ok("%div\n   %p\n"));
        let crlf = Settings { end_of_line: Some(LineEnding::CrLf), trim_trailing_whitespace: false, ..d() };
        assert_eq!(fmt("a.haml", "%p \n%i\n\n  \n", &crlf).as_deref(), Ok("%p\n%i\n"), "ending kept, trailing lines collapse");
        let more_blank = Settings { collapse_blank_lines: true, ..d() };
        assert_eq!(fmt("a.haml", "%p\n\n\n%i\n", &more_blank).as_deref(), Ok("%p\n\n\n%i\n"));
        let range = Settings { final_newline: None, ..d() };
        assert_eq!(fmt("a.haml", "%p", &range).as_deref(), Ok("%p"));
    }

    #[test]
    fn haml_continuation_lines_follow_a_width_of_four() {
        let four = Settings { indent_size: 4, ..Settings::default() };
        let text = "%div\n  %p= link_to \"Home\",\n    root_path\n";
        assert_eq!(fmt("a.html.haml", text, &four).as_deref(), Ok("%div\n    %p= link_to \"Home\",\n        root_path\n"));
    }

    #[test]
    fn haml_files_with_an_error_are_left_alone() {
        // Misindented, so formatting would change them; the inconsistent indentation is an error, not something to repair.
        for text in ["%div\n    %p\n  %i\n", "%div\n    %p= x\n        %b\n", "  %p\n", "%div\n    :nope\n        a\n"] {
            let skipped = fmt("a.html.haml", text, &Settings::default()).expect_err(text);
            assert_eq!((skipped.kind, skipped.reason.as_str()), (SkipKind::Doubt, "Haml would reject the file"), "{text:?}");
        }
    }

    #[test]
    fn erb_keeps_the_end_of_the_file() {
        let s = Settings::default();
        assert_eq!(fmt("a.html.erb", "<div>\n<p>x</p>\n</div>", &s).as_deref(), Ok("<div>\n  <p>x</p>\n</div>"));
        assert_eq!(fmt("a.html.erb", "<div>\n<p>x</p>\n</div>  ", &s).as_deref(), Ok("<div>\n  <p>x</p>\n</div>"));
        assert_eq!(fmt("a.html.erb", "<p>x</p>\n\n\n", &s).as_deref(), Ok("<p>x</p>\n\n\n"));
        assert_eq!(fmt("a.html.erb", "<p>x</p>\n\n  ", &s).as_deref(), Ok("<p>x</p>\n\n"));
    }

    #[test]
    fn final_newline_and_line_endings() {
        let s = Settings::default();
        assert_eq!(fmt("a.slim", "p", &s).as_deref(), Ok("p\n"));
        assert_eq!(fmt("a.slim", "p\r\nq", &s).as_deref(), Ok("p\r\nq\r\n"));
        let keep = Settings { final_newline: None, ..Settings::default() };
        assert_eq!(fmt("a.slim", "p", &keep).as_deref(), Ok("p"));
        let drop = Settings { final_newline: Some(false), ..Settings::default() };
        assert_eq!(fmt("a.html.haml", "p", &drop).as_deref(), Ok("p\n"), "Haml ends the file by its own rules");
        assert_eq!(fmt("a.html.erb", "<p>\n\n\n", &drop).as_deref(), Ok("<p>\n\n\n"), "ERB keeps the end of the file");
        let lf = Settings { end_of_line: Some(LineEnding::Lf), ..Settings::default() };
        assert_eq!(fmt("a.html.erb", "<p>\r\n<br>\r\n", &lf).as_deref(), Ok("<p>\n<br>\n"));
        let crlf = Settings { end_of_line: Some(LineEnding::CrLf), ..Settings::default() };
        assert_eq!(fmt("a.html.erb", "<p>\n<br>", &crlf).as_deref(), Ok("<p>\r\n<br>"));
    }

    #[test]
    fn editor_options_come_first() {
        let project = Settings { indent_size: 4, trim_trailing_whitespace: false, final_newline: Some(false), ..Settings::default() };
        let editor = FormattingOptions { tab_size: 3, insert_spaces: false, trim_trailing_whitespace: Some(true), ..Default::default() };
        let s = project.clone().with_editor(&editor);
        assert_eq!((s.indent_style, s.indent_size, s.trim_trailing_whitespace, s.final_newline), (IndentStyle::Tab, 3, true, Some(false)));
        let quiet = FormattingOptions { tab_size: 2, insert_spaces: true, trim_trailing_whitespace: Some(false), ..Default::default() };
        assert!(!project.with_editor(&quiet).trim_trailing_whitespace, "an editor option that is off leaves the project's");
    }

    #[test]
    fn leaves_other_files_alone() {
        let s = Settings::default();
        assert_eq!(fmt("a.js.erb", "x", &s).map_err(|e| e.kind), Err(SkipKind::NotHandled));
        assert!(fmt("a.json.jbuilder", "json.x 1", &s).is_err());
        let template = Url::parse("file:///lib/templates/slim/scaffold/index.html.slim.tt").unwrap();
        assert!(format(&template, TemplateLang::Slim, "p", &s, &[]).is_err());
    }

    const SAMPLES: [(&str, &str); 10] = [
        ("sample.html.erb", include_str!("../../test/sample.html.erb")),
        ("sample.html.haml", include_str!("../../test/sample.html.haml")),
        ("sample.html.slim", include_str!("../../test/sample.html.slim")),
        ("sample.skim", include_str!("../../test/sample.skim")),
        ("sample.js.erb", include_str!("../../test/sample.js.erb")),
        ("sample.json.erb", include_str!("../../test/sample.json.erb")),
        ("sample.text.erb", include_str!("../../test/sample.text.erb")),
        ("bad.html.erb", include_str!("../../test/broken/bad.html.erb")),
        ("bad.html.haml", include_str!("../../test/broken/bad.html.haml")),
        ("bad.html.slim", include_str!("../../test/broken/bad.html.slim")),
    ];

    /// The sample with every line's indentation replaced: none, doubled, tabs; and with CRLF endings.
    fn variants(text: &str) -> Vec<String> {
        let map = |f: &dyn Fn(&str, &str) -> String| {
            text.split('\n').map(|l| f(util::leading_ws(l), l.trim_start())).collect::<Vec<_>>().join("\n")
        };
        vec![
            text.to_string(),
            map(&|_, rest| rest.to_string()),
            map(&|ws, rest| format!("{ws}{ws}{rest}")),
            map(&|ws, rest| format!("{}{rest}", "\t".repeat(ws.len() / 2))),
            text.replace('\n', "\r\n"),
        ]
    }

    fn settings_variants() -> Vec<Settings> {
        let d = Settings::default;
        vec![
            d(),
            Settings { indent_size: 4, ..d() },
            Settings { indent_style: IndentStyle::Tab, ..d() },
            Settings { end_of_line: Some(LineEnding::CrLf), final_newline: Some(false), ..d() },
            Settings { collapse_blank_lines: true, trim_trailing_blank_lines: true, trim_trailing_whitespace: false, ..d() },
        ]
    }

    /// The invariants checked on the unguarded formatter, so a violation fails here instead of being refused.
    #[test]
    fn invariants_hold_on_the_samples() {
        let ruby = RubyContext::outside_project();
        let mut formatted = 0;
        for (name, sample) in SAMPLES {
            let uri = uri(name);
            let lang = TemplateLang::from_uri_or_id(&uri, None).unwrap();
            for text in variants(sample) {
                for settings in settings_variants() {
                    let Ok(once) = format_once(&uri, lang, &text, &settings, &[], &ruby) else { continue };
                    formatted += 1;
                    let broken = broken_invariant(&uri, lang, &text, &once.text, &settings, &[], &ruby);
                    assert_eq!(broken, None, "{name} with {settings:?}:\n{text}\n---\n{}", once.text);
                }
            }
        }
        assert!(formatted >= 40, "only {formatted} sample variants were formatted");
    }

    #[test]
    fn broken_fixtures_come_back_unchanged() {
        for (name, text) in &SAMPLES[7..] {
            let uri = uri(name);
            let lang = TemplateLang::from_uri_or_id(&uri, None).unwrap();
            let kind = format(&uri, lang, text, &Settings::default(), &[]).map(|f| f.text);
            assert!(kind.as_ref().is_err_and(|e| e.kind == SkipKind::Doubt), "{name}: {kind:?}");
        }
    }

    /// `RUBY_TEMPLATES_CORPUS=<dir> cargo test --release corpus -- --ignored --nocapture`: the sample invariants on
    /// every template under `<dir>` (real apps' `app/views`, kept outside the repository).
    #[test]
    #[ignore]
    fn corpus() {
        let dir = std::env::var("RUBY_TEMPLATES_CORPUS").expect("RUBY_TEMPLATES_CORPUS");
        let (mut files, mut formatted, mut refused) = (0, 0, std::collections::BTreeMap::<String, usize>::new());
        let mut violations = Vec::new();
        let ruby = RubyContext::outside_project();
        for entry in walkdir::WalkDir::new(dir).into_iter().filter_map(Result::ok).filter(|e| e.file_type().is_file()) {
            let Ok(uri) = Url::from_file_path(entry.path()) else { continue };
            let (Some(lang), Ok(sample)) = (TemplateLang::from_uri_or_id(&uri, None), std::fs::read_to_string(entry.path())) else {
                continue;
            };
            files += 1;
            for (v, text) in variants(&sample).into_iter().enumerate() {
                for (s, settings) in settings_variants().into_iter().enumerate() {
                    match format_once(&uri, lang, &text, &settings, &[], &ruby) {
                        Ok(once) => {
                            formatted += 1;
                            if let Some(broken) = broken_invariant(&uri, lang, &text, &once.text, &settings, &[], &ruby) {
                                violations.push(format!("{} variant {v} settings {s}: {broken}", entry.path().display()));
                            }
                        }
                        Err(e) => *refused.entry(format!("{:?}", e.kind)).or_default() += 1,
                    }
                }
            }
        }
        println!("{files} templates, {formatted} formatted variants, refused: {refused:?}");
        assert!(violations.is_empty(), "{} violations:\n{}", violations.len(), violations.join("\n"));
    }
}
