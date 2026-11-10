//! Whitespace-only formatting of HTML ERB, Haml and Slim: indentation, trailing whitespace, blank lines and the final
//! newline. Anything else stays byte for byte; a file whose structure is in doubt is left unchanged, with the reason.

mod attributes_per_line;
pub(crate) mod blank_lines;
mod element_content;
mod element_style;
pub mod erb;
pub mod haml;
mod inline;
pub mod slim;
mod tag_style;
mod turbo_attributes;

use crate::checks::{language_diagnostics, ruby_context, Doc};
use crate::lang::{html_host, is_partial, slim_html_host, TemplateLang};
use crate::rails::project::Projects;
use crate::ruby::strict_locals::RubyContext;
use crate::util;
use blank_lines::LineInfo;
use std::borrow::Cow;
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
    /// `Layout/EmptyLinesAroundBlockBody`; `None` leaves those blank lines.
    pub around_block_body: Option<Style>,
    /// `Layout/EmptyLinesAroundElementBody`.
    pub around_element_body: Option<Style>,
    /// `Layout/EmptyLinesAroundBranches`.
    pub around_branches: Option<Style>,
    /// `Layout/SpaceInsideErbTag`.
    pub space_inside_erb_tag: bool,
    /// `Layout/SpaceAfterScriptMarker`.
    pub space_after_script_marker: bool,
    /// `Style/ErbCommentSyntax`.
    pub erb_comment_syntax: bool,
    /// `Layout/SpaceInHtmlTag`.
    pub space_in_html_tag: bool,
    /// `Layout/IndentationStyle`: what an ERB template's indent level is written with.
    pub indent_style: IndentStyle,
    /// `Layout/TrailingWhitespace`: strip spaces and tabs at line ends where they don't count.
    pub trailing_whitespace: bool,
    /// `Turbo/PreferTurboAttributes`: the one rule that changes the page; the others keep what the template compiles to.
    pub prefer_turbo_attributes: bool,
    /// `Style/AttributeQuotes`; `None` leaves the quotes.
    pub attribute_quotes: Option<Quote>,
    /// `Style/BooleanAttributes`.
    pub boolean_attributes: bool,
    /// `Style/VoidTagClosing`; `None` leaves the tags.
    pub void_tag_closing: Option<VoidClosing>,
    /// `Style/TagNameCase`.
    pub tag_name_case: bool,
    /// `Style/ImplicitDiv`.
    pub implicit_div: bool,
    /// `Style/ClassesBeforeIds`.
    pub classes_before_ids: bool,
    /// `Style/HashAttributes`; `None` leaves the attributes.
    pub hash_attributes: Option<AttributeStyle>,
    /// `Layout/ElementContentOnOwnLine`: the lower-case names of the elements whose content moves to its own line; `None` leaves it.
    pub content_on_own_line: Option<Vec<String>>,
    /// `Layout/AttributesPerLine`: the longest line a start tag may stay on; `None` leaves the tags.
    pub attributes_per_line: Option<usize>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            indent_size: 2,
            trailing_empty_lines: true,
            max_empty_lines: None,
            align_comments: false,
            around_block_body: None,
            around_element_body: None,
            around_branches: None,
            space_inside_erb_tag: false,
            space_after_script_marker: false,
            erb_comment_syntax: false,
            space_in_html_tag: false,
            indent_style: IndentStyle::Spaces,
            trailing_whitespace: true,
            prefer_turbo_attributes: false,
            attribute_quotes: None,
            boolean_attributes: false,
            void_tag_closing: None,
            tag_name_case: false,
            implicit_div: false,
            classes_before_ids: false,
            hash_attributes: None,
            content_on_own_line: None,
            attributes_per_line: None,
        }
    }
}

/// What one indent level is made of: `indent_size` spaces, or a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentStyle {
    Spaces,
    Tabs,
}

/// The quote an attribute value is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quote {
    Double,
    Single,
}

/// How a Haml tag's attributes are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeStyle {
    /// `%a{href: "/"}`
    RubyHash,
    /// `%a(href="/")`
    Html,
}

/// How a void element's start tag ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoidClosing {
    /// `<br>`
    NoSlash,
    /// `<br />`
    Slash,
}

/// How many blank lines the `EmptyLinesAround…` rules leave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    NoEmptyLines,
    /// Exactly one.
    EmptyLines,
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
    /// These settings under the editor's indentation width and style.
    pub fn with_editor(self, options: &FormattingOptions) -> Self {
        let indent_style = if options.insert_spaces { IndentStyle::Spaces } else { IndentStyle::Tabs };
        Self { indent_size: options.tab_size as usize, indent_style, ..self }
    }

    /// The text of one indent level.
    fn indent_unit(&self) -> String {
        match self.indent_style {
            IndentStyle::Spaces => " ".repeat(self.indent_size),
            IndentStyle::Tabs => "\t".to_string(),
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

/// A language's re-indented text.
pub(crate) struct Indented {
    pub text: String,
    /// What each line of `text` is to the blank-line rules.
    pub lines: Vec<LineInfo>,
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
    // The rewrites of attributes and tags come first, so every check below compares the result with their text and still guards the rest.
    let mut notes = Vec::new();
    let mut source = Cow::Borrowed(text);
    if settings.prefer_turbo_attributes {
        keep(turbo_attributes::apply(lang, &source, extra_engines, ruby.grammar), &mut source, &mut notes);
    }
    if tag_style::is_on(settings) {
        let html = match lang {
            TemplateLang::Slim | TemplateLang::Skim => slim_html_host(uri),
            TemplateLang::Erb => html_host(uri, plain_erb, &source),
            _ => true,
        };
        keep(tag_style::apply(lang, &source, settings, extra_engines, html), &mut source, &mut notes);
    }
    if element_style::heads_are_on(settings) {
        keep(element_style::heads(lang, &source, settings, extra_engines, ruby.grammar), &mut source, &mut notes);
    }
    if lang == TemplateLang::Haml {
        keep(element_style::attributes(&source, settings, ruby.grammar), &mut source, &mut notes);
    }
    keep(element_content::apply(lang, &source, settings, extra_engines), &mut source, &mut notes);
    if attributes_per_line::is_on(lang, settings) {
        // Lines are measured where the re-indenter puts them; a file it refuses gets its note from the run below.
        if let Ok(indented) = format_once(template, &source, settings, extra_engines, &ruby) {
            keep(attributes_per_line::apply(&indented.text, settings), &mut source, &mut notes);
        }
    }
    let mut formatted = format_once(template, &source, settings, extra_engines, &ruby)?;
    formatted.notes.extend(notes);
    if formatted.text == *source {
        return Ok(formatted);
    }
    if let Some(broken) = broken_invariant(template, &source, &formatted.text, settings, extra_engines, &ruby) {
        return skip(SkipKind::Internal, broken);
    }
    Ok(formatted)
}

/// Takes a rewrite stage's result as the text the rest works on, or its note when the stage refused.
fn keep(stage: Result<Option<String>, String>, source: &mut Cow<str>, notes: &mut Vec<String>) {
    match stage {
        Ok(Some(rewritten)) => *source = Cow::Owned(rewritten),
        Ok(None) => {}
        Err(note) => notes.push(note),
    }
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

/// `text` with the in-line rules applied first, then re-indented; without them, with a note, when they would change the
/// template's meaning.
fn format_once(
    template: Template,
    text: &str,
    settings: &Settings,
    extra_engines: &[String],
    ruby: &RubyContext,
) -> Result<Formatted, Skipped> {
    let staged = inline::apply(template.lang, text, settings, extra_engines);
    format_staged(template, text, staged, settings, extra_engines, ruby)
}

/// `format_once` with the in-line rules' result given. The Haml/Slim comparisons chain: `staged` against `text` here,
/// then the re-indented text against `staged`, and the blank-line pass against that, so the output is compared with the
/// input throughout.
fn format_staged(
    template: Template,
    text: &str,
    staged: Option<String>,
    settings: &Settings,
    extra_engines: &[String],
    ruby: &RubyContext,
) -> Result<Formatted, Skipped> {
    let Some(staged) = staged else { return format_stages(template, text, settings, extra_engines, ruby) };
    let formatted = format_stages(template, &staged, settings, extra_engines, ruby)
        .ok()
        .filter(|formatted| inline::keeps_meaning(template.lang, text, &staged, &formatted.text, extra_engines));
    if let Some(formatted) = formatted {
        return Ok(formatted);
    }
    let mut formatted = format_stages(template, text, settings, extra_engines, ruby)?;
    formatted.notes.push("in-line spacing kept: changing it would change the compiled output".to_string());
    Ok(formatted)
}

fn format_stages(
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
        TemplateLang::Erb => erb::indent(text, settings, is_partial(uri), ruby)?,
        TemplateLang::Haml => indent_haml(text, settings)?,
        TemplateLang::Ruby => return skip(SkipKind::NotHandled, "Ruby views are formatted by Ruby tools"),
        TemplateLang::Liquid => return skip(SkipKind::NotHandled, "Liquid has no formatter here"),
    };
    Ok(tidy(lang, indented, text, settings, extra_engines))
}

/// The blank-line rules applied to `indented`; left out, with a note, when removing or adding lines would change the
/// compiled template.
fn tidy(lang: TemplateLang, indented: Indented, original: &str, settings: &Settings, extra_engines: &[String]) -> Formatted {
    let Indented { text: indented, lines, mut notes } = indented;
    let tidied = blank_lines::apply(&indented, &lines, settings, original);
    if tidied == indented || same_compilation_ignoring_lines(lang, &indented, &tidied, extra_engines) {
        return Formatted { text: tidied, notes };
    }
    notes.push("blank lines kept: changing them would change the compiled output".to_string());
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
    let Some(indented) = slim::format(text, settings, extra_engines) else {
        return skip(SkipKind::Doubt, "Slim would reject the file, or its indentation can't move safely");
    };
    if !crate::slim::ruby::same_compilation(original, &indented.text, extra_engines) {
        return skip(SkipKind::Doubt, "the compiled output would change");
    }
    Ok(indented)
}

fn indent_haml(text: &str, settings: &Settings) -> Result<Indented, Skipped> {
    let Some(indented) = haml::format(text, settings) else {
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
    language_diagnostics(template.uri, &doc, extra_engines, ruby, false).iter().map(key).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use blank_lines::tests::removable;
    use blank_lines::{Body, LineRole};

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
        let indented = Indented { text: "div\n  | a\n\n    b\n".to_string(), lines: removable(5), notes: vec![] };
        let tidied = tidy(TemplateLang::Slim, indented, text, &collapsing(0), &[]);
        assert_eq!(tidied.text, "div\n  | a\n\n    b\n");
        assert_eq!(tidied.notes, ["blank lines kept: changing them would change the compiled output"]);
        let indented = Indented { text: "p\n\n\nq\n".to_string(), lines: removable(5), notes: vec![] };
        assert_eq!(tidy(TemplateLang::Slim, indented, text, &collapsing(0), &[]).text, "p\nq\n");
    }

    fn around(style: Style) -> Settings {
        Settings { around_block_body: Some(style), around_element_body: Some(style), around_branches: Some(style), ..Settings::default() }
    }

    #[test]
    fn the_rules_around_bodies_and_branches_work_in_all_three_languages() {
        let (no, one) = (around(Style::NoEmptyLines), around(Style::EmptyLines));
        let erb = "<div>\n\n<% if a %>\n\n<p>x</p>\n\n<% else %>\n<p>y</p>\n<% end %>\n\n</div>\n";
        assert_eq!(
            fmt("a.html.erb", erb, &no).as_deref(),
            Ok("<div>\n  <% if a %>\n    <p>x</p>\n  <% else %>\n    <p>y</p>\n  <% end %>\n</div>\n")
        );
        let erb_one = "<div>\n\n  <% if a %>\n\n    <p>x</p>\n\n  <% else %>\n\n    <p>y</p>\n\n  <% end %>\n\n</div>\n";
        assert_eq!(fmt("a.html.erb", erb, &one).as_deref(), Ok(erb_one));
        assert_eq!(fmt("a.html.erb", erb_one, &one).as_deref(), Ok(erb_one), "stable");
        let haml = "%div\n\n    - if a\n\n        %p x\n\n    - else\n        %p y\n\n%footer\n";
        assert_eq!(fmt("a.haml", haml, &no).as_deref(), Ok("%div\n  - if a\n    %p x\n  - else\n    %p y\n\n%footer\n"));
        let haml_one = "%div\n\n  - if a\n\n    %p x\n\n  - else\n\n    %p y\n\n%footer\n";
        assert_eq!(fmt("a.haml", haml, &one).as_deref(), Ok(haml_one));
        let slim = "div\n\n    - if a\n\n        p x\n\n    - else\n        p y\n\nfooter\n";
        assert_eq!(fmt("a.slim", slim, &no).as_deref(), Ok("div\n  - if a\n    p x\n  - else\n    p y\n\nfooter\n"));
        let slim_one = "div\n\n  - if a\n\n    p x\n\n  - else\n\n    p y\n\nfooter\n";
        assert_eq!(fmt("a.slim", slim, &one).as_deref(), Ok(slim_one));
        assert_eq!(fmt("a.slim", slim_one, &one).as_deref(), Ok(slim_one), "stable");
        // Off by default, and `max` has the last word.
        assert_eq!(fmt("a.slim", "div\n\n  p x\n", &Settings::default()).as_deref(), Ok("div\n\n  p x\n"));
        let max_zero = Settings { max_empty_lines: Some(0), ..one };
        assert_eq!(fmt("a.slim", "div\n  p x\n", &max_zero).as_deref(), Ok("div\n  p x\n"));
    }

    #[test]
    fn a_filter_before_a_node_does_not_stop_the_blank_lines_around_it() {
        let haml = "%div\n  :plain\n    a\n  %p x\n";
        assert_eq!(fmt("a.haml", haml, &around(Style::EmptyLines)).as_deref(), Ok("%div\n\n  :plain\n    a\n  %p x\n"));
    }

    #[test]
    fn a_blank_line_added_inside_a_text_block_is_refused() {
        // The line info puts `b`, which is text, among the nodes, as a bug in a formatter would.
        let text = "div\n  | a\n    b\n";
        let node = |depth, role| LineInfo { depth: Some(depth), role, ..LineInfo::default() };
        let opens = LineRole::Opens(Body::Element);
        let lines = vec![node(0, opens), node(1, opens), node(2, LineRole::Other)];
        let indented = Indented { text: text.to_string(), lines, notes: vec![] };
        let tidied = tidy(TemplateLang::Slim, indented, text, &around(Style::EmptyLines), &[]);
        assert_eq!(tidied.text, text);
        assert_eq!(tidied.notes, ["blank lines kept: changing them would change the compiled output"]);
        // The same bug on Haml, in a filter.
        let haml = "%div\n  :plain\n    a\n    b\n";
        let filter = LineInfo { tail: 1, ..node(1, opens) };
        let lines = vec![node(0, opens), filter, LineInfo::default(), node(2, LineRole::Other)];
        let indented = Indented { text: haml.to_string(), lines, notes: vec![] };
        let tidied = tidy(TemplateLang::Haml, indented, haml, &around(Style::EmptyLines), &[]);
        assert_eq!(tidied.text, haml);
        assert_eq!(tidied.notes.len(), 1);
    }

    #[test]
    fn a_blank_pass_that_would_change_the_compiled_output_is_refused() {
        // Flagging a text block's blank line as removable is the bug this guards against.
        let slim = "div\n  | a\n\n    b\n";
        let flagged = blank_lines::apply(slim, &removable(5), &collapsing(0), slim);
        assert_eq!(flagged, "div\n  | a\n    b\n");
        assert!(!same_compilation_ignoring_lines(TemplateLang::Slim, slim, &flagged, &[]));
        let haml = ":plain\n  a\n\n  b\n";
        let flagged = blank_lines::apply(haml, &removable(5), &collapsing(0), haml);
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
    fn editor_insert_spaces_sets_the_indentation_style() {
        let editor = |insert_spaces| FormattingOptions { tab_size: 2, insert_spaces, ..Default::default() };
        let tabbed = Settings { indent_style: IndentStyle::Tabs, ..Settings::default() };
        assert_eq!(tabbed.clone().with_editor(&editor(true)).indent_style, IndentStyle::Spaces, "the editor beats .editorconfig");
        assert_eq!(Settings::default().with_editor(&editor(false)).indent_style, IndentStyle::Tabs);
    }

    fn tabs() -> Settings {
        Settings { indent_style: IndentStyle::Tabs, ..Settings::default() }
    }

    fn erb(text: &str, settings: &Settings) -> String {
        let once = fmt("a.html.erb", text, settings).unwrap();
        assert_eq!(fmt("a.html.erb", &once, settings).as_deref(), Ok(once.as_str()), "idempotent");
        once
    }

    #[test]
    fn erb_tabs_replace_the_levels_the_indenter_writes() {
        let text = "<ul>\n<% @users.each do |user| %>\n<li>\n<%= user.name %>\n</li>\n<% end %>\n</ul>\n";
        let want = "<ul>\n\t<% @users.each do |user| %>\n\t\t<li>\n\t\t\t<%= user.name %>\n\t\t</li>\n\t<% end %>\n</ul>\n";
        assert_eq!(erb(text, &tabs()), want);
        let four = Settings { indent_size: 4, ..tabs() };
        assert_eq!(erb(text, &four), want, "the width doesn't matter with tabs");
        assert_eq!(
            erb(want, &Settings::default()),
            "<ul>\n  <% @users.each do |user| %>\n    <li>\n      <%= user.name %>\n    </li>\n  <% end %>\n</ul>\n"
        );
    }

    #[test]
    fn erb_tabs_leave_fixed_regions_and_inner_text_alone() {
        let text = "<div>\n  <pre>\n    a  b\n  </pre>\n  <script>\n    var x = 1;\n      y();\n  </script>\n  <p class=\"a\n     b\">x</p>\n  <%# note\n    more %>\n  <% foo(1,\n      2) %>\n</div>\n";
        let want = "<div>\n\t<pre>\n    a  b\n  </pre>\n\t<script>\n    var x = 1;\n      y();\n  </script>\n\t<p class=\"a\n     b\">x</p>\n\t<%# note\n    more %>\n\t<% foo(1,\n      2) %>\n</div>\n";
        assert_eq!(erb(text, &tabs()), want);
    }

    #[test]
    fn erb_tabs_move_a_multi_line_tag_with_its_owner_keeping_the_relative_part() {
        let text = "<div>\n<a href=\"/x\"\n   class=\"y\">z</a>\n</div>\n";
        assert_eq!(erb(text, &tabs()), "<div>\n\t<a href=\"/x\"\n\t   class=\"y\">z</a>\n</div>\n");
    }

    #[test]
    fn erb_tabs_and_crlf() {
        assert_eq!(erb("<div>\r\n<p>\r\nx\r\n</p>\r\n</div>\r\n", &tabs()), "<div>\r\n\t<p>\r\n\t\tx\r\n\t</p>\r\n</div>\r\n");
    }

    #[test]
    fn haml_and_slim_ignore_the_indentation_style() {
        assert_eq!(fmt("a.html.haml", "%div\n    %p\n", &tabs()).as_deref(), Ok("%div\n  %p\n"));
        assert_eq!(fmt("a.html.slim", "div\n    p\n", &tabs()).as_deref(), Ok("div\n  p\n"));
    }

    #[test]
    fn trailing_whitespace_off_keeps_every_line_end_as_written() {
        let off = Settings { trailing_whitespace: false, ..Settings::default() };
        for (name, text, trimmed) in [
            ("a.html.erb", "<div>  \n<p>x \t\n  \n</p>\n</div>\n", "<div>\n  <p>x\n\n  </p>\n</div>\n"),
            ("a.html.haml", "%div  \n    %p x \t\n  \n    %b\n", "%div\n  %p x\n\n  %b\n"),
            ("a.html.slim", "div  \n    p \t\n  \n    b\n", "div\n  p\n\n  b\n"),
        ] {
            assert_eq!(fmt(name, text, &Settings::default()).as_deref(), Ok(trimmed), "{name} on");
            let kept = fmt(name, text, &off).unwrap();
            assert!(kept.contains("  \n") && kept.contains(" \t\n"), "{name} off: {kept:?}");
            assert_eq!(fmt(name, &kept, &off).as_deref(), Ok(kept.as_str()), "{name} off, idempotent");
        }
        for (name, text) in [
            ("a.html.erb", "<div>\n \t\n    \n  <p>x</p>\n</div>\n"),
            ("a.html.haml", "%div\n \t\n    \n  %p x\n"),
            ("a.html.slim", "div\n \t\n    \n  p x\n"),
        ] {
            assert_eq!(fmt(name, text, &off).as_deref(), Ok(text), "{name}: whitespace-only lines kept exactly");
            let on = text.replace(" \t\n    \n", "\n\n");
            assert_eq!(fmt(name, text, &Settings::default()).as_deref(), Ok(on.as_str()), "{name}: emptied when on");
        }
        assert_eq!(fmt("a.html.erb", "<div>  \n<p>x \t\n</p>\n</div>\n", &off).as_deref(), Ok("<div>  \n  <p>x \t\n  </p>\n</div>\n"));
        assert_eq!(fmt("a.html.haml", "%div  \n    %p x \t\n", &off).as_deref(), Ok("%div  \n  %p x \t\n"));
        assert_eq!(fmt("a.html.slim", "div  \n    p \t\n", &off).as_deref(), Ok("div  \n  p \t\n"));
    }

    #[test]
    fn the_inline_rules_run_before_the_indentation_in_all_three_languages() {
        let s = inline();
        assert_eq!(
            fmt("a.html.erb", "<div>\n<%if a%>\n<% # note %>\n<p><%=x%></p>\n<%end%>\n</div>\n", &s).as_deref(),
            Ok("<div>\n  <% if a %>\n    <%# note %>\n    <p><%= x %></p>\n  <% end %>\n</div>\n")
        );
        let haml = "%div\n    -if a\n        %a(href=\"x=y\")=z\n        =render \"u\"\n        \\=text\n";
        assert_eq!(
            fmt("a.html.haml", haml, &s).as_deref(),
            Ok("%div\n  - if a\n    %a(href=\"x=y\")= z\n    = render \"u\"\n    \\=text\n")
        );
        let slim = "div\n    -if a\n        a href=\"x=y\"=z\n        =render \"u\"\n        | =text\n";
        assert_eq!(fmt("a.html.slim", slim, &s).as_deref(), Ok("div\n  - if a\n    a href=\"x=y\"= z\n    = render \"u\"\n    | =text\n"));
        // Off by default, and stable.
        assert_eq!(fmt("a.html.slim", "p=x\n", &Settings::default()).as_deref(), Ok("p=x\n"));
        for (name, text) in [("a.html.erb", "<%=x%><% # a %>\n"), ("a.haml", "=x\n%p=y\n"), ("a.slim", "=x\np=y\n")] {
            let once = fmt(name, text, &s).unwrap();
            assert_eq!(fmt(name, &once, &s).as_deref(), Ok(once.as_str()), "{name}");
        }
    }

    #[test]
    fn the_inline_rules_work_with_the_settings_of_a_range() {
        let range = Settings { trailing_empty_lines: false, max_empty_lines: None, ..inline() };
        assert_eq!(fmt("a.slim", "p=x", &range).as_deref(), Ok("p= x"));
        assert_eq!(fmt("a.html.erb", "<%=x%>", &range).as_deref(), Ok("<%= x %>"));
        assert_eq!(fmt("a.html.erb", "<p  a = \"1\" >", &range).as_deref(), Ok("<p a=\"1\">"));
    }

    #[test]
    fn inline_edits_that_change_the_template_are_dropped_with_a_note() {
        let erb_uri = uri("a.html.erb");
        let note = "in-line spacing kept: changing it would change the compiled output";
        let ruby = RubyContext::outside_project();
        let template = Template { uri: &erb_uri, lang: TemplateLang::Erb, plain_erb: false };
        let text = "<div>\n<%= a %>\n</div>\n";
        // A stage that changed a tag's Ruby, as a bug in an edit function would.
        let formatted = format_staged(template, text, Some("<div>\n<%= b %>\n</div>\n".to_string()), &inline(), &[], &ruby).unwrap();
        assert_eq!((formatted.text.as_str(), formatted.notes.as_slice()), ("<div>\n  <%= a %>\n</div>\n", [note.to_string()].as_slice()));
        let spaced = Some("<div>\n<%= a %>\n</div>\n".to_string());
        assert!(format_staged(template, text, spaced, &inline(), &[], &ruby).unwrap().notes.is_empty());
        // Haml and Slim: spaces that change a text line.
        for (name, lang, text, staged) in [
            ("a.haml", TemplateLang::Haml, "%p a b\n=x\n", "%p a  b\n=x\n"),
            ("a.haml", TemplateLang::Haml, "%p a\n=x\n", "%p a\n= y\n"),
            ("a.slim", TemplateLang::Slim, "p a b\n=x\n", "p a  b\n=x\n"),
        ] {
            let uri = uri(name);
            let template = Template { uri: &uri, lang, plain_erb: false };
            let formatted = format_staged(template, text, Some(staged.to_string()), &inline(), &[], &ruby).unwrap();
            assert_eq!((formatted.text.as_str(), formatted.notes.as_slice()), (text, [note.to_string()].as_slice()), "{name} {text:?}");
        }
    }

    #[test]
    fn turbo_attributes_are_renamed_first_and_only_when_asked() {
        let on = Settings { prefer_turbo_attributes: true, ..Settings::default() };
        for (name, text, want) in [
            (
                "a.html.erb",
                "<div>\n<a href=\"/x\" data-method=\"delete\" data-confirm=\"Sure?\">x</a>\n</div>\n",
                "<div>\n  <a href=\"/x\" data-turbo-method=\"delete\" data-turbo-confirm=\"Sure?\">x</a>\n</div>\n",
            ),
            (
                "a.html.haml",
                "%div\n    %a{data: {confirm: \"s\", method: :delete}} x\n",
                "%div\n  %a{data: {turbo_confirm: \"s\", turbo_method: :delete}} x\n",
            ),
            ("a.html.slim", "div\n    button data-disable-with=\"s\" x\n", "div\n  button data-turbo-submits-with=\"s\" x\n"),
        ] {
            assert_eq!(fmt(name, text, &on).as_deref(), Ok(want), "{name}");
            assert_eq!(fmt(name, want, &on).as_deref(), Ok(want), "{name}: stable");
            let off = fmt(name, text, &Settings::default()).unwrap();
            assert!(!off.contains("turbo"), "{name}: off by default: {off}");
        }
        // The other checks still guard the result: a file the formatter refuses keeps its attributes.
        let broken = "%div\n    %a{data: {confirm: 1, method: 2}} x\n  %b\n";
        assert_eq!(fmt("a.html.haml", broken, &on).map_err(|e| e.kind), Err(SkipKind::Doubt));
        // The renames compose with the other in-line rules.
        let both = Settings { space_in_html_tag: true, ..on };
        assert_eq!(
            fmt("a.html.erb", "<a  data-method=\"post\"  href=\"/x\" >x</a>\n", &both).as_deref(),
            Ok("<a data-turbo-method=\"post\" href=\"/x\">x</a>\n")
        );
    }

    #[test]
    fn element_style_rules_compose_with_the_tag_and_layout_rules() {
        let on = Settings {
            implicit_div: true,
            classes_before_ids: true,
            hash_attributes: Some(AttributeStyle::Html),
            tag_name_case: true,
            max_empty_lines: Some(0),
            ..Settings::default()
        };
        let haml = "%div#main.wide{role: \"main\"}\n\n\n    %div#a.b\n        %a{href: \"/x\", title: @t} x\n";
        let want = ".wide#main(role=\"main\")\n  .b#a\n    %a(href=\"/x\" title=@t) x\n";
        assert_eq!(fmt("a.html.haml", haml, &on).as_deref(), Ok(want));
        assert_eq!(fmt("a.html.haml", want, &on).as_deref(), Ok(want), "stable");
        let slim = "DIV#main.wide\n\n\n    div.card Hi\n    DIV.card\n";
        assert_eq!(fmt("a.html.slim", slim, &on).as_deref(), Ok("#main.wide\n  .card Hi\n  .card\n"));
        let off = Settings::default();
        assert_eq!(fmt("a.html.haml", ".b#a{x: 1}\n%div.c\n", &off).as_deref(), Ok(".b#a{x: 1}\n%div.c\n"), "off by default");
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

    fn inline() -> Settings {
        Settings {
            space_inside_erb_tag: true,
            space_after_script_marker: true,
            erb_comment_syntax: true,
            space_in_html_tag: true,
            ..Settings::default()
        }
    }

    fn settings_variants() -> Vec<Settings> {
        let d = Settings::default;
        vec![
            d(),
            inline(),
            Settings { max_empty_lines: Some(0), align_comments: true, ..inline() },
            Settings { indent_size: 4, ..d() },
            Settings { trailing_empty_lines: false, ..d() },
            Settings { max_empty_lines: Some(0), ..d() },
            around(Style::NoEmptyLines),
            around(Style::EmptyLines),
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
                    assert!(!once.notes.iter().any(|n| n.starts_with("in-line")), "{name} with {settings:?}: {:?}", once.notes);
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
