//! The diagnostics of a template: its language's own checks plus the `render` checks against app/views.

use crate::code_context::render_filter;
use crate::lang::{html_host, is_partial, slim_html_host, TemplateLang};
use crate::{
    erb, haml_indent, haml_parser, haml_ruby, html_lint, partials,
    project::{Framework, Project, Projects},
    rails_paths, slim_indent, strict_locals, tags, util,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{Diagnostic, Url};

#[derive(Clone)]
pub(crate) struct Doc {
    pub lang: TemplateLang,
    pub text: String,
    pub version: i32,
}

/// `initializationOptions`, from the `rubyTemplates.{slim,erb,haml,jbuilder}.*` settings.
#[derive(Default, Debug, PartialEq)]
pub(crate) struct Options {
    pub extra_engines: Vec<String>,
    disabled_slim: Vec<String>,
    disabled_erb: Vec<String>,
    disabled_haml: Vec<String>,
    disabled_jbuilder: Vec<String>,
    diagnostics_off: Vec<TemplateLang>,
    format_off: Vec<TemplateLang>,
    /// `erb.format.alignComments`.
    pub align_erb_comments: bool,
}

/// The settings sections that switch diagnostics and formatting on or off, and the languages each covers.
const SWITCHED_SECTIONS: [(&str, &[TemplateLang]); 3] =
    [("erb", &[TemplateLang::Erb]), ("haml", &[TemplateLang::Haml]), ("slim", &[TemplateLang::Slim, TemplateLang::Skim])];

impl Options {
    /// Reads each field on its own so one malformed field keeps the others; returns what was ignored.
    pub(crate) fn from_init(init: Option<&serde_json::Value>) -> (Self, Vec<String>) {
        let mut ignored = Vec::new();
        let mut strings = |section: &str, name: &str| -> Vec<String> {
            let Some(value) = init.and_then(|v| v.get(section)?.get(name)) else {
                return vec![];
            };
            let Some(items) = value.as_array() else {
                ignored.push(format!("{section}.{name}: expected an array, got {value}"));
                return vec![];
            };
            items
                .iter()
                .filter_map(|item| {
                    let s = item.as_str().map(String::from);
                    if s.is_none() {
                        ignored.push(format!("{section}.{name}: expected a string, got {item}"));
                    }
                    s
                })
                .collect()
        };
        let mut options = Self {
            extra_engines: strings("slim", "extraEngines"),
            disabled_slim: strings("slim", "disabledDiagnostics"),
            disabled_erb: strings("erb", "disabledDiagnostics"),
            disabled_haml: strings("haml", "disabledDiagnostics"),
            disabled_jbuilder: strings("jbuilder", "disabledDiagnostics"),
            ..Self::default()
        };
        for (section, langs) in SWITCHED_SECTIONS {
            for (group, off) in [("diagnostics", &mut options.diagnostics_off), ("format", &mut options.format_off)] {
                let Some(value) = init.and_then(|v| v.get(section)?.get(group)?.get("enabled")) else { continue };
                match value.as_bool() {
                    Some(false) => off.extend(langs),
                    Some(true) => {}
                    None => ignored.push(format!("{section}.{group}.enabled: expected a boolean, got {value}")),
                }
            }
        }
        if let Some(value) = init.and_then(|v| v.get("erb")?.get("format")?.get("alignComments")) {
            match value.as_bool() {
                Some(align) => options.align_erb_comments = align,
                None => ignored.push(format!("erb.format.alignComments: expected a boolean, got {value}")),
            }
        }
        (options, ignored)
    }

    fn diagnostics_enabled(&self, lang: TemplateLang) -> bool {
        !self.diagnostics_off.contains(&lang)
    }

    pub(crate) fn format_enabled(&self, lang: TemplateLang) -> bool {
        !self.format_off.contains(&lang)
    }

    /// Diagnostic codes turned off for `lang`; Skim shares Slim's setting.
    fn disabled(&self, lang: TemplateLang) -> &[String] {
        match lang {
            TemplateLang::Slim | TemplateLang::Skim => &self.disabled_slim,
            TemplateLang::Erb => &self.disabled_erb,
            TemplateLang::Haml => &self.disabled_haml,
            TemplateLang::Ruby => &self.disabled_jbuilder,
        }
    }
}

pub(crate) fn diagnostics(uri: &Url, doc: &Doc, options: &Options) -> Vec<Diagnostic> {
    diagnostics_in(uri, doc, options, &mut Projects::new())
}

/// `diagnostics`, taking the template's project from `projects` when a run already loaded it.
pub(crate) fn diagnostics_in(uri: &Url, doc: &Doc, options: &Options, projects: &mut Projects) -> Vec<Diagnostic> {
    // Thor templates (`*.html.erb.tt`, `*.haml.tt`) mix the generator's ERB with the template's own tags or
    // lines, and their partials live outside app/views, so no check applies.
    if uri.path().to_ascii_lowercase().ends_with(".tt") || !options.diagnostics_enabled(doc.lang) {
        return vec![];
    }
    let path = uri.to_file_path().ok();
    let project = path.as_deref().map(|path| Project::memoized(projects, path));
    let ruby = path.as_deref().zip(project).map_or_else(RubyContext::outside_project, |(path, project)| RubyContext::of(path, project));
    let mut out = language_diagnostics(uri, doc.lang, &doc.text, &options.extra_engines, &ruby);
    if let (Some(path), Some(project)) = (&path, project) {
        out.extend(render_diagnostics(path, project, &doc.text, render_filter(doc.lang, &doc.text, &options.extra_engines)));
    }
    let disabled = options.disabled(doc.lang);
    out.retain(|d| !util::code(d).is_some_and(|c| disabled.iter().any(|off| off == c)));
    for d in &mut out {
        d.source = Some(doc.lang.diagnostic_source().into());
    }
    out
}

/// How Rails evals a template: the Prism grammar of its project's Ruby, and strict locals where they exist (Rails 7.1
/// or newer). Without them the `locals:` line is a plain comment.
pub(crate) struct RubyContext {
    pub grammar: ruby_prism::Version,
    pub strict_locals: Option<strict_locals::Rails>,
}

impl RubyContext {
    pub(crate) fn outside_project() -> Self {
        RubyContext { grammar: ruby_prism::Version::Latest, strict_locals: None }
    }

    fn of(path: &Path, project: &Project) -> Self {
        RubyContext {
            grammar: project.prism_version(),
            strict_locals: supports_strict_locals(project)
                .then(|| project.short_identifier(path))
                .flatten()
                .map(|identifier| strict_locals::Rails { close: strict_locals::Close::of(project), identifier }),
        }
    }
}

/// The context of the template at `uri`: its project's, the latest grammar and no strict locals outside any.
pub(crate) fn ruby_context(uri: &Url, projects: &mut Projects) -> RubyContext {
    uri.to_file_path().map_or_else(|_| RubyContext::outside_project(), |path| RubyContext::of(&path, Project::memoized(projects, &path)))
}

/// The template language's own checks, without the render checks that need app/views.
pub(crate) fn language_diagnostics(
    uri: &Url,
    lang: TemplateLang,
    text: &str,
    extra_engines: &[String],
    ruby: &RubyContext,
) -> Vec<Diagnostic> {
    let mut out = match lang {
        TemplateLang::Erb => erb::diagnostics(text, ruby),
        TemplateLang::Haml => {
            let parsed = haml_parser::parse_with_ruby(text);
            let mut out = haml_indent::diagnostics(text, &parsed);
            let comment =
                ruby.strict_locals.as_ref().and_then(|rails| strict_locals::find(text, rails.close).map(|comment| (comment, rails)));
            // Haml stops at its first error, so there is no Ruby to check.
            if parsed.error.is_none() {
                let ruby_errors = haml_ruby::diagnostics(text, &parsed, ruby.grammar, comment.as_ref().map(|(comment, _)| comment));
                out.extend(ruby_errors.into_iter().map(|d| util::coded(d, RUBY_SYNTAX)));
            }
            if let Some((comment, rails)) = &comment {
                let signature = strict_locals::comment_diagnostics(text, comment, ruby.grammar, rails);
                out.extend(signature.into_iter().map(|d| util::coded(d, RUBY_SYNTAX)));
            }
            out
        }
        TemplateLang::Slim => {
            let mut out = slim_indent::diagnostics(text, extra_engines, ruby.grammar);
            out.extend(locals_signature(text, ruby));
            out
        }
        // Skim is Slim syntax with CoffeeScript code, so the Ruby checks would misfire.
        TemplateLang::Skim => slim_indent::diagnostics(text, extra_engines, ruby.grammar)
            .into_iter()
            .filter(|d| !util::code(d).is_some_and(|c| slim_indent::code::RUBY.contains(&c)))
            .collect(),
        // Syntax errors are Ruby LSP's job, but it doesn't know the `locals:` comment is code to Rails.
        TemplateLang::Ruby => locals_signature(text, ruby),
    };
    // Void elements are an HTML rule; Slim itself renders `<img>x</img>` for `.xml.slim` and friends.
    if lang == TemplateLang::Slim && !slim_html_host(uri) {
        out.retain(|d| util::code(d) != Some(slim_indent::code::VOID_CHILDREN));
    }
    if lang == TemplateLang::Erb && html_host(uri) {
        out.extend(erb::html_diagnostics(text));
        out.extend(tags::diagnostics(text, is_partial(uri)));
        out.extend(html_lint::diagnostics(text));
    }
    out
}

fn locals_signature(text: &str, ruby: &RubyContext) -> Vec<Diagnostic> {
    strict_locals::diagnostics(text, ruby.grammar, ruby.strict_locals.as_ref()).into_iter().map(|d| util::coded(d, RUBY_SYNTAX)).collect()
}

/// Diagnostic codes every language shares.
pub(crate) const UNRESOLVED_RENDER: &str = "unresolved-render";
const RUBY_SYNTAX: &str = "ruby-syntax";
pub(crate) const STRICT_LOCALS: &str = "strict-locals";
const STRICT_LOCALS_SINCE: &str = "7.1";
const STRICT_LOCALS_TEMPLATE_SUFFIX_SINCE: &str = "7.2";

/// Line index, line, the call's text from the line's start to the end of its arguments, and the render.
type CheckedRender<'a> = (usize, &'a str, &'a str, partials::RenderRef);

/// The render checks of a Rails template in app/views: unresolved renders, then strict-locals mismatches.
fn render_diagnostics(
    path: &Path,
    project: &Project,
    text: &str,
    in_code: impl Fn(usize, usize, &partials::RenderRef) -> bool,
) -> Vec<Diagnostic> {
    if project.framework != Framework::Rails {
        return vec![];
    }
    let Some(views) = rails_paths::views_root(path) else { return vec![] };
    let renders = checked_renders(&views, text, in_code);
    let mut partials = Partials { project, path, resolved: HashMap::new(), signatures: HashMap::new() };
    let mut out = unresolved_renders(project, &renders, &mut partials);
    out.extend(strict_locals_mismatches(project, &views, &renders, &mut partials));
    out
}

/// Literal renders in code that name `dir/name` under an existing app/views/dir, as (line index, line, render).
fn checked_renders<'a>(
    views: &Path,
    text: &'a str,
    in_code: impl Fn(usize, usize, &partials::RenderRef) -> bool,
) -> Vec<CheckedRender<'a>> {
    // ponytail: only `dir/name` with an existing app/views/dir is checked. Bare names resolve via the
    // controller's prefix and its ancestors, and missing dirs are usually gem/engine views (devise/…);
    // neither is knowable without reading controllers or the bundle.
    let mut dirs: HashMap<String, bool> = HashMap::new();
    let mut in_views_dir = |name: &str| match name.rsplit_once('/') {
        Some((dir, _)) => *dirs.entry(dir.to_string()).or_insert_with(|| views.join(dir).is_dir()),
        None => false,
    };
    let mut out = Vec::new();
    for (i, (line_start, line)) in util::lines_with_offsets(text).enumerate() {
        for r in partials::render_refs(line) {
            if in_code(i, line_start, &r) && in_views_dir(&r.name) {
                out.push((i, line, partials::call_text(&text[line_start..], &r), r));
            }
        }
    }
    out
}

/// A template's partials, each resolved and read once per pass: a view renders the same few many times.
struct Partials<'a> {
    project: &'a Project,
    path: &'a Path,
    resolved: HashMap<(partials::RenderKind, String), Vec<PathBuf>>,
    signatures: HashMap<PathBuf, Option<partials::StrictLocals>>,
}

impl Partials<'_> {
    fn resolve(&mut self, r: &partials::RenderRef) -> &[PathBuf] {
        let (project, path) = (self.project, self.path);
        self.resolved.entry((r.kind, r.name.clone())).or_insert_with(|| partials::resolve(project, path, r))
    }

    fn signature(&mut self, file: &Path) -> Option<&partials::StrictLocals> {
        let close = strict_locals::Close::of(self.project);
        self.signatures.entry(file.to_path_buf()).or_insert_with(|| partials::file_strict_locals(file, close)).as_ref()
    }
}

/// Literal `render "x"` / `render partial:|template: "x"` with no matching file under app/views.
fn unresolved_renders(project: &Project, renders: &[CheckedRender], partials: &mut Partials) -> Vec<Diagnostic> {
    renders
        .iter()
        .filter(|(.., r)| partials.resolve(r).is_empty() && !is_gem_view(project, &r.name))
        .map(|(i, line, _, r)| {
            util::coded(util::warning(util::span_range(*i, line, r.start, r.end), missing_message(r)), UNRESOLVED_RENDER)
        })
        .collect()
}

/// Rails' MissingTemplate message up to ` with {…}.`; the rest lists the request's formats and paths, which vary.
fn missing_message(r: &partials::RenderRef) -> String {
    let (dir, name) = r.name.rsplit_once('/').unwrap_or(("", &r.name));
    let (kind, prefix) = if r.kind == partials::RenderKind::Partial { ("partial", "_") } else { ("template", "") };
    let dir = if dir.is_empty() { String::new() } else { format!("{dir}/") };
    format!("Missing {kind} {dir}{prefix}{name}")
}

/// Is `name`'s top folder (`devise/shared/links`) a gem's own view folder? Gems and engines ship views under their
/// name (`devise/`, `spree/` from `spree_core`), and the app overrides only some of them, so the rest aren't on disk.
fn is_gem_view(project: &Project, name: &str) -> bool {
    name.split_once('/').is_some_and(|(top, _)| project.has_gem_family(top))
}

/// Strict locals exist from Rails 7.1; before, the `locals:` line is a plain comment.
fn supports_strict_locals(project: &Project) -> bool {
    project.framework == Framework::Rails && project.gem_version("actionview").is_some_and(|v| v.at_least(STRICT_LOCALS_SINCE))
}

/// Literal renders whose locals don't fit the partial's `locals: (…)` signature.
fn strict_locals_mismatches(project: &Project, views: &Path, renders: &[CheckedRender], partials: &mut Partials) -> Vec<Diagnostic> {
    if !supports_strict_locals(project) {
        return vec![];
    }
    let suffix_with_template = project.gem_version("actionview").is_some_and(|v| v.at_least(STRICT_LOCALS_TEMPLATE_SUFFIX_SINCE));
    let mut out = Vec::new();
    // A partial's layout gets the partial's locals, which this check can't see.
    for (i, line, call, r) in renders.iter().filter(|(.., r)| !r.from_layout) {
        let Some(passed) = partials::passed_locals(call, r) else { continue };
        // Several variants/formats: which one Rails picks depends on the request.
        let [file] = partials.resolve(r) else { continue };
        let file = file.clone();
        let Some(sig) = partials.signature(&file) else { continue };
        let missing: Vec<&str> =
            sig.locals.iter().filter(|(n, d)| d.is_none() && !passed.contains(&n.as_str())).map(|(n, _)| n.as_str()).collect();
        let mut seen = HashSet::new();
        let unknown: Vec<&str> =
            passed.into_iter().filter(|p| seen.insert(*p) && !sig.rest && !sig.locals.iter().any(|(n, _)| n == p)).collect();
        let no_locals = sig.locals.is_empty() && !sig.rest;
        let Some(mut message) = strict_locals_message(&missing, &unknown, no_locals) else { continue };
        if suffix_with_template {
            // Rails prints the absolute path; relative to the app folder is stable and readable.
            let app_root = views.parent().and_then(Path::parent).unwrap_or(views);
            let identifier = file.strip_prefix(app_root).unwrap_or(&file).to_string_lossy().replace('\\', "/");
            message.push_str(&format!(" for {identifier}"));
        }
        out.push(util::coded(util::warning(util::span_range(*i, line, r.start, r.end), message), STRICT_LOCALS));
    }
    out
}

/// Rails' error for one render: a missing local wins over an unknown one, and Ruby's plural `keywords` stays.
fn strict_locals_message(missing: &[&str], unknown: &[&str], no_locals: bool) -> Option<String> {
    let symbols = |names: &[&str]| names.iter().map(|n| format!(":{n}")).collect::<Vec<_>>().join(", ");
    match (missing, unknown) {
        ([], []) => None,
        ([one], _) => Some(format!("missing local: :{one}")),
        ([_, ..], _) => Some(format!("missing keywords: {}", symbols(missing))),
        ([], _) if no_locals => Some("no locals accepted".to_string()),
        ([], [one]) => Some(format!("unknown local: :{one}")),
        ([], _) => Some(format!("unknown keywords: {}", symbols(unknown))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_context::in_ruby_context;
    use std::collections::BTreeSet;
    use tower_lsp::lsp_types::{DiagnosticSeverity, NumberOrString, Position, Range};

    #[test]
    fn void_children_only_for_html_slim() {
        let text = "img\n  | x\n";
        let has_void = |path: &str| {
            let uri = Url::parse(path).unwrap();
            diagnostics(&uri, &Doc { lang: TemplateLang::Slim, text: text.into(), version: 0 }, &Options::default())
                .iter()
                .any(|d| d.code == Some(NumberOrString::String(slim_indent::code::VOID_CHILDREN.into())))
        };
        for (path, html) in [
            ("file:///v/show.html.slim", true),
            ("file:///v/show.html+phone.slim", true),
            ("file:///v/show.html%2Bphone.slim", true),
            ("file:///v/show.xml%2Bfeed.slim", false),
            ("file:///v/show.slim", true),
            ("file:///v/show.de.slim", true),
            ("file:///v/show.de.xml.slim", false),
            ("untitled:Untitled-1", true),
            ("file:///v/feed.xml.slim", false),
            ("file:///v/welcome.text.slim", false),
            ("file:///v/index.js.slim", false),
        ] {
            assert_eq!(has_void(path), html, "{path}");
        }
    }

    #[test]
    fn options_keep_valid_fields() {
        let init = serde_json::json!({
            "slim": { "extraEngines": "mjml", "disabledDiagnostics": ["bad-dedent", 3] },
            "erb": { "disabledDiagnostics": ["unclosed-element"] },
            "haml": { "disabledDiagnostics": "illegal-nesting" }
        });
        let (options, ignored) = Options::from_init(Some(&init));
        let expected =
            Options { disabled_slim: vec!["bad-dedent".into()], disabled_erb: vec!["unclosed-element".into()], ..Options::default() };
        assert_eq!(options, expected);
        assert_eq!(ignored.len(), 3, "{ignored:?}");
        assert_eq!(Options::from_init(None), (Options::default(), vec![]));
    }

    #[test]
    fn languages_switch_diagnostics_and_formatting_off() {
        let init = serde_json::json!({
            "erb": { "diagnostics": { "enabled": false } },
            "slim": { "format": { "enabled": false }, "diagnostics": { "enabled": "no" } },
            "haml": { "diagnostics": { "enabled": true }, "format": {} }
        });
        let (options, ignored) = Options::from_init(Some(&init));
        assert_eq!(ignored.len(), 1, "{ignored:?}");
        assert!(!options.align_erb_comments);
        let align = |value| Options::from_init(Some(&serde_json::json!({ "erb": { "format": { "alignComments": value } } })));
        assert!(align(serde_json::json!(true)).0.align_erb_comments);
        assert_eq!(align(serde_json::json!("yes")).1.len(), 1);
        let count = |lang, text: &str, options: &Options| {
            let uri = Url::parse("file:///v/show.html.erb").unwrap();
            diagnostics(&uri, &Doc { lang, text: text.into(), version: 0 }, options).len()
        };
        assert_eq!(count(TemplateLang::Erb, "<% end %>\n", &Options::default()), 1);
        assert_eq!(count(TemplateLang::Erb, "<% end %>\n", &options), 0);
        let haml = "%p hi\n  %b x\n";
        assert_eq!(count(TemplateLang::Haml, haml, &options), count(TemplateLang::Haml, haml, &Options::default()));
        let slim = "p\n   a\n  b\n";
        assert_ne!(count(TemplateLang::Slim, slim, &options), 0);
        let formats = |lang| options.format_enabled(lang);
        assert!(formats(TemplateLang::Erb) && formats(TemplateLang::Haml) && formats(TemplateLang::Ruby));
        assert!(!formats(TemplateLang::Slim) && !formats(TemplateLang::Skim));
        // Skim follows Slim.
        let off = Options::from_init(Some(&serde_json::json!({ "slim": { "diagnostics": { "enabled": false } } }))).0;
        assert_eq!(count(TemplateLang::Skim, slim, &off), 0);
    }

    #[test]
    fn disabled_diagnostics_per_language() {
        let codes = |path: &str, lang, text: &str, options: &Options| -> Vec<String> {
            let uri = Url::parse(path).unwrap();
            diagnostics(&uri, &Doc { lang, text: text.into(), version: 0 }, options)
                .into_iter()
                .map(|d| match d.code {
                    Some(NumberOrString::String(c)) => c,
                    _ => panic!("uncoded diagnostic: {}", d.message),
                })
                .collect()
        };

        let haml = "%p hi\n  %b x\n";
        let all = Options::default();
        let erb_path = "file:///v/show.html.erb";
        // Each code gets its own text, so one check does not hide another.
        let erb_cases =
            [("<% end %>\n", erb::code::RUBY_SYNTAX), ("<div>\n", tags::code::UNCLOSED_ELEMENT), ("hi\n<% foo", erb::code::UNCLOSED_TAG)];
        for (text, code) in erb_cases {
            assert_eq!(codes(erb_path, TemplateLang::Erb, text, &all), [code]);
        }
        let options = Options {
            disabled_erb: vec![erb::code::RUBY_SYNTAX.into(), tags::code::UNCLOSED_ELEMENT.into(), erb::code::UNCLOSED_TAG.into()],
            disabled_haml: vec![haml_indent::code::ILLEGAL_NESTING.into()],
            ..Options::default()
        };
        assert!(erb_cases.iter().all(|(text, _)| codes(erb_path, TemplateLang::Erb, text, &options).is_empty()));
        // Text that is not HTML has no `<` to misparse.
        assert!(codes("file:///v/show.text.erb", TemplateLang::Erb, "hi <% foo", &all).is_empty());
        let haml_path = "file:///v/show.html.haml";
        assert!(codes(haml_path, TemplateLang::Haml, haml, &all).contains(&haml_indent::code::ILLEGAL_NESTING.to_string()));
        let haml_codes = codes(haml_path, TemplateLang::Haml, haml, &options);
        assert!(!haml_codes.contains(&haml_indent::code::ILLEGAL_NESTING.to_string()), "{haml_codes:?}");
        // Turning one code off leaves the others.
        let void = "%br\n  %p\n";
        assert_eq!(codes(haml_path, TemplateLang::Haml, void, &all), [haml_indent::code::VOID_CHILDREN]);
        let quiet = Options { disabled_haml: vec![haml_indent::code::VOID_CHILDREN.into()], ..Options::default() };
        assert!(codes(haml_path, TemplateLang::Haml, void, &quiet).is_empty());
        // Haml's Ruby is parsed once Haml itself accepts the template.
        let ruby = codes(haml_path, TemplateLang::Haml, "= foo(\n", &all);
        assert!(!ruby.is_empty() && ruby.iter().all(|c| c == RUBY_SYNTAX), "{ruby:?}");
        let ruby_off = Options { disabled_haml: vec![RUBY_SYNTAX.into()], ..Options::default() };
        assert!(codes(haml_path, TemplateLang::Haml, "= foo(\n", &ruby_off).is_empty());
        let after_error = codes(haml_path, TemplateLang::Haml, "- if x\n  %p\n- end\n= foo(\n", &all);
        assert_eq!(after_error, [haml_indent::code::EXPLICIT_END]);
        // Every check carries a code, or the setting can't turn it off.
        assert!(!codes(erb_path, TemplateLang::Erb, include_str!("../../test/broken/bad.html.erb"), &all).is_empty());
        assert!(!codes(haml_path, TemplateLang::Haml, include_str!("../../test/broken/bad.html.haml"), &all).is_empty());
    }

    #[test]
    fn settings_list_every_diagnostic_code() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../../package.json")).unwrap();
        let setting = |lang: &str| -> BTreeSet<String> {
            let key = format!("rubyTemplates.{lang}.disabledDiagnostics");
            let items = &manifest["contributes"]["configuration"]["properties"][&key]["items"]["enum"];
            items.as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect()
        };
        let declared = |sources: &[&str]| -> BTreeSet<String> {
            let re = regex::Regex::new(r#"pub const [A-Z_]+: &str = "([a-z0-9-]+)""#).unwrap();
            let shared = [UNRESOLVED_RENDER, STRICT_LOCALS, RUBY_SYNTAX].map(String::from);
            sources.iter().flat_map(|src| re.captures_iter(src).map(|c| c[1].to_string()).collect::<Vec<_>>()).chain(shared).collect()
        };
        assert_eq!(setting("erb"), declared(&[include_str!("erb.rs"), include_str!("tags.rs"), include_str!("html_lint.rs")]));
        assert_eq!(setting("haml"), declared(&[include_str!("haml_indent.rs")]));
        assert_eq!(setting("slim"), declared(&[include_str!("slim_indent.rs")]));
        assert_eq!(setting("jbuilder"), declared(&[]));
        // A literal code would bypass the constants this test collects.
        let literal = regex::Regex::new(r#"(?m)coded\(.*,\s*"[a-z0-9-]+"\)[);,]*$"#).unwrap();
        for src in [
            include_str!("main.rs"),
            include_str!("erb.rs"),
            include_str!("tags.rs"),
            include_str!("html_lint.rs"),
            include_str!("haml_indent.rs"),
            include_str!("slim_indent.rs"),
        ] {
            assert!(!literal.is_match(src.split("#[cfg(test)]").next().unwrap()), "coded(…, \"literal\")");
        }
    }

    #[test]
    fn diagnostic_source_is_the_file_language() {
        let root = std::env::temp_dir().join(format!("rt-source-{}", std::process::id()));
        let views = root.join("app/views/users");
        std::fs::create_dir_all(&views).unwrap();
        std::fs::write(root.join("Gemfile.lock"), format!("{GEM_LOCK}    actionview (8.0.2)\n")).unwrap();
        let render = "<%= render \"users/gone\" %>";
        let cases = [
            ("a.html.erb", TemplateLang::Erb, "<% end %>\n<div>\n", "erb"),
            ("b.html.erb", TemplateLang::Erb, render, "erb"),
            ("c.haml", TemplateLang::Haml, "%p hi\n  %b x\n= render \"users/gone\"\n", "haml"),
            ("d.slim", TemplateLang::Slim, "p\n   a\n  b\n= render \"users/gone\"\n", "slim"),
            ("e.skim", TemplateLang::Skim, "p\n   a\n  b\n", "slim"),
            ("f.jbuilder", TemplateLang::Ruby, "json.partial! \"users/gone\"\n", "ruby"),
        ];
        for (name, lang, text, source) in cases {
            let uri = Url::from_file_path(views.join(name)).unwrap();
            let found = diagnostics(&uri, &Doc { lang, text: text.into(), version: 0 }, &Options::default());
            assert!(!found.is_empty(), "{name}");
            for d in found {
                assert_eq!(d.source.as_deref(), Some(source), "{name}: {}", d.message);
            }
        }
        let uri = Url::from_file_path(views.join("g.html.erb")).unwrap();
        let html = Doc { lang: TemplateLang::Erb, text: "<div>\n".into(), version: 0 };
        let found = diagnostics(&uri, &html, &Options::default());
        assert_eq!(found.iter().filter_map(util::code).collect::<Vec<_>>(), [tags::code::UNCLOSED_ELEMENT]);
        assert_eq!(found[0].source.as_deref(), Some("erb"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn skim_skips_ruby_checks() {
        let uri = Url::parse("untitled:Untitled-1").unwrap();
        assert_eq!(TemplateLang::from_uri_or_id(&uri, Some("skim")), Some(TemplateLang::Skim));
        let ruby = "- else\n= alert(\"hi\"\n";
        let explicit_end = "- end = start + 10\n";
        let dedent = "  p\n    a\n   span\n";
        let codes = |lang, text: &str| -> Vec<String> {
            diagnostics(&uri, &Doc { lang, text: text.into(), version: 0 }, &Options::default())
                .into_iter()
                .filter_map(|d| match d.code {
                    Some(NumberOrString::String(c)) => Some(c),
                    _ => None,
                })
                .collect()
        };
        let slim = [codes(TemplateLang::Slim, ruby), codes(TemplateLang::Slim, explicit_end)].concat();
        assert!(slim_indent::code::RUBY.iter().all(|c| slim.iter().any(|s| s == c)), "{slim:?}");
        assert_eq!(codes(TemplateLang::Skim, ruby), Vec::<String>::new());
        assert_eq!(codes(TemplateLang::Skim, explicit_end), Vec::<String>::new());
        assert_eq!(codes(TemplateLang::Skim, dedent), [slim_indent::code::BAD_DEDENT]);
    }

    const GEM_LOCK: &str = "GEM\n  specs:\n    devise (4.9.4)\n      bcrypt (~> 3.0)\n    spree_core (4.8.0)\n    administrate (0.20.1)\n";

    fn lock_with(actionview: &str) -> Project {
        Project::from_lockfile(&format!("{GEM_LOCK}    actionview ({actionview})\n"))
    }

    fn render_checks(lang: TemplateLang, path: &Path, project: &Project, text: &str, code: &str) -> Vec<Diagnostic> {
        let found = render_diagnostics(path, project, text, render_filter(lang, text, &[]));
        found.into_iter().filter(|d| util::code(d) == Some(code)).collect()
    }

    fn unresolved(lang: TemplateLang, path: &Path, text: &str) -> Vec<Diagnostic> {
        render_checks(lang, path, &lock_with("7.1.0"), text, UNRESOLVED_RENDER)
    }

    #[test]
    fn thor_templates_skip_structure_checks() {
        let text = "%table\n  %tr\n<% attributes.each do |attribute| -%>\n    %th <%= attribute.human_name %>\n<% end -%>\n";
        let doc = || Doc { lang: TemplateLang::Haml, text: text.to_string(), version: 0 };
        let uri = |name: &str| Url::parse(&format!("file:///app/lib/templates/haml/scaffold/{name}")).unwrap();
        assert!(diagnostics(&uri("index.html.haml.tt"), &doc(), &Options::default()).is_empty());
        assert!(!diagnostics(&uri("index.html.haml"), &doc(), &Options::default()).is_empty());
        let erb = Doc { lang: TemplateLang::Erb, text: "<%% if x %>\n<% attributes.each do |a| -%>\n<div>\n".into(), version: 0 };
        assert!(diagnostics(&uri("_form.html.erb.tt"), &erb, &Options::default()).is_empty());
    }

    #[test]
    fn unresolved_render_warnings() {
        let root = std::env::temp_dir().join(format!("rt-unresolved-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        std::fs::write(users.join("_card.html.erb"), "").unwrap();
        let file = users.join("index.html.erb");
        let text = concat!(
            "<%= render \"card\" %> <%= render partial: \"users/missing\" %>\n",
            "é<%= render \"users/nope\" %>\n",
            "<%= render \"bare\" %> <%= render \"devise/shared/links\" %> <% # render \"users/old\" %>\n",
            "<%= render @users %> <%= render \"a/#{b}\" %> <%= render layout: \"box\" do %>\n",
            "<%= render layout: \"users/frame\" do %> <%= render partial: \"users/card\", layout: \"users/card\" %>\n",
            "<p>we render \"prose\"</p>\n",
        );
        let diags = unresolved(TemplateLang::Erb, &file, text);
        let got: Vec<(String, Range)> = diags.iter().map(|d| (d.message.clone(), d.range)).collect();
        let at = |l, s, e| Range::new(Position::new(l, s), Position::new(l, e));
        assert_eq!(
            got,
            vec![
                ("Missing partial users/_missing".to_string(), at(0, 42, 55)),
                ("Missing partial users/_nope".to_string(), at(1, 13, 23)),
                ("Missing partial users/_frame".to_string(), at(4, 20, 31)),
            ]
        );
        std::fs::create_dir_all(users.join("sub")).unwrap();
        std::fs::create_dir_all(root.join("app/views/layouts")).unwrap();
        std::fs::write(root.join("app/views/layouts/lt.html.erb"), "").unwrap();
        let kinds = concat!(
            "<%= render template: \"users/t\" %>\n",
            "<%= render :template => \"users/t2\" %>\n",
            "<%= render template: \"users/t\", layout: \"layouts/lt\" %>\n",
            "<%= render template: \"users/t\", layout: \"layouts/gone\" %>\n",
            "<%= render partial: \"users/sub/p\", layout: \"users/pl\" %>\n",
            "<%= render :partial => \"users/p2\" %>\n",
        );
        let messages: Vec<String> = unresolved(TemplateLang::Erb, &file, kinds).into_iter().map(|d| d.message).collect();
        assert_eq!(
            messages,
            [
                "Missing template users/t",
                "Missing template users/t2",
                "Missing template users/t",
                "Missing template users/t",
                "Missing template layouts/gone",
                "Missing partial users/sub/_p",
                "Missing partial users/_pl",
                "Missing partial users/_p2",
            ]
        );
        let card = partials::render_refs(r#"<%= render layout: "card" do %>"#);
        assert_eq!(partials::resolve(&Project::unknown(), &file, &card[0]), vec![users.join("_card.html.erb")]);
        assert!(diags.iter().all(|d| d.severity == Some(DiagnosticSeverity::WARNING)));

        let layouts = root.join("app/views/layouts");
        std::fs::create_dir_all(&layouts).unwrap();
        assert!(unresolved(TemplateLang::Erb, &layouts.join("application.html.erb"), "<%= render \"header\" %>").is_empty());
        assert!(unresolved(TemplateLang::Erb, Path::new("/tmp/x.html.erb"), "<%= render \"users/nope\" %>").is_empty());
        let haml = users.join("show.html.haml");
        assert!(unresolved(TemplateLang::Haml, &haml, "%p (we render \"users/x\")").is_empty());
        assert_eq!(unresolved(TemplateLang::Haml, &haml, "= render \"users/x\"").len(), 1);
        let haml_text = concat!(
            "%div= render \"users/a\"\n",
            ".box{data: {x: 1}}= render \"users/b\"\n",
            "#main!= render \"users/c\"\n",
            "%p&= render \"users/d\"\n",
            "~ render \"users/e\"\n",
            "%p<~ render \"users/f\"\n",
            "%p render \"users/g\"\n",
            "- # render \"users/h\"\n",
            "-# render \"users/i\"\n",
        );
        let lines: Vec<u32> = unresolved(TemplateLang::Haml, &haml, haml_text).iter().map(|d| d.range.start.line).collect();
        assert_eq!(lines, vec![0, 1, 2, 3, 4, 5]);
        assert!(unresolved(TemplateLang::Skim, &users.join("show.skim"), "= render \"users/x\"").is_empty());
        let slim = users.join("show.html.slim");
        let slim_text =
            "=' render \"users/a\"\n==' render \"users/b\"\n=> render \"users/c\"\np = render \"users/d\"\n| render \"users/e\"\n";
        let lines: Vec<u32> = unresolved(TemplateLang::Slim, &slim, slim_text).iter().map(|d| d.range.start.line).collect();
        assert_eq!(lines, vec![0, 1, 2]);
        for dir in ["devise/shared", "spree/admin", "admin"] {
            std::fs::create_dir_all(root.join("app/views").join(dir)).unwrap();
        }
        let gem_text = "<%= render \"devise/shared/error_messages\" %><%= render \"spree/admin/x\" %><%= render \"admin/x\" %>";
        let lines: Vec<String> = unresolved(TemplateLang::Erb, &file, gem_text).into_iter().map(|d| d.message).collect();
        assert_eq!(lines, ["Missing partial admin/_x"]);
        assert!(!in_ruby_context(TemplateLang::Skim, "= render \"x\"", &[], 10));
        assert!(in_ruby_context(TemplateLang::Slim, "= render \"x\"", &[], 10));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn strict_locals_messages_follow_rails() {
        let root = std::env::temp_dir().join(format!("rt-locals-msg-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        std::fs::write(users.join("_card.html.erb"), "<%# locals: (a:, b:) -%>").unwrap();
        std::fs::write(users.join("_none.html.erb"), "<%# locals: () -%>").unwrap();
        std::fs::write(users.join("_opt.html.erb"), "<%# locals: (a: 1) -%>").unwrap();
        let file = users.join("index.html.erb");
        let renders = [
            r#"render "users/card""#,
            r#"render "users/card", c: 3"#,
            r#"render "users/card", a: 1"#,
            r#"render "users/card", a: 1, c: 3"#,
            r#"render "users/card", a: 1, b: 2, c: 3"#,
            r#"render "users/card", a: 1, b: 2, d: 4, c: 3"#,
            r#"render "users/none", x: 1"#,
            r#"render "users/opt", z: 1"#,
            r#"render "users/opt", a: 1"#,
            r#"render "users/card", a: 1, b: 2, c: 3, c: 4"#,
        ];
        let text = renders.map(|r| format!("<%= {r} %>\n")).concat();
        let messages = |actionview: &str| -> Vec<String> {
            let project = lock_with(actionview);
            let found = render_checks(TemplateLang::Erb, &file, &project, &text, STRICT_LOCALS);
            found.into_iter().map(|d| d.message).collect()
        };
        let rails_71 = [
            "missing keywords: :a, :b",
            "missing keywords: :a, :b",
            "missing local: :b",
            "missing local: :b",
            "unknown local: :c",
            "unknown keywords: :d, :c",
            "no locals accepted",
            "unknown local: :z",
            "unknown local: :c",
        ];
        assert_eq!(messages("7.1.5"), rails_71);
        let rails_80: Vec<String> = rails_71
            .iter()
            .zip(["card", "card", "card", "card", "card", "card", "none", "opt", "card"])
            .map(|(m, partial)| format!("{m} for app/views/users/_{partial}.html.erb"))
            .collect();
        assert_eq!(messages("7.2.2"), rails_80);
        assert_eq!(messages("8.0.1"), rails_80);
        assert_eq!(messages("8.1.4"), rails_80);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn jbuilder_and_builder_views() {
        let lang = |path: &str, id| TemplateLang::from_uri_or_id(&Url::parse(path).unwrap(), id);
        assert_eq!(lang("file:///app/views/users/index.json.jbuilder", Some("ruby")), Some(TemplateLang::Ruby));
        assert_eq!(lang("file:///app/views/feed.atom.builder", Some("ruby")), Some(TemplateLang::Ruby));
        assert_eq!(lang("file:///app/views/users/index.json.jbuilder", None), Some(TemplateLang::Ruby));
        assert_eq!(lang("file:///app/models/user.rb", Some("ruby")), None);

        let root = std::env::temp_dir().join(format!("rt-jbuilder-{}", std::process::id()));
        let posts = root.join("app/views/posts");
        std::fs::create_dir_all(&posts).unwrap();
        std::fs::write(posts.join("_post.json.jbuilder"), "# locals: (post:)\njson.title post.title\n").unwrap();
        let file = posts.join("index.json.jbuilder");
        let text = concat!(
            "json.array! @posts, partial: \"posts/post\", as: :post\n",
            "json.partial! \"posts/post\", collection: @posts, as: :post\n",
            "json.partial! \"posts/missing\"\n",
            "# json.partial! \"posts/old\"\n",
            "json.partial! \"posts/post\", post: @post, extra: 1\n",
            "json.array! @x, title: \"render\", partial: \"posts/missing3\"\n",
        );
        let lines = |d: Vec<Diagnostic>| d.iter().map(|d| (d.range.start.line, d.message.clone())).collect::<Vec<_>>();
        assert_eq!(
            lines(unresolved(TemplateLang::Ruby, &file, text)),
            [(2, "Missing partial posts/_missing".to_string()), (5, "Missing partial posts/_missing3".to_string())]
        );
        let mismatches = render_checks(TemplateLang::Ruby, &file, &lock_with("7.1.0"), text, STRICT_LOCALS);
        assert_eq!(lines(mismatches), [(4, "unknown local: :extra".to_string())]);
        assert!(!in_ruby_context(TemplateLang::Ruby, text, &[], text.find("posts/old").unwrap()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn render_checks_follow_framework_and_version() {
        let root = std::env::temp_dir().join(format!("rt-gates-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        std::fs::write(users.join("_card.html.erb"), "<%# locals: (user:) -%>").unwrap();
        let file = users.join("index.html.erb");
        let doc = Doc { lang: TemplateLang::Erb, text: "<%= render \"users/card\" %><%= render \"users/gone\" %>".into(), version: 0 };
        let uri = Url::from_file_path(&file).unwrap();
        let codes = || -> Vec<String> {
            let found = diagnostics(&uri, &doc, &Options::default());
            found.iter().filter_map(|d| util::code(d).map(String::from)).collect()
        };
        let lock = root.join("Gemfile.lock");
        let (both, unresolved_only) = (vec![STRICT_LOCALS, UNRESOLVED_RENDER], vec![UNRESOLVED_RENDER]);
        assert!(codes().is_empty(), "no lockfile");
        for (specs, expected) in [
            ("sinatra (4.0.0)", vec![]),
            ("actionview (7.0.8)", unresolved_only),
            ("actionview (7.1.0)", both.clone()),
            ("actionview (8.0.2)", both),
        ] {
            std::fs::write(&lock, format!("GEM\n  specs:\n    {specs}\n")).unwrap();
            let mut got = codes();
            got.sort();
            assert_eq!(got, expected, "{specs}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn positional_locals_are_reported_from_rails_7_1_with_the_path_from_the_project_root() {
        let root = std::env::temp_dir().join(format!("rt-locals-keywords-{}", std::process::id()));
        let file = root.join("app/views/users/_card.html.erb");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let doc = Doc { lang: TemplateLang::Erb, text: "<%# locals: (user, size:) -%>\n".into(), version: 0 };
        let uri = Url::from_file_path(&file).unwrap();
        let messages = || diagnostics(&uri, &doc, &Options::default()).into_iter().map(|d| d.message).collect::<Vec<_>>();
        let lock = |actionview: &str| {
            std::fs::write(root.join("Gemfile.lock"), format!("GEM\n  specs:\n    actionview ({actionview})\n")).unwrap()
        };
        assert!(messages().is_empty(), "no lockfile");
        lock("7.0.8");
        assert!(messages().is_empty(), "Rails 7.0");
        lock("7.1.0");
        let expected =
            "`user` set as non-keyword argument for app/views/users/_card.html.erb. Locals can only be set as keyword arguments.";
        assert_eq!(messages(), [expected]);
        let broken = Doc { lang: TemplateLang::Erb, text: "<%# locals: (a: ] ) %>\n".into(), version: 0 };
        let broken_messages = || diagnostics(&uri, &broken, &Options::default()).len();
        assert!(broken_messages() > 0, "Rails 7.1");
        lock("7.0.8");
        assert_eq!(broken_messages(), 0, "Rails 7.0: a plain comment");
        lock("7.1.0");
        let others = [
            (TemplateLang::Haml, "html.haml", "-#", "%p"),
            (TemplateLang::Slim, "html.slim", "/#", "p"),
            (TemplateLang::Ruby, "json.jbuilder", "#", "json.x"),
        ];
        for (lang, ext, comment, body) in others {
            let file = root.join(format!("app/views/users/_card.{ext}"));
            let doc = Doc { lang, text: format!("{comment} locals: (user, size:)\n{body} x\n"), version: 0 };
            let found: Vec<_> = diagnostics(&Url::from_file_path(&file).unwrap(), &doc, &Options::default())
                .into_iter()
                .map(|d| (d.range.start.line, util::code(&d).map(String::from), d.message))
                .collect();
            let expected =
                format!("`user` set as non-keyword argument for app/views/users/_card.{ext}. Locals can only be set as keyword arguments.");
            assert_eq!(found, [(0, Some("ruby-syntax".to_string()), expected)], "{ext}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn strict_locals_diagnostics() {
        let root = std::env::temp_dir().join(format!("rt-locals-diag-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        std::fs::write(users.join("_card.html.erb"), "<%# locals: (user:, size: :md) -%>\n<p><%= user.name %></p>\n").unwrap();
        std::fs::write(users.join("_any.html.erb"), "<%# locals: (user:, **rest) -%>").unwrap();
        std::fs::write(users.join("_plain.html.erb"), "").unwrap();
        std::fs::write(users.join("_multi.html.erb"), "<%# locals: (a:) -%>").unwrap();
        std::fs::write(users.join("_multi.html+phone.erb"), "<%# locals: (b:) -%>").unwrap();
        let file = users.join("index.html.erb");

        let text = concat!(
            "<%= render \"users/card\", user: u %> <%= render \"users/plain\", x: 1 %> <%= render \"users/any\", user: u, x: 1 %>\n",
            "<%= render \"users/card\", size: :lg, colour: 1 %>\n",
            "<%= render partial: \"users/card\", locals: { user: u } %> <%= render \"users/card\", **opts %>\n",
            "<%= render \"users/card\", Card::DEFAULTS %> <%= render \"card\" %> <%= render \"users/multi\" %>\n",
            "<%= render partial: \"users/plain\", locals: { a: 1 }, layout: \"users/card\" %>\n",
            "<%= render \"users/card\",\n      user: u %>\n",
            "<%= render(\"users/card\",\n  size: :lg) %>\n",
            "<%= render partial: \"users/card\", formats: [:html] %>\n",
            "<%= render partial: \"users/card\", object: u %> <%= render :partial => \"users/card\", :locals => { :user => u } %>\n",
        );
        let got: Vec<(String, Range)> = render_checks(TemplateLang::Erb, &file, &lock_with("7.1.0"), text, STRICT_LOCALS)
            .into_iter()
            .map(|d| (d.message, d.range))
            .collect();
        let at = |l, s, e| Range::new(Position::new(l, s), Position::new(l, e));
        let missing = |line| ("missing local: :user".to_string(), at(line, 12, 22));
        // `formats:` passes no locals; `object:` passes the partial's own, which the check can't see.
        let options_missing = ("missing local: :user".to_string(), at(9, 21, 31));
        assert_eq!(got, vec![missing(1), missing(7), options_missing]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
