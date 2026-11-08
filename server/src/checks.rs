//! The diagnostics of a template: its language's own checks plus the `render` checks against app/views.

use crate::code_context::render_filter;
use crate::erb::{html_lint, tags};
use crate::haml;
use crate::lang::{html_host, is_partial, slim_html_host, TemplateLang};
use crate::rails::project::{Framework, Project, Projects};
use crate::rails::render_call::HamlPipes;
use crate::rails::{controller_classes, output_lints, partials, paths, render_refs};
use crate::ruby::generated::RUBY_SYNTAX;
use crate::ruby::strict_locals;
use crate::ruby::strict_locals::RubyContext;
use crate::rules::resolve::ResolvedRules;
use crate::slim;
use crate::{erb, util};
use std::cell::LazyCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tower_lsp_server::ls_types::{Diagnostic, Uri};

/// Diagnostic codes every language shares, with `ruby::generated::RUBY_SYNTAX`.
pub(crate) const UNRESOLVED_RENDER: &str = "Rails/MissingPartial";
pub(crate) const STRICT_LOCALS: &str = "Rails/StrictLocals";
const STRICT_LOCALS_TEMPLATE_SUFFIX_SINCE: &str = "7.2";

#[derive(Clone)]
pub(crate) struct Doc {
    pub lang: TemplateLang,
    /// Opened with the plain `erb` language id, not a host-specific one such as `erb-js`.
    pub plain_erb: bool,
    /// Opened with the `erb-percent` language id: a line starting with `%` is Ruby.
    pub percent_lines: bool,
    pub text: String,
    pub version: i32,
}

impl Doc {
    /// HTML ERB; a `%`-lines document is never HTML, since its `%` lines would read as text.
    pub fn html_host(&self, uri: &Uri) -> bool {
        !self.percent_lines && html_host(uri, self.plain_erb, &self.text)
    }
}

#[cfg(test)]
pub(crate) fn diagnostics(uri: &Uri, doc: &Doc, options: &crate::options::Options) -> Vec<Diagnostic> {
    let rules = crate::rules::resolve::resolve(uri, doc.lang, options, &mut Default::default());
    diagnostics_in(uri, doc, &rules, &mut Projects::new())
}

/// `diagnostics`, taking the template's project from `projects` when a run already loaded it.
pub(crate) fn diagnostics_in(uri: &Uri, doc: &Doc, rules: &ResolvedRules, projects: &mut Projects) -> Vec<Diagnostic> {
    // Thor templates (`*.html.erb.tt`, `*.haml.tt`) mix the generator's ERB with the template's own tags or
    // lines, and their partials live outside app/views, so no check applies.
    if doc.lang == TemplateLang::Liquid || uri.path().as_str().to_ascii_lowercase().ends_with(".tt") || !rules.diagnostics_enabled() {
        return vec![];
    }
    let path = util::file_path(uri);
    let project = path.as_deref().map(|path| Project::memoized(projects, path, None));
    let ruby = path.as_deref().zip(project).map_or_else(RubyContext::outside_project, |(path, project)| project.ruby_context(path));
    let mut out = language_diagnostics(uri, doc, rules.extra_engines(), &ruby);
    if let (Some(path), Some(project)) = (&path, project) {
        out.extend(output_diagnostics(doc, rules, project, path, ruby.grammar));
        out.extend(render_diagnostics(path, project, &doc.text, doc.lang, render_filter(doc.lang, &doc.text, rules.extra_engines())));
    }
    out.retain(|d| util::code(d).is_none_or(|code| rules.reports(code)));
    for d in &mut out {
        d.source = Some(doc.lang.diagnostic_source().into());
    }
    out
}

/// The silent-output and debugging lints, for Rails templates only: Padrino and Middleman write helper output to the
/// buffer from `<% %>`, and an `.erb` outside app/views isn't a view.
fn output_diagnostics(doc: &Doc, rules: &ResolvedRules, project: &Project, path: &Path, grammar: ruby_prism::Version) -> Vec<Diagnostic> {
    let wanted = [output_lints::SILENT_RENDER, output_lints::SILENT_HELPER, output_lints::DEBUG_OUTPUT];
    if project.framework != Framework::Rails
        || project.is_gem_file(path)
        || paths::views_root(path).is_none()
        || !wanted.iter().any(|code| rules.reports(code))
    {
        return vec![];
    }
    output_lints::diagnostics(doc.lang, &doc.text, rules.extra_engines(), grammar)
}

/// The context of the template at `uri`: its project's, the latest grammar and no strict locals outside any.
pub(crate) fn ruby_context(uri: &Uri, projects: &mut Projects) -> RubyContext {
    util::file_path(uri).map_or_else(RubyContext::outside_project, |path| Project::memoized(projects, &path, None).ruby_context(&path))
}

/// The template language's own checks, without the render checks that need app/views.
pub(crate) fn language_diagnostics(uri: &Uri, doc: &Doc, extra_engines: &[String], ruby: &RubyContext) -> Vec<Diagnostic> {
    let (lang, text) = (doc.lang, doc.text.as_str());
    let mut out = match lang {
        TemplateLang::Erb => erb::diagnostics(text, ruby, doc.percent_lines),
        TemplateLang::Haml => {
            let parsed = haml::parser::parse_with_ruby(text);
            let mut out = haml::indent::diagnostics(text, &parsed);
            let comment =
                ruby.strict_locals.as_ref().and_then(|rails| strict_locals::find(text, rails.close).map(|comment| (comment, rails)));
            // Haml stops at its first error, so there is no Ruby to check.
            if parsed.error.is_none() {
                let ruby_errors =
                    haml::ruby::compiled(text, &parsed, comment.as_ref().map(|(comment, _)| comment)).diagnostics(text, ruby.grammar);
                out.extend(ruby_errors.into_iter().map(|d| util::coded(d, RUBY_SYNTAX)));
            }
            if let Some((comment, rails)) = &comment {
                let signature = strict_locals::comment_diagnostics(text, comment, ruby.grammar, rails);
                out.extend(signature.into_iter().map(|d| util::coded(d, RUBY_SYNTAX)));
            }
            out
        }
        TemplateLang::Slim => {
            let mut out = slim::indent::diagnostics(text, extra_engines, ruby.grammar);
            out.extend(locals_signature(text, ruby));
            out
        }
        // Skim is Slim syntax with CoffeeScript code, so the Ruby checks would misfire.
        TemplateLang::Skim => slim::indent::diagnostics(text, extra_engines, ruby.grammar)
            .into_iter()
            .filter(|d| !util::code(d).is_some_and(|c| slim::indent::code::RUBY.contains(&c)))
            .collect(),
        // Syntax errors are Ruby LSP's job, but it doesn't know the `locals:` comment is code to Rails.
        TemplateLang::Ruby => locals_signature(text, ruby),
        TemplateLang::Liquid => Vec::new(),
    };
    // Void elements are an HTML rule; Slim itself renders `<img>x</img>` for `.xml.slim` and friends.
    if lang == TemplateLang::Slim && !slim_html_host(uri) {
        out.retain(|d| util::code(d) != Some(slim::indent::code::VOID_CHILDREN));
    }
    if lang == TemplateLang::Erb && doc.html_host(uri) {
        out.extend(erb::html_diagnostics(text));
        out.extend(tags::diagnostics(text, is_partial(uri)));
        out.extend(html_lint::diagnostics(text));
    }
    out
}

fn locals_signature(text: &str, ruby: &RubyContext) -> Vec<Diagnostic> {
    strict_locals::diagnostics(text, ruby.grammar, ruby.strict_locals.as_ref()).into_iter().map(|d| util::coded(d, RUBY_SYNTAX)).collect()
}

/// Line index, line, the call as read, and the render.
type CheckedRender<'a> = (usize, &'a str, render_refs::RenderCall<'a>, render_refs::RenderRef);

/// The render checks of a Rails template in app/views: unresolved renders, then strict-locals mismatches.
fn render_diagnostics(
    path: &Path,
    project: &Project,
    text: &str,
    lang: TemplateLang,
    in_code: impl Fn(usize, usize, &render_refs::RenderRef) -> bool,
) -> Vec<Diagnostic> {
    if project.framework != Framework::Rails || project.is_gem_file(path) {
        return vec![];
    }
    let Some(views) = paths::views_root(path) else { return vec![] };
    let renders = checked_renders(project, path, &views, text, lang, in_code);
    let mut partials = Partials { project, path, resolved: HashMap::new(), signatures: HashMap::new() };
    let mut out = unresolved_renders(project, &renders, &mut partials);
    out.extend(strict_locals_mismatches(project, &renders, &mut partials));
    out
}

/// Literal renders in code that can be checked, as (line index, line, call, render): `dir/name` with an existing
/// app/views/dir or one no view root has (`view_roots`; a missing dir is usually a gem's or a pack's), and a bare name
/// in an action template when its controller chain is in the app and the app isn't a gem or engine (a partial's
/// bare names follow whichever controller renders it).
fn checked_renders<'a>(
    project: &Project,
    path: &Path,
    views: &Path,
    text: &'a str,
    lang: TemplateLang,
    in_code: impl Fn(usize, usize, &render_refs::RenderRef) -> bool,
) -> Vec<CheckedRender<'a>> {
    let in_engine = LazyCell::new(|| views.parent().and_then(Path::parent).is_some_and(is_gem_root));
    let bare_names_known =
        LazyCell::new(|| !*in_engine && !paths::is_partial_path(path) && controller_classes::bare_lookup_is_certain(project, path));
    let mut dirs: HashMap<String, bool> = HashMap::new();
    let mut checkable = |name: &str| {
        let Some(dir) = partials::split_render_name(name).0 else { return *bare_names_known };
        *dirs.entry(dir.to_string()).or_insert_with(|| views.join(dir).is_dir() || (!*in_engine && project.lacks_view_folder(dir)))
    };
    let pipes = HamlPipes::new(lang);
    let mut out = Vec::new();
    for (i, (line_start, line)) in util::lines_with_offsets(text).enumerate() {
        let refs = render_refs::render_calls_in(text, line_start, line, &pipes, project.prism_version(), |r| {
            in_code(i, line_start, r) && checkable(&r.name)
        });
        out.extend(refs.into_iter().map(|(r, call)| (i, line, call, r)));
    }
    out
}

/// Is `root` a gem or engine, whose templates also render the host app's partials?
fn is_gem_root(root: &Path) -> bool {
    std::fs::read_dir(root).is_ok_and(|mut files| files.any(|f| f.is_ok_and(|f| f.path().extension().is_some_and(|e| e == "gemspec"))))
}

/// A template's partials, each resolved and read once per pass: a view renders the same few many times.
struct Partials<'a> {
    project: &'a Project,
    path: &'a Path,
    resolved: HashMap<(render_refs::RenderKind, String, render_refs::Lookup), Vec<PathBuf>>,
    signatures: HashMap<PathBuf, Option<strict_locals::Signature>>,
}

impl Partials<'_> {
    fn resolve(&mut self, r: &render_refs::RenderRef) -> &[PathBuf] {
        let (project, path) = (self.project, self.path);
        self.resolved
            .entry((r.kind, r.name.clone(), r.lookup.clone()))
            .or_insert_with(|| partials::first_folder_only(partials::resolve(project, path, r)))
    }

    /// The signatures of `files`; none when a file has no `locals:` comment, so takes any local.
    fn signatures(&mut self, files: &[PathBuf]) -> Option<Vec<&strict_locals::Signature>> {
        let project = self.project;
        for file in files {
            self.signatures.entry(file.clone()).or_insert_with(|| project.file_strict_locals(file));
        }
        files.iter().map(|file| self.signatures[file].as_ref()).collect()
    }
}

/// Literal `render "x"` / `render partial:|template: "x"` with no matching file under app/views.
fn unresolved_renders(project: &Project, renders: &[CheckedRender], partials: &mut Partials) -> Vec<Diagnostic> {
    let path = partials.path;
    renders
        .iter()
        .filter(|(.., r)| partials.resolve(r).is_empty() && !is_gem_view(project, &r.name))
        .map(|(i, line, _, r)| {
            let message = missing_message(r, &searched_folders(project, path));
            util::coded(util::warning(util::span_range(*i, line, r.start, r.end), message), UNRESOLVED_RENDER)
        })
        .collect()
}

/// The folders Rails searches for a bare name in `view`: its controller's view folders; `application` when they're unknown.
fn searched_folders(project: &Project, view: &Path) -> Vec<String> {
    controller_classes::view_prefixes(project, view).unwrap_or_else(|| vec!["application".to_string()])
}

/// Rails' MissingTemplate message up to ` with {…}.`, which lists the request's formats and paths; they vary.
/// A bare name lists each folder in `searched`.
fn missing_message(r: &render_refs::RenderRef, searched: &[String]) -> String {
    let (dir, name) = partials::split_render_name(&r.name);
    let (kind, prefix) = if r.kind == render_refs::RenderKind::Partial { ("partial", "_") } else { ("template", "") };
    let paths = match dir {
        Some(dir) => format!("{dir}/{prefix}{name}"),
        None => searched.iter().map(|folder| format!("{folder}/{prefix}{name}")).collect::<Vec<_>>().join(", "),
    };
    format!("Missing {kind} {paths}")
}

/// Is `name`'s top folder (`devise/shared/links`) a gem's own view folder? Gems and engines ship views under their
/// name (`devise/`, `spree/` from `spree_core`), and the app overrides only some of them, so the rest aren't on disk.
fn is_gem_view(project: &Project, name: &str) -> bool {
    let top = partials::split_render_name(name).0.and_then(|dir| dir.split('/').next());
    top.is_some_and(|top| project.has_gem_family(top))
}

/// Literal renders whose locals don't fit the partial's `locals: (…)` signature.
fn strict_locals_mismatches(project: &Project, renders: &[CheckedRender], partials: &mut Partials) -> Vec<Diagnostic> {
    if !project.supports_strict_locals() {
        return vec![];
    }
    let suffix_with_template = project.gem_version("actionview").is_some_and(|v| v.at_least(STRICT_LOCALS_TEMPLATE_SUFFIX_SINCE));
    let mut out = Vec::new();
    // A partial's layout gets the partial's locals, which this check can't see.
    for (i, line, call, r) in renders.iter().filter(|(.., r)| !r.from_layout) {
        let Some(passed) = render_refs::passed_locals(call, r) else { continue };
        // Several variants/formats: which one Rails picks depends on the request, so only what fails in all of them counts.
        let files = partials.resolve(r).to_vec();
        let Some(first) = files.first() else { continue };
        let Some(sigs) = partials.signatures(&files) else { continue };
        let no_locals = sigs.iter().all(|sig| sig.takes_no_locals());
        let missing = strict_locals::missing_in_all(&sigs, &passed);
        let Some(mut message) = strict_locals_message(&missing, &strict_locals::unknown_to_all(&sigs, &passed), no_locals) else {
            continue;
        };
        if suffix_with_template {
            // Rails prints the absolute path; relative to the app folder is stable and readable.
            message.push_str(&format!(" for {}", paths::rails_relative(first, first)));
            message.push_str(&partials::and_others(files.len() - 1));
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
    use crate::options::Options;
    use crate::util::write_files;
    use std::collections::BTreeSet;
    use tower_lsp_server::ls_types::{DiagnosticSeverity, NumberOrString, Position, Range};

    #[test]
    fn void_children_only_for_html_slim() {
        let text = "img\n  | x\n";
        let has_void = |path: &str| {
            let uri = path.parse::<Uri>().unwrap();
            diagnostics(
                &uri,
                &Doc { lang: TemplateLang::Slim, text: text.into(), plain_erb: false, percent_lines: false, version: 0 },
                &Options::default(),
            )
            .iter()
            .any(|d| d.code == Some(NumberOrString::String(slim::indent::code::VOID_CHILDREN.into())))
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
    fn switched_off_languages_get_no_diagnostics() {
        let init = serde_json::json!({
            "erb": { "diagnostics": { "enabled": false } },
            "slim": { "format": { "enabled": false } },
            "haml": { "diagnostics": { "enabled": true }, "format": {} }
        });
        let options = Options::from_init(Some(&init)).0;
        let count = |lang, text: &str, options: &Options| {
            let uri = "file:///v/show.html.erb".parse::<Uri>().unwrap();
            diagnostics(&uri, &Doc { lang, text: text.into(), plain_erb: false, percent_lines: false, version: 0 }, options).len()
        };
        assert_eq!(count(TemplateLang::Erb, "<% end %>\n", &Options::default()), 1);
        assert_eq!(count(TemplateLang::Erb, "<% end %>\n", &options), 0);
        let haml = "%p hi\n  %b x\n";
        assert_eq!(count(TemplateLang::Haml, haml, &options), count(TemplateLang::Haml, haml, &Options::default()));
        let slim = "p\n   a\n  b\n";
        assert_ne!(count(TemplateLang::Slim, slim, &options), 0);
        // Skim follows Slim.
        let off = Options::from_init(Some(&serde_json::json!({ "slim": { "diagnostics": { "enabled": false } } }))).0;
        assert_eq!(count(TemplateLang::Skim, slim, &off), 0);
    }

    fn disabling_haml(code: &str) -> Options {
        Options::from_init(Some(&serde_json::json!({ "haml": { "disabledDiagnostics": [code] } }))).0
    }

    #[test]
    fn disabled_diagnostics_per_language() {
        let codes = |path: &str, lang, text: &str, options: &Options| -> Vec<String> {
            let uri = path.parse::<Uri>().unwrap();
            diagnostics(&uri, &Doc { lang, text: text.into(), plain_erb: false, percent_lines: false, version: 0 }, options)
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
        let erb_cases = [("<% end %>\n", RUBY_SYNTAX), ("<div>\n", tags::code::UNCLOSED_ELEMENT), ("hi\n<% foo", erb::code::UNCLOSED_TAG)];
        for (text, code) in erb_cases {
            assert_eq!(codes(erb_path, TemplateLang::Erb, text, &all), [code]);
        }
        let options = Options::from_init(Some(&serde_json::json!({
            "erb": { "disabledDiagnostics": [RUBY_SYNTAX, tags::code::UNCLOSED_ELEMENT, erb::code::UNCLOSED_TAG] },
            "haml": { "disabledDiagnostics": [haml::indent::code::ILLEGAL_NESTING] }
        })))
        .0;
        assert!(erb_cases.iter().all(|(text, _)| codes(erb_path, TemplateLang::Erb, text, &options).is_empty()));
        // Text that is not HTML has no `<` to misparse.
        assert!(codes("file:///v/show.text.erb", TemplateLang::Erb, "hi <% foo", &all).is_empty());
        let haml_path = "file:///v/show.html.haml";
        assert!(codes(haml_path, TemplateLang::Haml, haml, &all).contains(&haml::indent::code::ILLEGAL_NESTING.to_string()));
        let haml_codes = codes(haml_path, TemplateLang::Haml, haml, &options);
        assert!(!haml_codes.contains(&haml::indent::code::ILLEGAL_NESTING.to_string()), "{haml_codes:?}");
        // Turning one code off leaves the others.
        let void = "%br\n  %p\n";
        assert_eq!(codes(haml_path, TemplateLang::Haml, void, &all), [haml::indent::code::VOID_CHILDREN]);
        let quiet = disabling_haml(haml::indent::code::VOID_CHILDREN);
        assert!(codes(haml_path, TemplateLang::Haml, void, &quiet).is_empty());
        // Haml's Ruby is parsed once Haml itself accepts the template.
        let ruby = codes(haml_path, TemplateLang::Haml, "= foo(\n", &all);
        assert!(!ruby.is_empty() && ruby.iter().all(|c| c == RUBY_SYNTAX), "{ruby:?}");
        let ruby_off = disabling_haml(RUBY_SYNTAX);
        assert!(codes(haml_path, TemplateLang::Haml, "= foo(\n", &ruby_off).is_empty());
        let after_error = codes(haml_path, TemplateLang::Haml, "- if x\n  %p\n- end\n= foo(\n", &all);
        assert_eq!(after_error, [haml::indent::code::EXPLICIT_END]);
        // Every check carries a code, or the setting can't turn it off.
        assert!(!codes(erb_path, TemplateLang::Erb, include_str!("../../test/broken/bad.html.erb"), &all).is_empty());
        assert!(!codes(haml_path, TemplateLang::Haml, include_str!("../../test/broken/bad.html.haml"), &all).is_empty());
    }

    #[test]
    fn a_rule_turned_off_in_the_config_file_reports_nothing() {
        let root = std::env::temp_dir().join(format!("rt-rule-off-{}", std::process::id()));
        write_files(
            &root,
            &[
                (".ruby-templates.toml", "root = true\n[Html.UnclosedElement]\nenabled = false\n"),
                ("quiet/show.html.erb", ""),
                ("loud/.ruby-templates.toml", "[Html]\nenabled = true\n"),
                ("loud/show.html.erb", ""),
            ],
        );
        let codes = |file: &str| -> Vec<String> {
            let doc = Doc { lang: TemplateLang::Erb, text: "<div>\n<%= x %>\n".into(), plain_erb: false, percent_lines: false, version: 0 };
            let found = diagnostics(&util::file_uri(root.join(file)).unwrap(), &doc, &Options::default());
            found.iter().filter_map(|d| util::code(d).map(String::from)).collect()
        };
        assert!(codes("quiet/show.html.erb").is_empty());
        // The nearer file's department switch decides over the rule's switch in the file above.
        assert_eq!(codes("loud/show.html.erb"), [tags::code::UNCLOSED_ELEMENT]);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(codes("quiet/show.html.erb"), [tags::code::UNCLOSED_ELEMENT]);
    }

    #[test]
    fn settings_list_every_rule_and_accept_the_old_codes() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../../package.json")).unwrap();
        let items = |lang: &str| {
            manifest["contributes"]["configuration"]["properties"][format!("rubyTemplates.{lang}.disabledDiagnostics")]["items"].clone()
        };
        for (section, lang) in
            [("erb", TemplateLang::Erb), ("haml", TemplateLang::Haml), ("slim", TemplateLang::Slim), ("jbuilder", TemplateLang::Ruby)]
        {
            let items = items(section);
            let listed: BTreeSet<&str> = items["anyOf"][0]["enum"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
            let lints = || crate::rules::RULES.iter().filter(|rule| rule.kind == crate::rules::Kind::Lint);
            let rules: BTreeSet<&str> = lints().filter(|rule| rule.applies_to(lang)).map(|rule| rule.name).collect();
            assert_eq!(listed, rules, "{section}");
            let old_codes = regex::Regex::new(items["anyOf"][1]["pattern"].as_str().unwrap()).unwrap();
            assert!(lints().all(|rule| rule.alias.is_none_or(|alias| old_codes.is_match(alias))), "{section}");
        }
        // A literal code would bypass the constants this test collects.
        let literal = regex::Regex::new(r#"(?m)coded\(.*,\s*"[a-z0-9-]+"\)[);,]*$"#).unwrap();
        for src in [
            include_str!("checks.rs"),
            include_str!("lsp.rs"),
            include_str!("ruby/generated.rs"),
            include_str!("erb.rs"),
            include_str!("erb/tags.rs"),
            include_str!("erb/html_lint.rs"),
            include_str!("haml/indent.rs"),
            include_str!("slim/indent.rs"),
        ] {
            assert!(!literal.is_match(src.split("#[cfg(test)]").next().unwrap()), "coded(…, \"literal\")");
        }
    }

    #[test]
    fn output_lints_run_in_rails_projects_only() {
        let root = std::env::temp_dir().join(format!("rt-output-lints-{}", std::process::id()));
        let (rails, padrino) = (root.join("rails"), root.join("padrino"));
        for dir in [&rails, &padrino] {
            std::fs::create_dir_all(dir.join("app/views")).unwrap();
        }
        std::fs::write(rails.join("Gemfile.lock"), format!("{GEM_LOCK}    actionview (8.0.2)\n")).unwrap();
        std::fs::write(padrino.join("Gemfile.lock"), format!("{GEM_LOCK}    padrino-helpers (0.16.0)\n")).unwrap();
        let codes = |dir: &Path, name: &str, options: &Options| {
            let uri = crate::util::file_uri(dir.join(name)).unwrap();
            let doc = Doc {
                lang: TemplateLang::Erb,
                text: "<% link_to \"/blog\" do %>x<% end %>".into(),
                plain_erb: false,
                percent_lines: false,
                version: 0,
            };
            diagnostics(&uri, &doc, options).iter().filter_map(|d| util::code(d).map(String::from)).collect::<Vec<_>>()
        };
        let options = Options::default();
        assert_eq!(codes(&rails, "app/views/a.html.erb", &options), ["Rails/SilentHelper"]);
        assert!(codes(&padrino, "app/views/a.html.erb", &options).is_empty());
        assert!(codes(&rails, "config/nginx.conf.erb", &options).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
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
            let uri = crate::util::file_uri(views.join(name)).unwrap();
            let found = diagnostics(
                &uri,
                &Doc { lang, text: text.into(), plain_erb: false, percent_lines: false, version: 0 },
                &Options::default(),
            );
            assert!(!found.is_empty(), "{name}");
            for d in found {
                assert_eq!(d.source.as_deref(), Some(source), "{name}: {}", d.message);
            }
        }
        let uri = crate::util::file_uri(views.join("g.html.erb")).unwrap();
        let html = Doc { lang: TemplateLang::Erb, text: "<div>\n".into(), plain_erb: false, percent_lines: false, version: 0 };
        let found = diagnostics(&uri, &html, &Options::default());
        assert_eq!(found.iter().filter_map(util::code).collect::<Vec<_>>(), [tags::code::UNCLOSED_ELEMENT]);
        assert_eq!(found[0].source.as_deref(), Some("erb"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn untitled_erb_gets_html_diagnostics() {
        let codes = |uri: &str, plain_erb| {
            let doc = Doc { lang: TemplateLang::Erb, text: "<div>\n".into(), plain_erb, percent_lines: false, version: 0 };
            let found = diagnostics(&uri.parse::<Uri>().unwrap(), &doc, &Options::default());
            found.iter().filter_map(util::code).map(str::to_owned).collect::<Vec<_>>()
        };
        assert_eq!(codes("untitled:Untitled-1", true), [tags::code::UNCLOSED_ELEMENT]);
        assert!(codes("untitled:Untitled-1", false).is_empty());
        assert!(codes("file:///v/nginx.erb", true).is_empty());
    }

    #[test]
    fn bare_erb_view_that_starts_with_markup_gets_html_diagnostics() {
        let codes = |uri: &str, text: &str| {
            let doc = Doc { lang: TemplateLang::Erb, text: text.into(), plain_erb: false, percent_lines: false, version: 0 };
            let found = diagnostics(&uri.parse::<Uri>().unwrap(), &doc, &Options::default());
            found.iter().filter_map(util::code).map(str::to_owned).collect::<Vec<_>>()
        };
        let unclosed = [tags::code::UNCLOSED_ELEMENT];
        assert_eq!(codes("file:///r/app/views/users/show.erb", "<%# hi %>\n<div>\n"), unclosed);
        assert!(codes("file:///r/app/views/users/show.erb", "Hello <div>\n").is_empty());
        assert_eq!(codes("file:///r/app/views/users/show.erb", "<%= x %>\n<div>\n"), unclosed);
        assert!(codes("file:///r/app/views/users/show.erb", "<%= x %>\nHello <div>\n").is_empty());
        assert!(codes("file:///r/config/nginx.erb", "<div>\n").is_empty());
        assert!(codes("file:///r/app/views/users/nginx.conf.erb", "<div>\n").is_empty());
        assert!(codes("file:///r/app/views/users/Dockerfile.erb", "<?xml version=\"1.0\"?><a>\n").is_empty());
    }

    #[test]
    fn missing_message_names_each_searched_folder_only_for_bare_names() {
        let message = |text: &str, searched: &[&str]| {
            let refs = render_refs::render_refs(text);
            let searched: Vec<String> = searched.iter().map(|s| s.to_string()).collect();
            missing_message(&refs[0], &searched)
        };
        assert_eq!(message("render \"crad\"", &["posts", "application"]), "Missing partial posts/_crad, application/_crad");
        assert_eq!(message("render \"crad\"", &["application"]), "Missing partial application/_crad");
        assert_eq!(message("render \"shared/crad\"", &["posts", "application"]), "Missing partial shared/_crad");
        assert_eq!(message("render template: \"crad\"", &["posts", "application"]), "Missing template posts/crad, application/crad");
    }

    #[test]
    fn skim_skips_ruby_checks() {
        let uri = "untitled:Untitled-1".parse::<Uri>().unwrap();
        assert_eq!(TemplateLang::from_uri_or_id(&uri, Some("skim")), Some(TemplateLang::Skim));
        let ruby = "- else\n= alert(\"hi\"\n";
        let explicit_end = "- end = start + 10\n";
        let dedent = "  p\n    a\n   span\n";
        let codes = |lang, text: &str| -> Vec<String> {
            diagnostics(&uri, &Doc { lang, text: text.into(), plain_erb: false, percent_lines: false, version: 0 }, &Options::default())
                .into_iter()
                .filter_map(|d| match d.code {
                    Some(NumberOrString::String(c)) => Some(c),
                    _ => None,
                })
                .collect()
        };
        let slim = [codes(TemplateLang::Slim, ruby), codes(TemplateLang::Slim, explicit_end)].concat();
        assert!(slim::indent::code::RUBY.iter().all(|c| slim.iter().any(|s| s == c)), "{slim:?}");
        assert_eq!(codes(TemplateLang::Skim, ruby), Vec::<String>::new());
        assert_eq!(codes(TemplateLang::Skim, explicit_end), Vec::<String>::new());
        assert_eq!(codes(TemplateLang::Skim, dedent), [slim::indent::code::BAD_DEDENT]);
    }

    const GEM_LOCK: &str = "GEM\n  specs:\n    devise (4.9.4)\n      bcrypt (~> 3.0)\n    spree_core (4.8.0)\n    administrate (0.20.1)\n";

    fn lock_with(actionview: &str) -> Project {
        Project::from_lockfile(&format!("{GEM_LOCK}    actionview ({actionview})\n"))
    }

    fn render_checks(lang: TemplateLang, path: &Path, project: &Project, text: &str, code: &str) -> Vec<Diagnostic> {
        let found = render_diagnostics(path, project, text, lang, render_filter(lang, text, &[]));
        found.into_iter().filter(|d| util::code(d) == Some(code)).collect()
    }

    fn unresolved(lang: TemplateLang, path: &Path, text: &str) -> Vec<Diagnostic> {
        render_checks(lang, path, &lock_with("7.1.0"), text, UNRESOLVED_RENDER)
    }

    #[test]
    fn thor_templates_skip_structure_checks() {
        let text = "%table\n  %tr\n<% attributes.each do |attribute| -%>\n    %th <%= attribute.human_name %>\n<% end -%>\n";
        let doc = || Doc { lang: TemplateLang::Haml, text: text.to_string(), plain_erb: false, percent_lines: false, version: 0 };
        let uri = |name: &str| format!("file:///app/lib/templates/haml/scaffold/{name}").parse::<Uri>().unwrap();
        assert!(diagnostics(&uri("index.html.haml.tt"), &doc(), &Options::default()).is_empty());
        assert!(!diagnostics(&uri("index.html.haml"), &doc(), &Options::default()).is_empty());
        let erb = Doc {
            lang: TemplateLang::Erb,
            text: "<%% if x %>\n<% attributes.each do |a| -%>\n<div>\n".into(),
            plain_erb: false,
            percent_lines: false,
            version: 0,
        };
        assert!(diagnostics(&uri("_form.html.erb.tt"), &erb, &Options::default()).is_empty());
    }

    #[test]
    fn leading_slash_renders_are_checked_under_views() {
        let root = std::env::temp_dir().join(format!("rt-slash-unresolved-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        std::fs::write(users.join("_card.html.erb"), "").unwrap();
        let text = "<%= render \"/users/missing\" %> <%= render \"/users/card\" %>\n";
        let got: Vec<String> = unresolved(TemplateLang::Erb, &users.join("index.html.erb"), text).into_iter().map(|d| d.message).collect();
        assert_eq!(got, ["Missing partial users/_missing"]);
        std::fs::remove_dir_all(&root).unwrap();
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
        let card = render_refs::render_refs(r#"<%= render layout: "card" do %>"#);
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
        let gem_text = "<%= render \"devise/shared/error_messages\" %><%= render \"spree/admin/x\" %><%= render \"/devise/shared/links\" %><%= render \"admin/x\" %>";
        let lines: Vec<String> = unresolved(TemplateLang::Erb, &file, gem_text).into_iter().map(|d| d.message).collect();
        assert_eq!(lines, ["Missing partial admin/_x"]);
        assert!(!in_ruby_context(TemplateLang::Skim, "= render \"x\"", &[], 10));
        assert!(in_ruby_context(TemplateLang::Slim, "= render \"x\"", &[], 10));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_folders_are_reported_only_when_every_view_root_is_known() {
        let lock = |gems: &str| format!("GEM\n  specs:\n    actionview (7.1.0)\n{gems}");
        let viewless = "    zz_pg (1.5.0)\n";
        let engine = "    zz_engine (1.0.0)\n      railties (>= 6.0)\n    railties (7.1.0)\n";
        let text = "<%= render \"uesrs/card\" %><%= render \"packs_x/box\" %><%= render \"fancy/nav\" %><%= render \"rails/info\" %>";
        let messages = |name: &str, gems: &str, extra: &[(&str, &str)]| {
            let root = std::env::temp_dir().join(format!("rt-missing-folder-{name}-{}", std::process::id()));
            std::fs::remove_dir_all(&root).ok();
            write_files(&root, &[("app/views/users/show.html.erb", ""), ("Gemfile.lock", &lock(gems))]);
            write_files(&root, extra);
            let view = root.join("app/views/users/show.html.erb");
            let project = Project::for_template(&view);
            let found = render_checks(TemplateLang::Erb, &view, &project, text, UNRESOLVED_RENDER);
            std::fs::remove_dir_all(&root).unwrap();
            found.into_iter().map(|d| d.message).collect::<Vec<_>>()
        };
        let packs = ("packs/packs_x/app/views/packs_x/_box.html.erb", "");
        let (typo, box_) = ("Missing partial uesrs/_card", "Missing partial packs_x/_box");
        assert_eq!(messages("plain", viewless, &[]), [typo, box_, "Missing partial fancy/_nav"]);
        assert_eq!(messages("pack", viewless, &[packs]), [typo, "Missing partial fancy/_nav"], "a pack's views are a root");
        assert!(messages("engine", engine, &[packs]).is_empty(), "an engine gem that isn't installed may hold any folder");
        let installed = ("vendor/bundle/ruby/3.3.0/gems/zz_engine-1.0.0/app/views/fancy/_nav.html.erb", "");
        assert_eq!(messages("installed", engine, &[packs, installed]), [typo]);
        let prepended = ("config/initializers/views.rb", "ActionController::Base.prepend_view_path \"themes\"\n");
        assert!(messages("prepended", viewless, &[prepended]).is_empty());
    }

    #[test]
    fn bare_names_are_checked_against_the_controllers_prefixes() {
        let root = std::env::temp_dir().join(format!("rt-unresolved-bare-{}", std::process::id()));
        write_files(
            &root,
            &[
                ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
                ("app/controllers/admin/base_controller.rb", "class Admin::BaseController < ApplicationController\nend\n"),
                ("app/controllers/admin/users_controller.rb", "class Admin::UsersController < Admin::BaseController\nend\n"),
                ("app/controllers/engine/pages_controller.rb", "class Engine::PagesController < Engine::Gem::Controller\nend\n"),
                ("app/views/admin/users/index.html.erb", ""),
                ("app/views/admin/users/_row.html.erb", ""),
                ("app/views/admin/base/_nav.html.erb", ""),
                ("app/views/application/_flash.html.erb", ""),
                ("app/views/engine/pages/index.html.erb", ""),
                ("app/views/shared/_box.html.erb", ""),
                ("app/views/user_mailer/welcome.html.erb", ""),
            ],
        );
        let messages = |view: &str, text: &str| {
            let found = unresolved(TemplateLang::Erb, &root.join("app/views").join(view), text);
            found.into_iter().map(|d| d.message).collect::<Vec<_>>()
        };
        let text = "<%= render \"row\" %><%= render \"nav\" %><%= render \"flash\" %><%= render partial: \"crad\" %>";
        assert_eq!(
            messages("admin/users/index.html.erb", text),
            ["Missing partial admin/users/_crad, admin/base/_crad, application/_crad"]
        );
        assert!(messages("engine/pages/index.html.erb", text).is_empty(), "a superclass outside the app");
        assert!(messages("shared/_box.html.erb", text).is_empty(), "the rendering controller is unknown");
        assert!(messages("user_mailer/welcome.html.erb", text).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bare_names_follow_the_controller_class_however_it_is_written() {
        let root = std::env::temp_dir().join(format!("rt-unresolved-bare-forms-{}", std::process::id()));
        write_files(
            &root,
            &[
                ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
                ("app/controllers/admin/base_controller.rb", "module Admin\n  class BaseController < ApplicationController\n  end\nend\n"),
                ("app/controllers/admin/users_controller.rb", "module Admin\n  class UsersController < BaseController\n  end\nend\n"),
                ("app/controllers/users/sessions_controller.rb", "class Users::SessionsController < Devise::SessionsController\nend\n"),
                ("app/views/admin/users/index.html.haml", ""),
                ("app/views/admin/base/_nav.html.haml", ""),
                ("app/views/users/sessions/new.html.slim", ""),
            ],
        );
        let messages = |lang, view: &str, text: &str| {
            let found = unresolved(lang, &root.join("app/views").join(view), text);
            found.into_iter().map(|d| d.message).collect::<Vec<_>>()
        };
        let haml = "= render \"nav\"\n= render \"crad\"\n";
        assert_eq!(
            messages(TemplateLang::Haml, "admin/users/index.html.haml", haml),
            ["Missing partial admin/users/_crad, admin/base/_crad, application/_crad"]
        );
        let slim = "== render \"crad\"\n";
        assert!(messages(TemplateLang::Slim, "users/sessions/new.html.slim", slim).is_empty(), "a gem controller's views");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_controller_below_action_controller_base_does_not_search_application() {
        let root = std::env::temp_dir().join(format!("rt-unresolved-api-{}", std::process::id()));
        write_files(
            &root,
            &[
                ("app/controllers/api_controller.rb", "class ApiController < ActionController::Base\nend\n"),
                ("app/views/api/index.html.erb", ""),
                ("app/views/application/_crad.html.erb", ""),
            ],
        );
        let view = root.join("app/views/api/index.html.erb");
        let found = unresolved(TemplateLang::Erb, &view, "<%= render \"crad\" %>");
        assert_eq!(found.into_iter().map(|d| d.message).collect::<Vec<_>>(), ["Missing partial api/_crad"]);
        assert!(crate::rails::partials::resolve(
            &Project::for_template(&view),
            &view,
            &crate::rails::render_refs::render_refs("render \"crad\"")[0]
        )
        .is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bare_names_are_unchecked_when_lookup_paths_come_from_elsewhere() {
        let path_gem = "PATH\n  remote: engines/theme\n  specs:\n    theme (0.1.0)\n\n";
        let themeable = "module Themeable\n  included { prepend_view_path \"themes\" }\nend\n";
        type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a str);
        let includes = |module: &str| format!("class UsersController < ApplicationController\n  include {module}\nend\n");
        let users = "app/controllers/users_controller.rb";
        let cases: [Case; 10] = [
            ("baseline", &[], ""),
            ("absolute-parent", &[("app/controllers/admin/users_controller.rb", "class Admin::UsersController < ::UsersController\n")], ""),
            ("concern", &[("app/controllers/concerns/themeable.rb", themeable)], ""),
            ("initializer", &[("config/initializers/views.rb", "ActionController::Base.prepend_view_path \"themes\"\n")], ""),
            ("path-gem", &[("engines/theme/app/views/x/.keep", "")], path_gem),
            ("lib-concern", &[(users, &includes("Themeable")), ("lib/themeable.rb", themeable)], ""),
            ("model-concern", &[(users, &includes("Themeable")), ("app/models/concerns/themeable.rb", themeable)], ""),
            ("namespaced", &[(users, &includes("::Ui::Themeable")), ("app/models/ui/themeable.rb", themeable)], ""),
            (
                "nested-include",
                &[
                    (users, &includes("Ui::Outer")),
                    ("lib/ui/outer.rb", "module Ui::Outer\n  include Themeable\nend\n"),
                    ("lib/themeable.rb", themeable),
                ],
                "",
            ),
            ("benign-include", &[(users, &includes("Pagy::Backend, Plain")), ("lib/plain.rb", "module Plain\nend\n")], ""),
        ];
        for (name, extra, lock_head) in cases {
            let root = std::env::temp_dir().join(format!("rt-unresolved-lookup-{name}-{}", std::process::id()));
            let lock = format!("{lock_head}{GEM_LOCK}    actionview (7.1.0)\n");
            write_files(
                &root,
                &[
                    ("Gemfile.lock", &lock),
                    ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
                    ("app/controllers/users_controller.rb", "class UsersController < ApplicationController\nend\n"),
                    ("app/views/users/index.html.erb", ""),
                    ("app/views/admin/users/_admin_bar.html.erb", ""),
                ],
            );
            write_files(&root, extra);
            let view = root.join("app/views/users/index.html.erb");
            let found =
                render_checks(TemplateLang::Erb, &view, &Project::for_template(&view), "<%= render \"admin_bar\" %>", UNRESOLVED_RENDER);
            assert_eq!(found.len(), usize::from(name == "baseline" || name == "benign-include"), "{name}");
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    #[test]
    fn bare_names_are_unchecked_when_an_installed_gem_ships_the_controllers_views() {
        let gem_views = "vendor/bundle/ruby/3.3.0/gems/theme-1.0/app/views";
        let lock = format!("{GEM_LOCK}    theme (1.0)\n      railties\n    railties (7.1.0)\n    actionview (7.1.0)\n");
        let cases = [
            ("shipped", Some("users/_admin_bar.html.erb"), true, 0),
            ("other-folder", Some("posts/_x.html.erb"), true, 1),
            ("not-an-engine", Some("users/_admin_bar.html.erb"), false, 1),
            ("missing", None, true, 0),
        ];
        for (name, gem_file, engine, expected) in cases {
            let root = std::env::temp_dir().join(format!("rt-unresolved-gem-views-{name}-{}", std::process::id()));
            write_files(
                &root,
                &[
                    ("Gemfile.lock", &lock),
                    ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
                    ("app/controllers/users_controller.rb", "class UsersController < ApplicationController\nend\n"),
                    ("app/views/users/index.html.erb", ""),
                ],
            );
            if let Some(file) = gem_file {
                write_files(&root, &[(&format!("{gem_views}/{file}"), "")]);
                if engine {
                    write_files(&root, &[("vendor/bundle/ruby/3.3.0/gems/theme-1.0/lib/theme/engine.rb", "")]);
                }
            }
            let view = root.join("app/views/users/index.html.erb");
            let found =
                render_checks(TemplateLang::Erb, &view, &Project::for_template(&view), "<%= render \"admin_bar\" %>", UNRESOLVED_RENDER);
            assert_eq!(found.len(), expected, "{name}");
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    #[test]
    fn a_partial_is_checked_for_qualified_names_only_since_its_renderer_varies() {
        let root = std::env::temp_dir().join(format!("rt-unresolved-shared-folder-{}", std::process::id()));
        write_files(
            &root,
            &[
                ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
                ("app/controllers/users_controller.rb", "class UsersController < ApplicationController\nend\n"),
                ("app/views/users/_card.html.erb", ""),
            ],
        );
        let card = root.join("app/views/users/_card.html.erb");
        assert!(unresolved(TemplateLang::Erb, &card, "<%= render \"badge\" %>").is_empty(), "a bare name follows the rendering controller");
        let found = unresolved(TemplateLang::Erb, &card, "<%= render \"users/badge\" %>");
        assert_eq!(found.into_iter().map(|d| d.message).collect::<Vec<_>>(), ["Missing partial users/_badge"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bare_names_are_unchecked_where_the_lookup_folders_are_not_the_chain() {
        let root = std::env::temp_dir().join(format!("rt-unresolved-bare-unknown-{}", std::process::id()));
        let base = "class ApplicationController < ActionController::Base\nend\n";
        write_files(
            &root,
            &[
                ("app/controllers/application_controller.rb", base),
                ("app/controllers/posts_controller.rb", "class PostsController < ApplicationController\nend\n"),
                (
                    "app/controllers/users_controller.rb",
                    "class UsersController < ApplicationController\n  prepend_view_path \"themes\"\nend\n",
                ),
                (
                    "app/controllers/pages_controller.rb",
                    "class PagesController < ApplicationController\n  def self.local_prefixes = [\"x\"]\nend\n",
                ),
                ("app/controllers/admin/base_controller.rb", "class Admin::BaseController < ApplicationController\nend\n"),
                ("app/controllers/admin/users_controller.rb", "class Admin::UsersController < Admin::BaseController\nend\n"),
                ("app/views/posts/index.html.erb", ""),
                ("app/views/users/index.html.erb", ""),
                ("app/views/pages/index.html.erb", ""),
                ("app/views/application/index.html.erb", ""),
                ("app/views/admin/base/_nav.html.erb", ""),
            ],
        );
        let messages = |view: &str| {
            let found = unresolved(TemplateLang::Erb, &root.join("app/views").join(view), "<%= render \"crad\" %>");
            found.into_iter().map(|d| d.message).collect::<Vec<_>>()
        };
        assert_eq!(messages("posts/index.html.erb"), ["Missing partial posts/_crad, application/_crad"]);
        for view in ["users/index.html.erb", "pages/index.html.erb"] {
            assert!(messages(view).is_empty(), "{view} changes its lookup folders");
        }
        for view in ["application/index.html.erb", "admin/base/_nav.html.erb"] {
            assert!(messages(view).is_empty(), "{view} is rendered by subclasses too");
        }
        write_files(
            &root,
            &[(
                "app/controllers/application_controller.rb",
                "class ApplicationController < ActionController::Base\n  before_action { append_view_path \"x\" }\nend\n",
            )],
        );
        assert!(messages("posts/index.html.erb").is_empty(), "a parent changes the lookup folders");
        write_files(&root, &[("app/controllers/application_controller.rb", base), ("shop.gemspec", "")]);
        assert!(messages("posts/index.html.erb").is_empty(), "an engine renders the host app's partials");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn block_layout_keeps_the_calls_formats() {
        let root = std::env::temp_dir().join(format!("rt-unresolved-block-layout-{}", std::process::id()));
        let shared = root.join("app/views/shared");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("_wrap.json.erb"), "").unwrap();
        let file = root.join("app/views/index.html.erb");
        let text = "<%= render layout: \"shared/wrap\", formats: [:json] do %>x<% end %>";
        assert!(unresolved(TemplateLang::Erb, &file, text).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn block_layout_after_formats_is_checked_in_those_formats() {
        let root = std::env::temp_dir().join(format!("rt-unresolved-formats-layout-{}", std::process::id()));
        let shared = root.join("app/views/shared");
        std::fs::create_dir_all(&shared).unwrap();
        let file = root.join("app/views/index.html.erb");
        let text = "<%= render formats: [:json], layout: \"shared/wrap\" do %>x<% end %>";
        let messages = || unresolved(TemplateLang::Erb, &file, text).into_iter().map(|d| d.message).collect::<Vec<_>>();
        assert_eq!(messages(), ["Missing partial shared/_wrap"]);
        std::fs::write(shared.join("_wrap.json.erb"), "").unwrap();
        assert!(messages().is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_partial_on_a_continuation_line_is_reported_there() {
        let root = std::env::temp_dir().join(format!("rt-unresolved-continued-{}", std::process::id()));
        std::fs::create_dir_all(root.join("app/views/users")).unwrap();
        let file = root.join("app/views/users/index.html.erb");
        let missing = |text: &str| -> Vec<(u32, u32, String)> {
            let found = unresolved(TemplateLang::Erb, &file, text);
            found.into_iter().map(|d| (d.range.start.line, d.range.start.character, d.message)).collect()
        };
        let text = "<%= render formats: [:json],\r\n           partial: \"users/card\" %>\r\n";
        assert_eq!(missing(text), [(1, 21, "Missing partial users/_card".to_string())]);
        assert!(missing("<%= link_to \"x\",\n  partial: \"users/card\" %>\n").is_empty());
        std::fs::write(root.join("app/views/users/_card.json.erb"), "").unwrap();
        assert!(missing(text).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lone_layout_is_missing_only_with_a_block() {
        let root = std::env::temp_dir().join(format!("rt-unresolved-lone-layout-{}", std::process::id()));
        std::fs::create_dir_all(root.join("app/views/shared")).unwrap();
        let file = root.join("app/views/index.html.erb");
        let messages = |text: &str| unresolved(TemplateLang::Erb, &file, text).into_iter().map(|d| d.message).collect::<Vec<_>>();
        for options in ["inline: \"<p>x</p>\"", "html: \"x\"", "plain: \"x\"", "locals: { a: 1 }", "collection: @x"] {
            assert!(messages(&format!("<%= render {options}, layout: \"shared/box\" %>")).is_empty(), "{options}");
            assert_eq!(
                messages(&format!("<%= render {options}, layout: \"shared/box\" do %>x<% end %>")),
                ["Missing partial shared/_box"],
                "{options}"
            );
        }
        for text in [
            "<%= render html: \"x\", layout: \"shared/box\" %><% @items.each do |i| %>x<% end %>",
            "<%= render collection: @x, layout: \"shared/box\", cb: proc { |a| a } %>",
        ] {
            assert!(messages(text).is_empty(), "{text}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lookup_options_decide_unresolved_renders() {
        let root = std::env::temp_dir().join(format!("rt-unresolved-lookup-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        std::fs::write(users.join("_card.json.jbuilder"), "").unwrap();
        let file = users.join("index.html.erb");
        let missing = |text: &str| -> Vec<(u32, String)> {
            unresolved(TemplateLang::Erb, &file, text).into_iter().map(|d| (d.range.start.line, d.message)).collect()
        };
        assert!(missing(r#"<%= render partial: "users/card", formats: [:json] %>"#).is_empty());
        assert_eq!(missing(r#"<%= render partial: "users/card", formats: [:xml] %>"#), [(0, "Missing partial users/_card".to_string())]);
        // One partial, two lookups: the first result must not be reused.
        let twice = "<%= render partial: \"users/card\", formats: [:json] %>\n<%= render partial: \"users/card\", formats: [:xml] %>\n";
        assert_eq!(missing(twice), [(1, "Missing partial users/_card".to_string())]);
        let multi_line =
            "<%= render partial: \"users/card\",\n  formats: [:json] %>\n<%= render(partial: \"users/card\",\n  formats: [:xml]) %>\n";
        assert_eq!(missing(multi_line), [(2, "Missing partial users/_card".to_string())]);
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
        let lang = |path: &str, id| TemplateLang::from_uri_or_id(&path.parse::<Uri>().unwrap(), id);
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
        let doc = Doc {
            lang: TemplateLang::Erb,
            text: "<%= render \"users/card\" %><%= render \"users/gone\" %>".into(),
            plain_erb: false,
            percent_lines: false,
            version: 0,
        };
        let uri = crate::util::file_uri(&file).unwrap();
        let codes = || -> Vec<String> {
            let found = diagnostics(&uri, &doc, &Options::default());
            found.iter().filter_map(|d| util::code(d).map(String::from)).collect()
        };
        let lock = root.join("Gemfile.lock");
        let (both, unresolved_only) = (vec![UNRESOLVED_RENDER, STRICT_LOCALS], vec![UNRESOLVED_RENDER]);
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
        let doc = Doc {
            lang: TemplateLang::Erb,
            text: "<%# locals: (user, size:) -%>\n".into(),
            plain_erb: false,
            percent_lines: false,
            version: 0,
        };
        let uri = crate::util::file_uri(&file).unwrap();
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
        let broken =
            Doc { lang: TemplateLang::Erb, text: "<%# locals: (a: ] ) %>\n".into(), plain_erb: false, percent_lines: false, version: 0 };
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
            let doc = Doc {
                lang,
                text: format!("{comment} locals: (user, size:)\n{body} x\n"),
                plain_erb: false,
                percent_lines: false,
                version: 0,
            };
            let found: Vec<_> = diagnostics(&crate::util::file_uri(&file).unwrap(), &doc, &Options::default())
                .into_iter()
                .map(|d| (d.range.start.line, util::code(&d).map(String::from), d.message))
                .collect();
            let expected =
                format!("`user` set as non-keyword argument for app/views/users/_card.{ext}. Locals can only be set as keyword arguments.");
            assert_eq!(found, [(0, Some("Syntax/Ruby".to_string()), expected)], "{ext}");
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

    #[test]
    fn strict_locals_of_several_candidates_flag_what_fails_in_all() {
        let root = std::env::temp_dir().join(format!("rt-locals-variants-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        let write = |name: &str, locals: &str| std::fs::write(users.join(name), locals).unwrap();
        write("_both.html.erb", "<%# locals: (user:) -%>");
        write("_both.html+phone.erb", "<%# locals: (user:) -%>");
        write("_default.html.erb", "<%# locals: (user:) -%>");
        write("_default.html+phone.erb", "<%# locals: (user: nil) -%>");
        write("_bare.html.erb", "<%# locals: (user:) -%>");
        write("_bare.html+phone.erb", "<p>any</p>");
        write("_rest.html.erb", "<%# locals: (a:) -%>");
        write("_rest.html+phone.erb", "<%# locals: (b:, **rest) -%>");
        write("_two.html.erb", "<%# locals: (a:) -%>");
        write("_two.html.haml", "-# locals: (b: 1)\n");
        write("_three.html.erb", "<%# locals: (user:) -%>");
        write("_three.html+phone.erb", "<%# locals: (user:) -%>");
        write("_three.html+tablet.erb", "<%# locals: (user:) -%>");
        let file = users.join("index.html.erb");
        let messages = |actionview: &str, render: &str| -> Vec<String> {
            let text = format!("<%= {render} %>\n");
            let found = render_checks(TemplateLang::Erb, &file, &lock_with(actionview), &text, STRICT_LOCALS);
            found.into_iter().map(|d| d.message).collect()
        };
        assert_eq!(messages("8.0.1", r#"render "users/both""#), ["missing local: :user for app/views/users/_both.html.erb and 1 other"]);
        assert_eq!(messages("7.1.5", r#"render "users/both""#), ["missing local: :user"]);
        assert_eq!(messages("8.0.1", r#"render "users/three""#), ["missing local: :user for app/views/users/_three.html.erb and 2 others"]);
        assert!(messages("8.0.1", r#"render "users/default""#).is_empty(), "optional in one");
        assert!(messages("8.0.1", r#"render "users/bare""#).is_empty(), "one takes any local");
        assert_eq!(messages("7.1.5", r#"render "users/two", z: 1, a: 1"#), ["unknown local: :z"]);
        assert!(messages("7.1.5", r#"render "users/two", a: 1, b: 2"#).is_empty(), "each key known to one");
        assert!(messages("7.1.5", r#"render "users/rest", a: 1, b: 2, c: 3"#).is_empty(), "one takes the rest");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn strict_locals_candidates_stop_at_the_first_folder_and_mix_no_locals_with_locals() {
        let root = std::env::temp_dir().join(format!("rt-locals-shadow-{}", std::process::id()));
        let views = root.join("app/views");
        std::fs::create_dir_all(views.join("users")).unwrap();
        std::fs::create_dir_all(views.join("application")).unwrap();
        std::fs::create_dir_all(root.join("app/controllers")).unwrap();
        std::fs::write(
            root.join("app/controllers/application_controller.rb"),
            "class ApplicationController < ActionController::Base\nend\n",
        )
        .unwrap();
        std::fs::write(root.join("app/controllers/users_controller.rb"), "class UsersController < ApplicationController\nend\n").unwrap();
        std::fs::write(views.join("users/_card.html.erb"), "<%# locals: (user:) -%>").unwrap();
        std::fs::write(views.join("application/_card.html.erb"), "<%# locals: (title: nil) -%>").unwrap();
        std::fs::write(views.join("users/_mixed.html.erb"), "<%# locals: () -%>").unwrap();
        std::fs::write(views.join("users/_mixed.html+phone.erb"), "<%# locals: (a:) -%>").unwrap();
        std::fs::write(views.join("users/_none.html.erb"), "<%# locals: () -%>").unwrap();
        std::fs::write(views.join("users/_none.html+phone.erb"), "<%# locals: () -%>").unwrap();
        let file = views.join("users/index.html.erb");
        let messages = |render: &str| -> Vec<String> {
            let text = format!("<%= {render} %>\n");
            let found = render_checks(TemplateLang::Erb, &file, &lock_with("8.0.1"), &text, STRICT_LOCALS);
            found.into_iter().map(|d| d.message).collect()
        };
        assert_eq!(messages(r#"render "card""#), ["missing local: :user for app/views/users/_card.html.erb"]);
        let missing_user = ["missing local: :user for app/views/users/_card.html.erb"];
        assert_eq!(messages(r#"render "card", title: 1"#), missing_user, "missing wins; :title is unknown to users/_card");
        assert_eq!(messages(r#"render "card", user: 1, title: 1"#), ["unknown local: :title for app/views/users/_card.html.erb"]);
        assert_eq!(messages(r#"render "users/mixed", x: 1"#), ["unknown local: :x for app/views/users/_mixed.html.erb and 1 other"]);
        assert_eq!(messages(r#"render "users/none", x: 1"#), ["no locals accepted for app/views/users/_none.html.erb and 1 other"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn strict_locals_read_a_call_continued_by_a_leading_dot_in_a_code_tag() {
        let root = std::env::temp_dir().join(format!("rt-locals-dot-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        std::fs::write(users.join("_card.html.erb"), "<%# locals: (user:, size:) -%>\n").unwrap();
        let file = users.join("index.html.erb");
        // The first call's `size:` only counts if the read goes on past the `.first,` line.
        let text = "<p>héllo 日本</p>\n<%\n  render \"users/card\", user: users\n    .first,\n    size: 1\n%>\n<%= render \"users/card\", size: sizes\n  .first %>\n";
        let got: Vec<(String, Range)> = render_checks(TemplateLang::Erb, &file, &lock_with("7.1.0"), text, STRICT_LOCALS)
            .into_iter()
            .map(|d| (d.message, d.range))
            .collect();
        let range = Range::new(Position::new(6, 12), Position::new(6, 22));
        assert_eq!(got, vec![("missing local: :user".to_string(), range)]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn strict_locals_read_a_call_continued_on_the_line_of_its_partial() {
        let root = std::env::temp_dir().join(format!("rt-locals-continued-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        std::fs::write(users.join("_card.html.erb"), "<%# locals: (user:, size:) -%>\n").unwrap();
        let file = users.join("index.html.erb");
        let text = "<p>héllo</p>\r\n<%= render formats: [:html], locals: { user: u },\r\n  partial: \"users/card\" %>\r\n";
        let got: Vec<(String, Range)> = render_checks(TemplateLang::Erb, &file, &lock_with("7.1.0"), text, STRICT_LOCALS)
            .into_iter()
            .map(|d| (d.message, d.range))
            .collect();
        let range = Range::new(Position::new(2, 12), Position::new(2, 22));
        assert_eq!(got, vec![("missing local: :size".to_string(), range)]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn strict_locals_read_a_call_with_the_grammar_of_the_projects_ruby() {
        let root = std::env::temp_dir().join(format!("rt-locals-ruby-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        std::fs::write(users.join("_card.html.erb"), "<%# locals: (user:, size:) -%>\n").unwrap();
        let file = users.join("index.html.erb");
        // Ruby 3.3 rejects a line starting with `&&`, so that call's locals can't be read; the latest grammar reads them.
        let text = "<%= render \"users/card\", user: (u\n  && v) %>\n";
        let missing = |lock: String| {
            let found = render_checks(TemplateLang::Erb, &file, &Project::from_lockfile(&lock), text, STRICT_LOCALS);
            found.into_iter().map(|d| d.message).collect::<Vec<_>>()
        };
        let lock = format!("{GEM_LOCK}    actionview (7.1.0)\n");
        assert_eq!(missing(lock.clone()), ["missing local: :size"]);
        assert!(missing(format!("{lock}\nRUBY VERSION\n   ruby 3.3.6p108\n")).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
