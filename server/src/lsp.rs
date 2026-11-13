mod complete;
mod fixes;
mod locals;

use crate::checks::{diagnostics_in, Doc, STRICT_LOCALS, UNRESOLVED_RENDER};
use crate::code_context;
use crate::code_context::{block_locals, code_at, line_roles, render_filter};
use crate::completion::helpers_for;
use crate::erb::output_control_flow::OUTPUT_CONTROL_FLOW;
use crate::erb::tags;
use crate::lang::{file_name, is_partial, TemplateLang};
use crate::options::Options;
use crate::rails::controllers::Target;
use crate::rails::output_lints::{output_marker_edit, SILENT_HELPER, SILENT_RENDER};
use crate::rails::project::{Project, Projects};
use crate::rails::render_call::HamlPipes;
use crate::rails::view_lints::{remove_local_edit, UNUSED_BLOCK_ARGUMENT, UNUSED_STRICT_LOCAL};
use crate::rails::{
    assets, controller_classes, controllers, helper_defs, i18n, ivars, model_columns, partials, paths, project, render_refs, routes,
    stimulus, unknown_route, view_roots,
};
use crate::rules::file::{ConfigFile, FILE_NAME};
use crate::rules::resolve::{resolve, Configs, ResolvedRules};
use crate::slim;
use crate::slim::hover;
use crate::util::{file_location, line_location};
use crate::{config, erb, format, haml, helpers, outline, partial_rename, ruby, semantic_tokens, signature_help, util};
use dashmap::mapref::entry::Entry;
use dashmap::{DashMap, DashSet};
use haml::indent::LineRole;
use std::cell::LazyCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

type LocaleFiles = Arc<i18n::Locales>;

const DIAGNOSTICS_DEBOUNCE: Duration = Duration::from_millis(200);

const SERVER_NAME: &str = "ruby-templates-ls";

/// The client's notification with the app's routes.
const ROUTES_NOTIFICATION: &str = "rubyTemplates/routes";

const SLIM_FORMATTING_REGISTRATION: &str = "slim-formatting";

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
        routes: Arc::default(),
        options: Arc::default(),
        resolved: Arc::default(),
        config_reports: Arc::default(),
        roots: OnceLock::new(),
        dynamic_formatting: OnceLock::new(),
        dynamic_watching: OnceLock::new(),
        semantic_refresh: OnceLock::new(),
        file_operations: OnceLock::new(),
        publishes: Arc::default(),
        next_publish: AtomicU64::new(0),
    })
    .custom_method(ROUTES_NOTIFICATION, Backend::set_routes)
    .finish();
    Server::new(stdin, stdout, socket).serve(service).await;
}

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
    routes: Arc<DashMap<PathBuf, Arc<Vec<routes::Route>>>>,
    // Set once in `initialize`; the client restarts the server when these settings change.
    options: Arc<OnceLock<Options>>,
    /// What applied to each open document at its latest diagnostics run, for the handlers that read the Slim engines.
    resolved: Arc<DashMap<PathBuf, Arc<ResolvedRules>>>,
    config_reports: Arc<ConfigReports>,
    /// Workspace folders from `initialize`, for workspace symbols before any view is open.
    roots: OnceLock<Vec<PathBuf>>,
    /// Formatting is registered for Slim/Skim only, when the client supports dynamic registration:
    /// a static capability would cover ERB/Haml too, with a formatter that does nothing there.
    dynamic_formatting: OnceLock<bool>,
    /// The client takes `workspace/didChangeWatchedFiles` registrations, for config files outside the workspace folders.
    dynamic_watching: OnceLock<Option<Watching>>,
    /// The client takes `workspace/semanticTokens/refresh`.
    semantic_refresh: OnceLock<bool>,
    /// The client applies workspace edits that create files and edit them in one go.
    file_operations: OnceLock<bool>,
    /// How many `render` calls pass each (partial, local), for hover; cleared when any view's text may have changed.
    local_callers: DashMap<(PathBuf, String), usize>,
    /// The latest diagnostics run per open document: runs finish in any order, and only the latest may publish.
    publishes: Arc<DashMap<Uri, u64>>,
    next_publish: AtomicU64,
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
        let workspace = params.capabilities.workspace.unwrap_or_default();
        let watching = workspace.did_change_watched_files.unwrap_or_default();
        let roots = self.roots.get().cloned().unwrap_or_default();
        let relative = watching.relative_pattern_support.unwrap_or(false);
        let _ = self.dynamic_watching.set((watching.dynamic_registration == Some(true)).then_some(Watching { roots, relative }));
        let _ = self.semantic_refresh.set(workspace.semantic_tokens.and_then(|t| t.refresh_support).unwrap_or(false));
        let workspace_edit = workspace.workspace_edit.unwrap_or_default();
        let creates = workspace_edit.resource_operations.is_some_and(|operations| operations.contains(&ResourceOperationKind::Create));
        let _ = self.file_operations.set(creates && workspace_edit.document_changes == Some(true));
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
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(["(", ",", " "].map(String::from).to_vec()),
                    retrigger_characters: Some(vec![",".into()]),
                    work_done_progress_options: Default::default(),
                }),
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
        if self.dynamic_formatting.get() == Some(&true) {
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
        for (path, _) in events.iter().filter(|(path, _)| path.ends_with(FILE_NAME)) {
            self.config_reports.forget(&self.client, path).await;
        }
        if may_change_view_roots(&events) {
            self.view_roots.clear();
        }
        if may_change_controllers(&events) {
            stimulus::clear_cache();
        }
        if may_change_config(&events) {
            i18n::clear_catalogs();
        }
        if may_change_helpers(&events) {
            unknown_route::clear_cache();
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
        if let Some(path) = util::file_path(&p.text_document.uri) {
            self.resolved.remove(&path);
        }
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
            return Ok(locations_response(found.into_iter().filter_map(file_location).collect()));
        }
        let Some(doc) = self.template_doc(&uri) else { return Ok(None) };
        let (line_start, line, col) = util::line_at(&doc.text, pos);
        let engines = &self.engines(&uri);
        if let Some(key) = i18n_key_at(&project, &doc, &path, pos, engines) {
            return Ok(locations_response(translation_locations(&self.locale_files(&path), &key)));
        }
        let targets: Vec<PathBuf> =
            if let Some(r) = render_refs::render_ref_at(&doc.text, line_start, line, doc.lang, project.prism_version(), col) {
                partials::resolve(&project, &path, &r)
            } else if let Some(c) = render_refs::component_ref_at(line, col) {
                partials::resolve_component(&project, &path, &c)
            } else {
                convention_targets(&project, &path, &doc.text, line_start, line, Some(col))
                    .into_iter()
                    .next()
                    .map(|target| target.files)
                    .unwrap_or_default()
            };
        let mut locs: Vec<Location> = targets.into_iter().filter_map(file_location).collect();
        if !locs.is_empty() || !code_at(doc.lang, &doc.text, line_start + col, engines) {
            return Ok(locations_response(locs));
        }
        let declaration = |at| code_context::local_definition(doc.lang, &doc.text, at, engines).filter(|d| d.parameter).map(|d| d.start);
        if let Some(found) = model_columns::column_at(&project, &path, &doc.text, line_start, col, declaration) {
            return Ok(line_location(&found.schema, found.column.line).map(GotoDefinitionResponse::Scalar));
        }
        // Cheapest first: Ctrl+hover asks for a definition on every word the mouse crosses.
        let helper = helper_defs::name_at(line, col).map(str::to_string);
        let route = helper.as_deref().and_then(|name| routes::for_helper(&self.routes_for(&path), name)?.0.source.clone());
        // A block variable or a local the template assigns hides a helper of the same name.
        if let Some(name) = helper.as_deref().filter(|name| !is_builtin_helper(name)) {
            if let Some(range) = self.local_definition_range(&uri, &project, &doc, line_start + col, name).await? {
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
            locs.extend(self.variable_definitions(&doc.text, &path, line, col).await?);
        }
        Ok(locations_response(locs))
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
        let in_code = render_filter(doc.lang, &doc.text, &self.engines(&uri));
        let pipes = HamlPipes::new(doc.lang);
        let mut links = Vec::new();
        for (i, (line_start, line)) in util::lines_with_offsets(&doc.text).enumerate() {
            // Same renders as diagnostics and references: code only, and the file Rails would pick.
            let renders =
                render_refs::render_refs_in(&doc.text, line_start, line, &pipes, project.prism_version(), |r| in_code(i, line_start, r))
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
        let Some(doc) = self.template_doc(&uri) else { return Ok(None) };
        let engines = &self.engines(&uri);
        if doc.lang == TemplateLang::Slim {
            let parsed = slim::parser::parse(&doc.text, engines);
            let marker = util::position_to_offset(&doc.text, pos).and_then(|o| hover::marker_doc(&doc.text, o, &parsed));
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
            if let Some(r) = render_refs::render_ref_at(&doc.text, line_start, line, doc.lang, project.prism_version(), col) {
                return Ok(markdown_hover(render_hover(project, path, &r)));
            }
            if let Some(md) = i18n_key_at(project, &doc, path, pos, engines).and_then(|key| i18n::hover(&self.locale_files(path), &key)) {
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
        let code_here = role == LineRole::Ruby || code_at(doc.lang, &doc.text, line_start + col, engines);
        // `@tag` is a variable, though `tag` is a helper.
        if let Some(name) = ivars::name_at(line, col).filter(|_| code_here) {
            let assignments = file.as_ref().map(|(project, path)| ivars::for_view(project, path)).unwrap_or_default();
            let view = file.as_ref().map(|(_, path)| path.as_path()).unwrap_or(Path::new(""));
            return Ok(ivars::hover(&assignments, name, view).and_then(markdown_hover));
        }
        if let Some((project, path)) = file.as_ref().filter(|_| code_here) {
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
            (!block_locals(doc.lang, &doc.text, line_start + col, engines).contains(&local.0)).then_some(local)
        });
        if let (Some((name, default)), Some((project, path))) = (declared_local, &file) {
            let passes = self.local_caller_count(project.clone(), path, &name).await?;
            return Ok(markdown_hover(locals::local_hover(&name, default.as_deref(), passes)));
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
        let (name, lang, text, offset, engines) = (name.to_string(), doc.lang, doc.text, line_start + col, engines.clone());
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

    async fn signature_help(&self, p: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = p.text_document_position_params.text_document.uri;
        let pos = p.text_document_position_params.position;
        let Some(doc) = self.template_doc(&uri) else { return Ok(None) };
        let Some(offset) = util::position_to_offset(&doc.text, pos) else { return Ok(None) };
        let role = line_roles(doc.lang, &doc.text).get(pos.line as usize).copied().unwrap_or(LineRole::Start);
        let Some(start) = code_context::code_start(doc.lang, &doc.text, offset, role, &self.engines(&uri)) else { return Ok(None) };
        let calls = signature_help::calls_before(&doc.text[start..offset]);
        if calls.is_empty() {
            return Ok(None);
        }
        let (lang, path) = (doc.lang, util::file_path(&uri));
        off_server_task(move || {
            calls.iter().find_map(|call| {
                let (label, docs) = known_signature(lang, path.as_deref(), &call.name)?;
                signature_help::signature_help(&label, docs, call)
            })
        })
        .await
    }

    async fn folding_range(&self, p: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let Some((lang, html, text)) =
            self.docs.get(&p.text_document.uri).map(|doc| (doc.lang, doc.html_host(&p.text_document.uri), doc.text.clone()))
        else {
            return Ok(None);
        };
        let extra_engines = self.engines(&p.text_document.uri);
        // A provider's ranges replace VS Code's indentation folding; `None` falls back to it. That fallback is
        // what folds the host language of `.js.erb`/`.yml.erb`/… where ERB blocks alone would regress.
        off_server_task(move || match lang {
            TemplateLang::Haml => Some(outline::haml::folding_ranges(&text)),
            TemplateLang::Slim | TemplateLang::Skim => Some(outline::slim::folding_ranges(&text, &extra_engines)),
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
        let extra_engines = self.engines(&uri);
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
        let extra_engines = self.engines(&uri);
        off_server_task(move || {
            let symbols = match lang {
                TemplateLang::Erb => outline::document_symbols(&text, html),
                TemplateLang::Haml => outline::haml::document_symbols(&text),
                TemplateLang::Slim | TemplateLang::Skim => outline::slim::document_symbols(&text, &extra_engines),
                TemplateLang::Ruby | TemplateLang::Liquid => return None,
            };
            Some(DocumentSymbolResponse::Nested(symbols))
        })
        .await
    }

    async fn code_action(&self, p: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = p.text_document.uri;
        let Some(doc) = self.template_doc(&uri) else { return Ok(None) };
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
        let diagnostics = p.context.diagnostics;
        let unresolved = diagnostics.iter().filter(|d| util::code(d) == Some(UNRESOLVED_RENDER)).cloned();
        let pipes = HamlPipes::new(doc.lang);
        let quick_fixes = unresolved.filter_map(|d| {
            let (line_start, line, start) = util::line_at(&doc.text, d.range.start);
            let r =
                render_refs::render_refs_in(&doc.text, line_start, line, &pipes, project.prism_version(), |r| r.start == start).pop()?;
            let target = partials::new_template_path(&project, &path, &r)?;
            let create = CreateFile {
                uri: crate::util::file_uri(&target)?,
                options: Some(CreateFileOptions { overwrite: Some(false), ignore_if_exists: Some(true) }),
                annotation_id: None,
            };
            let shown = util::slash_path(target.strip_prefix(paths::views_root(&path)?).ok()?);
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
        actions.extend(silent_output_fixes(&uri, &doc, &diagnostics));
        actions.extend(output_control_flow_fixes(&uri, &diagnostics));
        actions.extend(unused_variable_fixes(&uri, &doc, &project, &path, &diagnostics));
        actions.extend(fixes::document_fixes(&uri, &doc, &diagnostics));
        actions.extend(self.file_fixes(&path, &diagnostics).await?);
        Ok(Some(actions))
    }

    async fn prepare_rename(&self, p: TextDocumentPositionParams) -> Result<Option<PrepareRenameResponse>> {
        let Some(doc) = self.template_doc(&p.text_document.uri) else { return Ok(None) };
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
        let Some(doc) = self.template_doc(&uri) else { return Ok(None) };
        let Some(local) = self.strict_local_at(&uri, &doc, p.text_document_position.position) else { return Ok(None) };
        let Some(path) = util::file_path(&uri) else { return Ok(None) };
        let project = self.project_for(&path);
        if project.is_gem_file(&path) {
            return Ok(None);
        }
        let engines = self.engines_lookup();
        if !self.free_local_name(&uri, &doc, &project, &new) {
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
        let Some(views) = paths::views_root(&path) else { return Ok(None) };
        let (open, partial, old, renamed) = (self.open_texts(), path.clone(), name.clone(), new.clone());
        let (callers, unread) = off_server_task(move || {
            let sources = partial_rename::sources(&views, &open, engines);
            partial_rename::local_key_edits(&project, &partial, &old, &renamed, &sources)
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
        let extra_engines = self.engines(&uri);
        let i18n_key = {
            let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
            i18n_key_at(&project, &doc, &path, pos, &extra_engines)
        };
        let declaration = p.context.include_declaration;
        if let Some((key, app)) = i18n_key.zip(paths::app_root(&path)) {
            let declarations = if declaration { translation_locations(&self.locale_files(&path), &key) } else { vec![] };
            let (open, engines) = (self.open_texts(), self.engines_lookup());
            let mut locations = off_server_task(move || {
                let sources = partial_rename::i18n_sources(&app, &open, engines);
                partial_rename::i18n_references(&project, &key, &sources)
            })
            .await?;
            locations.extend(declarations);
            return Ok(Some(locations));
        }
        let Some(doc) = self.template_doc(&uri) else { return Ok(None) };
        if let Some(local) = self.strict_local_at(&uri, &doc, pos) {
            return Ok(Some(self.local_references(&path, &doc.text, &local, declaration).await?));
        }
        let Some(views) = paths::views_root(&path) else { return Ok(None) };
        let Some(files) = referenced_partials(&project, &path, &doc, pos) else { return Ok(None) };
        let declarations: Vec<Location> = if declaration { files.iter().filter_map(file_location).collect() } else { vec![] };
        let (open, engines) = (self.open_texts(), self.engines_lookup());
        let mut locations = off_server_task(move || {
            let sources = partial_rename::sources(&views, &open, engines);
            partial_rename::references(&project, &files, &sources)
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
        let open = self.docs.iter().filter_map(|d| paths::views_root(&util::file_path(d.key())?)).collect::<Vec<_>>();
        let mut roots: Vec<PathBuf> =
            self.roots.get().into_iter().flatten().map(|r| r.join(paths::APP_VIEWS)).filter(|v| v.is_dir()).chain(open).collect();
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
        let mut roots: Vec<PathBuf> = renames.iter().filter_map(|(old, _)| paths::views_root(old)).collect();
        roots.sort();
        roots.dedup();
        let open = self.open_texts();
        let mut engines = self.engines_lookup();
        let view_roots = Arc::clone(&self.view_roots);
        let changes = off_server_task(move || {
            let mut changes = HashMap::new();
            for views in roots {
                let sources = partial_rename::sources(&views, &open, &mut engines);
                let project = Project::for_views(&views).with_view_cache(Arc::clone(&view_roots));
                changes.extend(partial_rename::render_edits(&project, &renames, &sources));
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
        if let Some((kind, prefix)) = render_refs::render_string_prefix(line, col) {
            let Some(path) = path else { return Ok(None) };
            // The guard would block the next `did_change` while the work below waits.
            drop(doc);
            return Ok(Some(CompletionResponse::Array(self.render_path_completions(&path, kind, &prefix).await?)));
        }
        Ok(self.completions_at(&uri, &doc, path.as_deref(), pos, at).map(CompletionResponse::Array))
    }
}

impl Backend {
    /// The document's text and the text formatted with the editor's `options`; `None` (logged) when it is left as it is,
    /// and, unlogged, when formatting is switched off for its language.
    /// `lines_only` leaves out the steps that add or remove lines, which a range can't hold.
    async fn formatted(&self, uri: &Uri, options: &FormattingOptions, lines_only: bool) -> Option<(String, String)> {
        let (lang, plain_erb, text) =
            self.docs.get(uri).filter(|doc| !doc.percent_lines).map(|doc| (doc.lang, doc.plain_erb, doc.text.clone()))?;
        let (uri, options) = (uri.clone(), options.clone());
        let editor = Arc::clone(&self.options);
        let started = self.config_reports.generation();
        // Reads the config files, .editorconfig and the project from disk, and formats a whole file: off the server task.
        let (rules, result) = off_server_task(move || {
            let rules = resolve(&uri, lang, editor.get_or_init(Options::default), &mut Configs::new());
            if rules.excluded_by().is_some() || !rules.format_enabled() {
                return (Arc::new(rules), None);
            }
            let base = util::file_path(&uri).map_or_else(format::Settings::default, |path| config::settings_for(&path).settings);
            let mut settings = rules.format_settings(base.with_editor(&options));
            if lines_only {
                // A range can't add or remove a line, and may hold a comment's inner lines without the line they move with.
                settings.trailing_empty_lines = false;
                settings.max_empty_lines = None;
                settings.align_comments = false;
                settings.around_block_body = None;
                settings.around_element_body = None;
                settings.around_branches = None;
                settings.content_on_own_line = None;
                settings.attributes_per_line = None;
            }
            let result = format::format(&uri, lang, plain_erb, &text, &settings, rules.extra_engines());
            (Arc::new(rules), Some((uri, text, result)))
        })
        .await
        .ok()?;
        self.config_reports.report(&self.client, rules.files(), started, self.dynamic_watching.get().and_then(Option::as_ref)).await;
        let result = result?;
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

    /// A copy of the open template at `uri`, so no `docs` guard is held across an `.await` (it would block the next
    /// `did_change`) or while other documents are read; none for Liquid.
    fn template_doc(&self, uri: &Uri) -> Option<Doc> {
        self.docs.get(uri).filter(|doc| doc.lang != TemplateLang::Liquid).map(|doc| doc.clone())
    }

    /// The Slim engines that apply to `uri`.
    fn engines(&self, uri: &Uri) -> Vec<String> {
        util::file_path(uri).map_or_else(|| self.options().extra_engines.clone(), |path| self.engines_at(&path))
    }

    /// The Slim engines of the file at `path`: its latest resolved rules', else the editor's.
    fn engines_at(&self, path: &Path) -> Vec<String> {
        if let Some(rules) = self.resolved.get(path) {
            return rules.extra_engines().to_vec();
        }
        // Before the first diagnostics run (a document just opened).
        engines_from_config(path, self.options(), &mut Configs::new())
    }

    /// A lookup of each file's Slim engines for work that reads many files off the server task, reading each config folder once.
    fn engines_lookup(&self) -> impl FnMut(&Path) -> Vec<String> + Send + 'static {
        let (options, mut configs) = (Arc::clone(&self.options), Configs::new());
        move |path| engines_from_config(path, options.get_or_init(Options::default), &mut configs)
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
        let (resolved, config_reports, app_routes) =
            (Arc::clone(&self.resolved), Arc::clone(&self.config_reports), Arc::clone(&self.routes));
        let (watching, refresh) = (self.dynamic_watching.get().cloned().flatten(), self.semantic_refresh.get() == Some(&true));
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let latest = || publishes.get(&uri).is_some_and(|latest| *latest == run);
            let Some(doc) = docs.get(&uri).filter(|_| latest()).map(|doc| doc.clone()) else { return };
            let (checked, version) = (uri.clone(), doc.version);
            let started = config_reports.generation();
            let Ok((diags, rules)) = off_server_task(move || {
                let mut projects = Projects::new();
                if let Some(path) = util::file_path(&checked) {
                    Project::memoized(&mut projects, &path, Some(&view_roots));
                }
                let rules = resolve(&checked, doc.lang, options.get_or_init(Options::default), &mut Configs::new());
                let routes = util::file_path(&checked).map(|path| routes_of(&app_routes, &path)).unwrap_or_default();
                (diagnostics_in(&checked, &doc, &rules, &mut projects, &routes), rules)
            })
            .await
            else {
                return;
            };
            config_reports.report(&client, rules.files(), started, watching.as_ref()).await;
            if latest() {
                let engines = rules.extra_engines().to_vec();
                let before = util::file_path(&uri).and_then(|path| resolved.insert(path, Arc::new(rules)));
                // Tokens computed from the engines of an earlier run are stale.
                if refresh && before.is_some_and(|before| before.extra_engines() != engines) {
                    let _ = client.semantic_tokens_refresh().await;
                }
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
        let (templates, generation) = (Arc::clone(&self.templates), Arc::clone(&self.cache_generation));
        off_server_task(move || views_listing(&templates, &views, &generation)).await
    }

    fn template_files(&self, current: &Path) -> Arc<Vec<PathBuf>> {
        let Some(views) = paths::views_root(current) else {
            return Arc::new(current.parent().map(|dir| paths::files_under(dir, 1)).unwrap_or_default());
        };
        self.views_files(&views)
    }

    /// Every file under an app/views root, cached until a watched-file event says otherwise.
    fn views_files(&self, views: &Path) -> Arc<Vec<PathBuf>> {
        views_listing(&self.templates, views, &self.cache_generation)
    }

    /// Fills the app/views and locale caches of `uri`'s app in the background, so the first render or i18n request
    /// after a view opens or files change finds them ready instead of walking the app on the server task.
    fn warm_caches(&self, uri: &Uri) {
        let Some(path) = util::file_path(uri) else { return };
        let claim = |root: &PathBuf, cached: bool| !cached && self.warming.insert(root.clone());
        let views = paths::views_root(&path).filter(|views| claim(views, self.templates.contains_key(views)));
        let locales = i18n::locales_dir(&path).filter(|dir| claim(dir, self.locales.contains_key(dir)));
        if views.is_none() && locales.is_none() {
            return;
        }
        let (templates, cached_locales, generation, warming) =
            (Arc::clone(&self.templates), Arc::clone(&self.locales), Arc::clone(&self.cache_generation), Arc::clone(&self.warming));
        tokio::task::spawn_blocking(move || {
            if let Some(views) = views {
                views_listing(&templates, &views, &generation);
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
        // `Rails/UnknownRoute` reads them.
        let open: Vec<Uri> = self.docs.iter().map(|d| d.key().clone()).collect();
        for uri in open {
            self.publish_later(uri, Duration::ZERO);
        }
    }

    /// Routes of the Rails app `current` belongs to, empty until the client has sent them.
    fn routes_for(&self, current: &Path) -> Arc<Vec<routes::Route>> {
        routes_of(&self.routes, current)
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

/// tower-lsp polls every handler in one task, so a scan of the whole app here would hold up all other requests.
async fn off_server_task<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.map_err(|_| tower_lsp_server::jsonrpc::Error::internal_error())
}

/// The entry under `root`, from `read()` unless it is filled meanwhile; reads again when a watched-file event came in
/// while it read, since what it read may predate the change. The generation is checked under the entry's lock,
/// which the event's invalidation (`retain`, after the bump) takes too.
fn fill<T: Clone>(cache: &DashMap<PathBuf, T>, root: &Path, generation: &AtomicU64, read: impl Fn() -> T) -> T {
    loop {
        let started = generation.load(Ordering::SeqCst);
        let value = read();
        match cache.entry(root.to_path_buf()) {
            Entry::Occupied(entry) => return entry.get().clone(),
            Entry::Vacant(slot) if generation.load(Ordering::SeqCst) == started => return slot.insert(value).clone(),
            Entry::Vacant(_) => {}
        }
    }
}

/// Every file under the app/views root `views`, from `templates` or read into it.
fn views_listing(templates: &DashMap<PathBuf, Arc<Vec<PathBuf>>>, views: &Path, generation: &AtomicU64) -> Arc<Vec<PathBuf>> {
    if let Some(files) = templates.get(views) {
        return Arc::clone(&files);
    }
    fill(templates, views, generation, || Arc::new(paths::files_under(views, usize::MAX)))
}

fn rename_filter(glob: &str, matches: FileOperationPatternKind) -> FileOperationFilter {
    FileOperationFilter {
        scheme: Some("file".into()),
        pattern: FileOperationPattern { glob: glob.into(), matches: Some(matches), options: None },
    }
}

/// The Slim engines of the file at `path` from its nearest config file, else the editor's.
fn engines_from_config(path: &Path, editor: &Options, configs: &mut Configs) -> Vec<String> {
    let resolved = TemplateLang::from_file_name(&file_name_of(path)).zip(util::file_uri(path));
    resolved.map_or_else(|| editor.extra_engines.clone(), |(lang, uri)| resolve(&uri, lang, editor, configs).extra_engines().to_vec())
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

/// How to watch a config file the client's own watchers don't cover.
#[derive(Clone)]
struct Watching {
    /// The workspace folders, which the client watches already.
    roots: Vec<PathBuf>,
    /// The client takes `{ baseUri, pattern }` globs.
    relative: bool,
}

/// Config files' problems already reported, so each is logged and published once until its file changes.
#[derive(Default)]
struct ConfigReports {
    /// Bumped when a config file changes: a run that started earlier may have read the old text, so it reports nothing.
    generation: AtomicU64,
    logged: DashSet<PathBuf>,
    /// The files with problems published on them.
    flagged: DashSet<Uri>,
    /// The config files outside the workspace folders that a watcher was registered for.
    watched: DashSet<PathBuf>,
    /// Held while a report or a forget runs, so one can't land between the other's check and its publish.
    busy: tokio::sync::Mutex<()>,
}

impl ConfigReports {
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Reports the problems of `files` read by a run that began at `started`; with `watching`, also watches the files
    /// that are outside those folders, which the client's own watchers don't cover.
    async fn report(&self, client: &Client, files: &[Arc<ConfigFile>], started: u64, watching: Option<&Watching>) {
        let _busy = self.busy.lock().await;
        if started != self.generation() {
            return;
        }
        for file in files {
            if let Some(watching) = watching.filter(|w| !w.roots.iter().any(|root| file.path.starts_with(root))) {
                self.watch(client, &file.path, watching.relative).await;
            }
            if file.problems.is_empty() || !self.logged.insert(file.path.clone()) {
                continue;
            }
            let shown = file.path.display();
            let mut diagnostics = Vec::new();
            for problem in &file.problems {
                let (line, col) = (problem.range.start.line + 1, problem.range.start.character + 1);
                client.log_message(MessageType::WARNING, format!("{shown}:{line}:{col}: {}", problem.message)).await;
                diagnostics.push(Diagnostic { source: Some(SERVER_NAME.into()), ..util::warning(problem.range, problem.message.clone()) });
            }
            let Some(uri) = util::file_uri(&file.path) else { continue };
            client.publish_diagnostics(uri.clone(), diagnostics, None).await;
            self.flagged.insert(uri);
        }
    }

    async fn watch(&self, client: &Client, path: &Path, relative: bool) {
        if !self.watched.insert(path.to_path_buf()) {
            return;
        }
        let pattern = match path.parent().and_then(util::file_uri) {
            Some(base) if relative => serde_json::json!({ "baseUri": base, "pattern": FILE_NAME }),
            _ => serde_json::json!(util::slash_path(path)),
        };
        let registration = Registration {
            id: format!("watch-{}", path.display()),
            method: "workspace/didChangeWatchedFiles".into(),
            register_options: Some(serde_json::json!({ "watchers": [{ "globPattern": pattern }] })),
        };
        if let Err(e) = client.register_capability(vec![registration]).await {
            self.watched.remove(path);
            client.log_message(MessageType::WARNING, format!("{} not watched: {e}", path.display())).await;
        }
    }

    /// The file at `path` changed on disk: report it again if it still has problems, and clear what was published.
    async fn forget(&self, client: &Client, path: &Path) {
        let _busy = self.busy.lock().await;
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.logged.remove(path);
        let Some(uri) = util::file_uri(path) else { return };
        if self.flagged.remove(&uri).is_some() {
            client.publish_diagnostics(uri, vec![], None).await;
        }
    }
}

/// Whether the watched-file `events` make the cache for `root` stale. Listings change only when files are
/// created or deleted; `contents` caches also on edits. Deleting an ancestor of `root` counts too.
fn stale(root: &Path, events: &[(PathBuf, FileChangeType)], contents: bool) -> bool {
    events.iter().any(|(path, kind)| (contents || *kind != FileChangeType::CHANGED) && (path.starts_with(root) || root.starts_with(path)))
}

/// A created or deleted file may add or remove a view root; a changed one only when it is the lockfile that names the gems.
fn may_change_view_roots(events: &[(PathBuf, FileChangeType)]) -> bool {
    events.iter().any(|(path, kind)| !path.ends_with(FILE_NAME) && (*kind != FileChangeType::CHANGED || path.ends_with(project::LOCKFILE)))
}

/// A file under app/javascript or the importmap may add, rename or register a Stimulus controller; so may the deletion of an `app`.
fn may_change_controllers(events: &[(PathBuf, FileChangeType)]) -> bool {
    let in_javascript = |path: &Path| path.components().any(|part| part.as_os_str() == "javascript");
    events.iter().any(|(path, kind)| {
        path.ends_with("config/importmap.rb") || in_javascript(path) || (*kind != FileChangeType::CHANGED && path.ends_with("app"))
    })
}

/// A file under a `config` folder may add a translation or change where translations come from and the default locale.
fn may_change_config(events: &[(PathBuf, FileChangeType)]) -> bool {
    events.iter().any(|(path, _)| path.components().any(|part| part.as_os_str() == "config"))
}

/// A file in the app's helpers or controllers, or its config (the route files), may define a helper a view calls.
fn may_change_helpers(events: &[(PathBuf, FileChangeType)]) -> bool {
    let relevant = |part: &std::ffi::OsStr| part == "helpers" || part == "controllers" || part == "config";
    events.iter().any(|(path, _)| path.components().any(|part| relevant(part.as_os_str())))
}

/// The routes the client sent for the Rails app `current` belongs to; none until it has.
fn routes_of(routes: &DashMap<PathBuf, Arc<Vec<routes::Route>>>, current: &Path) -> Arc<Vec<routes::Route>> {
    let root = paths::rails_root(current);
    root.and_then(|root| routes.get(&root).map(|r| Arc::clone(&r))).unwrap_or_default()
}

/// Byte ranges on `line` that lead to files by convention: `render @model`, Stimulus identifiers, asset paths.
/// With `col`, only the ones under it are resolved.
fn convention_targets(project: &Project, path: &Path, text: &str, line_start: usize, line: &str, col: Option<usize>) -> Vec<Target> {
    let at = |s: usize, e: usize| col.is_none_or(|c| (s..=e).contains(&c));
    let models = render_refs::model_renders(line).into_iter().filter(|&(s, e, _)| at(s, e)).map(|(start, end, name)| Target {
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

/// Full key of the `t("…")` literal under `col`, when the call is Ruby.
fn i18n_key_at(project: &Project, doc: &Doc, path: &Path, pos: Position, extra_engines: &[String]) -> Option<String> {
    let (line_start, line, col) = util::line_at(&doc.text, pos);
    let (start, end) = i18n::key_at(line, col)?;
    if !code_at(doc.lang, &doc.text, line_start + start, extra_engines) {
        return None;
    }
    i18n::full_key(project, path, &line[start..end])
}

/// The line of `key` in each locale file that defines it.
fn translation_locations(files: &i18n::Locales, key: &str) -> Vec<Location> {
    i18n::lookup(files, key).filter_map(|(file, t)| line_location(file, t.line)).collect()
}

fn locations_response(locations: Vec<Location>) -> Option<GotoDefinitionResponse> {
    (!locations.is_empty()).then_some(GotoDefinitionResponse::Array(locations))
}

/// A link to `target` from the file `from`, its tooltip the path from `from`'s Rails root.
fn file_link(range: Range, target: &Path, from: &Path) -> Option<DocumentLink> {
    Some(DocumentLink {
        range,
        target: Some(crate::util::file_uri(target)?),
        tooltip: Some(paths::rails_relative(target, from)),
        data: None,
    })
}

fn render_hover(project: &Project, path: &Path, r: &render_refs::RenderRef) -> String {
    let found = partials::resolve(project, path, r);
    if found.is_empty() {
        return format!("**{}** `{}`\n\nNo matching template found.", r.kind.name(), r.name);
    }
    let list: Vec<String> = found.iter().map(|f| format!("- `{}`", paths::rails_relative(f, path))).collect();
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

fn markdown_hover(value: String) -> Option<Hover> {
    Some(Hover { contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value }), range: None })
}

fn helper_hover(h: &helpers::Helper) -> String {
    let signature = format!("```ruby\n{}\n```", h.detail);
    match helper_docs(h) {
        Some(docs) => format!("{signature}\n\n{docs}"),
        None => signature,
    }
}

fn helper_docs(h: &helpers::Helper) -> Option<String> {
    Some(format!("`{}`\n\n[API docs]({})", h.module?, h.doc_url()?))
}

/// The signature label and documentation of the helper `name`: a Rails helper, else one the app defines (found from `path`).
fn known_signature(lang: TemplateLang, path: Option<&Path>, name: &str) -> Option<(String, Option<String>)> {
    if let Some(h) = helpers_for(lang).find(|h| h.name == name) {
        return Some((h.detail.to_string(), helper_docs(h)));
    }
    let (signature, markdown) = helper_defs::describe_first(path?, name)?;
    Some((signature, Some(markdown)))
}

/// "Output it" for each silent render or helper, which turns its tag into an output tag.
fn silent_output_fixes(uri: &Uri, doc: &Doc, diagnostics: &[Diagnostic]) -> Vec<CodeActionOrCommand> {
    let silent = |d: &&Diagnostic| matches!(util::code(d), Some(SILENT_RENDER | SILENT_HELPER));
    diagnostics
        .iter()
        .filter(silent)
        .filter_map(|d| {
            let edit = output_marker_edit(doc.lang, &doc.text, d.range)?;
            Some(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Output the result".into(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![d.clone()]),
                edit: Some(WorkspaceEdit { changes: Some(HashMap::from([(uri.clone(), vec![edit])])), ..Default::default() }),
                is_preferred: Some(true),
                ..Default::default()
            }))
        })
        .collect()
}

/// "Use a code tag" for each control-flow keyword in an output tag, which removes the `=`.
fn output_control_flow_fixes(uri: &Uri, diagnostics: &[Diagnostic]) -> Vec<CodeActionOrCommand> {
    diagnostics
        .iter()
        .filter(|d| util::code(d) == Some(OUTPUT_CONTROL_FLOW))
        .map(|d| {
            let edit = TextEdit::new(d.range, String::new());
            CodeActionOrCommand::CodeAction(CodeAction {
                title: "Use a code tag `<%`".into(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![d.clone()]),
                edit: Some(WorkspaceEdit { changes: Some(HashMap::from([(uri.clone(), vec![edit])])), ..Default::default() }),
                is_preferred: Some(true),
                ..Default::default()
            })
        })
        .collect()
}

/// "Remove the local" for an unused strict local and "Prefix with `_`" for an unused block argument.
fn unused_variable_fixes(uri: &Uri, doc: &Doc, project: &Project, path: &Path, diagnostics: &[Diagnostic]) -> Vec<CodeActionOrCommand> {
    diagnostics
        .iter()
        .filter_map(|d| {
            let (title, edit) = match util::code(d)? {
                UNUSED_BLOCK_ARGUMENT => {
                    ("Prefix the argument with `_`", TextEdit::new(Range::new(d.range.start, d.range.start), "_".into()))
                }
                UNUSED_STRICT_LOCAL => {
                    let ruby = project.ruby_context(path);
                    ("Remove the local", remove_local_edit(&doc.text, d.range, ruby.strict_locals?.close, ruby.grammar)?)
                }
                _ => return None,
            };
            Some(CodeActionOrCommand::CodeAction(CodeAction {
                title: title.into(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![d.clone()]),
                edit: Some(WorkspaceEdit { changes: Some(HashMap::from([(uri.clone(), vec![edit])])), ..Default::default() }),
                is_preferred: Some(true),
                ..Default::default()
            }))
        })
        .collect()
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
    let to_edit = |(range, tag): &erb::Edit| TextEdit::new(index.range(range.start, range.end), tag.clone());
    let action = |title: &str, kind: CodeActionKind, edits: Vec<TextEdit>| {
        CodeActionOrCommand::CodeAction(CodeAction {
            title: title.into(),
            kind: Some(kind),
            edit: Some(WorkspaceEdit { changes: Some(HashMap::from([(uri.clone(), edits)])), ..Default::default() }),
            ..Default::default()
        })
    };
    let touching: Vec<TextEdit> = edits.iter().filter(|(range, _)| range.start <= to && from <= range.end).map(to_edit).collect();
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

/// The partials whose callers references at `pos` of the template at `path` look for: the ones the `render` under it names,
/// else the partial itself; `None` on a component render or in a template that isn't a partial.
fn referenced_partials(project: &Project, path: &Path, doc: &Doc, pos: Position) -> Option<Vec<PathBuf>> {
    let (line_start, line, col) = util::line_at(&doc.text, pos);
    if let Some(r) = render_refs::render_ref_at(&doc.text, line_start, line, doc.lang, project.prism_version(), col) {
        return Some(partials::first_folder_only(partials::resolve(project, path, &r)));
    }
    if let Some((_, _, name)) = render_refs::model_renders(line).into_iter().find(|&(s, e, _)| (s..=e).contains(&col)) {
        return Some(partials::resolve_model(project, path, name));
    }
    // On a component render the question isn't about this partial.
    if render_refs::component_ref_at(line, col).is_some() || !paths::is_partial_path(path) {
        return None;
    }
    Some(vec![path.to_path_buf()])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stimulus_controllers_are_dropped_when_app_javascript_or_the_importmap_changes() {
        let event = |path: &str, kind| vec![(PathBuf::from(path), kind)];
        assert!(may_change_controllers(&event("/r/app/javascript/controllers/index.js", FileChangeType::CHANGED)));
        assert!(may_change_controllers(&event("/r/app/javascript/controllers", FileChangeType::DELETED)));
        assert!(may_change_controllers(&event("/r/config/importmap.rb", FileChangeType::CHANGED)));
        assert!(may_change_controllers(&event("/r/app", FileChangeType::DELETED)));
        assert!(!may_change_controllers(&event("/r/app/views/users/_card.erb", FileChangeType::CHANGED)));
        assert!(!may_change_controllers(&event("/r/app", FileChangeType::CHANGED)));
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
        let views = paths::views_root(&renames[0].0).unwrap();
        let sources = partial_rename::sources(&views, &HashMap::new(), |_| vec![]);
        let mut got: Vec<String> = partial_rename::render_edits(&Project::unknown(), renames, &sources)
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
            &render_refs::render_ref_at(line, 0, line, TemplateLang::Erb, ruby_prism::Version::Latest, 22).unwrap(),
        );
        assert!(md.ends_with("```ruby\nlocals: (user:, size: :md)\n```"), "{md}");
        assert!(md.contains("- `app/views/users/_card.html.erb`"), "a path from the Rails root: {md}");
        assert!(!render_hover(
            &Project::unknown(),
            &file,
            &render_refs::render_ref_at(r#"render "plain""#, 0, r#"render "plain""#, TemplateLang::Erb, ruby_prism::Version::Latest, 9)
                .unwrap()
        )
        .contains("locals:"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_listing_read_across_a_watched_file_event_is_read_again() {
        let (cache, generation, reads) = (DashMap::new(), AtomicU64::new(0), AtomicU64::new(0));
        let root = Path::new("/app/views");
        let read = || {
            // The first read sees the tree before the event that comes in while it runs.
            if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                generation.fetch_add(1, Ordering::SeqCst);
                return "before";
            }
            "after"
        };
        assert_eq!(fill(&cache, root, &generation, read), "after");
        assert_eq!(*cache.get(root).unwrap(), "after");
        assert_eq!(fill(&cache, root, &generation, || "other"), "after", "a filled entry is kept");
    }
}
