//! Whitespace-only formatting of HTML ERB, Haml and Slim: indentation, trailing whitespace, blank lines and the final
//! newline. Anything else stays byte for byte; a file whose structure is in doubt is left unchanged, with the reason.

pub(crate) mod blank_lines;
pub mod erb;
pub mod haml;
pub mod slim;

use crate::checks::{language_diagnostics, ruby_context, Doc};
use crate::lang::{html_host, is_partial, TemplateLang};
use crate::rails::project::Projects;
use crate::ruby::strict_locals::RubyContext;
use crate::util;
use std::fmt;
use tower_lsp_server::ls_types::{Diagnostic, DiagnosticSeverity, FormattingOptions, Uri};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Columns per nesting level.
    pub indent_size: usize,
    /// `Layout/TrailingEmptyLines`: end the file with exactly one line ending; off for a range, which can't change the lines.
    pub trailing_empty_lines: bool,
    /// `Layout/EmptyLines`: the longest run of blank lines kept; `None` leaves them all.
    pub max_empty_lines: Option<usize>,
    /// Move the lines inside an ERB template's comments with the line the comment opens on.
    pub align_comments: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { indent_size: 2, trailing_empty_lines: true, max_empty_lines: None, align_comments: false }
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
    /// These settings under the editor's indentation width.
    pub fn with_editor(self, options: &FormattingOptions) -> Self {
        Self { indent_size: options.tab_size as usize, ..self }
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

/// A language's re-indented text.
pub(crate) struct Indented {
    pub text: String,
    /// Per line of `text`: a blank one may be removed. False where whitespace counts: filter and text blocks, `<pre>`,
    /// continuations of a tag or of Ruby.
    pub removable: Vec<bool>,
    /// What was left alone inside an otherwise formatted file, for `--verbose`.
    pub notes: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Formatted {
    pub text: String,
    pub notes: Vec<String>,
}

/// `text` formatted, or why it is left as it is.
pub fn format(
    uri: &Uri,
    lang: TemplateLang,
    plain_erb: bool,
    text: &str,
    settings: &Settings,
    extra_engines: &[String],
) -> Result<Formatted, Skipped> {
    format_in(uri, lang, plain_erb, text, settings, extra_engines, &mut Projects::new())
}

/// `format`, taking the template's project from `projects` when a run already loaded it.
pub(crate) fn format_in(
    uri: &Uri,
    lang: TemplateLang,
    plain_erb: bool,
    text: &str,
    settings: &Settings,
    extra_engines: &[String],
    projects: &mut Projects,
) -> Result<Formatted, Skipped> {
    if uri.path().as_str().to_ascii_lowercase().ends_with(".tt") {
        return skip(SkipKind::NotHandled, "generator template: the generator's ERB is mixed in");
    }
    // The formatter refuses on errors, so it must see the ones the server shows: those of the project's Ruby.
    let ruby = ruby_context(uri, projects);
    let template = Template { uri, lang, plain_erb };
    let formatted = format_once(template, text, settings, extra_engines, &ruby)?;
    if formatted.text == text {
        return Ok(formatted);
    }
    if let Some(broken) = broken_invariant(template, text, &formatted.text, settings, extra_engines, &ruby) {
        return skip(SkipKind::Internal, broken);
    }
    Ok(formatted)
}

/// What is being formatted: where it lives, its language, and whether it was opened as plain `erb`.
#[derive(Clone, Copy)]
struct Template<'a> {
    uri: &'a Uri,
    lang: TemplateLang,
    plain_erb: bool,
}

/// Which invariant turning `text` into `formatted` breaks, if one does.
fn broken_invariant(
    template: Template,
    text: &str,
    formatted: &str,
    settings: &Settings,
    extra_engines: &[String],
    ruby: &RubyContext,
) -> Option<&'static str> {
    if !same_non_whitespace(text, formatted) {
        return Some("formatting would change more than whitespace");
    }
    if format_once(template, formatted, settings, extra_engines, ruby).map(|again| again.text).as_deref() != Ok(formatted) {
        return Some("formatting is not stable");
    }
    // The original's diagnostics matter only when the result has some, which is rare.
    let after = structure_diagnostics(template, formatted, extra_engines, ruby);
    if after.is_empty() {
        return None;
    }
    let before = structure_diagnostics(template, text, extra_engines, ruby);
    after.iter().any(|d| !before.contains(d)).then_some("formatting would add a diagnostic")
}

fn format_once(
    template: Template,
    text: &str,
    settings: &Settings,
    extra_engines: &[String],
    ruby: &RubyContext,
) -> Result<Formatted, Skipped> {
    let Template { uri, lang, plain_erb } = template;
    let indented = match lang {
        TemplateLang::Slim | TemplateLang::Skim => indent_slim(text, settings, extra_engines, ruby.grammar)?,
        TemplateLang::Erb if !html_host(uri, plain_erb, text) => return skip(SkipKind::NotHandled, "not an HTML ERB template"),
        TemplateLang::Erb => erb::indent(text, &" ".repeat(settings.indent_size), is_partial(uri), settings.align_comments, ruby)?,
        TemplateLang::Haml => indent_haml(text, settings)?,
        TemplateLang::Ruby => return skip(SkipKind::NotHandled, "Ruby views are formatted by Ruby tools"),
        TemplateLang::Liquid => return skip(SkipKind::NotHandled, "Liquid has no formatter here"),
    };
    Ok(tidy(lang, indented, text, settings, extra_engines))
}

/// The blank-line rules applied to `indented`; left out, with a note, when the compiled template would change.
fn tidy(lang: TemplateLang, indented: Indented, original: &str, settings: &Settings, extra_engines: &[String]) -> Formatted {
    let Indented { text: indented, removable, mut notes } = indented;
    let tidied = blank_lines::apply(&indented, &removable, settings, original);
    if tidied == indented || same_compilation_ignoring_lines(lang, &indented, &tidied, extra_engines) {
        return Formatted { text: tidied, notes };
    }
    notes.push("blank lines kept: removing them would change the compiled output".to_string());
    Formatted { text: indented, notes }
}

/// Whether Haml or Slim compile both texts alike, line numbers aside; ERB has no compiled form to compare.
fn same_compilation_ignoring_lines(lang: TemplateLang, before: &str, after: &str, extra_engines: &[String]) -> bool {
    match lang {
        TemplateLang::Haml => crate::haml::parser::same_compilation_ignoring_lines(before, after),
        TemplateLang::Slim | TemplateLang::Skim => crate::slim::ruby::same_compilation_ignoring_lines(before, after, extra_engines),
        _ => true,
    }
}

fn indent_slim(text: &str, settings: &Settings, extra_engines: &[String], ruby: ruby_prism::Version) -> Result<Indented, Skipped> {
    let rejected =
        crate::slim::indent::diagnostics(text, extra_engines, ruby).iter().any(|d| d.severity == Some(DiagnosticSeverity::ERROR));
    if rejected {
        return skip(SkipKind::Doubt, "Slim would reject the file");
    }
    let original = text;
    let Some(indented) = slim::format(text, settings.indent_size, extra_engines) else {
        return skip(SkipKind::Doubt, "Slim would reject the file, or its indentation can't move safely");
    };
    if !crate::slim::ruby::same_compilation(original, &indented.text, extra_engines) {
        return skip(SkipKind::Doubt, "the compiled output would change");
    }
    Ok(indented)
}

fn indent_haml(text: &str, settings: &Settings) -> Result<Indented, Skipped> {
    let Some(indented) = haml::format(text, settings.indent_size) else {
        return skip(SkipKind::Doubt, "Haml would reject the file");
    };
    if !crate::haml::parser::same_compilation(text, &indented.text) {
        return skip(SkipKind::Doubt, "the compiled output would change");
    }
    Ok(indented)
}

// ASCII only: a no-break space is content.
fn same_non_whitespace(a: &str, b: &str) -> bool {
    a.chars().filter(|c| !c.is_ascii_whitespace()).eq(b.chars().filter(|c| !c.is_ascii_whitespace()))
}

/// The language's own structure checks, as `(code, message)` with line numbers dropped, since Slim and Haml drop
/// trailing blank lines.
fn structure_diagnostics(template: Template, text: &str, extra_engines: &[String], ruby: &RubyContext) -> Vec<(String, String)> {
    let key = |d: &Diagnostic| {
        let code = util::code(d).unwrap_or_default().to_string();
        (code, d.message.chars().filter(|c| !c.is_ascii_digit()).collect())
    };
    let doc = Doc { lang: template.lang, plain_erb: template.plain_erb, percent_lines: false, text: text.to_string(), version: 0 };
    language_diagnostics(template.uri, &doc, extra_engines, ruby).iter().map(key).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(name: &str) -> Uri {
        format!("file:///app/views/users/{name}").parse::<Uri>().unwrap()
    }

    fn fmt(name: &str, text: &str, settings: &Settings) -> Result<String, Skipped> {
        let lang = TemplateLang::from_uri_or_id(&uri(name), None).unwrap();
        format(&uri(name), lang, false, text, settings, &[]).map(|f| f.text)
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
        let uri = crate::util::file_uri(&file).unwrap();
        // Rails 7.1 cuts `# locals: (a:)` out of the tag; before, the `#` comments out the rest of the line.
        let text = "<div>\n<%= render \"x\" # locals: (a:) %>\n</div>\n";
        let formatted = |actionview: &str| {
            std::fs::write(root.join("Gemfile.lock"), format!("GEM\n  specs:\n    actionview ({actionview})\n")).unwrap();
            format(&uri, TemplateLang::Erb, false, text, &Settings::default(), &[]).map(|f| f.text)
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
        assert_eq!(fmt("a.slim", "p\r\n  q\nr", &Settings::default()).as_deref(), Ok("p\r\n  q\nr\n"), "each line keeps its ending");
        assert_eq!(
            fmt("a.slim", "p  \nq\n\n\n", &Settings::default()).as_deref(),
            Ok("p\nq\n"),
            "trailing empty lines become the final newline"
        );
        assert_eq!(fmt("a.slim", "p\n\n\nq\n   \n", &Settings::default()).as_deref(), Ok("p\n\n\nq\n"), "inner blank lines stay");
        // A range leaves the final newline as it is.
        let range = Settings { trailing_empty_lines: false, ..Settings::default() };
        assert_eq!(fmt("a.slim", "p", &range).as_deref(), Ok("p"));
    }

    #[test]
    fn haml_takes_only_the_indentation_width_from_the_settings() {
        let d = Settings::default;
        assert_eq!(fmt("a.html.haml", "%div\n\t%p  \n", &d()).as_deref(), Ok("%div\n  %p\n"));
        assert_eq!(fmt("a.haml", "%div\n  %p\n", &Settings { indent_size: 3, ..d() }).as_deref(), Ok("%div\n   %p\n"));
        assert_eq!(fmt("a.haml", "%p \r\n%i\r\n\r\n  \r\n", &d()).as_deref(), Ok("%p\r\n%i\r\n"), "ending kept, trailing lines collapse");
        assert_eq!(fmt("a.haml", "%p\n\n\n%i\n", &d()).as_deref(), Ok("%p\n\n\n%i\n"));
        let range = Settings { trailing_empty_lines: false, ..d() };
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
    fn every_language_ends_with_one_line_ending() {
        let s = Settings::default();
        assert_eq!(fmt("a.html.erb", "<div>\n<p>x</p>\n</div>", &s).as_deref(), Ok("<div>\n  <p>x</p>\n</div>\n"));
        assert_eq!(fmt("a.html.erb", "<div>\n<p>x</p>\n</div>  ", &s).as_deref(), Ok("<div>\n  <p>x</p>\n</div>\n"));
        assert_eq!(fmt("a.html.erb", "<p>x</p>\n\n\n", &s).as_deref(), Ok("<p>x</p>\n"));
        assert_eq!(fmt("a.html.erb", "<p>x</p>\n\n  ", &s).as_deref(), Ok("<p>x</p>\n"));
        assert_eq!(fmt("a.html.erb", "<p>\r\n<br>", &s).as_deref(), Ok("<p>\r\n<br>\r\n"));
        assert_eq!(fmt("a.slim", "p", &s).as_deref(), Ok("p\n"));
        assert_eq!(fmt("a.slim", "p\r\nq", &s).as_deref(), Ok("p\r\nq\r\n"));
        assert_eq!(fmt("a.haml", "%p\n\n\n", &s).as_deref(), Ok("%p\n"));
        assert_eq!(fmt("a.haml", ":plain\n  a\n\n  \n", &s).as_deref(), Ok(":plain\n  a\n"));
        assert_eq!(fmt("a.slim", "div\n    | text\n\n\n        ", &s).as_deref(), Ok("div\n  | text\n"), "an editor's auto-indent");
    }

    #[test]
    fn trailing_empty_lines_can_be_turned_off() {
        let off = Settings { trailing_empty_lines: false, ..Settings::default() };
        assert_eq!(fmt("a.html.erb", "<p>\n\n\n", &off).as_deref(), Ok("<p>\n\n\n"));
        assert_eq!(fmt("a.html.erb", "<p>\n<br>", &off).as_deref(), Ok("<p>\n<br>"));
        assert_eq!(fmt("a.slim", "p\nq\n\n\n", &off).as_deref(), Ok("p\nq\n\n\n"));
        assert_eq!(fmt("a.haml", "%p\n\n \n", &off).as_deref(), Ok("%p\n\n\n"));
        assert_eq!(fmt("a.haml", "%p", &off).as_deref(), Ok("%p"));
    }

    fn collapsing(max: usize) -> Settings {
        Settings { max_empty_lines: Some(max), ..Settings::default() }
    }

    #[test]
    fn empty_lines_are_cut_to_max_in_all_three_languages() {
        assert_eq!(fmt("a.html.erb", "<h1>x</h1>\n\n\n\n<ul>\n</ul>\n", &collapsing(1)).as_deref(), Ok("<h1>x</h1>\n\n<ul>\n</ul>\n"));
        assert_eq!(fmt("a.html.erb", "\n\n<p>\n\n\n\n</p>\n", &collapsing(2)).as_deref(), Ok("\n\n<p>\n\n\n</p>\n"));
        assert_eq!(fmt("a.html.erb", "<p>\n\n\n</p>\n", &collapsing(0)).as_deref(), Ok("<p>\n</p>\n"));
        assert_eq!(fmt("a.haml", "%h1 x\n\n\n\n%ul\n", &collapsing(1)).as_deref(), Ok("%h1 x\n\n%ul\n"));
        assert_eq!(fmt("a.slim", "h1 x\n\n\n\nul\n", &collapsing(1)).as_deref(), Ok("h1 x\n\nul\n"));
        assert_eq!(fmt("a.slim", "h1 x\r\n\r\n\r\nul\r\n", &collapsing(1)).as_deref(), Ok("h1 x\r\n\r\nul\r\n"));
        assert_eq!(fmt("a.slim", "\u{feff}\n\n\nh1 x\n", &collapsing(1)).as_deref(), Ok("\u{feff}\nh1 x\n"));
        // Off by default.
        assert_eq!(fmt("a.slim", "h1 x\n\n\n\nul\n", &Settings::default()).as_deref(), Ok("h1 x\n\n\n\nul\n"));
    }

    #[test]
    fn empty_lines_are_kept_where_whitespace_counts() {
        let s = collapsing(0);
        let erb = "<pre>\na\n\n\nb</pre>\n<listing>\na\n\n\nb</listing>\n<xmp>\na\n\n\nb</xmp>\n<textarea>\n\n\n</textarea>\n<script>\na\n\n\nb\n</script>\n<style>\na\n\n\nb\n</style>\n";
        assert_eq!(fmt("a.html.erb", erb, &s).as_deref(), Ok(erb));
        let ruby = "<%= link_to(\n\n\n  x) %>\n<p title=\"a\n\n\nb\">x</p>\n<!-- a\n\n\nb -->\n<p\n\n\nid=\"x\">y</p>\n";
        assert_eq!(fmt("a.html.erb", ruby, &s).as_deref(), Ok(ruby));
        let haml = "%div\n  :plain\n    a\n\n\n    b\n  :preserve\n    c\n\n    d\n";
        assert_eq!(fmt("a.haml", haml, &s).as_deref(), Ok(haml));
        let slim = "div\n  | a\n\n\n    b\n  javascript:\n    x\n\n    y\n";
        assert_eq!(fmt("a.slim", slim, &s).as_deref(), Ok(slim));
        // A blank line after the block is not part of it.
        assert_eq!(fmt("a.slim", "div\n  | a\n\n\np\n", &s).as_deref(), Ok("div\n  | a\np\n"));
    }

    #[test]
    fn a_failed_comparison_keeps_the_indented_text_and_says_so() {
        // The mask flags a text block's blank line, as a bug in a formatter would.
        let text = "div\n    | a\n\n      b\n";
        let indented = Indented { text: "div\n  | a\n\n    b\n".to_string(), removable: vec![true; 5], notes: vec![] };
        let tidied = tidy(TemplateLang::Slim, indented, text, &collapsing(0), &[]);
        assert_eq!(tidied.text, "div\n  | a\n\n    b\n");
        assert_eq!(tidied.notes, ["blank lines kept: removing them would change the compiled output"]);
        let indented = Indented { text: "p\n\n\nq\n".to_string(), removable: vec![true; 5], notes: vec![] };
        assert_eq!(tidy(TemplateLang::Slim, indented, text, &collapsing(0), &[]).text, "p\nq\n");
    }

    #[test]
    fn a_blank_pass_that_would_change_the_compiled_output_is_refused() {
        // Flagging a text block's blank line as removable is the bug this guards against.
        let slim = "div\n  | a\n\n    b\n";
        let flagged = blank_lines::apply(slim, &[true; 5], &collapsing(0), slim);
        assert_eq!(flagged, "div\n  | a\n    b\n");
        assert!(!same_compilation_ignoring_lines(TemplateLang::Slim, slim, &flagged, &[]));
        let haml = ":plain\n  a\n\n  b\n";
        let flagged = blank_lines::apply(haml, &[true; 5], &collapsing(0), haml);
        assert!(!same_compilation_ignoring_lines(TemplateLang::Haml, haml, &flagged, &[]));
        // Removing blank lines between nodes changes only line numbers.
        assert!(same_compilation_ignoring_lines(TemplateLang::Slim, "p\n\n\nq\n", "p\nq\n", &[]));
        assert!(same_compilation_ignoring_lines(TemplateLang::Haml, "%p\n\n\n%q\n", "%p\n%q\n", &[]));
    }

    #[test]
    fn erb_takes_only_the_indentation_width_from_the_settings() {
        let three = Settings { indent_size: 3, ..Settings::default() };
        assert_eq!(fmt("a.html.erb", "<div>\n<p>x</p>\n</div>\n", &three).as_deref(), Ok("<div>\n   <p>x</p>\n</div>\n"));
        assert_eq!(fmt("a.html.erb", "<p>x</p>  \n", &Settings::default()).as_deref(), Ok("<p>x</p>\n"));
        assert_eq!(fmt("a.html.erb", "<p>\n\n\n<br>\n\n", &Settings::default()).as_deref(), Ok("<p>\n\n\n<br>\n"));
    }

    #[test]
    fn editor_tab_size_sets_the_indentation_width() {
        let project = Settings { indent_size: 4, ..Settings::default() };
        let editor = FormattingOptions { tab_size: 3, insert_spaces: false, ..Default::default() };
        assert_eq!(project.with_editor(&editor).indent_size, 3);
    }

    #[test]
    fn leaves_other_files_alone() {
        let s = Settings::default();
        assert_eq!(fmt("a.js.erb", "x", &s).map_err(|e| e.kind), Err(SkipKind::NotHandled));
        assert!(fmt("a.json.jbuilder", "json.x 1", &s).is_err());
        let template = "file:///lib/templates/slim/scaffold/index.html.slim.tt".parse::<Uri>().unwrap();
        assert!(format(&template, TemplateLang::Slim, false, "p", &s, &[]).is_err());
    }

    #[test]
    fn bare_erb_view_is_formatted_only_when_it_starts_with_markup() {
        let s = Settings::default();
        let view = "app/views/pages/about.erb";
        assert_eq!(fmt(view, "<%# c %>\n<div>\n<p>x</p>\n</div>\n", &s).as_deref(), Ok("<%# c %>\n<div>\n  <p>x</p>\n</div>\n"));
        assert_eq!(fmt(view, "Hello <%= name %>\n<div>\n<p>x</p>\n</div>\n", &s).map_err(|e| e.kind), Err(SkipKind::NotHandled));
        assert_eq!(fmt("app/views/pages/nginx.conf.erb", "<div>\n<p>x</p>\n</div>\n", &s).map_err(|e| e.kind), Err(SkipKind::NotHandled));
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
            Settings { trailing_empty_lines: false, ..d() },
            Settings { max_empty_lines: Some(0), ..d() },
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
                    let Ok(once) = format_once(Template { uri: &uri, lang, plain_erb: false }, &text, &settings, &[], &ruby) else {
                        continue;
                    };
                    formatted += 1;
                    let broken = broken_invariant(Template { uri: &uri, lang, plain_erb: false }, &text, &once.text, &settings, &[], &ruby);
                    assert_eq!(broken, None, "{name} with {settings:?}:\n{text}\n---\n{}", once.text);
                }
            }
        }
        assert!(formatted >= 50, "only {formatted} sample variants were formatted");
    }

    #[test]
    fn broken_fixtures_come_back_unchanged() {
        for (name, text) in &SAMPLES[7..] {
            let uri = uri(name);
            let lang = TemplateLang::from_uri_or_id(&uri, None).unwrap();
            let kind = format(&uri, lang, false, text, &Settings::default(), &[]).map(|f| f.text);
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
            let Some(uri) = crate::util::file_uri(entry.path()) else { continue };
            let (Some(lang), Ok(sample)) = (TemplateLang::from_uri_or_id(&uri, None), std::fs::read_to_string(entry.path())) else {
                continue;
            };
            files += 1;
            for (v, text) in variants(&sample).into_iter().enumerate() {
                for (s, settings) in settings_variants().into_iter().enumerate() {
                    match format_once(Template { uri: &uri, lang, plain_erb: false }, &text, &settings, &[], &ruby) {
                        Ok(once) => {
                            formatted += 1;
                            if let Some(broken) =
                                broken_invariant(Template { uri: &uri, lang, plain_erb: false }, &text, &once.text, &settings, &[], &ruby)
                            {
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
