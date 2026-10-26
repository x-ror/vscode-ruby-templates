use crate::lang::{file_name, html_host, slim_html_host, TemplateLang};
use crate::{
    assets, config, controllers, erb, erb_complete, format, haml, haml_indent, helper_defs, helpers, html_lint, i18n, ivars, outline,
    partial_rename, partials,
    project::{Framework, Project},
    routes, ruby, schema, slim_complete, slim_hover, slim_indent, slim_outline, slim_parser, slots, stimulus, tags, util,
};
use dashmap::DashMap;
use haml_indent::LineRole;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Duration;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

pub(crate) struct Doc {
    pub lang: TemplateLang,
    pub text: String,
    pub version: i32,
}

type LocaleFiles = Arc<Vec<(PathBuf, Vec<i18n::Translation>)>>;

/// `initializationOptions`, from the `rubyTemplates.{slim,erb,haml,jbuilder}.*` settings.
#[derive(Default, Debug, PartialEq)]
pub(crate) struct Options {
    extra_engines: Vec<String>,
    disabled_slim: Vec<String>,
    disabled_erb: Vec<String>,
    disabled_haml: Vec<String>,
    disabled_jbuilder: Vec<String>,
}

impl Options {
    /// Reads each field on its own so one malformed field keeps the others; returns what was ignored.
    fn from_init(init: Option<&serde_json::Value>) -> (Self, Vec<String>) {
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
        let options = Self {
            extra_engines: strings("slim", "extraEngines"),
            disabled_slim: strings("slim", "disabledDiagnostics"),
            disabled_erb: strings("erb", "disabledDiagnostics"),
            disabled_haml: strings("haml", "disabledDiagnostics"),
            disabled_jbuilder: strings("jbuilder", "disabledDiagnostics"),
        };
        (options, ignored)
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

struct Backend {
    client: Client,
    docs: Arc<DashMap<Url, Doc>>,
    /// Open controllers, for navigation to their views only; no other request reads them.
    controllers: DashMap<Url, String>,
    /// Files under each app/views root, for render completion.
    templates: DashMap<PathBuf, Arc<Vec<PathBuf>>>,
    /// Parsed locale files per config/locales directory.
    locales: DashMap<PathBuf, LocaleFiles>,
    /// Routes per Rails root, as the client last sent them.
    routes: DashMap<PathBuf, Arc<Vec<routes::Route>>>,
    // Set once in `initialize`; the client restarts the server when these settings change.
    options: Arc<OnceLock<Options>>,
    /// Workspace folders from `initialize`, for workspace symbols before any view is open.
    roots: OnceLock<Vec<PathBuf>>,
    /// Formatting is registered for Slim/Skim only, and only when the client supports dynamic registration:
    /// a static capability would cover ERB/Haml too, with a formatter that does nothing there.
    dynamic_formatting: OnceLock<bool>,
}

const DIAGNOSTICS_DEBOUNCE: Duration = Duration::from_millis(200);

pub(crate) fn diagnostics(uri: &Url, doc: &Doc, options: &Options) -> Vec<Diagnostic> {
    // Thor templates (`*.html.erb.tt`, `*.haml.tt`) mix the generator's ERB with the template's own tags or
    // lines, and their partials live outside app/views, so no check applies.
    if uri.path().to_ascii_lowercase().ends_with(".tt") {
        return vec![];
    }
    let mut out = match doc.lang {
        TemplateLang::Erb => erb::diagnostics(&doc.text),
        TemplateLang::Haml => haml_indent::diagnostics(&doc.text),
        TemplateLang::Slim => slim_indent::diagnostics(&doc.text, &options.extra_engines),
        // Skim is Slim syntax with CoffeeScript code, so the Ruby checks would misfire.
        TemplateLang::Skim => slim_indent::diagnostics(&doc.text, &options.extra_engines)
            .into_iter()
            .filter(|d| !util::code(d).is_some_and(|c| slim_indent::code::RUBY.contains(&c)))
            .collect(),
        // Syntax errors are Ruby LSP's job; only the render checks below apply.
        TemplateLang::Ruby => vec![],
    };
    // Void elements are an HTML rule; Slim itself renders `<img>x</img>` for `.xml.slim` and friends.
    if doc.lang == TemplateLang::Slim && !slim_html_host(uri) {
        out.retain(|d| util::code(d) != Some(slim_indent::code::VOID_CHILDREN));
    }
    if doc.lang == TemplateLang::Erb && html_host(uri) {
        let partial = uri.path().rsplit('/').next().is_some_and(|f| f.starts_with('_'));
        out.extend(tags::diagnostics(&doc.text, partial));
        out.extend(html_lint::diagnostics(&doc.text));
    }
    if let Ok(path) = uri.to_file_path() {
        let in_code = render_filter(doc.lang, &doc.text, &options.extra_engines);
        let project = Project::for_template(&path);
        out.extend(unresolved_renders(&path, &project, &doc.text, &in_code));
        out.extend(strict_locals_mismatches(&path, &project, &doc.text, &in_code));
    }
    let disabled = options.disabled(doc.lang);
    out.retain(|d| !util::code(d).is_some_and(|c| disabled.iter().any(|off| off == c)));
    out
}

/// Diagnostic codes every language shares.
const UNRESOLVED_RENDER: &str = "unresolved-render";
const STRICT_LOCALS: &str = "strict-locals";
const STRICT_LOCALS_SINCE: &str = "7.1";
const STRICT_LOCALS_TEMPLATE_SUFFIX_SINCE: &str = "7.2";

/// Literal renders in code that name `dir/name` under an existing app/views/dir, as (line index, line, render).
fn checked_renders<'a>(
    views: &Path,
    text: &'a str,
    in_code: impl Fn(usize, usize, &partials::RenderRef) -> bool,
) -> Vec<(usize, &'a str, partials::RenderRef)> {
    let mut out = Vec::new();
    for (i, (line_start, line)) in util::lines_with_offsets(text).enumerate() {
        let checked =
            partials::render_refs(line).into_iter().filter(|r| partials::in_app_views_dir(views, &r.name) && in_code(i, line_start, r));
        out.extend(checked.map(|r| (i, line, r)));
    }
    out
}

/// Literal `render "x"` / `render partial:|template: "x"` with no matching file under app/views.
fn unresolved_renders(
    path: &Path,
    project: &Project,
    text: &str,
    in_code: impl Fn(usize, usize, &partials::RenderRef) -> bool,
) -> Vec<Diagnostic> {
    if project.framework != Framework::Rails {
        return vec![];
    }
    let Some(views) = partials::views_root(path) else { return vec![] };
    checked_renders(&views, text, in_code)
        .into_iter()
        .filter(|(_, _, r)| partials::resolve(project, path, r).is_empty() && !is_gem_view(project, &r.name))
        .map(|(i, line, r)| {
            let message = missing_message(&r);
            util::coded(util::warning(util::span_range(i, line, r.start, r.end), message), UNRESOLVED_RENDER)
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

/// Literal one-line renders whose locals don't fit the partial's `locals: (…)` signature.
fn strict_locals_mismatches(
    path: &Path,
    project: &Project,
    text: &str,
    in_code: impl Fn(usize, usize, &partials::RenderRef) -> bool,
) -> Vec<Diagnostic> {
    if !supports_strict_locals(project) {
        return vec![];
    }
    let Some(views) = partials::views_root(path) else { return vec![] };
    let suffix_with_template = project.gem_version("actionview").is_some_and(|v| v.at_least(STRICT_LOCALS_TEMPLATE_SUFFIX_SINCE));
    let mut out = Vec::new();
    // A partial's layout gets the partial's locals, which this check can't see.
    for (i, line, r) in checked_renders(&views, text, in_code).into_iter().filter(|(_, _, r)| !r.from_layout) {
        let Some(passed) = partials::passed_locals(line, &r) else { continue };
        // Several variants/formats: which one Rails picks depends on the request.
        let found = partials::resolve(project, path, &r);
        let [file] = found.as_slice() else { continue };
        let Some(sig) = partials::file_strict_locals(file) else { continue };
        let missing: Vec<&str> =
            sig.locals.iter().filter(|(n, d)| d.is_none() && !passed.contains(&n.as_str())).map(|(n, _)| n.as_str()).collect();
        let mut seen = HashSet::new();
        let unknown: Vec<&str> =
            passed.into_iter().filter(|p| seen.insert(*p) && !sig.rest && !sig.locals.iter().any(|(n, _)| n == p)).collect();
        let no_locals = sig.locals.is_empty() && !sig.rest;
        let Some(mut message) = strict_locals_message(&missing, &unknown, no_locals) else { continue };
        if suffix_with_template {
            // Rails prints the absolute path; relative to the app folder is stable and readable.
            let app_root = views.parent().and_then(Path::parent).unwrap_or(&views);
            let identifier = file.strip_prefix(app_root).unwrap_or(file).to_string_lossy().replace('\\', "/");
            message.push_str(&format!(" for {identifier}"));
        }
        out.push(util::coded(util::warning(util::span_range(i, line, r.start, r.end), message), STRICT_LOCALS));
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

fn render_hover(project: &Project, path: &Path, r: &partials::RenderRef) -> String {
    let found = partials::resolve(project, path, r);
    if found.is_empty() {
        return format!("**{}** `{}`\n\nNo matching template found.", r.kind.name(), r.name);
    }
    let list: Vec<String> = found.iter().map(|f| format!("- `{}`", f.display())).collect();
    let mut md = format!("**{}** `{}`\n\n{}", r.kind.name(), r.name, list.join("\n"));
    if let Some(sig) = partials::file_strict_locals(&found[0]) {
        md.push_str(&format!("\n\n```ruby\nlocals: ({})\n```", sig.params));
    }
    md
}

/// Byte ranges on `line` that lead to files by convention: `render @model`, Stimulus identifiers, asset paths.
/// With `col`, only the ones under it are resolved.
fn convention_targets(project: &Project, path: &Path, line: &str, col: Option<usize>) -> Vec<(usize, usize, Vec<PathBuf>)> {
    let at = |s: usize, e: usize| col.is_none_or(|c| (s..=e).contains(&c));
    let models = partials::model_renders(line)
        .into_iter()
        .filter(|&(s, e, _)| at(s, e))
        .map(|(s, e, name)| (s, e, partials::resolve_model(project, path, name)));
    let controllers =
        stimulus::identifiers(line).into_iter().filter(|&(s, e, _)| at(s, e)).map(|(s, e, id)| (s, e, stimulus::resolve(path, &id)));
    let assets = assets::asset_refs(line).into_iter().filter(|r| at(r.start, r.end)).map(|r| (r.start, r.end, assets::resolve(path, &r)));
    models.chain(controllers).chain(assets).filter(|(_, _, found)| !found.is_empty()).collect()
}

/// Full key of the `t("…")` literal under `col`, when the call is Ruby.
fn i18n_key_at(
    project: &Project,
    doc: &Doc,
    path: &Path,
    line_start: usize,
    line: &str,
    col: usize,
    extra_engines: &[String],
) -> Option<String> {
    let (start, end) = i18n::key_at(line, col)?;
    if !code_at(doc.lang, &doc.text, line_start + start, extra_engines) {
        return None;
    }
    i18n::full_key(project, path, &line[start..end])
}

fn file_link(range: Range, target: &Path) -> Option<DocumentLink> {
    Some(DocumentLink {
        range,
        target: Some(Url::from_file_path(target).ok()?),
        tooltip: Some(target.to_string_lossy().into_owned()),
        data: None,
    })
}

/// The line of `key` in each locale file that defines it.
fn translation_locations(files: &[(PathBuf, Vec<i18n::Translation>)], key: &str) -> Vec<Location> {
    let start = |t: &i18n::Translation| Position::new(t.line as u32, 0);
    i18n::lookup(files, key)
        .filter_map(|(file, t)| Some(Location::new(Url::from_file_path(file).ok()?, Range::new(start(t), start(t)))))
        .collect()
}

/// `in_ruby_context`, with Slim asked through its parser: its line heuristic also takes `p See (docs)` as Ruby.
fn code_at(lang: TemplateLang, text: &str, offset: usize, extra_engines: &[String]) -> bool {
    match lang {
        TemplateLang::Slim => slim_parser::in_ruby(text, &slim_parser::parse(text, extra_engines), offset),
        _ => in_ruby_context(lang, text, &erb::code_spans(text), offset),
    }
}

fn markdown_hover(value: String) -> Option<Hover> {
    Some(Hover { contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value }), range: None })
}

fn helper_hover(h: &helpers::Helper) -> String {
    let signature = format!("```ruby\n{}\n```", h.detail);
    match h.doc_url() {
        Some(url) => format!("{signature}\n\n[API docs]({url})"),
        None => signature,
    }
}

/// The current template's own strict locals, as completions.
fn local_completions(text: &str, typed: &str) -> Vec<CompletionItem> {
    let Some(sig) = partials::strict_locals(text) else { return vec![] };
    sig.locals
        .into_iter()
        .filter(|(name, _)| name.starts_with(typed))
        .map(|(name, default)| {
            let detail = default.map_or_else(|| "local (required)".to_string(), |d| format!("local = {d}"));
            util::completion_item(name, CompletionItemKind::VARIABLE, detail)
        })
        .collect()
}

/// Whether the watched-file `events` make the cache for `root` stale. Listings change only when files are
/// created or deleted; `contents` caches also on edits. Deleting an ancestor of `root` counts too.
fn stale(root: &Path, events: &[(PathBuf, FileChangeType)], contents: bool) -> bool {
    events.iter().any(|(path, kind)| (contents || *kind != FileChangeType::CHANGED) && (path.starts_with(root) || root.starts_with(path)))
}

/// Haml line roles; every other language's lines all start their own construct.
fn line_roles(lang: TemplateLang, text: &str) -> Vec<LineRole> {
    match lang {
        TemplateLang::Haml => haml_indent::haml_line_roles(text),
        _ => vec![LineRole::Start; text.lines().count()],
    }
}

/// Is the `render` whose string starts at byte `start` of the line at `line_start` a call in Ruby code, not template
/// text, a Ruby comment or a Ruby string (`link_to "render 'x'"`)? `spans` are `erb::code_spans(text)`.
fn render_in_code(
    lang: TemplateLang,
    role: LineRole,
    text: &str,
    spans: &[(usize, usize)],
    line_start: usize,
    r: &partials::RenderRef,
) -> bool {
    let line = util::lines_with_offsets(&text[line_start..]).next().map_or("", |(_, line)| line);
    let in_ruby = || in_ruby_context(lang, text, spans, line_start + r.start);
    let code_start = match (lang, role) {
        // A tag opened on an earlier line puts the whole line before `render` in Ruby.
        (TemplateLang::Erb, _) => in_ruby().then(|| line[..r.call].rfind("<%").map_or(0, |open| open + 2)),
        (TemplateLang::Haml, LineRole::Start) => haml_indent::haml_code_start(line).filter(|&code| code <= r.call),
        (TemplateLang::Haml, LineRole::Ruby) => Some(0),
        (TemplateLang::Haml, LineRole::Text) => None,
        (TemplateLang::Slim, _) => {
            let ws = util::leading_ws(line).len();
            let prefix = ["!=", "&=", "=", "-"].into_iter().find(|p| line[ws..].starts_with(p));
            // `==`, `=<`, `=>`, `='` and the like: the rest of the marker isn't Ruby.
            let marker_end =
                |p: &str| ws + p.len() + line[ws + p.len()..].len() - line[ws + p.len()..].trim_start_matches(['=', '<', '>', '\'']).len();
            prefix.filter(|_| in_ruby()).map(marker_end)
        }
        (TemplateLang::Skim, _) => None,
        (TemplateLang::Ruby, _) => in_ruby().then_some(0),
    };
    code_start.is_some_and(|code| code <= r.call && ruby::code_end(&line[code..r.call]) == ruby::CodeEnd::Code)
}

/// `render_in_code` for each render ref in `text`, given its line index and line start; Slim lines
/// also have to be Ruby to Slim's parser.
pub(crate) fn render_filter<'a>(
    lang: TemplateLang,
    text: &'a str,
    extra_engines: &[String],
) -> impl Fn(usize, usize, &partials::RenderRef) -> bool + 'a {
    let roles = line_roles(lang, text);
    let spans = erb::code_spans(text);
    let slim = (lang == TemplateLang::Slim).then(|| slim_parser::parse(text, extra_engines));
    move |i, line_start, r| {
        render_in_code(lang, roles[i], text, &spans, line_start, r)
            && slim.as_ref().is_none_or(|parsed| slim_parser::in_ruby(text, parsed, line_start + r.start))
    }
}

/// Edits replacing each line in `lines` that differs between `text` and `formatted`; `None` when formatting added or
/// removed lines, which a line range can't express.
fn line_edits(text: &str, formatted: &str, lines: std::ops::RangeInclusive<usize>) -> Option<Vec<TextEdit>> {
    let old: Vec<&str> = text.lines().collect();
    let new: Vec<&str> = formatted.lines().collect();
    if old.len() != new.len() {
        return None;
    }
    let changed = old.iter().zip(&new).enumerate().filter(|(i, (a, b))| lines.contains(i) && a != b);
    let edit = |(i, (a, b)): (usize, (&&str, &&str))| {
        let end = util::offset_to_position(a, a.len()).character;
        TextEdit::new(Range::new(Position::new(i as u32, 0), Position::new(i as u32, end)), b.to_string())
    };
    Some(changed.map(edit).collect())
}

/// The source action that spaces every ERB tag in a file; `editor.codeActionsOnSave` can run it. Outside
/// `source.fixAll`, which many users run on save for other languages' linters.
const ERB_SPACING: CodeActionKind = CodeActionKind::new("source.erbSpacing");

/// "Add spaces inside this ERB tag" for tags touching `range`, and the same for the whole file when `only` asks
/// for source actions: VS Code requests code actions on every cursor move, without `only`.
fn spacing_actions(uri: &Url, text: &str, range: Range, only: Option<&[CodeActionKind]>) -> Vec<CodeActionOrCommand> {
    let (Some(from), Some(to)) = (util::position_to_offset(text, range.start), util::position_to_offset(text, range.end)) else {
        return vec![];
    };
    let edits = erb::spacing_edits(text);
    let to_edit = |(s, e, tag): &(usize, usize, String)| {
        TextEdit::new(Range::new(util::offset_to_position(text, *s), util::offset_to_position(text, *e)), tag.clone())
    };
    let action = |title: &str, kind: CodeActionKind, edits: Vec<TextEdit>| {
        CodeActionOrCommand::CodeAction(CodeAction {
            title: title.into(),
            kind: Some(kind),
            edit: Some(WorkspaceEdit { changes: Some(HashMap::from([(uri.clone(), edits)])), ..Default::default() }),
            ..Default::default()
        })
    };
    let touching: Vec<TextEdit> = edits.iter().filter(|(s, e, _)| *s <= to && from <= *e).map(to_edit).collect();
    let mut out = Vec::new();
    if !touching.is_empty() {
        out.push(action("Add spaces inside this ERB tag", CodeActionKind::REFACTOR_REWRITE, touching));
    }
    let file_wide = only.is_some_and(|kinds| {
        kinds.iter().any(|k| ERB_SPACING.as_str() == k.as_str() || ERB_SPACING.as_str().starts_with(&format!("{}.", k.as_str())))
    });
    if file_wide && !edits.is_empty() {
        out.push(action("Add spaces inside every ERB tag", ERB_SPACING, edits.iter().map(to_edit).collect()));
    }
    out
}

fn file_location(file: impl AsRef<Path>) -> Option<Location> {
    Some(Location::new(Url::from_file_path(file).ok()?, Range::default()))
}

fn rename_filter(glob: &str, matches: FileOperationPatternKind) -> FileOperationFilter {
    FileOperationFilter {
        scheme: Some("file".into()),
        pattern: FileOperationPattern { glob: glob.into(), matches: Some(matches), options: None },
    }
}

impl Backend {
    /// The document's text and the text formatted with the editor's `options`; `None` (logged) when it is left as it is.
    /// `lines_only` leaves out the steps that add or remove lines, which a range can't hold.
    async fn formatted(&self, uri: &Url, options: &FormattingOptions, lines_only: bool) -> Option<(String, String)> {
        let (lang, text) = self.docs.get(uri).map(|doc| (doc.lang, doc.text.clone()))?;
        let path = uri.to_file_path().ok();
        let mut settings =
            path.map_or_else(format::Settings::default, |path| config::settings_for(&path, lang).settings).with_editor(options);
        if lines_only {
            settings = format::Settings { final_newline: None, collapse_blank_lines: false, trim_trailing_blank_lines: false, ..settings };
        }
        match format::format(uri, lang, &text, &settings, &self.options().extra_engines) {
            Ok(formatted) => Some((text, formatted.text)),
            Err(skipped) => {
                self.client.log_message(MessageType::INFO, format!("{}: left unchanged: {skipped}", file_name(uri))).await;
                None
            }
        }
    }

    fn options(&self) -> &Options {
        self.options.get_or_init(Options::default)
    }

    async fn publish(&self, uri: Url) {
        let Some((diags, version)) = self.docs.get(&uri).map(|doc| (diagnostics(&uri, &doc, self.options()), doc.version)) else {
            return;
        };
        self.client.publish_diagnostics(uri, diags, Some(version)).await;
    }

    /// Publish after `DIAGNOSTICS_DEBOUNCE`, unless a newer change to the document supersedes this one.
    fn publish_later(&self, uri: Url, version: i32) {
        let (client, docs, options) = (self.client.clone(), Arc::clone(&self.docs), Arc::clone(&self.options));
        tokio::spawn(async move {
            tokio::time::sleep(DIAGNOSTICS_DEBOUNCE).await;
            let options = options.get_or_init(Options::default);
            let Some(diags) = docs.get(&uri).filter(|doc| doc.version == version).map(|doc| diagnostics(&uri, &doc, options)) else {
                return;
            };
            client.publish_diagnostics(uri, diags, Some(version)).await;
        });
    }

    fn template_files(&self, current: &Path) -> Arc<Vec<PathBuf>> {
        let Some(views) = partials::views_root(current) else {
            return Arc::new(current.parent().map(|dir| partials::files_under(dir, 1)).unwrap_or_default());
        };
        self.views_files(&views)
    }

    /// Every file under an app/views root, cached until a watched-file event says otherwise.
    fn views_files(&self, views: &Path) -> Arc<Vec<PathBuf>> {
        self.templates.entry(views.to_path_buf()).or_insert_with(|| Arc::new(partials::files_under(views, usize::MAX))).clone()
    }

    fn locale_files(&self, current: &Path) -> LocaleFiles {
        let Some(dir) = i18n::locales_dir(current) else { return LocaleFiles::default() };
        self.locales.entry(dir.clone()).or_insert_with(|| Arc::new(i18n::locale_files(&dir))).clone()
    }

    /// Open documents' text, which may have unsaved edits. Keyed by path: VS Code's URIs percent-encode `+`, `@`
    /// and drive colons where `Url::from_file_path` doesn't.
    /// `rubyTemplates/routes`: `{ root, routes }` from the client's `bin/rails runner`.
    async fn set_routes(&self, params: serde_json::Value) {
        let Some(root) = params.get("root").and_then(serde_json::Value::as_str).map(PathBuf::from) else { return };
        let parsed = routes::parse(&root, params.get("routes").unwrap_or(&serde_json::Value::Null));
        self.routes.insert(root, Arc::new(parsed));
    }

    /// Where the `@ivar` or partial local under `col` comes from: its assignments in the controller, or the
    /// `render` calls passing the local (else the strict-locals comment declaring it).
    fn variable_definitions(&self, text: &str, path: &Path, line: &str, col: usize) -> Vec<Location> {
        let at_line = |file: &Path, line: usize| {
            let start = Position::new(line as u32, 0);
            Some(Location::new(Url::from_file_path(file).ok()?, Range::new(start, start)))
        };
        let word = util::word_at(line, col);
        let before = &line[..line[..col.min(line.len())].trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len()];
        if before.ends_with('@') && !before.ends_with("@@") {
            return ivars::for_view(&Project::for_template(path), path)
                .into_iter()
                .filter(|a| a.name == word)
                .filter_map(|a| at_line(&a.file, a.line))
                .collect();
        }
        let is_partial = path.file_name().and_then(|f| f.to_str()).is_some_and(|f| f.starts_with('_'));
        // `link_to` or `t` is never a local; the scan below reads every view.
        let builtin = |name: &str| helpers::HELPERS.iter().chain(helpers::HAML_HELPERS).any(|h| h.name == name);
        let Some(name) = helper_defs::name_at(line, col).filter(|name| is_partial && !builtin(name)) else { return vec![] };
        let Some(views) = partials::views_root(path) else { return vec![] };
        let sources = partial_rename::sources(&views, &self.open_texts());
        let passes = partial_rename::local_passes(&Project::for_template(path), path, name, &sources, &self.options().extra_engines);
        if !passes.is_empty() {
            return passes;
        }
        let declared = partials::strict_locals(text).is_some_and(|sig| sig.locals.iter().any(|(local, _)| local == name));
        let comment = text.lines().position(|l| l.contains("locals:"));
        comment.filter(|_| declared).and_then(|line| at_line(path, line)).into_iter().collect()
    }

    /// Routes of the Rails app `current` belongs to, empty until the client has sent them.
    fn routes_for(&self, current: &Path) -> Arc<Vec<routes::Route>> {
        let root = partials::app_root(current).and_then(|app| app.parent().map(Path::to_path_buf));
        root.and_then(|root| self.routes.get(&root).map(|r| Arc::clone(&r))).unwrap_or_default()
    }

    fn open_texts(&self) -> HashMap<PathBuf, String> {
        let templates = self.docs.iter().filter_map(|d| Some((d.key().to_file_path().ok()?, d.text.clone())));
        // Partial and i18n references also scan controllers; their unsaved text is the current one.
        let controllers = self.controllers.iter().filter_map(|c| Some((c.key().to_file_path().ok()?, c.value().clone())));
        templates.chain(controllers).collect()
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let (options, ignored) = Options::from_init(params.initialization_options.as_ref());
        for message in ignored {
            let message = format!("ignoring initializationOptions.{message}");
            self.client.log_message(MessageType::WARNING, message).await;
        }
        let _ = self.options.set(options);
        #[allow(deprecated)]
        let root_uri = params.root_uri.iter().cloned();
        let folders = params.workspace_folders.unwrap_or_default().into_iter().map(|f| f.uri);
        let _ = self.roots.set(folders.chain(root_uri).filter_map(|u| u.to_file_path().ok()).collect());
        let dynamic_formatting =
            params.capabilities.text_document.and_then(|t| t.formatting).and_then(|f| f.dynamic_registration).unwrap_or(false);
        let _ = self.dynamic_formatting.set(dynamic_formatting);
        Ok(InitializeResult {
            server_info: Some(ServerInfo { name: "ruby-templates-ls".into(), version: Some(env!("CARGO_PKG_VERSION").into()) }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::INCREMENTAL)),
                definition_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(["\"", "'", ".", "/", "%", ":", "{", "(", "<", "@"].map(String::from).to_vec()),
                    resolve_provider: Some(false),
                    ..Default::default()
                }),
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: Default::default(),
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                linked_editing_range_provider: Some(LinkedEditingRangeServerCapabilities::Simple(true)),
                code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
                    code_action_kinds: Some(vec![CodeActionKind::QUICKFIX, CodeActionKind::REFACTOR_REWRITE, ERB_SPACING]),
                    ..Default::default()
                })),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: None,
                    file_operations: Some(WorkspaceFileOperationsServerCapabilities {
                        will_rename: Some(FileOperationRegistrationOptions {
                            filters: vec![
                                rename_filter("**/_*.{erb,haml,slim,skim}", FileOperationPatternKind::File),
                                // Partials of other formats (`_card.json.jbuilder`) can be rendered from views too.
                                rename_filter("**/app/views/**/_*", FileOperationPatternKind::File),
                                rename_filter("**/app/views/**", FileOperationPatternKind::Folder),
                            ],
                        }),
                        ..Default::default()
                    }),
                }),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        if self.dynamic_formatting.get() == Some(&true) {
            let selector = serde_json::json!({ "documentSelector": [{ "language": "slim" }, { "language": "skim" }] });
            let registration =
                Registration { id: "slim-formatting".into(), method: "textDocument/formatting".into(), register_options: Some(selector) };
            if let Err(e) = self.client.register_capability(vec![registration]).await {
                self.client.log_message(MessageType::WARNING, format!("Slim formatting not registered: {e}")).await;
            }
        }
        self.client.log_message(MessageType::INFO, "ruby-templates-ls ready").await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, p: DidOpenTextDocumentParams) {
        let Some(lang) = TemplateLang::from_uri_or_id(&p.text_document.uri, Some(&p.text_document.language_id)) else {
            if p.text_document.uri.to_file_path().is_ok_and(|path| controllers::controller_path(&path).is_some()) {
                self.controllers.insert(p.text_document.uri, p.text_document.text);
            }
            return;
        };
        self.docs.insert(p.text_document.uri.clone(), Doc { lang, text: p.text_document.text, version: p.text_document.version });
        self.publish(p.text_document.uri).await;
    }

    async fn did_change(&self, p: DidChangeTextDocumentParams) {
        let version = p.text_document.version;
        if let Some(mut text) = self.controllers.get_mut(&p.text_document.uri) {
            for change in p.content_changes {
                util::apply_change(&mut text, change);
            }
            return;
        }
        {
            let Some(mut doc) = self.docs.get_mut(&p.text_document.uri) else { return };
            for change in p.content_changes {
                util::apply_change(&mut doc.text, change);
            }
            doc.version = version;
        }
        self.publish_later(p.text_document.uri, version);
    }

    async fn did_change_watched_files(&self, p: DidChangeWatchedFilesParams) {
        let events: Vec<(PathBuf, FileChangeType)> =
            p.changes.into_iter().filter_map(|e| Some((e.uri.to_file_path().ok()?, e.typ))).collect();
        self.templates.retain(|root, _| !stale(root, &events, false));
        self.locales.retain(|root, _| !stale(root, &events, true));
        // Unresolved-render and strict-locals warnings read other templates from disk.
        let open: Vec<Url> = self.docs.iter().map(|d| d.key().clone()).collect();
        for uri in open {
            self.publish(uri).await;
        }
    }

    async fn did_close(&self, p: DidCloseTextDocumentParams) {
        if self.controllers.remove(&p.text_document.uri).is_some() {
            return;
        }
        self.docs.remove(&p.text_document.uri);
        self.client.publish_diagnostics(p.text_document.uri, vec![], None).await;
    }

    async fn goto_definition(&self, p: GotoDefinitionParams) -> Result<Option<GotoDefinitionResponse>> {
        let uri = p.text_document_position_params.text_document.uri;
        let pos = p.text_document_position_params.position;
        let Ok(path) = uri.to_file_path() else { return Ok(None) };
        let project = Project::for_template(&path);
        if let Some(text) = self.controllers.get(&uri) {
            let (_, line, col) = util::line_at(&text, pos);
            let found =
                controllers::targets(&project, &path, line, Some(col)).into_iter().next().map(|(_, _, found)| found).unwrap_or_default();
            let locs: Vec<Location> = found.into_iter().filter_map(file_location).collect();
            return Ok((!locs.is_empty()).then_some(GotoDefinitionResponse::Array(locs)));
        }
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let (line_start, line, col) = util::line_at(&doc.text, pos);

        if let Some(key) = i18n_key_at(&project, &doc, &path, line_start, line, col, &self.options().extra_engines) {
            let locs = translation_locations(&self.locale_files(&path), &key);
            return Ok((!locs.is_empty()).then_some(GotoDefinitionResponse::Array(locs)));
        }
        let targets: Vec<PathBuf> = if let Some(r) = partials::render_ref_at(line, col) {
            partials::resolve(&project, &path, &r)
        } else if let Some(c) = partials::component_ref_at(line, col) {
            partials::resolve_component(&project, &path, &c)
        } else {
            convention_targets(&project, &path, line, Some(col)).into_iter().next().map(|(_, _, found)| found).unwrap_or_default()
        };
        let mut locs: Vec<Location> = targets.into_iter().filter_map(file_location).collect();
        if !locs.is_empty() || !code_at(doc.lang, &doc.text, line_start + col, &self.options().extra_engines) {
            return Ok((!locs.is_empty()).then_some(GotoDefinitionResponse::Array(locs)));
        }
        // Cheapest first: Ctrl+hover asks for a definition on every word the mouse crosses.
        {
            let helper = helper_defs::name_at(line, col);
            let at_line = |(file, line): (PathBuf, usize)| {
                let start = Position::new(line as u32, 0);
                Some(Location::new(Url::from_file_path(file).ok()?, Range::new(start, start)))
            };
            let route = helper.and_then(|name| routes::for_helper(&self.routes_for(&path), name)?.0.source.clone());
            match route {
                Some(source) => locs.extend(at_line(source)),
                None => locs.extend(helper.into_iter().flat_map(|name| helper_defs::definitions(&path, name)).filter_map(at_line)),
            }
        }
        if locs.is_empty() {
            locs.extend(self.variable_definitions(&doc.text, &path, line, col));
        }
        Ok((!locs.is_empty()).then_some(GotoDefinitionResponse::Array(locs)))
    }

    async fn document_link(&self, p: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        let uri = p.text_document.uri;
        let Ok(path) = uri.to_file_path() else { return Ok(None) };
        let project = Project::for_template(&path);
        if let Some(text) = self.controllers.get(&uri) {
            let links = util::lines_with_offsets(&text)
                .enumerate()
                .flat_map(|(i, (_, line))| controllers::targets(&project, &path, line, None).into_iter().map(move |t| (i, line, t)))
                .filter_map(|(i, line, (start, end, found))| file_link(util::span_range(i, line, start, end), found.first()?));
            return Ok(Some(links.collect()));
        }
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let in_code = render_filter(doc.lang, &doc.text, &self.options().extra_engines);
        let mut links = Vec::new();
        for (i, (line_start, line)) in util::lines_with_offsets(&doc.text).enumerate() {
            // Same renders as diagnostics and references: code only, and the file Rails would pick.
            let renders = partials::render_refs(line)
                .into_iter()
                .filter(|r| in_code(i, line_start, r))
                .map(|r| (r.start, r.end, partials::first_folder_only(partials::resolve(&project, &path, &r))));
            for (start, end, found) in renders.chain(convention_targets(&project, &path, line, None)) {
                links.extend(found.first().and_then(|target| file_link(util::span_range(i, line, start, end), target)));
            }
        }
        Ok(Some(links))
    }

    async fn hover(&self, p: HoverParams) -> Result<Option<Hover>> {
        let uri = p.text_document_position_params.text_document.uri;
        let pos = p.text_document_position_params.position;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        if doc.lang == TemplateLang::Slim {
            let parsed = slim_parser::parse(&doc.text, &self.options().extra_engines);
            let marker = util::position_to_offset(&doc.text, pos).and_then(|o| slim_hover::marker_doc(&doc.text, o, &parsed));
            if let Some(md) = marker {
                return Ok(markdown_hover(md));
            }
        }
        let (line_start, line, col) = util::line_at(&doc.text, pos);
        if doc.lang == TemplateLang::Erb {
            if let Some(md) = erb::tag_doc(line, col) {
                return Ok(markdown_hover(md.to_string()));
            }
        }
        if let Ok(path) = uri.to_file_path() {
            let project = Project::for_template(&path);
            if let Some(r) = partials::render_ref_at(line, col) {
                return Ok(markdown_hover(render_hover(&project, &path, &r)));
            }
            if let Some(md) = i18n_key_at(&project, &doc, &path, line_start, line, col, &self.options().extra_engines)
                .and_then(|key| i18n::hover(&self.locale_files(&path), &key))
            {
                return Ok(markdown_hover(md));
            }
        }
        let role = line_roles(doc.lang, &doc.text).get(pos.line as usize).copied().unwrap_or(LineRole::Start);
        if role == LineRole::Text {
            return Ok(None);
        }
        if let Some(md) = haml::hover(line, col).filter(|_| doc.lang == TemplateLang::Haml && role == LineRole::Start) {
            return Ok(markdown_hover(md));
        }
        let word = util::word_at(line, col);
        // `<p>Clear the cache</p>` names no helper.
        let code_here = role == LineRole::Ruby || code_at(doc.lang, &doc.text, line_start + col, &self.options().extra_engines);
        if let Some(h) = helpers_for(doc.lang).find(|h| h.name == word).filter(|_| code_here) {
            return Ok(markdown_hover(helper_hover(h)));
        }
        let routes = uri.to_file_path().map(|p| self.routes_for(&p)).unwrap_or_default();
        Ok(routes::hover(&routes, word).filter(|_| code_here).and_then(markdown_hover))
    }

    async fn folding_range(&self, p: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let Some(doc) = self.docs.get(&p.text_document.uri) else { return Ok(None) };
        // A provider's ranges replace VS Code's indentation folding; `None` falls back to it. That fallback is
        // what folds the host language of `.js.erb`/`.yml.erb`/… where ERB blocks alone would regress.
        match doc.lang {
            TemplateLang::Haml => Ok(Some(outline::haml_folding_ranges(&doc.text))),
            TemplateLang::Slim | TemplateLang::Skim => Ok(Some(slim_outline::folding_ranges(&doc.text, &self.options().extra_engines))),
            TemplateLang::Erb if html_host(&p.text_document.uri) => Ok(Some(outline::folding_ranges(&doc.text))),
            TemplateLang::Erb | TemplateLang::Ruby => Ok(None),
        }
    }

    async fn document_symbol(&self, p: DocumentSymbolParams) -> Result<Option<DocumentSymbolResponse>> {
        let uri = p.text_document.uri;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let symbols = match doc.lang {
            TemplateLang::Erb => outline::document_symbols(&doc.text, html_host(&uri)),
            TemplateLang::Haml => outline::haml_document_symbols(&doc.text),
            TemplateLang::Slim | TemplateLang::Skim => slim_outline::document_symbols(&doc.text, &self.options().extra_engines),
            TemplateLang::Ruby => return Ok(None),
        };
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn code_action(&self, p: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = p.text_document.uri;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let Ok(path) = uri.to_file_path() else { return Ok(None) };
        let project = Project::for_template(&path);
        let mut actions: Vec<CodeActionOrCommand> = Vec::new();
        if doc.lang == TemplateLang::Erb {
            actions.extend(spacing_actions(&uri, &doc.text, p.range, p.context.only.as_deref()));
        }
        let unresolved = p.context.diagnostics.into_iter().filter(|d| util::code(d) == Some(UNRESOLVED_RENDER));
        let quick_fixes = unresolved.filter_map(|d| {
            let (_, line, start) = util::line_at(&doc.text, d.range.start);
            let r = partials::render_refs(line).into_iter().find(|r| r.start == start)?;
            let target = partials::new_template_path(&project, &path, &r)?;
            let create = CreateFile {
                uri: Url::from_file_path(&target).ok()?,
                options: Some(CreateFileOptions { overwrite: Some(false), ignore_if_exists: Some(true) }),
                annotation_id: None,
            };
            let shown = target.strip_prefix(partials::views_root(&path)?).ok()?.to_string_lossy().replace('\\', "/");
            Some(CodeActionOrCommand::CodeAction(CodeAction {
                title: format!("Create {} `{shown}`", r.kind.name().to_lowercase()),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![d]),
                edit: Some(WorkspaceEdit {
                    document_changes: Some(DocumentChanges::Operations(vec![DocumentChangeOperation::Op(ResourceOp::Create(create))])),
                    ..Default::default()
                }),
                is_preferred: Some(true),
                ..Default::default()
            }))
        });
        actions.extend(quick_fixes);
        Ok(Some(actions))
    }

    /// Who renders the partial under the cursor's `render "…"`, or the current partial anywhere else in it.
    async fn references(&self, p: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = p.text_document_position.text_document.uri;
        let pos = p.text_document_position.position;
        let Ok(path) = uri.to_file_path() else { return Ok(None) };
        let project = Project::for_template(&path);
        let extra_engines = self.options().extra_engines.clone();
        let i18n_key = {
            let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
            let (line_start, line, col) = util::line_at(&doc.text, pos);
            i18n_key_at(&project, &doc, &path, line_start, line, col, &extra_engines)
        };
        if let Some((key, app)) = i18n_key.zip(partials::app_root(&path)) {
            let sources = partial_rename::i18n_sources(&app, &self.open_texts());
            let mut locations = partial_rename::i18n_references(&project, &key, &sources, &extra_engines);
            if p.context.include_declaration {
                locations.extend(translation_locations(&self.locale_files(&path), &key));
            }
            return Ok(Some(locations));
        }
        let Some(views) = partials::views_root(&path) else { return Ok(None) };
        let files = {
            let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
            let (_, line, col) = util::line_at(&doc.text, pos);
            let model = partials::model_renders(line).into_iter().find(|&(s, e, _)| (s..=e).contains(&col));
            match (partials::render_ref_at(line, col), model) {
                (Some(r), _) => partials::first_folder_only(partials::resolve(&project, &path, &r)),
                (None, Some((_, _, name))) => partials::resolve_model(&project, &path, name),
                // On a component render the question isn't about this partial.
                (None, None) if partials::component_ref_at(line, col).is_some() => return Ok(None),
                (None, None) if file_name(&uri).starts_with('_') => vec![path],
                (None, None) => return Ok(None),
            }
        };
        let sources = partial_rename::sources(&views, &self.open_texts());
        let mut locations = partial_rename::references(&project, &files, &sources, &extra_engines);
        if p.context.include_declaration {
            locations.extend(files.iter().filter_map(file_location));
        }
        Ok(Some(locations))
    }

    async fn linked_editing_range(&self, p: LinkedEditingRangeParams) -> Result<Option<LinkedEditingRanges>> {
        let uri = p.text_document_position_params.text_document.uri;
        let Some(doc) = self.docs.get(&uri).filter(|d| d.lang == TemplateLang::Erb && html_host(&uri)) else { return Ok(None) };
        let Some(offset) = util::position_to_offset(&doc.text, p.text_document_position_params.position) else { return Ok(None) };
        let to_range = |(s, e)| Range::new(util::offset_to_position(&doc.text, s), util::offset_to_position(&doc.text, e));
        Ok(tags::linked_names(&doc.text, offset)
            .map(|names| LinkedEditingRanges { ranges: names.map(to_range).to_vec(), word_pattern: None }))
    }

    /// Partials and templates by their render name (`users/card`), matched like VS Code's fuzzy filter.
    async fn symbol(&self, p: WorkspaceSymbolParams) -> Result<Option<Vec<SymbolInformation>>> {
        let query = p.query.to_lowercase();
        let open = self.docs.iter().filter_map(|d| partials::views_root(&d.key().to_file_path().ok()?)).collect::<Vec<_>>();
        let mut roots: Vec<PathBuf> =
            self.roots.get().into_iter().flatten().map(|r| r.join("app/views")).filter(|v| v.is_dir()).chain(open).collect();
        roots.sort();
        roots.dedup();
        let mut out = Vec::new();
        for views in roots {
            let is_template = |f: &&PathBuf| Url::from_file_path(f).is_ok_and(|u| TemplateLang::from_uri_or_id(&u, None).is_some());
            let templates: Vec<PathBuf> = self.views_files(&views).iter().filter(is_template).cloned().collect();
            let project = Project::for_template(&views.join("_"));
            for (name, kind, file) in partials::named_templates(&project, &views, &templates) {
                if !util::fuzzy_match(&name.to_lowercase(), &query) {
                    continue;
                }
                let Some(location) = file_location(&file) else { continue };
                #[allow(deprecated)]
                out.push(SymbolInformation {
                    name,
                    kind: SymbolKind::FILE,
                    tags: None,
                    deprecated: None,
                    location,
                    container_name: Some(kind.name().to_lowercase()),
                });
            }
        }
        Ok(Some(out))
    }

    async fn will_rename_files(&self, p: RenameFilesParams) -> Result<Option<WorkspaceEdit>> {
        let to_path = |uri: &str| Url::parse(uri).ok()?.to_file_path().ok();
        let renames: Vec<(PathBuf, PathBuf)> = p.files.iter().filter_map(|f| Some((to_path(&f.old_uri)?, to_path(&f.new_uri)?))).collect();
        let mut roots: Vec<PathBuf> = renames.iter().filter_map(|(old, _)| partials::views_root(old)).collect();
        roots.sort();
        roots.dedup();
        let open = self.open_texts();
        let mut changes = HashMap::new();
        for views in roots {
            let sources = partial_rename::sources(&views, &open);
            let project = Project::for_template(&views.join("_"));
            changes.extend(partial_rename::render_edits(&project, &renames, &sources, &self.options().extra_engines));
        }
        Ok((!changes.is_empty()).then(|| WorkspaceEdit { changes: Some(changes), ..Default::default() }))
    }

    async fn formatting(&self, p: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let Some((text, formatted)) = self.formatted(&p.text_document.uri, &p.options, false).await else { return Ok(None) };
        if formatted == text {
            return Ok(Some(vec![]));
        }
        let end = util::offset_to_position(&text, text.len());
        Ok(Some(vec![TextEdit::new(Range::new(Position::new(0, 0), end), formatted)]))
    }

    /// Only the lines in the range change, with nesting read from the whole document; not advertised, the client's
    /// ERB and Haml providers ask for it.
    async fn range_formatting(&self, p: DocumentRangeFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let Some((text, formatted)) = self.formatted(&p.text_document.uri, &p.options, true).await else { return Ok(None) };
        Ok(line_edits(&text, &formatted, p.range.start.line as usize..=p.range.end.line as usize))
    }

    async fn completion(&self, p: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = p.text_document_position.text_document.uri;
        let pos = p.text_document_position.position;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let (line_start, line, col) = util::line_at(&doc.text, pos);
        let before = &line[..col.min(line.len())];

        let path = uri.to_file_path().ok();
        let items = |items| Ok(Some(CompletionResponse::Array(items)));

        if let Some((kind, prefix)) = partials::render_string_prefix(line, col) {
            let Some(path) = path.as_deref() else { return Ok(None) };
            return items(render_completions(&Project::for_template(path), path, kind, &prefix, &self.template_files(path)));
        }
        let i18n_prefix = i18n::completion_prefix(line, col)
            .filter(|&(start, ..)| code_at(doc.lang, &doc.text, line_start + start, &self.options().extra_engines));
        if let Some((start, end, prefix)) = i18n_prefix {
            let Some(path) = path.as_deref() else { return Ok(None) };
            let range = util::span_range(pos.line as usize, line, start, end);
            return items(i18n_completions(&Project::for_template(path), &self.locale_files(path), path, prefix, range));
        }
        if let Some((start, typed, slot)) = stimulus::completion_prefix(line, col) {
            let Some(path) = path.as_deref() else { return Ok(None) };
            let range = util::span_range(pos.line as usize, line, start, col);
            return items(stimulus_completions(path, typed, slot, range));
        }
        // `form.` and `f.text_field :` in prose (`Please fill in the form.`) aren't Ruby.
        let roles = line_roles(doc.lang, &doc.text);
        let role = roles.get(pos.line as usize).copied().unwrap_or(LineRole::Start);
        let code_here = || role == LineRole::Ruby || code_at(doc.lang, &doc.text, line_start + col, &self.options().extra_engines);
        if doc.lang == TemplateLang::Erb && html_host(&uri) && !code_here() {
            if let Some(context) = erb_complete::context(&doc.text, line_start + col) {
                return items(erb_complete::items(context, &doc.text, line_start + col));
            }
        }
        if let Some(typed) = schema::attribute_prefix(before).filter(|_| code_here()) {
            let Some(path) = path.as_deref() else { return Ok(None) };
            let Some(table) = schema::form_table(&doc.text[..line_start + col]) else { return Ok(None) };
            let columns = schema::columns(path, &table).into_iter().filter(|(name, _)| name.starts_with(typed));
            return items(
                columns.map(|(name, kind)| util::completion_item(name, CompletionItemKind::FIELD, format!("{table}.{kind}"))).collect(),
            );
        }
        if let Some(found) = path
            .as_deref()
            .and_then(|p| slot_setter_completions(&Project::for_template(p), p, &doc.text[..line_start + col]))
            .filter(|_| code_here())
        {
            return items(found);
        }
        if let Some(found) = receiver_completions(before).filter(|_| code_here()) {
            return items(found);
        }
        if let Some(typed) = component_prefix(before).filter(|_| code_here()) {
            let Some(path) = path.as_deref() else { return Ok(None) };
            let range = util::span_range(pos.line as usize, line, col - typed.len(), col);
            return items(component_completions(path, typed, range));
        }

        // Haml tag, filter and attribute names; Slim tag, engine and attribute names.
        if doc.lang == TemplateLang::Haml {
            if let Some(context) = haml::completion_context(line, col).filter(|_| role == LineRole::Start) {
                return items(haml::completions(context));
            }
            let lines: Vec<&str> = doc.text.lines().collect();
            let head = haml::continued_head(&lines, &roles, pos.line as usize, col).filter(|_| role == LineRole::Ruby);
            if let Some(context) = head.as_deref().and_then(|head| haml::completion_context(head, head.len())) {
                return items(haml::completions(context));
            }
        }

        // Rails helpers, only when we're in a Ruby context.
        let in_code = match (doc.lang, role) {
            (TemplateLang::Slim, _) => {
                let engines = &self.options().extra_engines;
                let parsed = slim_parser::parse(&doc.text, engines);
                let Some(offset) = util::position_to_offset(&doc.text, pos) else { return Ok(None) };
                if let Some(markup) = slim_complete::items(&doc.text, offset, &parsed, engines) {
                    return items(markup);
                }
                // An unclosed `#{` being typed is Ruby too, for completion only.
                slim_parser::in_ruby(&doc.text, &parsed, offset) || slim_parser::in_open_interpolation(&doc.text, &parsed, offset)
            }
            // Ruby LSP completes Ruby files; offering view helpers on every identifier would duplicate it.
            (TemplateLang::Ruby, _) => false,
            (_, LineRole::Start) => in_ruby_context(doc.lang, &doc.text, &erb::code_spans(&doc.text), line_start + col),
            (_, LineRole::Ruby) => !ruby::comment_before(before),
            (_, LineRole::Text) => false,
        };
        if !in_code {
            return Ok(None);
        }
        let typed = before.rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == '?')).next().unwrap_or("");
        let ivar = before[..before.len() - typed.len()].strip_suffix('@').filter(|b| !b.ends_with('@'));
        if let (Some(_), Some(path)) = (ivar, path.as_deref()) {
            let range = util::span_range(pos.line as usize, line, col - typed.len() - 1, col);
            return items(ivar_completions(&Project::for_template(path), path, typed, range));
        }
        if typed.is_empty() {
            return Ok(None);
        }
        let mut found = local_completions(&doc.text, typed);
        found.extend(helpers_for(doc.lang).filter(|h| h.name.starts_with(typed)).map(helper_item));
        let own_slots = path
            .as_deref()
            .and_then(|p| slots::class_of_template(&Project::for_template(p), p))
            .and_then(|class| std::fs::read_to_string(class).ok());
        let readers = own_slots.map(|class| slots::readers(&slots::slots(&class))).unwrap_or_default();
        found.extend(
            readers.into_iter().filter(|r| r.starts_with(typed)).map(|r| util::completion_item(r, CompletionItemKind::METHOD, "slot")),
        );
        let routes = path.as_deref().map(|p| self.routes_for(p)).unwrap_or_default();
        found.extend(
            routes::completions(&routes, typed)
                .into_iter()
                .map(|(helper, detail)| util::completion_item(helper, CompletionItemKind::FUNCTION, detail)),
        );
        items(found)
    }
}

/// Partial, template or layout names for the `render "…"` string being typed.
fn render_completions(
    project: &Project,
    path: &Path,
    kind: partials::RenderKind,
    prefix: &str,
    templates: &[PathBuf],
) -> Vec<CompletionItem> {
    partials::render_candidates(project, path, kind, templates)
        .into_iter()
        .filter(|(name, _)| name.starts_with(prefix))
        .map(|(name, file)| {
            let detail = file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            CompletionItem { insert_text: Some(name.clone()), ..util::completion_item(name, CompletionItemKind::FILE, detail) }
        })
        .collect()
}

/// i18n keys for the `t("…")` string being typed; each replaces `range`.
fn i18n_completions(
    project: &Project,
    locales: &[(PathBuf, Vec<i18n::Translation>)],
    path: &Path,
    prefix: &str,
    range: Range,
) -> Vec<CompletionItem> {
    i18n::completions(project, locales, path, prefix)
        .into_iter()
        .map(|(key, value)| CompletionItem {
            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(range, key.clone()))),
            ..util::completion_item(key, CompletionItemKind::CONSTANT, value)
        })
        .collect()
}

/// Stimulus controllers, or a controller's action methods or targets, for a `data-*` value; each replaces `range`.
fn stimulus_completions(path: &Path, typed: &str, slot: stimulus::Slot, range: Range) -> Vec<CompletionItem> {
    let source = |id: &str| stimulus::resolve(path, id).first().and_then(|file| std::fs::read_to_string(file).ok()).unwrap_or_default();
    let (names, kind, detail) = match slot {
        stimulus::Slot::Controller => (stimulus::controllers(path), CompletionItemKind::CLASS, "Stimulus controller".to_string()),
        stimulus::Slot::Method(id) => (stimulus::methods(&source(&id)), CompletionItemKind::METHOD, format!("{id} action")),
        stimulus::Slot::Target(id) => (stimulus::targets(&source(&id)), CompletionItemKind::FIELD, format!("{id} target")),
    };
    names
        .into_iter()
        .filter(|name| name.starts_with(typed))
        .map(|name| CompletionItem {
            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(range, name.clone()))),
            ..util::completion_item(name, kind, detail.clone())
        })
        .collect()
}

/// `@name` for the instance variables the view's controller action assigns; each replaces `range`.
fn ivar_completions(project: &Project, path: &Path, typed: &str, range: Range) -> Vec<CompletionItem> {
    let mut seen = std::collections::HashSet::new();
    ivars::for_view(project, path)
        .into_iter()
        .filter(|a| a.name.starts_with(typed) && seen.insert(a.name.clone()))
        .map(|a| {
            let label = format!("@{}", a.name);
            let from = a.file.file_name().map_or_else(String::new, |f| f.to_string_lossy().into_owned());
            CompletionItem {
                text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(range, label.clone()))),
                filter_text: Some(label.clone()),
                ..util::completion_item(label, CompletionItemKind::VARIABLE, format!("assigned in {from}:{}", a.line + 1))
            }
        })
        .collect()
}

/// `with_header` … after `card.` inside `render CardComponent.new do |card|`.
fn slot_setter_completions(project: &Project, path: &Path, before: &str) -> Option<Vec<CompletionItem>> {
    let (head, typed) = before.rsplit_once('.')?;
    let var = head.rsplit(|c: char| !(c.is_alphanumeric() || c == '_')).next()?;
    if var.is_empty() || !typed.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let component = slots::block_component(head, var)?;
    let detail = format!("{component} slot");
    let setters = slots::setters(&slots::component_slots(project, path, component));
    // Not a component with slots (or a reused name like `f`): leave `f.` to the form builder completions.
    if setters.is_empty() {
        return None;
    }
    Some(
        setters
            .into_iter()
            .filter(|s| s.starts_with(typed))
            .map(|s| util::completion_item(s, CompletionItemKind::METHOD, detail.clone()))
            .collect(),
    )
}

/// The component constant being typed after `render ` / `render(`: `Us`, `Admin::Us`.
fn component_prefix(before: &str) -> Option<&str> {
    static OPEN: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\brender\s*\(?\s*((?:[A-Z]\w*::)*[A-Z]\w*)$").unwrap());
    Some(OPEN.captures(before)?.get(1)?.as_str())
}

/// ViewComponent classes matching `typed`, inserted as `Name.new(…)`.
fn component_completions(path: &Path, typed: &str, range: Range) -> Vec<CompletionItem> {
    partials::component_classes(path)
        .into_iter()
        .filter(|(name, _)| name.starts_with(typed) || name.rsplit("::").next().is_some_and(|last| last.starts_with(typed)))
        .map(|(name, file)| CompletionItem {
            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(range, format!("{name}.new($1)")))),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            filter_text: Some(name.clone()),
            ..util::completion_item(
                name,
                CompletionItemKind::CLASS,
                file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
            )
        })
        .collect()
}

/// Form builder methods after `f.` / `form.` / `ff.`…, Turbo Stream actions after `turbo_stream.`.
fn receiver_completions(before: &str) -> Option<Vec<CompletionItem>> {
    let idx = before.rfind('.')?;
    let recv = before[..idx].rsplit(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
    let typed = &before[idx + 1..];
    if !typed.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    match recv {
        "f" | "form" | "ff" | "builder" | "fields" => Some(
            helpers::FORM_BUILDER
                .iter()
                .filter(|m| m.starts_with(typed))
                .map(|m| util::completion_item(*m, CompletionItemKind::METHOD, "ActionView::Helpers::FormBuilder"))
                .collect(),
        ),
        "turbo_stream" => Some(helpers::TURBO_STREAM.iter().filter(|h| h.name.starts_with(typed)).map(helper_item).collect()),
        _ => None,
    }
}

fn helper_item(h: &helpers::Helper) -> CompletionItem {
    CompletionItem {
        insert_text: Some(h.snippet.to_string()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..util::completion_item(h.name, CompletionItemKind::FUNCTION, h.detail)
    }
}

fn helpers_for(lang: TemplateLang) -> impl Iterator<Item = &'static helpers::Helper> {
    let haml_only = if lang == TemplateLang::Haml { helpers::HAML_HELPERS } else { &[] };
    helpers::HELPERS.iter().chain(haml_only)
}

/// Is byte `offset` of `text` in Ruby: inside an ERB code tag (`spans` from `erb::code_spans`, may cross lines)
/// or on a Haml/Slim Ruby line. Slim documents go through `slim_parser` where the whole text is at hand.
fn in_ruby_context(lang: TemplateLang, text: &str, spans: &[(usize, usize)], offset: usize) -> bool {
    let offset = offset.min(text.len());
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    match lang {
        // A Ruby `#` comment ends at the newline, so only this line of the tag matters.
        TemplateLang::Erb => erb::span_at(spans, offset).is_some_and(|code| !ruby::comment_before(&text[code.max(line_start)..offset])),
        TemplateLang::Haml => {
            let line = util::lines_with_offsets(&text[line_start..]).next().map_or("", |(_, line)| line);
            haml::in_ruby(line, offset - line_start)
        }
        // Skim code is CoffeeScript: no Rails helpers, renders or form builders.
        TemplateLang::Skim => false,
        TemplateLang::Ruby => !ruby::comment_before(&text[line_start..offset]),
        TemplateLang::Slim => {
            let before = &text[line_start..offset];
            let t = before.trim_start();
            t.starts_with('-')
                || t.starts_with('=')
                || t.starts_with("!=")
                || t.starts_with("&=")
                || before.contains("#{")
                || before.contains("= ")
                || before.contains('{')
                || before.contains('(')
        }
    }
}

/// Runs the language server on stdin/stdout until the client exits.
pub async fn serve() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::build(|client| Backend {
        client,
        docs: Arc::default(),
        controllers: DashMap::new(),
        templates: DashMap::new(),
        locales: DashMap::new(),
        routes: DashMap::new(),
        options: Arc::default(),
        roots: OnceLock::new(),
        dynamic_formatting: OnceLock::new(),
    })
    .custom_method("rubyTemplates/routes", Backend::set_routes)
    .finish();
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

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
    fn slot_setters_leave_other_receivers_alone() {
        let before = "<%= render CardComponent.new do |f| %><% end %>\n<%= form_with model: @user do |f| %>\n  <%= f.te";
        assert!(slot_setter_completions(&Project::unknown(), Path::new("/nowhere/app/views/users/new.html.erb"), before).is_none());
        assert!(receiver_completions(before).is_some_and(|items| !items.is_empty()));
    }
    #[test]
    fn watched_file_events_invalidate_caches() {
        let views = Path::new("/r/app/views");
        let event = |path: &str, kind| vec![(PathBuf::from(path), kind)];
        assert!(stale(views, &event("/r/app/views/users/_card.html.erb", FileChangeType::CREATED), false));
        assert!(stale(views, &event("/r/app/views/users", FileChangeType::DELETED), false));
        assert!(stale(views, &event("/r/app", FileChangeType::DELETED), false));
        assert!(!stale(views, &event("/r/app/views/users/_card.html.erb", FileChangeType::CHANGED), false));
        assert!(!stale(views, &event("/r/app/views_old/x.erb", FileChangeType::CREATED), false));
        assert!(!stale(views, &event("/other/app/views/x.erb", FileChangeType::DELETED), false));
        let locales = Path::new("/r/config/locales");
        assert!(stale(locales, &event("/r/config/locales/en.yml", FileChangeType::CHANGED), true));
        assert!(!stale(locales, &[], true));
    }

    #[test]
    fn line_edits_leave_line_endings_alone() {
        let edits = line_edits("%div\r\n    %p\r\n  %i\r\n", "%div\r\n    %p\r\n    %i\r\n", 0..=2).unwrap();
        assert_eq!(edits, [TextEdit::new(Range::new(Position::new(2, 0), Position::new(2, 4)), "    %i".into())]);
        assert!(line_edits("a\nb", "a\nb\nc", 0..=1).is_none());
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

        let haml = "%p hi\n  %b x\n- end\n:nope\n%br/ x\n";
        let all = Options::default();
        let erb_path = "file:///v/show.html.erb";
        // The tag check stays quiet while ERB blocks don't balance, so each code gets its own text.
        let erb_cases = [("<% end %>\n", erb::code::STRAY_END), ("<div>\n", tags::code::UNCLOSED_ELEMENT)];
        for (text, code) in erb_cases {
            assert_eq!(codes(erb_path, TemplateLang::Erb, text, &all), [code]);
        }
        let options = Options {
            disabled_erb: vec![erb::code::STRAY_END.into(), tags::code::UNCLOSED_ELEMENT.into()],
            disabled_haml: vec![haml_indent::code::ILLEGAL_NESTING.into()],
            ..Options::default()
        };
        assert!(erb_cases.iter().all(|(text, _)| codes(erb_path, TemplateLang::Erb, text, &options).is_empty()));
        let haml_path = "file:///v/show.html.haml";
        assert!(codes(haml_path, TemplateLang::Haml, haml, &all).contains(&haml_indent::code::ILLEGAL_NESTING.to_string()));
        let haml_codes = codes(haml_path, TemplateLang::Haml, haml, &options);
        assert!(!haml_codes.contains(&haml_indent::code::ILLEGAL_NESTING.to_string()), "{haml_codes:?}");
        assert!(!haml_codes.is_empty());
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
            let shared = [UNRESOLVED_RENDER, STRICT_LOCALS].map(String::from);
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
            include_str!("ruby_syntax.rs"),
            include_str!("haml_indent.rs"),
            include_str!("slim_indent.rs"),
        ] {
            assert!(!literal.is_match(src.split("#[cfg(test)]").next().unwrap()), "coded(…, \"literal\")");
        }
    }

    #[test]
    fn slim_code_at_asks_the_parser() {
        let text = "p See (t(\".a\"))\n= t(\".b\")\n";
        assert!(!code_at(TemplateLang::Slim, text, text.find(".a").unwrap(), &[]));
        assert!(code_at(TemplateLang::Slim, text, text.find(".b").unwrap(), &[]));
    }

    #[test]
    fn skim_skips_ruby_checks() {
        let uri = Url::parse("untitled:Untitled-1").unwrap();
        assert_eq!(TemplateLang::from_uri_or_id(&uri, Some("skim")), Some(TemplateLang::Skim));
        let text = "- else\n= alert(\"hi\"\n- form_with do |f|\n- end = start + 10\n  p\n    a\n   span\n";
        let codes = |lang| -> Vec<String> {
            diagnostics(&uri, &Doc { lang, text: text.into(), version: 0 }, &Options::default())
                .into_iter()
                .filter_map(|d| match d.code {
                    Some(NumberOrString::String(c)) => Some(c),
                    _ => None,
                })
                .collect()
        };
        let slim = codes(TemplateLang::Slim);
        assert!(slim_indent::code::RUBY.iter().all(|c| slim.iter().any(|s| s == c)), "{slim:?}");
        assert_eq!(codes(TemplateLang::Skim), [slim_indent::code::BAD_DEDENT]);
    }

    const GEM_LOCK: &str = "GEM\n  specs:\n    devise (4.9.4)\n      bcrypt (~> 3.0)\n    spree_core (4.8.0)\n    administrate (0.20.1)\n";

    fn lock_with(actionview: &str) -> Project {
        Project::from_lockfile(&format!("{GEM_LOCK}    actionview ({actionview})\n"))
    }

    fn unresolved(lang: TemplateLang, path: &Path, text: &str) -> Vec<Diagnostic> {
        unresolved_renders(path, &lock_with("7.1.0"), text, render_filter(lang, text, &[]))
    }

    /// `file line:col new_text` for each edit, sorted.
    fn rename(renames: &[(PathBuf, PathBuf)]) -> Vec<String> {
        let views = partials::views_root(&renames[0].0).unwrap();
        let sources = partial_rename::sources(&views, &HashMap::new());
        let mut got: Vec<String> = partial_rename::render_edits(&Project::unknown(), renames, &sources, &[])
            .into_iter()
            .flat_map(|(uri, edits)| {
                let file = uri.path().rsplit('/').next().unwrap_or("").to_string();
                edits.into_iter().map(move |e| format!("{file} {}:{} {}", e.range.start.line, e.range.start.character, e.new_text))
            })
            .collect();
        got.sort();
        got
    }

    fn views_tree(name: &str, files: &[(&str, &str)]) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("rt-rename-{name}-{}", std::process::id()));
        let views = root.join("app/views");
        for (file, text) in files {
            let path = views.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        (root, views)
    }

    #[test]
    fn partial_rename_updates_renders() {
        let (root, views) = views_tree("basic", &[
            ("users/_card.html.erb", ""),
            ("users/index.html.erb", "<%= render \"card\" %> <%= render \"users/card\" %> <p>render \"card\"</p>\n<%= link_to \"render 'card'\", x %>\n"),
            ("shared/box.html.haml", "%p render \"users/card\"\n.a= render \"users/card\", x: 1\n%p= \"render 'users/card'\"\n:javascript\n  = render \"users/card\"\n"),
            ("users/show.html.slim", "= render 'card'\n"),
        ]);
        let card = views.join("users/_card.html.erb");
        assert_eq!(
            rename(&[(card.clone(), views.join("users/_tile.html.erb"))]),
            ["box.html.haml 1:12 users/tile", "index.html.erb 0:12 tile", "index.html.erb 0:33 users/tile", "show.html.slim 0:10 tile",]
        );
        assert_eq!(
            rename(&[(card.clone(), views.join("shared/_card.html.erb"))]),
            [
                "box.html.haml 1:12 shared/card",
                "index.html.erb 0:12 shared/card",
                "index.html.erb 0:33 shared/card",
                "show.html.slim 0:10 shared/card",
            ]
        );
        assert!(rename(&[(card.clone(), views.join("users/card.html.erb"))]).is_empty());
        // Nothing names a partial directly in app/views.
        assert!(rename(&[(card, views.join("_card.html.erb"))]).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn partial_rename_needs_every_candidate_to_move() {
        let (root, views) = views_tree(
            "variants",
            &[
                ("users/_card.html.erb", ""),
                ("users/_card.html+phone.erb", ""),
                ("users/index.html.erb", "<%= render \"card\" %>\n<%= render partial: \"users/card\" %>\n"),
                ("application/_badge.html.erb", ""),
                ("users/_badge.html.erb", ""),
                ("users/show.html.erb", "<%= render \"badge\" %>\n"),
            ],
        );
        let (html, phone) = (views.join("users/_card.html.erb"), views.join("users/_card.html+phone.erb"));
        assert!(rename(&[(phone.clone(), views.join("users/_tile.html+phone.erb"))]).is_empty());
        let both = [(html.clone(), views.join("shared/_card.html.erb")), (phone.clone(), views.join("shared/_card.html+phone.erb"))];
        assert_eq!(rename(&both), ["index.html.erb 0:12 shared/card", "index.html.erb 1:21 shared/card"]);
        // Moving both, but to different names, leaves the render alone.
        assert!(rename(&[both[0].clone(), (phone, views.join("shared/_other.html+phone.erb"))]).is_empty());
        // `users/_badge` shadows `application/_badge` for users/show.
        assert!(rename(&[(views.join("application/_badge.html.erb"), views.join("application/_pill.html.erb"))]).is_empty());
        assert_eq!(rename(&[(views.join("users/_badge.html.erb"), views.join("users/_pill.html.erb"))]), ["show.html.erb 0:12 pill"]);
        std::fs::remove_dir_all(&root).unwrap();
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
    fn ruby_context_spans_erb_lines() {
        let text = "<%= link_to \"x\",\n  user_path(u) %> after <%# note %>\n<% # c\n  x %>";
        let at = |needle: &str| text.find(needle).unwrap();
        let spans = erb::code_spans(text);
        assert!(in_ruby_context(TemplateLang::Erb, text, &spans, at("user_path")));
        assert!(!in_ruby_context(TemplateLang::Erb, text, &spans, at("after")));
        assert!(!in_ruby_context(TemplateLang::Erb, text, &spans, at("note")));
        assert!(!in_ruby_context(TemplateLang::Erb, text, &spans, at("# c") + 2));
        assert!(in_ruby_context(TemplateLang::Erb, text, &spans, at("  x") + 2));
        let haml = "%p\n= link_to";
        assert!(in_ruby_context(TemplateLang::Haml, haml, &[], haml.len()));
        assert!(!in_ruby_context(TemplateLang::Haml, haml, &[], 2));
    }

    #[test]
    fn helper_hover_links_docs() {
        let link_to = helpers::HELPERS.iter().find(|h| h.name == "link_to").unwrap();
        assert!(helper_hover(link_to)
            .ends_with("[API docs](https://api.rubyonrails.org/classes/ActionView/Helpers/UrlHelper.html#method-i-link_to)"));
        let params = helpers::HELPERS.iter().find(|h| h.name == "params").unwrap();
        assert!(!helper_hover(params).contains("API docs"));
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
    fn strict_locals_hover_completion_and_diagnostics() {
        let root = std::env::temp_dir().join(format!("rt-locals-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        let card = "<%# locals: (user:, size: :md) -%>\n<p><%= user.name %></p>\n";
        std::fs::write(users.join("_card.html.erb"), card).unwrap();
        std::fs::write(users.join("_any.html.erb"), "<%# locals: (user:, **rest) -%>").unwrap();
        std::fs::write(users.join("_plain.html.erb"), "").unwrap();
        std::fs::write(users.join("_multi.html.erb"), "<%# locals: (a:) -%>").unwrap();
        std::fs::write(users.join("_multi.html+phone.erb"), "<%# locals: (b:) -%>").unwrap();
        let file = users.join("index.html.erb");

        let line = r#"<%= render partial: "card" %>"#;
        let md = render_hover(&Project::unknown(), &file, &partials::render_ref_at(line, 22).unwrap());
        assert!(md.ends_with("```ruby\nlocals: (user:, size: :md)\n```"), "{md}");
        assert!(!render_hover(&Project::unknown(), &file, &partials::render_ref_at(r#"render "plain""#, 9).unwrap()).contains("locals:"));

        let items: Vec<(String, Option<String>)> = local_completions(card, "").into_iter().map(|c| (c.label, c.detail)).collect();
        assert_eq!(
            items,
            vec![("user".to_string(), Some("local (required)".to_string())), ("size".to_string(), Some("local = :md".to_string())),]
        );
        assert_eq!(local_completions(card, "si").len(), 1);
        assert!(local_completions("<p>", "").is_empty());

        let text = concat!(
            "<%= render \"users/card\", user: u %> <%= render \"users/plain\", x: 1 %> <%= render \"users/any\", user: u, x: 1 %>\n",
            "<%= render \"users/card\", size: :lg, colour: 1 %>\n",
            "<%= render partial: \"users/card\", locals: { user: u } %> <%= render \"users/card\", **opts %>\n",
            "<%= render \"users/card\", Card::DEFAULTS %> <%= render \"card\" %> <%= render \"users/multi\" %>\n",
            "<%= render partial: \"users/plain\", locals: { a: 1 }, layout: \"users/card\" %>\n",
        );
        let got: Vec<(String, Range)> =
            strict_locals_mismatches(&file, &lock_with("7.1.0"), text, render_filter(TemplateLang::Erb, text, &[]))
                .into_iter()
                .map(|d| (d.message, d.range))
                .collect();
        let at = |l, s, e| Range::new(Position::new(l, s), Position::new(l, e));
        assert_eq!(got, vec![("missing local: :user".to_string(), at(1, 12, 22))]);
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
            let found = strict_locals_mismatches(&file, &project, &text, render_filter(TemplateLang::Erb, &text, &[]));
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
        let mismatches = strict_locals_mismatches(&file, &lock_with("7.1.0"), text, render_filter(TemplateLang::Ruby, text, &[]));
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
}
