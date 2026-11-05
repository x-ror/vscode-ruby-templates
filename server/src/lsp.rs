use crate::checks::{diagnostics_in, Doc, Options, STRICT_LOCALS, UNRESOLVED_RENDER};
use crate::code_context::{self, block_locals, code_at, in_ruby_context, line_roles, render_filter};
use crate::completion::{
    block_local_completions, component_completions, component_prefix, has_arguments, helper_item, helpers_for, i18n_completions,
    ivar_completions, local_completions, receiver_completions, render_completions, render_local_completions, slot_setter_completions,
    stimulus_completions,
};
use crate::controllers::Target;
use crate::lang::{file_name, is_partial, TemplateLang};
use crate::util::{file_location, line_location};
use crate::{
    assets, config, controller_classes, controllers, erb, erb_complete, format, haml, haml_indent, helper_defs, helpers, i18n, ivars,
    model_columns, outline, partial_rename, partials,
    project::{self, Project, Projects},
    rails_paths,
    render_call::HamlPipes,
    routes, ruby, schema, semantic_tokens, slim_complete, slim_hover, slim_outline, slim_parser, slots, stimulus, strict_locals, tags,
    util, view_roots,
};
use dashmap::mapref::entry::Entry;
use dashmap::{DashMap, DashSet};
use haml_indent::LineRole;
use std::cell::{LazyCell, OnceCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

type LocaleFiles = Arc<i18n::Locales>;

struct Backend {
    client: Client,
    docs: Arc<DashMap<Uri, Doc>>,
    /// Open controllers, for navigation to their views only; no other request reads them.
    controllers: DashMap<Uri, String>,
    /// Files under each app/views root, for render completion.
    templates: Arc<DashMap<PathBuf, Arc<Vec<PathBuf>>>>,
    /// Parsed locale files per config/locales directory.
    locales: Arc<DashMap<PathBuf, LocaleFiles>>,
    /// Bumped before watched-file events invalidate the caches, so a background fill started earlier doesn't
    /// store what it read before the change.
    cache_generation: Arc<AtomicU64>,
    /// The cache roots a background fill is reading, so documents opened together start one fill each.
    warming: Arc<DashSet<PathBuf>>,
    /// Each app's view roots, found off the server task and dropped when a watched file may have changed them.
    view_roots: Arc<view_roots::Cache>,
    /// Routes per Rails root, as the client last sent them.
    routes: DashMap<PathBuf, Arc<Vec<routes::Route>>>,
    // Set once in `initialize`; the client restarts the server when these settings change.
    options: Arc<OnceLock<Options>>,
    /// Workspace folders from `initialize`, for workspace symbols before any view is open.
    roots: OnceLock<Vec<PathBuf>>,
    /// Formatting is registered for Slim/Skim only, when the client supports dynamic registration and `slim.format.enabled` is on:
    /// a static capability would cover ERB/Haml too, with a formatter that does nothing there.
    dynamic_formatting: OnceLock<bool>,
    /// How many `render` calls pass each (partial, local), for hover; cleared when any view's text may have changed.
    local_callers: DashMap<(PathBuf, String), usize>,
    /// The latest diagnostics run per open document: runs finish in any order, and only the latest may publish.
    publishes: Arc<DashMap<Uri, u64>>,
    next_publish: AtomicU64,
}

const DIAGNOSTICS_DEBOUNCE: Duration = Duration::from_millis(200);
const SERVER_NAME: &str = "ruby-templates-ls";
/// The client's notification with the app's routes.
const ROUTES_NOTIFICATION: &str = "rubyTemplates/routes";
const SLIM_FORMATTING_REGISTRATION: &str = "slim-formatting";

fn render_hover(project: &Project, path: &Path, r: &partials::RenderRef) -> String {
    let found = partials::resolve(project, path, r);
    if found.is_empty() {
        return format!("**{}** `{}`\n\nNo matching template found.", r.kind.name(), r.name);
    }
    let list: Vec<String> = found.iter().map(|f| format!("- `{}`", rails_paths::rails_relative(f, path))).collect();
    let mut md = format!("**{}** `{}`\n\n{}", r.kind.name(), r.name, list.join("\n"));
    if let Some(sig) = project.file_strict_locals(&found[0]) {
        md.push_str(&format!("\n\n```ruby\nlocals: ({})\n```", sig.params));
    }
    md
}

/// Is the word under `col` a hash or keyword key (`size: 1`) rather than a value?
fn is_key(line: &str, col: usize) -> bool {
    let rest = &line[col.min(line.len())..];
    let after = rest.trim_start_matches(|c: char| c.is_alphanumeric() || matches!(c, '_' | '?' | '!'));
    after.starts_with(':') && !after.starts_with("::")
}

/// A Rails or Haml helper this extension knows, never a local of the template.
fn is_builtin_helper(name: &str) -> bool {
    helpers::HELPERS.iter().chain(helpers::HAML_HELPERS).any(|h| h.name == name)
}

/// Whether `name` can be a Ruby local variable's name.
fn is_local_name(name: &str) -> bool {
    helper_defs::name_at(name, 0) == Some(name) && !name.ends_with(['?', '!']) && !ruby::KEYWORDS.contains(&name)
}

/// A partial's strict local, and where the partial declares, reads and assigns it (template byte ranges).
struct StrictLocal {
    name: String,
    /// The span under the cursor.
    range: Range,
    declaration: Option<(usize, usize)>,
    uses: Vec<(usize, usize)>,
}

/// A partial's strict local: required or its default, and how many `render` calls pass it.
fn local_hover(name: &str, default: Option<&str>, passes: usize) -> String {
    let declared = default.map_or_else(|| "required".to_string(), |d| format!("default {}", util::inline_code(d)));
    let passed = match passes {
        0 => "No `render` passes it".to_string(),
        1 => "Passed by 1 `render` call".to_string(),
        n => format!("Passed by {n} `render` calls"),
    };
    format!("`{name}`: local, {declared}\n\n{passed}")
}

/// Byte ranges on `line` that lead to files by convention: `render @model`, Stimulus identifiers, asset paths.
/// With `col`, only the ones under it are resolved.
fn convention_targets(project: &Project, path: &Path, text: &str, line_start: usize, line: &str, col: Option<usize>) -> Vec<Target> {
    let at = |s: usize, e: usize| col.is_none_or(|c| (s..=e).contains(&c));
    let models = partials::model_renders(line).into_iter().filter(|&(s, e, _)| at(s, e)).map(|(start, end, name)| Target {
        start,
        end,
        files: partials::resolve_model(project, path, name),
    });
    let controllers = stimulus::identifiers(line).into_iter().filter(|&(s, e, _)| at(s, e)).map(|(start, end, id)| Target {
        start,
        end,
        files: stimulus::resolve(path, &id),
    });
    let assets = assets::asset_refs(text, line_start, line).into_iter().filter(|r| at(r.start, r.end)).map(|r| Target {
        start: r.start,
        end: r.end,
        files: assets::resolve(path, &r),
    });
    models.chain(controllers).chain(assets).filter(|target| !target.files.is_empty()).collect()
}

/// The partials whose callers references at `pos` of the template at `path` look for: the ones the `render` under it names,
/// else the partial itself; `None` on a component render or in a template that isn't a partial.
fn referenced_partials(project: &Project, path: &Path, doc: &Doc, pos: Position) -> Option<Vec<PathBuf>> {
    let (line_start, line, col) = util::line_at(&doc.text, pos);
    if let Some(r) = partials::render_ref_at(&doc.text, line_start, line, doc.lang, project.prism_version(), col) {
        return Some(partials::first_folder_only(partials::resolve(project, path, &r)));
    }
    if let Some((_, _, name)) = partials::model_renders(line).into_iter().find(|&(s, e, _)| (s..=e).contains(&col)) {
        return Some(partials::resolve_model(project, path, name));
    }
    // On a component render the question isn't about this partial.
    if partials::component_ref_at(line, col).is_some() || !rails_paths::is_partial_path(path) {
        return None;
    }
    Some(vec![path.to_path_buf()])
}

/// Full key of the `t("…")` literal under `col`, when the call is Ruby.
fn i18n_key_at(project: &Project, doc: &Doc, path: &Path, pos: Position, extra_engines: &[String]) -> Option<String> {
    let (line_start, line, col) = util::line_at(&doc.text, pos);
    let (start, end) = i18n::key_at(line, col)?;
    if !code_at(doc.lang, &doc.text, line_start + start, extra_engines) {
        return None;
    }
    i18n::full_key(project, path, &line[start..end])
}

/// A link to `target` from the file `from`, its tooltip the path from `from`'s Rails root.
fn file_link(range: Range, target: &Path, from: &Path) -> Option<DocumentLink> {
    Some(DocumentLink {
        range,
        target: Some(crate::util::file_uri(target)?),
        tooltip: Some(rails_paths::rails_relative(target, from)),
        data: None,
    })
}

/// The line of `key` in each locale file that defines it.
fn translation_locations(files: &i18n::Locales, key: &str) -> Vec<Location> {
    i18n::lookup(files, key).filter_map(|(file, t)| line_location(file, t.line)).collect()
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

/// Whether the watched-file `events` make the cache for `root` stale. Listings change only when files are
/// created or deleted; `contents` caches also on edits. Deleting an ancestor of `root` counts too.
fn stale(root: &Path, events: &[(PathBuf, FileChangeType)], contents: bool) -> bool {
    events.iter().any(|(path, kind)| (contents || *kind != FileChangeType::CHANGED) && (path.starts_with(root) || root.starts_with(path)))
}

/// Stores `read()` under `root` unless the entry is filled meanwhile; reads again when a watched-file event came in
/// while it read, since what it read may predate the change. The generation is checked under the entry's lock,
/// which the event's invalidation (`retain`, after the bump) takes too.
fn fill<T>(cache: &DashMap<PathBuf, T>, root: &Path, generation: &AtomicU64, read: impl Fn() -> T) {
    loop {
        let started = generation.load(Ordering::SeqCst);
        let value = read();
        let Entry::Vacant(slot) = cache.entry(root.to_path_buf()) else { return };
        if generation.load(Ordering::SeqCst) == started {
            slot.insert(value);
            return;
        }
    }
}

/// A created or deleted file may add or remove a view root; a changed one only when it is the lockfile that names the gems.
fn may_change_view_roots(events: &[(PathBuf, FileChangeType)]) -> bool {
    events.iter().any(|(path, kind)| *kind != FileChangeType::CHANGED || path.ends_with(project::LOCKFILE))
}

/// tower-lsp polls every handler in one task, so a scan of the whole app here would hold up all other requests.
async fn off_server_task<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.map_err(|_| tower_lsp_server::jsonrpc::Error::internal_error())
}

/// Edits replacing each line in `lines` that differs between `text` and `formatted`; `None` when formatting added or
/// removed lines, which a line range can't express, other than the empty lines at the end Haml and Slim drop.
fn line_edits(text: &str, formatted: &str, lines: std::ops::RangeInclusive<usize>) -> Option<Vec<TextEdit>> {
    let old: Vec<&str> = text.lines().collect();
    let new: Vec<&str> = formatted.lines().collect();
    let shared = old.len().min(new.len());
    if old[shared..].iter().chain(&new[shared..]).any(|line| !line.trim().is_empty()) {
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
fn spacing_actions(uri: &Uri, text: &str, range: Range, only: Option<&[CodeActionKind]>) -> Vec<CodeActionOrCommand> {
    let (Some(from), Some(to)) = (util::position_to_offset(text, range.start), util::position_to_offset(text, range.end)) else {
        return vec![];
    };
    let edits = erb::spacing_edits(text);
    let index = util::LineIndex::new(text);
    let to_edit = |(s, e, tag): &(usize, usize, String)| TextEdit::new(index.range(*s, *e), tag.clone());
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

fn rename_filter(glob: &str, matches: FileOperationPatternKind) -> FileOperationFilter {
    FileOperationFilter {
        scheme: Some("file".into()),
        pattern: FileOperationPattern { glob: glob.into(), matches: Some(matches), options: None },
    }
}

impl Backend {
    /// The document's text and the text formatted with the editor's `options`; `None` (logged) when it is left as it is,
    /// and, unlogged, when formatting is switched off for its language.
    /// `lines_only` leaves out the steps that add or remove lines, which a range can't hold.
    async fn formatted(&self, uri: &Uri, options: &FormattingOptions, lines_only: bool) -> Option<(String, String)> {
        let (lang, plain_erb, text) =
            self.docs.get(uri).filter(|doc| !doc.percent_lines).map(|doc| (doc.lang, doc.plain_erb, doc.text.clone()))?;
        if !self.options().format_enabled(lang) {
            return None;
        }
        let (uri, options) = (uri.clone(), options.clone());
        // A range may hold a comment's inner lines without the line they move with.
        let align_comments = self.options().align_erb_comments && !lines_only;
        let extra_engines = self.options().extra_engines.clone();
        // Reads .editorconfig and the project from disk, and formats a whole file: off the server task.
        let result = off_server_task(move || {
            let path = util::file_path(&uri);
            let mut settings =
                path.map_or_else(format::Settings::default, |path| config::settings_for(&path).settings).with_editor(&options);
            if lines_only {
                settings.final_newline = false;
            }
            settings.align_comments = align_comments;
            let result = format::format(&uri, lang, plain_erb, &text, &settings, &extra_engines);
            (uri, text, result)
        })
        .await
        .ok()?;
        match result {
            (_, text, Ok(formatted)) => Some((text, formatted.text)),
            (uri, _, Err(skipped)) => {
                self.client.log_message(MessageType::INFO, format!("{}: left unchanged: {skipped}", file_name(&uri))).await;
                None
            }
        }
    }

    /// Liquid is highlighted only: every Rails lookup would misfire on it.
    fn is_liquid(&self, uri: &Uri) -> bool {
        self.docs.get(uri).is_some_and(|doc| doc.lang == TemplateLang::Liquid)
    }

    fn options(&self) -> &Options {
        self.options.get_or_init(Options::default)
    }

    /// Publish the document's diagnostics after `delay`, unless a later call for it (an edit, a reopen, a file
    /// change on disk) supersedes this one. They are computed on a copy, off the server task: holding the `docs`
    /// guard would block the next `did_change`.
    fn publish_later(&self, uri: Uri, delay: Duration) {
        // Unique across closes and reopens, which may restart a document's versions.
        let run = self.next_publish.fetch_add(1, Ordering::Relaxed);
        self.publishes.insert(uri.clone(), run);
        let (client, docs, publishes, options, view_roots) = (
            self.client.clone(),
            Arc::clone(&self.docs),
            Arc::clone(&self.publishes),
            Arc::clone(&self.options),
            Arc::clone(&self.view_roots),
        );
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let latest = || publishes.get(&uri).is_some_and(|latest| *latest == run);
            let Some(doc) = docs.get(&uri).filter(|_| latest()).map(|doc| doc.clone()) else { return };
            let (checked, version) = (uri.clone(), doc.version);
            let Ok(diags) = off_server_task(move || {
                let mut projects = Projects::new();
                if let Some(path) = util::file_path(&checked) {
                    Project::memoized(&mut projects, &path, Some(&view_roots));
                }
                diagnostics_in(&checked, &doc, options.get_or_init(Options::default), &mut projects)
            })
            .await
            else {
                return;
            };
            if latest() {
                client.publish_diagnostics(uri, diags, Some(version)).await;
            }
        });
    }

    /// The project of `path` sharing the view roots kept in the cache.
    fn project_for(&self, path: &Path) -> Project {
        Project::for_template(path).with_view_cache(Arc::clone(&self.view_roots))
    }

    /// `project_for`, with the view roots found off the server task first when this app's aren't cached yet, and kept in
    /// the project so a cache refresh meanwhile doesn't search on the server task.
    async fn project_of(&self, path: &Path) -> Result<Project> {
        let project = self.project_for(path);
        let roots = match project.cached_view_roots() {
            Some(roots) => Some(roots),
            None => {
                let searching = project.clone();
                off_server_task(move || searching.load_view_roots()).await?
            }
        };
        if let Some(roots) = roots {
            project.use_view_roots(roots);
        }
        Ok(project)
    }

    /// `views_files`, reading a listing that isn't cached off the server task.
    async fn views_files_off_task(&self, views: PathBuf) -> Result<Arc<Vec<PathBuf>>> {
        if let Some(files) = self.templates.get(&views) {
            return Ok(Arc::clone(&files));
        }
        let templates = Arc::clone(&self.templates);
        off_server_task(move || {
            let files = rails_paths::files_under(&views, usize::MAX);
            Arc::clone(&*templates.entry(views).or_insert_with(|| Arc::new(files)))
        })
        .await
    }

    fn template_files(&self, current: &Path) -> Arc<Vec<PathBuf>> {
        let Some(views) = rails_paths::views_root(current) else {
            return Arc::new(current.parent().map(|dir| rails_paths::files_under(dir, 1)).unwrap_or_default());
        };
        self.views_files(&views)
    }

    /// Every file under an app/views root, cached until a watched-file event says otherwise.
    fn views_files(&self, views: &Path) -> Arc<Vec<PathBuf>> {
        self.templates.entry(views.to_path_buf()).or_insert_with(|| Arc::new(rails_paths::files_under(views, usize::MAX))).clone()
    }

    /// Fills the app/views and locale caches of `uri`'s app in the background, so the first render or i18n request
    /// after a view opens or files change finds them ready instead of walking the app on the server task.
    fn warm_caches(&self, uri: &Uri) {
        let Some(path) = util::file_path(uri) else { return };
        let claim = |root: &PathBuf, cached: bool| !cached && self.warming.insert(root.clone());
        let views = rails_paths::views_root(&path).filter(|views| claim(views, self.templates.contains_key(views)));
        let locales = i18n::locales_dir(&path).filter(|dir| claim(dir, self.locales.contains_key(dir)));
        if views.is_none() && locales.is_none() {
            return;
        }
        let (templates, cached_locales, generation, warming) =
            (Arc::clone(&self.templates), Arc::clone(&self.locales), Arc::clone(&self.cache_generation), Arc::clone(&self.warming));
        tokio::task::spawn_blocking(move || {
            if let Some(views) = views {
                fill(&templates, &views, &generation, || Arc::new(rails_paths::files_under(&views, usize::MAX)));
                warming.remove(&views);
            }
            if let Some(dir) = locales {
                fill(&cached_locales, &dir, &generation, || Arc::new(i18n::locale_files(&dir)));
                warming.remove(&dir);
            }
        });
    }

    fn locale_files(&self, current: &Path) -> LocaleFiles {
        let Some(dir) = i18n::locales_dir(current) else { return LocaleFiles::default() };
        self.locales.entry(dir.clone()).or_insert_with(|| Arc::new(i18n::locale_files(&dir))).clone()
    }

    /// `rubyTemplates/routes`: `{ root, routes }` from the client's `bin/rails runner`.
    async fn set_routes(&self, params: serde_json::Value) {
        let Some(root) = params.get("root").and_then(serde_json::Value::as_str).map(PathBuf::from) else { return };
        let parsed = routes::parse(&root, params.get("routes").unwrap_or(&serde_json::Value::Null));
        self.routes.insert(root, Arc::new(parsed));
    }

    /// Where the `@ivar` or partial local under `col` comes from: its assignments in the controller, or the
    /// `render` calls passing the local (else the strict-locals comment declaring it).
    async fn variable_definitions(&self, text: &str, path: &Path, line: &str, col: usize) -> Result<Vec<Location>> {
        if let Some(name) = ivars::name_at(line, col) {
            return Ok(ivars::for_view(&self.project_for(path), path)
                .into_iter()
                .filter(|a| a.name == name)
                .filter_map(|a| line_location(&a.file, a.line))
                .collect());
        }
        let is_partial = rails_paths::is_partial_path(path);
        // `link_to` or `t` is never a local; the scan below reads every view.
        let Some(name) = helper_defs::name_at(line, col).filter(|name| is_partial && !is_builtin_helper(name)) else { return Ok(vec![]) };
        let project = self.project_for(path);
        let (close, ruby) = (project.strict_locals_close(), project.prism_version());
        let passes = self.local_passes(project, path, name).await?;
        if !passes.is_empty() {
            return Ok(passes);
        }
        let declared = strict_locals::signature(text, close, ruby).is_some_and(|sig| sig.declares(name));
        let comment = text.lines().position(|l| l.contains("locals:"));
        Ok(comment.filter(|_| declared).and_then(|line| line_location(path, line)).into_iter().collect())
    }

    /// The `render` calls that pass `local` to the partial at `path`, read from app/views and the open documents.
    async fn local_passes(&self, project: Project, path: &Path, local: &str) -> Result<Vec<Location>> {
        let Some(views) = rails_paths::views_root(path) else { return Ok(vec![]) };
        let (open, partial, local) = (self.open_texts(), path.to_path_buf(), local.to_string());
        let extra_engines = self.options().extra_engines.clone();
        off_server_task(move || {
            let sources = partial_rename::sources(&views, &open);
            partial_rename::local_passes(&project, &partial, &local, &sources, &extra_engines)
        })
        .await
    }

    /// Where the template `uri` (`doc`) defines the local `name` read at `offset`: a block variable or an assignment.
    async fn local_definition_range(&self, uri: &Uri, project: &Project, doc: &Doc, offset: usize, name: &str) -> Result<Option<Range>> {
        let text = doc.text.as_str();
        let (lang, source, engines) = (doc.lang, text.to_string(), self.options().extra_engines.clone());
        let found = off_server_task(move || code_context::local_definition(lang, &source, offset, &engines)).await?;
        // A partial's strict local that the template also assigns (`user ||= …`) is defined by its callers.
        let strict_local = || is_partial(uri) && project.strict_locals(text).is_some_and(|sig| sig.declares(name));
        let Some(found) = found.filter(|found| found.parameter || !strict_local()) else { return Ok(None) };
        let index = util::LineIndex::new(text);
        Ok(Some(index.range(found.start, found.end)))
    }

    /// `local_passes` counted, from the `local_callers` cache.
    async fn local_caller_count(&self, project: Project, path: &Path, local: &str) -> Result<usize> {
        let key = (path.to_path_buf(), local.to_string());
        if let Some(count) = self.local_callers.get(&key).map(|count| *count) {
            return Ok(count);
        }
        let count = self.local_passes(project, path, local).await?.len();
        self.local_callers.insert(key, count);
        Ok(count)
    }

    /// Quick fixes for the strict-locals warning `d` on a render in `text`: pass the missing locals, or drop an unknown one
    /// from the call or declare it in the partial.
    fn strict_locals_fixes(&self, project: &Project, path: &Path, doc: &Doc, d: &Diagnostic) -> Vec<CodeAction> {
        let text = &doc.text;
        let (line_start, line, start) = util::line_at(text, d.range.start);
        let pipes = HamlPipes::new(doc.lang);
        let Some((r, call)) =
            partials::render_calls_in(text, line_start, line, &pipes, project.prism_version(), |r| r.start == start).pop()
        else {
            return vec![];
        };
        let Some(passed) = partials::passed_locals(&call, &r) else { return vec![] };
        // Several variants or formats: which one Rails picks depends on the request, as the check says.
        let Ok([partial]) = <[PathBuf; 1]>::try_from(partials::resolve(project, path, &r)) else { return vec![] };
        let (Some(uri), Some(partial_uri)) = (crate::util::file_uri(path), crate::util::file_uri(&partial)) else { return vec![] };
        let partial_text = util::open_or_read(self.docs.get(&partial_uri).map(|doc| doc.text.clone()), &partial).unwrap_or_default();
        let sig = project.strict_locals(&partial_text);
        let (Some(sig), Some(comment)) = (sig, strict_locals::find(&partial_text, project.strict_locals_close())) else {
            return vec![];
        };
        let missing = sig.missing(&passed);
        let unknown = sig.unknown(&passed);
        let base = call.base(line_start);
        let at = |text: &str, offset: usize| util::offset_to_position(text, offset);
        let here = |(range, new): partials::ByteEdit| {
            let range = Range::new(at(text, base + range.start), at(text, base + range.end));
            HashMap::from([(uri.clone(), vec![TextEdit::new(range, new)])])
        };
        let action = |title: String, changes, preferred| CodeAction {
            title,
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![d.clone()]),
            edit: Some(WorkspaceEdit { changes: Some(changes), ..Default::default() }),
            is_preferred: Some(preferred),
            ..Default::default()
        };
        let quoted = |names: &[&str]| names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
        let mut out = Vec::new();
        if let Some(edit) = (!missing.is_empty()).then(|| partials::add_locals_edit(&call, &r, &missing)).flatten() {
            // Not preferred: `nil` silences the warning without passing what the partial needs.
            out.push(action(format!("Pass {} to the render", quoted(&missing)), here(edit), false));
        }
        let shown = partial.file_name().map_or_else(String::new, |f| f.to_string_lossy().into_owned());
        for name in unknown {
            if let Some(edit) = partials::remove_local_edit(&call, &r, name) {
                out.push(action(format!("Remove `{name}` from the render"), here(edit), false));
            }
            let (range, new) = comment.declare(name, partials::NEW_LOCAL_VALUE);
            let range = Range::new(at(&partial_text, range.start), at(&partial_text, range.end));
            let changes = HashMap::from([(partial_uri.clone(), vec![TextEdit::new(range, new)])]);
            out.push(action(format!("Declare `{name}` in the locals of `{shown}`"), changes, false));
        }
        out
    }

    /// The strict local under `pos` of the partial `uri` and its word's range: a read in code, or its parameter in the
    /// `locals:` comment; not a hash key of that name, nor a block variable hiding it.
    fn strict_local_at(&self, uri: &Uri, doc: &Doc, pos: Position) -> Option<StrictLocal> {
        if !is_partial(uri) {
            return None;
        }
        let project = self.project_for(&util::file_path(uri)?);
        let close = project.strict_locals_close();
        let comment = strict_locals::find(&doc.text, close)?;
        let (line_start, line, col) = util::line_at(&doc.text, pos);
        let name = helper_defs::name_at(line, col)?;
        if !project.strict_locals(&doc.text)?.declares(name) {
            return None;
        }
        let at = line_start + line[..col.min(line.len())].trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len();
        let declaration = comment.parameter_at(name).map(|start| (start, start + name.len()));
        let mut uses = comment.parameter_reads(name);
        uses.extend(code_context::local_uses(doc.lang, &doc.text, close, name, &self.options().extra_engines).spans);
        let &(start, end) = declaration.iter().chain(&uses).find(|&&(start, _)| start == at)?;
        let index = util::LineIndex::new(&doc.text);
        Some(StrictLocal { name: name.to_string(), range: index.range(start, end), declaration, uses })
    }

    /// Where the partial `path` (text `text`) declares and uses `local`, then the `render` calls passing it.
    async fn local_references(&self, path: &Path, text: &str, local: &StrictLocal, declaration: bool) -> Result<Vec<Location>> {
        let Some(uri) = crate::util::file_uri(path) else { return Ok(vec![]) };
        let index = util::LineIndex::new(text);
        let span = |(start, end): (usize, usize)| Location::new(uri.clone(), index.range(start, end));
        let declared = local.declaration.filter(|_| declaration);
        let mut out: Vec<Location> = declared.into_iter().chain(local.uses.iter().copied()).map(span).collect();
        out.extend(self.local_passes(self.project_for(path), path, &local.name).await?);
        Ok(out)
    }

    /// The partials and templates a `render` string in `path` can name, this app's and its engines'.
    async fn render_path_completions(&self, path: &Path, kind: partials::RenderKind, prefix: &str) -> Result<Vec<CompletionItem>> {
        let project = self.project_of(path).await?;
        // Gem roots are left out: their listing can be large, and the app's own roots are what it edits.
        let engine_roots: Vec<PathBuf> = rails_paths::views_root(path)
            .zip(project.view_roots())
            .map(|(own, roots)| roots.other_app_roots(&own).into_iter().map(Path::to_path_buf).collect())
            .unwrap_or_default();
        let mut engines: Vec<(PathBuf, Arc<Vec<PathBuf>>)> = Vec::new();
        for root in engine_roots {
            let files = self.views_files_off_task(root.clone()).await?;
            engines.push((root, files));
        }
        let other_roots: Vec<(&Path, &[PathBuf])> = engines.iter().map(|(root, files)| (root.as_path(), files.as_slice())).collect();
        Ok(render_completions(&project, path, kind, prefix, &self.template_files(path), &other_roots))
    }

    /// Completions at `pos` outside a `render` string: the first cursor context that applies answers, in this order.
    /// `at` is `util::line_at(&doc.text, pos)`.
    fn completions_at(
        &self,
        uri: &Uri,
        doc: &Doc,
        path: Option<&Path>,
        pos: Position,
        (line_start, line, col): (usize, &str, usize),
    ) -> Option<Vec<CompletionItem>> {
        let before = &line[..col.min(line.len())];
        // Read from disk (lockfile, initializers), so built once per request and only when a branch needs it.
        let project_cell = OnceCell::new();
        let project = |path: &Path| project_cell.get_or_init(|| self.project_for(path));

        let engines = &self.options().extra_engines;
        let ruby = || path.map_or(ruby_prism::Version::Latest, |path| project(path).prism_version());
        let render_key = partials::local_key_before(&doc.text, line_start + col, doc.lang, ruby)
            .filter(|(r, ..)| code_at(doc.lang, &doc.text, r.call, engines));
        if let (Some((r, passed, typed)), Some(path)) = (render_key, path) {
            let found = render_local_completions(project(path), path, &r, &passed, typed);
            if !found.is_empty() {
                return Some(found);
            }
        }
        let i18n_prefix =
            i18n::completion_prefix(line, col).filter(|&(start, ..)| code_at(doc.lang, &doc.text, line_start + start, engines));
        if let Some((start, end, prefix)) = i18n_prefix {
            let path = path?;
            let range = util::span_range(pos.line as usize, line, start, end);
            return Some(i18n_completions(project(path), &self.locale_files(path), path, prefix, range));
        }
        if let Some((start, typed, slot)) = stimulus::completion_prefix(line, col) {
            let path = path?;
            let range = util::span_range(pos.line as usize, line, start, col);
            return Some(stimulus_completions(path, typed, slot, range));
        }
        // `form.` and `f.text_field :` in prose (`Please fill in the form.`) aren't Ruby.
        let roles = line_roles(doc.lang, &doc.text);
        let role = roles.get(pos.line as usize).copied().unwrap_or(LineRole::Start);
        let code_here = || role == LineRole::Ruby || code_at(doc.lang, &doc.text, line_start + col, engines);
        // The attributes the Stimulus controllers of the element being written read, from its start tag so far.
        let stimulus_attributes = |element: &str| path.map(|p| stimulus::attributes(p, element)).unwrap_or_default();
        let indented_element = || stimulus::indented_element(&doc.text[..line_start + col]);
        if doc.lang == TemplateLang::Erb && doc.html_host(uri) && !code_here() {
            let offset = line_start + col;
            if let Some(context) = erb_complete::context(&doc.text, offset) {
                let tag = erb_complete::start_tag(&doc.text[..offset]).unwrap_or_default();
                return Some(erb_complete::items(context, &doc.text, offset, || stimulus_attributes(tag)));
            }
        }
        if let Some(typed) = schema::attribute_prefix(before).filter(|_| code_here()) {
            let path = path?;
            let table = schema::form_table(&doc.text[..line_start + col])?;
            let columns = schema::columns(path, &table).into_iter().filter(|(name, _)| name.starts_with(typed));
            return Some(
                columns.map(|(name, kind)| util::completion_item(name, CompletionItemKind::FIELD, format!("{table}.{kind}"))).collect(),
            );
        }
        if let Some(found) =
            path.filter(|_| code_here()).and_then(|p| slot_setter_completions(project(p), p, &doc.text[..line_start + col]))
        {
            return Some(found);
        }
        if let Some(found) = receiver_completions(before, &line[col..]).filter(|_| code_here()) {
            return Some(found);
        }
        if let Some(typed) = component_prefix(before).filter(|_| code_here()) {
            let path = path?;
            let range = util::span_range(pos.line as usize, line, col - typed.len(), col);
            return Some(component_completions(path, typed, range));
        }

        // Haml tag, filter and attribute names; Slim tag, engine and attribute names.
        if doc.lang == TemplateLang::Haml {
            if let Some(context) = haml::completion_context(line, col).filter(|_| role == LineRole::Start) {
                return Some(haml::completions(context, || stimulus_attributes(indented_element())));
            }
            let lines: Vec<&str> = doc.text.lines().collect();
            let head = haml::continued_head(&lines, &roles, pos.line as usize, col).filter(|_| role == LineRole::Ruby);
            if let Some((head, context)) = head.as_deref().and_then(|head| Some((head, haml::completion_context(head, head.len())?))) {
                return Some(haml::completions(context, || stimulus_attributes(head)));
            }
        }

        // Rails helpers, only when we're in a Ruby context.
        let in_code = match (doc.lang, role) {
            (TemplateLang::Slim, _) => {
                let parsed = slim_parser::parse(&doc.text, engines);
                let offset = util::position_to_offset(&doc.text, pos)?;
                if let Some(markup) = slim_complete::items(&doc.text, offset, &parsed, engines, || stimulus_attributes(indented_element()))
                {
                    return Some(markup);
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
            return None;
        }
        let typed = before.rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == '?')).next().unwrap_or("");
        let ivar = before[..before.len() - typed.len()].strip_suffix('@').filter(|b| !b.ends_with('@'));
        if let (Some(_), Some(path)) = (ivar, path) {
            let range = util::span_range(pos.line as usize, line, col - typed.len() - 1, col);
            return Some(ivar_completions(project(path), path, typed, range));
        }
        if typed.is_empty() {
            return None;
        }
        Some(self.identifier_completions(doc, path.map(|p| (p, project(p))), line_start + col, &line[col..], typed))
    }

    /// Block variables, strict locals, helpers, the template's own slots and route helpers that start with `typed`.
    fn identifier_completions(
        &self,
        doc: &Doc,
        file: Option<(&Path, &Project)>,
        offset: usize,
        after: &str,
        typed: &str,
    ) -> Vec<CompletionItem> {
        let (close, ruby) = file
            .map(|(_, project)| (project.strict_locals_close(), project.prism_version()))
            .unwrap_or((Default::default(), ruby_prism::Version::Latest));
        let mut found = block_local_completions(block_locals(doc.lang, &doc.text, offset, &self.options().extra_engines), typed);
        // A block variable shadows the strict local of the same name.
        let locals: Vec<_> =
            local_completions(&doc.text, typed, close, ruby).into_iter().filter(|l| found.iter().all(|b| b.label != l.label)).collect();
        found.extend(locals);
        let called = has_arguments(after);
        found.extend(helpers_for(doc.lang).filter(|h| h.name.starts_with(typed)).map(|h| helper_item(h, called)));
        let own_slots =
            file.and_then(|(path, project)| slots::class_of_template(project, path)).and_then(|class| std::fs::read_to_string(class).ok());
        let readers = own_slots.map(|class| slots::readers(&slots::slots(&class))).unwrap_or_default();
        found.extend(
            readers.into_iter().filter(|r| r.starts_with(typed)).map(|r| util::completion_item(r, CompletionItemKind::METHOD, "slot")),
        );
        let routes = file.map(|(path, _)| self.routes_for(path)).unwrap_or_default();
        found.extend(
            routes::completions(&routes, typed)
                .into_iter()
                .map(|(helper, detail)| util::completion_item(helper, CompletionItemKind::FUNCTION, detail)),
        );
        found
    }

    /// Whether a partial's strict local can take the name `new`: a local's name nothing in the partial declares, reads or
    /// calls, so no read of the renamed local means something else.
    fn free_local_name(&self, doc: &Doc, project: &Project, new: &str) -> bool {
        let close = project.strict_locals_close();
        let declared = project.strict_locals(&doc.text).is_some_and(|sig| sig.declares(new));
        let in_defaults = strict_locals::find(&doc.text, close).is_some_and(|comment| !comment.parameter_reads(new).is_empty());
        let uses = code_context::local_uses(doc.lang, &doc.text, close, new, &self.options().extra_engines);
        is_local_name(new) && !declared && !in_defaults && uses.spans.is_empty() && !uses.shadowed
    }

    /// Routes of the Rails app `current` belongs to, empty until the client has sent them.
    fn routes_for(&self, current: &Path) -> Arc<Vec<routes::Route>> {
        let root = rails_paths::rails_root(current);
        root.and_then(|root| self.routes.get(&root).map(|r| Arc::clone(&r))).unwrap_or_default()
    }

    /// Open documents' text, which may have unsaved edits. Keyed by path: VS Code's Windows URIs differ in drive-letter
    /// case and colon encoding (`c%3A` vs `C:`) from `util::file_uri`.
    fn open_texts(&self) -> HashMap<PathBuf, String> {
        let templates = self.docs.iter().filter_map(|d| Some((util::file_path(d.key())?, d.text.clone())));
        // Partial and i18n references also scan controllers; their unsaved text is the current one.
        let controllers = self.controllers.iter().filter_map(|c| Some((util::file_path(c.key())?, c.value().clone())));
        templates.chain(controllers).collect()
    }
}

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
        let _ = self.roots.set(folders.chain(root_uri).filter_map(|u| util::file_path(&u)).collect());
        let dynamic_formatting =
            params.capabilities.text_document.and_then(|t| t.formatting).and_then(|f| f.dynamic_registration).unwrap_or(false);
        let _ = self.dynamic_formatting.set(dynamic_formatting);
        Ok(InitializeResult {
            offset_encoding: None,
            server_info: Some(ServerInfo { name: SERVER_NAME.into(), version: Some(env!("CARGO_PKG_VERSION").into()) }),
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
                semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(SemanticTokensOptions {
                    legend: semantic_tokens::legend(),
                    full: Some(SemanticTokensFullOptions::Bool(true)),
                    ..Default::default()
                })),
                document_symbol_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                // Only a partial's strict locals rename; Ruby LSP renames Ruby.
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
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
        if self.dynamic_formatting.get() == Some(&true) && self.options().format_enabled(TemplateLang::Slim) {
            let selector = serde_json::json!({ "documentSelector": [{ "language": "slim" }, { "language": "skim" }] });
            let registration = Registration {
                id: SLIM_FORMATTING_REGISTRATION.into(),
                method: "textDocument/formatting".into(),
                register_options: Some(selector),
            };
            if let Err(e) = self.client.register_capability(vec![registration]).await {
                self.client.log_message(MessageType::WARNING, format!("Slim formatting not registered: {e}")).await;
            }
        }
        self.client.log_message(MessageType::INFO, format!("{SERVER_NAME} ready")).await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, p: DidOpenTextDocumentParams) {
        let Some(lang) = TemplateLang::from_uri_or_id(&p.text_document.uri, Some(&p.text_document.language_id)) else {
            if util::file_path(&p.text_document.uri).is_some_and(|path| controller_classes::controller_path(&path).is_some()) {
                self.controllers.insert(p.text_document.uri, p.text_document.text);
            }
            return;
        };
        self.docs.insert(
            p.text_document.uri.clone(),
            Doc {
                lang,
                plain_erb: p.text_document.language_id == "erb",
                percent_lines: p.text_document.language_id == "erb-percent",
                text: p.text_document.text,
                version: p.text_document.version,
            },
        );
        self.local_callers.clear();
        self.warm_caches(&p.text_document.uri);
        self.publish_later(p.text_document.uri, Duration::ZERO);
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
        self.local_callers.clear();
        self.publish_later(p.text_document.uri, DIAGNOSTICS_DEBOUNCE);
    }

    async fn did_change_watched_files(&self, p: DidChangeWatchedFilesParams) {
        let events: Vec<(PathBuf, FileChangeType)> =
            p.changes.into_iter().filter_map(|e| Some((util::file_path(&e.uri)?, e.typ))).collect();
        self.cache_generation.fetch_add(1, Ordering::SeqCst);
        self.local_callers.clear();
        if may_change_view_roots(&events) {
            self.view_roots.clear();
        }
        self.templates.retain(|root, _| !stale(root, &events, false));
        self.locales.retain(|root, _| !stale(root, &events, true));
        // Unresolved-render and strict-locals warnings read other templates from disk.
        let open: Vec<Uri> = self.docs.iter().map(|d| d.key().clone()).collect();
        for uri in open {
            self.warm_caches(&uri);
            self.publish_later(uri, Duration::ZERO);
        }
    }

    async fn did_close(&self, p: DidCloseTextDocumentParams) {
        if self.controllers.remove(&p.text_document.uri).is_some() {
            return;
        }
        self.publishes.remove(&p.text_document.uri);
        self.docs.remove(&p.text_document.uri);
        self.local_callers.clear();
        self.client.publish_diagnostics(p.text_document.uri, vec![], None).await;
    }

    async fn goto_definition(&self, p: GotoDefinitionParams) -> Result<Option<GotoDefinitionResponse>> {
        let uri = p.text_document_position_params.text_document.uri;
        let pos = p.text_document_position_params.position;
        if self.is_liquid(&uri) {
            return Ok(None);
        }
        let Some(path) = util::file_path(&uri) else { return Ok(None) };
        let project = self.project_of(&path).await?;
        if let Some(text) = self.controllers.get(&uri) {
            let (_, line, col) = util::line_at(&text, pos);
            let found =
                controllers::targets(&project, &path, line, Some(col)).into_iter().next().map(|target| target.files).unwrap_or_default();
            let locs: Vec<Location> = found.into_iter().filter_map(file_location).collect();
            return Ok((!locs.is_empty()).then_some(GotoDefinitionResponse::Array(locs)));
        }
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let (line_start, line, col) = util::line_at(&doc.text, pos);

        if let Some(key) = i18n_key_at(&project, &doc, &path, pos, &self.options().extra_engines) {
            let locs = translation_locations(&self.locale_files(&path), &key);
            return Ok((!locs.is_empty()).then_some(GotoDefinitionResponse::Array(locs)));
        }
        let targets: Vec<PathBuf> =
            if let Some(r) = partials::render_ref_at(&doc.text, line_start, line, doc.lang, project.prism_version(), col) {
                partials::resolve(&project, &path, &r)
            } else if let Some(c) = partials::component_ref_at(line, col) {
                partials::resolve_component(&project, &path, &c)
            } else {
                convention_targets(&project, &path, &doc.text, line_start, line, Some(col))
                    .into_iter()
                    .next()
                    .map(|target| target.files)
                    .unwrap_or_default()
            };
        let mut locs: Vec<Location> = targets.into_iter().filter_map(file_location).collect();
        if !locs.is_empty() || !code_at(doc.lang, &doc.text, line_start + col, &self.options().extra_engines) {
            return Ok((!locs.is_empty()).then_some(GotoDefinitionResponse::Array(locs)));
        }
        let engines = &self.options().extra_engines;
        let declaration = |at| code_context::local_definition(doc.lang, &doc.text, at, engines).filter(|d| d.parameter).map(|d| d.start);
        if let Some(found) = model_columns::column_at(&project, &path, &doc.text, line_start, col, declaration) {
            return Ok(line_location(&found.schema, found.column.line).map(GotoDefinitionResponse::Scalar));
        }
        // Cheapest first: Ctrl+hover asks for a definition on every word the mouse crosses.
        let helper = helper_defs::name_at(line, col).map(str::to_string);
        let route = helper.as_deref().and_then(|name| routes::for_helper(&self.routes_for(&path), name)?.0.source.clone());
        let (owned, line) = (Doc::clone(&doc), line.to_string());
        // The guard would block the next `did_change` while the work below waits.
        drop(doc);
        // A block variable or a local the template assigns hides a helper of the same name.
        if let Some(name) = helper.as_deref().filter(|name| !is_builtin_helper(name)) {
            if let Some(range) = self.local_definition_range(&uri, &project, &owned, line_start + col, name).await? {
                return Ok(Some(GotoDefinitionResponse::Scalar(Location::new(uri, range))));
            }
        }
        let at_line = |(file, line): (PathBuf, usize)| line_location(file, line);
        match (route, helper) {
            (Some(source), _) => locs.extend(at_line(source)),
            (None, Some(name)) => {
                let current = path.clone();
                locs.extend(off_server_task(move || helper_defs::definitions(&current, &name)).await?.into_iter().filter_map(at_line));
            }
            (None, None) => {}
        }
        if locs.is_empty() {
            locs.extend(self.variable_definitions(&owned.text, &path, &line, col).await?);
        }
        Ok((!locs.is_empty()).then_some(GotoDefinitionResponse::Array(locs)))
    }

    async fn document_link(&self, p: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        let uri = p.text_document.uri;
        if self.is_liquid(&uri) {
            return Ok(None);
        }
        let Some(path) = util::file_path(&uri) else { return Ok(None) };
        let project = self.project_of(&path).await?;
        if let Some(text) = self.controllers.get(&uri) {
            let links = util::lines_with_offsets(&text)
                .enumerate()
                .flat_map(|(i, (_, line))| controllers::targets(&project, &path, line, None).into_iter().map(move |t| (i, line, t)))
                .filter_map(|(i, line, t)| file_link(util::span_range(i, line, t.start, t.end), t.files.first()?, &path));
            return Ok(Some(links.collect()));
        }
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let in_code = render_filter(doc.lang, &doc.text, &self.options().extra_engines);
        let pipes = HamlPipes::new(doc.lang);
        let mut links = Vec::new();
        for (i, (line_start, line)) in util::lines_with_offsets(&doc.text).enumerate() {
            // Same renders as diagnostics and references: code only, and the file Rails would pick.
            let renders =
                partials::render_refs_in(&doc.text, line_start, line, &pipes, project.prism_version(), |r| in_code(i, line_start, r))
                    .into_iter()
                    .map(|r| Target {
                        start: r.start,
                        end: r.end,
                        files: partials::first_folder_only(partials::resolve(&project, &path, &r)),
                    });
            for Target { start, end, files } in renders.chain(convention_targets(&project, &path, &doc.text, line_start, line, None)) {
                links.extend(files.first().and_then(|target| file_link(util::span_range(i, line, start, end), target, &path)));
            }
        }
        Ok(Some(links))
    }

    async fn hover(&self, p: HoverParams) -> Result<Option<Hover>> {
        let uri = p.text_document_position_params.text_document.uri;
        let pos = p.text_document_position_params.position;
        if self.is_liquid(&uri) {
            return Ok(None);
        }
        let file = match util::file_path(&uri) {
            Some(path) => Some((self.project_of(&path).await?, path)),
            None => None,
        };
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
        if let Some((project, path)) = &file {
            if let Some(r) = partials::render_ref_at(&doc.text, line_start, line, doc.lang, project.prism_version(), col) {
                return Ok(markdown_hover(render_hover(project, path, &r)));
            }
            if let Some(md) = i18n_key_at(project, &doc, path, pos, &self.options().extra_engines)
                .and_then(|key| i18n::hover(&self.locale_files(path), &key))
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
        // `@tag` is a variable, though `tag` is a helper.
        if let Some(name) = ivars::name_at(line, col).filter(|_| code_here) {
            let assignments = file.as_ref().map(|(project, path)| ivars::for_view(project, path)).unwrap_or_default();
            let view = file.as_ref().map(|(_, path)| path.as_path()).unwrap_or(Path::new(""));
            return Ok(ivars::hover(&assignments, name, view).and_then(markdown_hover));
        }
        if let Some((project, path)) = file.as_ref().filter(|_| code_here) {
            let engines = &self.options().extra_engines;
            let declaration =
                |at| code_context::local_definition(doc.lang, &doc.text, at, engines).filter(|d| d.parameter).map(|d| d.start);
            if let Some(found) = model_columns::column_at(project, path, &doc.text, line_start, col, declaration) {
                return Ok(markdown_hover(found.hover()));
            }
        }
        let declared_local = file.as_ref().filter(|_| code_here && is_partial(&uri)).and_then(|(project, _)| {
            let name = helper_defs::name_at(line, col).filter(|_| !is_key(line, col))?;
            let sig = project.strict_locals(&doc.text)?;
            let local = sig.locals.into_iter().find(|(local, _)| local == name)?;
            // A block variable of the same name shadows the local.
            let engines = &self.options().extra_engines;
            (!block_locals(doc.lang, &doc.text, line_start + col, engines).contains(&local.0)).then_some(local)
        });
        if let (Some((name, default)), Some((project, path))) = (declared_local, &file) {
            // The guard would block the next `did_change` while the views are read.
            drop(doc);
            let passes = self.local_caller_count(project.clone(), path, &name).await?;
            return Ok(markdown_hover(local_hover(&name, default.as_deref(), passes)));
        }
        if let Some(h) = helpers_for(doc.lang).find(|h| h.name == word).filter(|_| code_here) {
            return Ok(markdown_hover(helper_hover(h)));
        }
        let routes = file.as_ref().map(|(_, path)| self.routes_for(path)).unwrap_or_default();
        if let Some(md) = routes::hover(&routes, word).filter(|_| code_here) {
            return Ok(markdown_hover(md));
        }
        // Ctrl+hover asks on every word: the cheap checks come first.
        let name = helper_defs::name_at(line, col)
            .filter(|name| code_here && !is_key(line, col) && !is_builtin_helper(name) && !ruby::KEYWORDS.contains(name));
        // A quote opened earlier in the code makes the word text, as in `<%= "page_title" %>`.
        let code_before = line[..col].rfind("<%").map_or(&line[..col], |tag| &line[tag + 2..col]);
        let (Some(name), Some((_, path))) = (name.filter(|_| ruby::code_end(code_before) == ruby::CodeEnd::Code), file) else {
            return Ok(None);
        };
        let (name, lang, text, offset, engines) =
            (name.to_string(), doc.lang, doc.text.clone(), line_start + col, self.options().extra_engines.clone());
        drop(doc);
        off_server_task(move || {
            // A block variable (also where it is declared) or a local the template assigns hides a helper of the same name.
            let local = code_context::local_definition(lang, &text, offset, &engines).is_some()
                || code_context::block_locals(lang, &text, offset, &engines).contains(&name);
            if local {
                return None;
            }
            helper_defs::hover(&path, &name)
        })
        .await
        .map(|md| md.and_then(markdown_hover))
    }

    async fn folding_range(&self, p: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let Some((lang, html, text)) =
            self.docs.get(&p.text_document.uri).map(|doc| (doc.lang, doc.html_host(&p.text_document.uri), doc.text.clone()))
        else {
            return Ok(None);
        };
        let extra_engines = self.options().extra_engines.clone();
        // A provider's ranges replace VS Code's indentation folding; `None` falls back to it. That fallback is
        // what folds the host language of `.js.erb`/`.yml.erb`/… where ERB blocks alone would regress.
        off_server_task(move || match lang {
            TemplateLang::Haml => Some(outline::haml_folding_ranges(&text)),
            TemplateLang::Slim | TemplateLang::Skim => Some(slim_outline::folding_ranges(&text, &extra_engines)),
            TemplateLang::Erb if html => Some(outline::folding_ranges(&text)),
            TemplateLang::Erb | TemplateLang::Ruby | TemplateLang::Liquid => None,
        })
        .await
    }

    async fn semantic_tokens_full(&self, p: SemanticTokensParams) -> Result<Option<SemanticTokensResult>> {
        let uri = p.text_document.uri;
        let Some((lang, text)) = self.docs.get(&uri).map(|doc| (doc.lang, doc.text.clone())) else { return Ok(None) };
        // Ruby LSP colours Jbuilder and Builder views; an empty answer here would blank them.
        if !semantic_tokens::supports(lang) {
            return Ok(None);
        }
        let path = util::file_path(&uri);
        let extra_engines = self.options().extra_engines.clone();
        let data = off_server_task(move || {
            let close = path.map_or_else(Default::default, |path| Project::for_template(&path).strict_locals_close());
            semantic_tokens::tokens(lang, &text, close, &extra_engines)
        })
        .await?;
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens { result_id: None, data })))
    }

    async fn document_symbol(&self, p: DocumentSymbolParams) -> Result<Option<DocumentSymbolResponse>> {
        let uri = p.text_document.uri;
        let Some((lang, html, text)) = self.docs.get(&uri).map(|doc| (doc.lang, doc.html_host(&uri), doc.text.clone())) else {
            return Ok(None);
        };
        let extra_engines = self.options().extra_engines.clone();
        off_server_task(move || {
            let symbols = match lang {
                TemplateLang::Erb => outline::document_symbols(&text, html),
                TemplateLang::Haml => outline::haml_document_symbols(&text),
                TemplateLang::Slim | TemplateLang::Skim => slim_outline::document_symbols(&text, &extra_engines),
                TemplateLang::Ruby | TemplateLang::Liquid => return None,
            };
            Some(DocumentSymbolResponse::Nested(symbols))
        })
        .await
    }

    async fn code_action(&self, p: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = p.text_document.uri;
        if self.is_liquid(&uri) {
            return Ok(None);
        }
        // A copy, so no guard is held while the fixes below read the partial from the same map.
        let Some(doc) = self.docs.get(&uri).map(|doc| doc.clone()) else { return Ok(None) };
        let Some(path) = util::file_path(&uri) else { return Ok(None) };
        // Asked on every cursor move; the project is only needed for a missing partial's quick fix.
        let strict_fix = p.context.diagnostics.iter().any(|d| util::code(d) == Some(STRICT_LOCALS));
        let with_roots = if strict_fix { Some(self.project_of(&path).await?) } else { None };
        let project = LazyCell::new(|| with_roots.unwrap_or_else(|| self.project_for(&path)));
        let mut actions: Vec<CodeActionOrCommand> = Vec::new();
        if doc.lang == TemplateLang::Erb {
            actions.extend(spacing_actions(&uri, &doc.text, p.range, p.context.only.as_deref()));
        }
        for d in p.context.diagnostics.iter().filter(|d| util::code(d) == Some(STRICT_LOCALS)) {
            actions.extend(self.strict_locals_fixes(&project, &path, &doc, d).into_iter().map(CodeActionOrCommand::CodeAction));
        }
        let unresolved = p.context.diagnostics.into_iter().filter(|d| util::code(d) == Some(UNRESOLVED_RENDER));
        let pipes = HamlPipes::new(doc.lang);
        let quick_fixes = unresolved.filter_map(|d| {
            let (line_start, line, start) = util::line_at(&doc.text, d.range.start);
            let r = partials::render_refs_in(&doc.text, line_start, line, &pipes, project.prism_version(), |r| r.start == start).pop()?;
            let target = partials::new_template_path(&project, &path, &r)?;
            let create = CreateFile {
                uri: crate::util::file_uri(&target)?,
                options: Some(CreateFileOptions { overwrite: Some(false), ignore_if_exists: Some(true) }),
                annotation_id: None,
            };
            let shown = util::slash_path(target.strip_prefix(rails_paths::views_root(&path)?).ok()?);
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

    async fn prepare_rename(&self, p: TextDocumentPositionParams) -> Result<Option<PrepareRenameResponse>> {
        if self.is_liquid(&p.text_document.uri) {
            return Ok(None);
        }
        // A copy, so no guard is held while the template compiles.
        let Some(doc) = self.docs.get(&p.text_document.uri).map(|doc| doc.clone()) else { return Ok(None) };
        let Some(path) = util::file_path(&p.text_document.uri) else { return Ok(None) };
        if Project::for_template(&path).is_gem_file(&path) {
            return Ok(None);
        }
        Ok(self.strict_local_at(&p.text_document.uri, &doc, p.position).map(|local| PrepareRenameResponse::Range(local.range)))
    }

    /// Renames a partial's strict local: its parameter in the `locals:` comment, its reads and assignments, and the keys of
    /// the `render` calls that pass it.
    async fn rename(&self, p: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = p.text_document_position.text_document.uri;
        let new = p.new_name;
        if self.is_liquid(&uri) {
            return Ok(None);
        }
        let Some(doc) = self.docs.get(&uri).map(|doc| doc.clone()) else { return Ok(None) };
        let Some(local) = self.strict_local_at(&uri, &doc, p.text_document_position.position) else { return Ok(None) };
        let Some(path) = util::file_path(&uri) else { return Ok(None) };
        let project = self.project_for(&path);
        if project.is_gem_file(&path) {
            return Ok(None);
        }
        let extra_engines = self.options().extra_engines.clone();
        if !self.free_local_name(&doc, &project, &new) {
            return Err(tower_lsp_server::jsonrpc::Error::invalid_params(format!("`{new}` can't be the local's new name")));
        }
        let index = util::LineIndex::new(&doc.text);
        let edit = |start: usize, end: usize, text: String| TextEdit::new(index.range(start, end), text);
        let name = &local.name;
        let mut here: Vec<TextEdit> = local.declaration.map(|(start, end)| edit(start, end, new.clone())).into_iter().collect();
        for &(start, end) in &local.uses {
            // `render "x", user:` passes the local under the key `user`, which names the other partial's local.
            here.push(match ruby::omits_value(&doc.text[end..]) {
                true => edit(start, end + 1, format!("{name}: {new}")),
                false => edit(start, end, new.clone()),
            });
        }
        let Some(views) = rails_paths::views_root(&path) else { return Ok(None) };
        let (open, partial, old, renamed) = (self.open_texts(), path.clone(), name.clone(), new.clone());
        let (callers, unread) = off_server_task(move || {
            let sources = partial_rename::sources(&views, &open);
            partial_rename::local_key_edits(&project, &partial, &old, &renamed, &sources, &extra_engines)
        })
        .await?;
        if !unread.is_empty() {
            let calls: Vec<String> = unread.iter().map(|l| format!("{}:{}", file_name(&l.uri), l.range.start.line + 1)).collect();
            let message = format!("Renamed `{name}` without these `render` calls, whose locals can't be read: {}", calls.join(", "));
            self.client.show_message(MessageType::WARNING, message).await;
        }
        let mut changes = HashMap::from([(uri, here)]);
        for (file, text_edit) in callers {
            changes.entry(file).or_default().push(text_edit);
        }
        Ok(Some(WorkspaceEdit { changes: Some(changes), ..Default::default() }))
    }

    /// Who renders the partial under the cursor's `render "…"`, or the current partial anywhere else in it.
    async fn references(&self, p: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = p.text_document_position.text_document.uri;
        let pos = p.text_document_position.position;
        if self.is_liquid(&uri) {
            return Ok(None);
        }
        let Some(path) = util::file_path(&uri) else { return Ok(None) };
        let project = self.project_of(&path).await?;
        let extra_engines = self.options().extra_engines.clone();
        let i18n_key = {
            let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
            i18n_key_at(&project, &doc, &path, pos, &extra_engines)
        };
        let declaration = p.context.include_declaration;
        if let Some((key, app)) = i18n_key.zip(rails_paths::app_root(&path)) {
            let declarations = if declaration { translation_locations(&self.locale_files(&path), &key) } else { vec![] };
            let open = self.open_texts();
            let mut locations = off_server_task(move || {
                let sources = partial_rename::i18n_sources(&app, &open);
                partial_rename::i18n_references(&project, &key, &sources, &extra_engines)
            })
            .await?;
            locations.extend(declarations);
            return Ok(Some(locations));
        }
        let Some(doc) = self.docs.get(&uri).map(|doc| doc.clone()) else { return Ok(None) };
        if let Some(local) = self.strict_local_at(&uri, &doc, pos) {
            return Ok(Some(self.local_references(&path, &doc.text, &local, declaration).await?));
        }
        let Some(views) = rails_paths::views_root(&path) else { return Ok(None) };
        let Some(files) = referenced_partials(&project, &path, &doc, pos) else { return Ok(None) };
        let declarations: Vec<Location> = if declaration { files.iter().filter_map(file_location).collect() } else { vec![] };
        let open = self.open_texts();
        let mut locations = off_server_task(move || {
            let sources = partial_rename::sources(&views, &open);
            partial_rename::references(&project, &files, &sources, &extra_engines)
        })
        .await?;
        locations.extend(declarations);
        Ok(Some(locations))
    }

    async fn linked_editing_range(&self, p: LinkedEditingRangeParams) -> Result<Option<LinkedEditingRanges>> {
        let uri = p.text_document_position_params.text_document.uri;
        let Some(doc) = self.docs.get(&uri).filter(|d| d.lang == TemplateLang::Erb && d.html_host(&uri)) else {
            return Ok(None);
        };
        let Some(offset) = util::position_to_offset(&doc.text, p.text_document_position_params.position) else { return Ok(None) };
        let index = util::LineIndex::new(&doc.text);
        let to_range = |(s, e)| index.range(s, e);
        Ok(tags::linked_names(&doc.text, offset)
            .map(|names| LinkedEditingRanges { ranges: names.map(to_range).to_vec(), word_pattern: None }))
    }

    /// Partials and templates by their render name (`users/card`), matched like VS Code's fuzzy filter.
    async fn symbol(&self, p: WorkspaceSymbolParams) -> Result<Option<WorkspaceSymbolResponse>> {
        let query = p.query.to_lowercase();
        let open = self.docs.iter().filter_map(|d| rails_paths::views_root(&util::file_path(d.key())?)).collect::<Vec<_>>();
        let mut roots: Vec<PathBuf> =
            self.roots.get().into_iter().flatten().map(|r| r.join(rails_paths::APP_VIEWS)).filter(|v| v.is_dir()).chain(open).collect();
        roots.sort();
        roots.dedup();
        let mut out = Vec::new();
        for views in roots {
            let is_template = |f: &&PathBuf| {
                crate::util::file_uri(f)
                    .is_some_and(|u| TemplateLang::from_uri_or_id(&u, None).is_some_and(|lang| lang != TemplateLang::Liquid))
            };
            let templates: Vec<PathBuf> = self.views_files(&views).iter().filter(is_template).cloned().collect();
            let project = Project::for_views(&views).with_view_cache(Arc::clone(&self.view_roots));
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
        Ok(Some(WorkspaceSymbolResponse::Flat(out)))
    }

    async fn will_rename_files(&self, p: RenameFilesParams) -> Result<Option<WorkspaceEdit>> {
        let to_path = |uri: &str| util::file_path(&uri.parse::<Uri>().ok()?);
        let renames: Vec<(PathBuf, PathBuf)> = p.files.iter().filter_map(|f| Some((to_path(&f.old_uri)?, to_path(&f.new_uri)?))).collect();
        // A gem's files are read-only: renaming one rewrites nothing.
        let renames: Vec<_> = renames.into_iter().filter(|(old, _)| !Project::for_template(old).is_gem_file(old)).collect();
        let mut roots: Vec<PathBuf> = renames.iter().filter_map(|(old, _)| rails_paths::views_root(old)).collect();
        roots.sort();
        roots.dedup();
        let open = self.open_texts();
        let extra_engines = self.options().extra_engines.clone();
        let view_roots = Arc::clone(&self.view_roots);
        let changes = off_server_task(move || {
            let mut changes = HashMap::new();
            for views in roots {
                let sources = partial_rename::sources(&views, &open);
                let project = Project::for_views(&views).with_view_cache(Arc::clone(&view_roots));
                changes.extend(partial_rename::render_edits(&project, &renames, &sources, &extra_engines));
            }
            changes
        })
        .await?;
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
        if self.is_liquid(&uri) {
            return Ok(None);
        }
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let path = util::file_path(&uri);
        let at @ (_, line, col) = util::line_at(&doc.text, pos);
        if let Some((kind, prefix)) = partials::render_string_prefix(line, col) {
            let Some(path) = path else { return Ok(None) };
            // The guard would block the next `did_change` while the work below waits.
            drop(doc);
            return Ok(Some(CompletionResponse::Array(self.render_path_completions(&path, kind, &prefix).await?)));
        }
        Ok(self.completions_at(&uri, &doc, path.as_deref(), pos, at).map(CompletionResponse::Array))
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
        templates: Arc::default(),
        locales: Arc::default(),
        cache_generation: Arc::default(),
        local_callers: DashMap::new(),
        warming: Arc::default(),
        view_roots: Arc::default(),
        routes: DashMap::new(),
        options: Arc::default(),
        roots: OnceLock::new(),
        dynamic_formatting: OnceLock::new(),
        publishes: Arc::default(),
        next_publish: AtomicU64::new(0),
    })
    .custom_method(ROUTES_NOTIFICATION, Backend::set_routes)
    .finish();
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_names() {
        assert!(is_local_name("member") && is_local_name("_tmp"));
        for name in ["class", "end", "self", "User", "ok?", "a-b", ""] {
            assert!(!is_local_name(name), "{name}");
        }
    }

    #[test]
    fn view_roots_are_dropped_when_files_come_or_go_or_the_lockfile_changes() {
        let event = |path: &str, kind| vec![(PathBuf::from(path), kind)];
        assert!(may_change_view_roots(&event("/r/engines/blog/app/views/blog/_post.erb", FileChangeType::CREATED)));
        assert!(may_change_view_roots(&event("/r/engines/blog", FileChangeType::DELETED)));
        assert!(may_change_view_roots(&event("/r/Gemfile.lock", FileChangeType::CHANGED)));
        assert!(!may_change_view_roots(&event("/r/app/views/users/_card.erb", FileChangeType::CHANGED)));
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
        assert!(line_edits("a\n\nb\n", "a\nb\n", 0..=2).is_none());
        let trailing = line_edits("%div\n    %p\n\n\n", "%div\n  %p\n", 0..=3).unwrap();
        assert_eq!(trailing, [TextEdit::new(Range::new(Position::new(1, 0), Position::new(1, 6)), "  %p".into())]);
    }

    /// `file line:col new_text` for each edit, sorted.
    fn rename(renames: &[(PathBuf, PathBuf)]) -> Vec<String> {
        let views = rails_paths::views_root(&renames[0].0).unwrap();
        let sources = partial_rename::sources(&views, &HashMap::new());
        let mut got: Vec<String> = partial_rename::render_edits(&Project::unknown(), renames, &sources, &[])
            .into_iter()
            .flat_map(|(uri, edits)| {
                let file = uri.path().as_str().rsplit('/').next().unwrap_or("").to_string();
                edits.into_iter().map(move |e| format!("{file} {}:{} {}", e.range.start.line, e.range.start.character, e.new_text))
            })
            .collect();
        got.sort();
        got
    }

    fn views_tree(name: &str, files: &[(&str, &str)]) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("rt-lsp-rename-{name}-{}", std::process::id()));
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
        // Rails finds a partial directly in app/views as `/card`: the slash drops the controller prefixes.
        assert_eq!(
            rename(&[(card, views.join("_card.html.erb"))]),
            ["box.html.haml 1:12 /card", "index.html.erb 0:12 /card", "index.html.erb 0:33 /card", "show.html.slim 0:10 /card",]
        );
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
    fn helper_hover_links_docs() {
        let link_to = helpers::HELPERS.iter().find(|h| h.name == "link_to").unwrap();
        assert!(helper_hover(link_to)
            .ends_with("[API docs](https://api.rubyonrails.org/classes/ActionView/Helpers/UrlHelper.html#method-i-link_to)"));
        let params = helpers::HELPERS.iter().find(|h| h.name == "params").unwrap();
        assert!(!helper_hover(params).contains("API docs"));
    }

    #[test]
    fn strict_locals_hover() {
        let root = std::env::temp_dir().join(format!("rt-locals-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        let card = "<%# locals: (user:, size: :md) -%>\n<p><%= user.name %></p>\n";
        std::fs::write(users.join("_card.html.erb"), card).unwrap();
        std::fs::write(users.join("_plain.html.erb"), "").unwrap();
        let file = users.join("index.html.erb");

        let line = r#"<%= render partial: "card" %>"#;
        let md = render_hover(
            &Project::unknown(),
            &file,
            &partials::render_ref_at(line, 0, line, TemplateLang::Erb, ruby_prism::Version::Latest, 22).unwrap(),
        );
        assert!(md.ends_with("```ruby\nlocals: (user:, size: :md)\n```"), "{md}");
        assert!(md.contains("- `app/views/users/_card.html.erb`"), "a path from the Rails root: {md}");
        assert!(!render_hover(
            &Project::unknown(),
            &file,
            &partials::render_ref_at(r#"render "plain""#, 0, r#"render "plain""#, TemplateLang::Erb, ruby_prism::Version::Latest, 9)
                .unwrap()
        )
        .contains("locals:"));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
