use crate::checks::{diagnostics, Doc, Options, UNRESOLVED_RENDER};
use crate::code_context::{code_at, in_ruby_context, line_roles, render_filter};
use crate::completion::{
    component_completions, component_prefix, helper_item, helpers_for, i18n_completions, ivar_completions, local_completions,
    receiver_completions, render_completions, slot_setter_completions, stimulus_completions,
};
use crate::lang::{file_name, html_host, TemplateLang};
use crate::{
    assets, config, controllers, erb, erb_complete, format, haml, haml_indent, helper_defs, helpers, i18n, ivars, outline, partial_rename,
    partials, project::Project, rails_paths, routes, ruby, schema, slim_complete, slim_hover, slim_outline, slim_parser, slots, stimulus,
    strict_locals, tags, util,
};
use dashmap::DashMap;
use haml_indent::LineRole;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

type LocaleFiles = Arc<Vec<(PathBuf, Vec<i18n::Translation>)>>;

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
    /// Formatting is registered for Slim/Skim only, when the client supports dynamic registration and `slim.format.enabled` is on:
    /// a static capability would cover ERB/Haml too, with a formatter that does nothing there.
    dynamic_formatting: OnceLock<bool>,
}

const DIAGNOSTICS_DEBOUNCE: Duration = Duration::from_millis(200);

fn render_hover(project: &Project, path: &Path, r: &partials::RenderRef) -> String {
    let found = partials::resolve(project, path, r);
    if found.is_empty() {
        return format!("**{}** `{}`\n\nNo matching template found.", r.kind.name(), r.name);
    }
    let list: Vec<String> = found.iter().map(|f| format!("- `{}`", f.display())).collect();
    let mut md = format!("**{}** `{}`\n\n{}", r.kind.name(), r.name, list.join("\n"));
    if let Some(sig) = partials::file_strict_locals(&found[0], strict_locals::Close::of(project)) {
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

/// tower-lsp polls every handler in one task, so a scan of the whole app here would hold up all other requests.
async fn off_server_task<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.map_err(|_| tower_lsp::jsonrpc::Error::internal_error())
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
fn spacing_actions(uri: &Url, text: &str, range: Range, only: Option<&[CodeActionKind]>) -> Vec<CodeActionOrCommand> {
    let (Some(from), Some(to)) = (util::position_to_offset(text, range.start), util::position_to_offset(text, range.end)) else {
        return vec![];
    };
    let edits = erb::spacing_edits(text);
    let index = util::LineIndex::new(text);
    let to_edit = |(s, e, tag): &(usize, usize, String)| TextEdit::new(Range::new(index.position(*s), index.position(*e)), tag.clone());
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
    line_location(file, 0)
}

fn line_location(file: impl AsRef<Path>, line: usize) -> Option<Location> {
    let start = Position::new(line as u32, 0);
    Some(Location::new(Url::from_file_path(file).ok()?, Range::new(start, start)))
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
    async fn formatted(&self, uri: &Url, options: &FormattingOptions, lines_only: bool) -> Option<(String, String)> {
        let (lang, text) = self.docs.get(uri).map(|doc| (doc.lang, doc.text.clone()))?;
        if !self.options().format_enabled(lang) {
            return None;
        }
        let (uri, options) = (uri.clone(), options.clone());
        // A range may hold a comment's inner lines without the line they move with.
        let align_comments = self.options().align_erb_comments && !lines_only;
        let extra_engines = self.options().extra_engines.clone();
        // Reads .editorconfig and the project from disk, and formats a whole file: off the server task.
        let result = off_server_task(move || {
            let path = uri.to_file_path().ok();
            let mut settings =
                path.map_or_else(format::Settings::default, |path| config::settings_for(&path).settings).with_editor(&options);
            if lines_only {
                settings.final_newline = false;
            }
            settings.align_comments = align_comments;
            let result = format::format(&uri, lang, &text, &settings, &extra_engines);
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

    fn options(&self) -> &Options {
        self.options.get_or_init(Options::default)
    }

    /// Publish the diagnostics of `version` after `delay`, unless a newer change to the document supersedes it.
    /// They are computed on a copy, off the server task: holding the `docs` guard would block the next `did_change`.
    fn publish_later(&self, uri: Url, version: i32, delay: Duration) {
        let (client, docs, options) = (self.client.clone(), Arc::clone(&self.docs), Arc::clone(&self.options));
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let current = |docs: &DashMap<Url, Doc>, uri: &Url| docs.get(uri).filter(|doc| doc.version == version).map(|doc| doc.clone());
            let Some(doc) = current(&docs, &uri) else { return };
            let checked = uri.clone();
            let Ok(diags) = off_server_task(move || diagnostics(&checked, &doc, options.get_or_init(Options::default))).await else {
                return;
            };
            if current(&docs, &uri).is_some() {
                client.publish_diagnostics(uri, diags, Some(version)).await;
            }
        });
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
    async fn variable_definitions(&self, text: &str, path: &Path, line: &str, col: usize) -> Result<Vec<Location>> {
        let word = util::word_at(line, col);
        let before = &line[..line[..col.min(line.len())].trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len()];
        if before.ends_with('@') && !before.ends_with("@@") {
            return Ok(ivars::for_view(&Project::for_template(path), path)
                .into_iter()
                .filter(|a| a.name == word)
                .filter_map(|a| line_location(&a.file, a.line))
                .collect());
        }
        let is_partial = path.file_name().and_then(|f| f.to_str()).is_some_and(|f| f.starts_with('_'));
        // `link_to` or `t` is never a local; the scan below reads every view.
        let builtin = |name: &str| helpers::HELPERS.iter().chain(helpers::HAML_HELPERS).any(|h| h.name == name);
        let Some(name) = helper_defs::name_at(line, col).filter(|name| is_partial && !builtin(name)) else { return Ok(vec![]) };
        let Some(views) = rails_paths::views_root(path) else { return Ok(vec![]) };
        let (open, project, partial, local) = (self.open_texts(), Project::for_template(path), path.to_path_buf(), name.to_string());
        let close = strict_locals::Close::of(&project);
        let extra_engines = self.options().extra_engines.clone();
        let passes = off_server_task(move || {
            let sources = partial_rename::sources(&views, &open);
            partial_rename::local_passes(&project, &partial, &local, &sources, &extra_engines)
        })
        .await?;
        if !passes.is_empty() {
            return Ok(passes);
        }
        let declared = partials::strict_locals(text, close).is_some_and(|sig| sig.locals.iter().any(|(local, _)| local == name));
        let comment = text.lines().position(|l| l.contains("locals:"));
        Ok(comment.filter(|_| declared).and_then(|line| line_location(path, line)).into_iter().collect())
    }

    /// Routes of the Rails app `current` belongs to, empty until the client has sent them.
    fn routes_for(&self, current: &Path) -> Arc<Vec<routes::Route>> {
        let root = rails_paths::app_root(current).and_then(|app| app.parent().map(Path::to_path_buf));
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
        if self.dynamic_formatting.get() == Some(&true) && self.options().format_enabled(TemplateLang::Slim) {
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
        let version = p.text_document.version;
        self.docs.insert(p.text_document.uri.clone(), Doc { lang, text: p.text_document.text, version });
        self.publish_later(p.text_document.uri, version, Duration::ZERO);
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
        self.publish_later(p.text_document.uri, version, DIAGNOSTICS_DEBOUNCE);
    }

    async fn did_change_watched_files(&self, p: DidChangeWatchedFilesParams) {
        let events: Vec<(PathBuf, FileChangeType)> =
            p.changes.into_iter().filter_map(|e| Some((e.uri.to_file_path().ok()?, e.typ))).collect();
        self.templates.retain(|root, _| !stale(root, &events, false));
        self.locales.retain(|root, _| !stale(root, &events, true));
        // Unresolved-render and strict-locals warnings read other templates from disk.
        let open: Vec<(Url, i32)> = self.docs.iter().map(|d| (d.key().clone(), d.version)).collect();
        for (uri, version) in open {
            self.publish_later(uri, version, Duration::ZERO);
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
        let helper = helper_defs::name_at(line, col).map(str::to_string);
        let route = helper.as_deref().and_then(|name| routes::for_helper(&self.routes_for(&path), name)?.0.source.clone());
        let (text, line) = (doc.text.clone(), line.to_string());
        // The guard would block the next `did_change` while the work below waits.
        drop(doc);
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
            locs.extend(self.variable_definitions(&text, &path, &line, col).await?);
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
        let Some((lang, text)) = self.docs.get(&p.text_document.uri).map(|doc| (doc.lang, doc.text.clone())) else { return Ok(None) };
        let (html, extra_engines) = (html_host(&p.text_document.uri), self.options().extra_engines.clone());
        // A provider's ranges replace VS Code's indentation folding; `None` falls back to it. That fallback is
        // what folds the host language of `.js.erb`/`.yml.erb`/… where ERB blocks alone would regress.
        off_server_task(move || match lang {
            TemplateLang::Haml => Some(outline::haml_folding_ranges(&text)),
            TemplateLang::Slim | TemplateLang::Skim => Some(slim_outline::folding_ranges(&text, &extra_engines)),
            TemplateLang::Erb if html => Some(outline::folding_ranges(&text)),
            TemplateLang::Erb | TemplateLang::Ruby => None,
        })
        .await
    }

    async fn document_symbol(&self, p: DocumentSymbolParams) -> Result<Option<DocumentSymbolResponse>> {
        let uri = p.text_document.uri;
        let Some((lang, text)) = self.docs.get(&uri).map(|doc| (doc.lang, doc.text.clone())) else { return Ok(None) };
        let (html, extra_engines) = (html_host(&uri), self.options().extra_engines.clone());
        off_server_task(move || {
            let symbols = match lang {
                TemplateLang::Erb => outline::document_symbols(&text, html),
                TemplateLang::Haml => outline::haml_document_symbols(&text),
                TemplateLang::Slim | TemplateLang::Skim => slim_outline::document_symbols(&text, &extra_engines),
                TemplateLang::Ruby => return None,
            };
            Some(DocumentSymbolResponse::Nested(symbols))
        })
        .await
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
            let shown = target.strip_prefix(rails_paths::views_root(&path)?).ok()?.to_string_lossy().replace('\\', "/");
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
        if let Some((key, app)) = i18n_key.zip(rails_paths::app_root(&path)) {
            let declarations = if p.context.include_declaration { translation_locations(&self.locale_files(&path), &key) } else { vec![] };
            let open = self.open_texts();
            let mut locations = off_server_task(move || {
                let sources = partial_rename::i18n_sources(&app, &open);
                partial_rename::i18n_references(&project, &key, &sources, &extra_engines)
            })
            .await?;
            locations.extend(declarations);
            return Ok(Some(locations));
        }
        let Some(views) = rails_paths::views_root(&path) else { return Ok(None) };
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
        let declarations: Vec<Location> =
            if p.context.include_declaration { files.iter().filter_map(file_location).collect() } else { vec![] };
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
        let Some(doc) = self.docs.get(&uri).filter(|d| d.lang == TemplateLang::Erb && html_host(&uri)) else { return Ok(None) };
        let Some(offset) = util::position_to_offset(&doc.text, p.text_document_position_params.position) else { return Ok(None) };
        let index = util::LineIndex::new(&doc.text);
        let to_range = |(s, e)| Range::new(index.position(s), index.position(e));
        Ok(tags::linked_names(&doc.text, offset)
            .map(|names| LinkedEditingRanges { ranges: names.map(to_range).to_vec(), word_pattern: None }))
    }

    /// Partials and templates by their render name (`users/card`), matched like VS Code's fuzzy filter.
    async fn symbol(&self, p: WorkspaceSymbolParams) -> Result<Option<Vec<SymbolInformation>>> {
        let query = p.query.to_lowercase();
        let open = self.docs.iter().filter_map(|d| rails_paths::views_root(&d.key().to_file_path().ok()?)).collect::<Vec<_>>();
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
        let mut roots: Vec<PathBuf> = renames.iter().filter_map(|(old, _)| rails_paths::views_root(old)).collect();
        roots.sort();
        roots.dedup();
        let open = self.open_texts();
        let extra_engines = self.options().extra_engines.clone();
        let changes = off_server_task(move || {
            let mut changes = HashMap::new();
            for views in roots {
                let sources = partial_rename::sources(&views, &open);
                let project = Project::for_template(&views.join("_"));
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
        let close = path.as_deref().map(|p| strict_locals::Close::of(&Project::for_template(p))).unwrap_or_default();
        let mut found = local_completions(&doc.text, typed, close);
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
                let file = uri.path().rsplit('/').next().unwrap_or("").to_string();
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
        let md = render_hover(&Project::unknown(), &file, &partials::render_ref_at(line, 22).unwrap());
        assert!(md.ends_with("```ruby\nlocals: (user:, size: :md)\n```"), "{md}");
        assert!(!render_hover(&Project::unknown(), &file, &partials::render_ref_at(r#"render "plain""#, 9).unwrap()).contains("locals:"));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
