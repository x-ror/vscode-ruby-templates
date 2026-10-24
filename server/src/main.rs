mod erb;
mod haml;
mod helpers;
mod indented;
mod outline;
mod partials;
mod ruby;
mod tags;
mod util;

use dashmap::DashMap;
use indented::LineRole;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TemplateLang { Erb, Haml, Slim }

impl TemplateLang {
    fn from_uri_or_id(uri: &Url, language_id: Option<&str>) -> Option<Self> {
        match language_id {
            Some(id) if id == "erb" || id.starts_with("erb-") => return Some(Self::Erb),
            Some("haml") => return Some(Self::Haml),
            Some("slim") => return Some(Self::Slim),
            _ => {}
        }
        let path = uri.path();
        if path.ends_with(".erb") || path.ends_with(".rhtml") { Some(Self::Erb) }
        else if path.ends_with(".haml") { Some(Self::Haml) }
        else if path.ends_with(".slim") || path.ends_with(".skim") { Some(Self::Slim) }
        else { None }
    }
}

struct Doc { lang: TemplateLang, text: String }

struct Backend {
    client: Client,
    docs: DashMap<Url, Doc>,
}

fn diagnostics(uri: &Url, doc: &Doc) -> Vec<Diagnostic> {
    let mut out = match doc.lang {
        TemplateLang::Erb => erb::diagnostics(&doc.text),
        // Thor templates (`*.haml.tt`) interleave column-0 ERB with the Haml, so its structure can't be checked.
        TemplateLang::Haml if uri.path().ends_with(".tt") => vec![],
        TemplateLang::Haml => indented::diagnostics(&doc.text, indented::Lang::Haml),
        TemplateLang::Slim => indented::diagnostics(&doc.text, indented::Lang::Slim),
    };
    if doc.lang == TemplateLang::Erb && html_host(uri) {
        let partial = uri.path().rsplit('/').next().is_some_and(|f| f.starts_with('_'));
        out.extend(tags::diagnostics(&doc.text, partial));
    }
    if let Ok(path) = uri.to_file_path() {
        out.extend(unresolved_renders(doc.lang, &path, &doc.text));
    }
    out
}

/// `x.html.erb`, `x.html+phone.erb`, `x.turbo_stream.erb`, `x.rhtml`.
// ponytail: bare `.erb` and untitled documents are skipped; their host (`nginx.conf.erb`…) is unknown.
fn html_host(uri: &Url) -> bool {
    let path = uri.path();
    if path.ends_with(".rhtml") {
        return true;
    }
    let Some(stem) = path.strip_suffix(".erb") else { return false };
    let host = stem.rsplit('/').next().and_then(|file| file.rsplit_once('.')).map_or("", |(_, ext)| ext);
    host == "html" || host.starts_with("html+") || host == "turbo_stream"
}

/// Literal `render "x"` / `render partial:|template: "x"` with no matching file under app/views.
fn unresolved_renders(lang: TemplateLang, path: &Path, text: &str) -> Vec<Diagnostic> {
    let Some(views) = partials::views_root(path) else { return vec![] };
    let mut out = Vec::new();
    let roles = line_roles(lang, text);
    for (i, line) in text.lines().enumerate() {
        for r in partials::render_refs(line) {
            // `render layout: "x"` inside a view looks up a partial; skip rather than guess.
            if r.kind == partials::RenderKind::Layout { continue; }
            // ponytail: only `dir/name` with an existing app/views/dir is checked. Bare names resolve via the
            // controller's prefix and its ancestors, and missing dirs are usually gem/engine views (devise/…);
            // neither is knowable without reading controllers or the bundle.
            match r.name.rsplit_once('/') {
                Some((dir, _)) if views.join(dir).is_dir() => {}
                _ => continue,
            }
            if !render_in_code(lang, roles[i], line, r.start) || !partials::resolve(path, &r).is_empty() {
                continue;
            }
            out.push(util::warning(util::line_range(text, i, r.start, r.end), format!("{} `{}` not found", kind_name(r.kind), r.name)));
        }
    }
    out
}

/// Haml line roles; every other language's lines all start their own construct.
fn line_roles(lang: TemplateLang, text: &str) -> Vec<LineRole> {
    match lang {
        TemplateLang::Haml => indented::haml_line_roles(text),
        _ => vec![LineRole::Start; text.lines().count()],
    }
}

/// Is the `render` whose string starts at byte `start` a call in Ruby code, not template text, a Ruby comment or
/// a Ruby string (`link_to "render 'x'"`)?
fn render_in_code(lang: TemplateLang, role: LineRole, line: &str, start: usize) -> bool {
    let Some(call) = line[..start].rfind("render") else { return false };
    let code_start = match (lang, role) {
        (TemplateLang::Erb, _) => in_ruby_context(lang, line, start).then(|| line[..call].rfind("<%").map_or(0, |open| open + 2)),
        (TemplateLang::Haml, LineRole::Start) => indented::haml_code_start(line).filter(|&code| code <= call),
        (TemplateLang::Haml, LineRole::Ruby) => Some(0),
        (TemplateLang::Haml, LineRole::Text) => None,
        (TemplateLang::Slim, _) => {
            let ws = util::leading_ws(line).len();
            let prefix = ["!=", "&=", "=", "-"].into_iter().find(|p| line[ws..].starts_with(p));
            prefix.filter(|_| in_ruby_context(lang, line, start)).map(|p| ws + p.len())
        }
    };
    code_start.is_some_and(|code| code <= call && ruby::code_end(&line[code..call]) == ruby::CodeEnd::Code)
}

/// Edits to every `render "x"` under app/views whose partial moves in `renames` (old path, new path).
/// Rails renders the first match from the closest directory, so a render changes only when every file it could
/// pick there moves, all to one name. `read` supplies a file's current text (open buffers win over disk).
fn partial_rename_edits(renames: &[(PathBuf, PathBuf)], read: impl Fn(&Path) -> Option<String>) -> HashMap<Url, Vec<TextEdit>> {
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    let mut roots: Vec<PathBuf> = renames.iter().filter_map(|(old, _)| partials::views_root(old)).collect();
    roots.sort();
    roots.dedup();
    for views in &roots {
        for entry in walkdir::WalkDir::new(views).max_depth(partials::VIEWS_MAX_DEPTH).into_iter().flatten() {
            let file = entry.path();
            let (Ok(uri), true) = (Url::from_file_path(file), file.is_file()) else { continue };
            let Some(lang) = TemplateLang::from_uri_or_id(&uri, None) else { continue };
            let Some(text) = read(file) else { continue };
            let roles = line_roles(lang, &text);
            for (i, line) in text.lines().enumerate() {
                for r in partials::render_refs(line) {
                    if r.kind != partials::RenderKind::Partial || !render_in_code(lang, roles[i], line, r.start) {
                        continue;
                    }
                    let Some(name) = moved_reference(views, renames, &partials::resolve(file, &r), &r.name) else { continue };
                    if name != r.name {
                        changes.entry(uri.clone()).or_default().push(TextEdit::new(util::line_range(&text, i, r.start, r.end), name));
                    }
                }
            }
        }
    }
    changes
}

/// The new name for a render whose candidates are `hits`, if all of Rails' picks (the first hit's directory) move to one partial.
fn moved_reference(views: &Path, renames: &[(PathBuf, PathBuf)], hits: &[PathBuf], name: &str) -> Option<String> {
    let first_dir = hits.first()?.parent();
    let mut names = hits.iter().filter(|hit| hit.parent() == first_dir).map(|hit| {
        let (old, new) = renames.iter().find(|(old, _)| old == hit)?;
        // A partial directly in app/views has no `render` name that reaches it.
        let dir = new.parent()?.strip_prefix(views).ok()?.to_string_lossy().replace('\\', "/");
        let new_name = partials::partial_name(new).filter(|_| !dir.is_empty())?;
        let bare = !name.contains('/') && new.parent() == old.parent();
        Some(if bare { new_name } else { format!("{dir}/{new_name}") })
    });
    let first = names.next()??;
    names.all(|n| n.as_ref() == Some(&first)).then_some(first)
}

impl Backend {
    async fn publish(&self, uri: Url) {
        let diags = match self.docs.get(&uri) {
            Some(doc) => diagnostics(&uri, &doc),
            None => vec![],
        };
        self.client.publish_diagnostics(uri, diags, None).await;
    }

    fn line_at(text: &str, line: u32) -> &str {
        text.lines().nth(line as usize).unwrap_or("")
    }

    fn byte_col(line: &str, character: u32) -> usize {
        let mut units = 0u32;
        for (idx, ch) in line.char_indices() {
            if units >= character { return idx; }
            units += ch.len_utf16() as u32;
        }
        line.len()
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo { name: "ruby-templates-ls".into(), version: Some(env!("CARGO_PKG_VERSION").into()) }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
                definition_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(["\"", "'", ".", "/", "%", ":", "{", "("].map(String::from).to_vec()),
                    resolve_provider: Some(false),
                    ..Default::default()
                }),
                document_link_provider: Some(DocumentLinkOptions { resolve_provider: Some(false), work_done_progress_options: Default::default() }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: None,
                    file_operations: Some(WorkspaceFileOperationsServerCapabilities {
                        will_rename: Some(FileOperationRegistrationOptions {
                            filters: vec![FileOperationFilter {
                                scheme: Some("file".into()),
                                pattern: FileOperationPattern { glob: "**/app/views/**/_*".into(), matches: Some(FileOperationPatternKind::File), options: None },
                            }],
                        }),
                        ..Default::default()
                    }),
                }),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client.log_message(MessageType::INFO, "ruby-templates-ls ready").await;
    }

    async fn shutdown(&self) -> Result<()> { Ok(()) }

    async fn did_open(&self, p: DidOpenTextDocumentParams) {
        let Some(lang) = TemplateLang::from_uri_or_id(&p.text_document.uri, Some(&p.text_document.language_id)) else { return };
        self.docs.insert(p.text_document.uri.clone(), Doc { lang, text: p.text_document.text });
        self.publish(p.text_document.uri).await;
    }

    async fn did_change(&self, p: DidChangeTextDocumentParams) {
        if let Some(change) = p.content_changes.into_iter().last() {
            if let Some(mut doc) = self.docs.get_mut(&p.text_document.uri) {
                doc.text = change.text;
            }
        }
        self.publish(p.text_document.uri).await;
    }

    async fn did_close(&self, p: DidCloseTextDocumentParams) {
        self.docs.remove(&p.text_document.uri);
        self.client.publish_diagnostics(p.text_document.uri, vec![], None).await;
    }

    async fn will_rename_files(&self, p: RenameFilesParams) -> Result<Option<WorkspaceEdit>> {
        let read = |path: &Path| {
            let open = Url::from_file_path(path).ok().and_then(|uri| self.docs.get(&uri).map(|d| d.text.clone()));
            open.or_else(|| std::fs::read_to_string(path).ok())
        };
        let path = |uri: &str| Url::parse(uri).ok()?.to_file_path().ok();
        let renames: Vec<(PathBuf, PathBuf)> = p.files.iter().filter_map(|f| Some((path(&f.old_uri)?, path(&f.new_uri)?))).collect();
        let changes = partial_rename_edits(&renames, read);
        Ok((!changes.is_empty()).then(|| WorkspaceEdit { changes: Some(changes), ..Default::default() }))
    }

    async fn goto_definition(&self, p: GotoDefinitionParams) -> Result<Option<GotoDefinitionResponse>> {
        let uri = p.text_document_position_params.text_document.uri;
        let pos = p.text_document_position_params.position;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let Ok(path) = uri.to_file_path() else { return Ok(None) };
        let line = Self::line_at(&doc.text, pos.line);
        let col = Self::byte_col(line, pos.character);

        let targets: Vec<PathBuf> = if let Some(r) = partials::render_ref_at(line, col) {
            partials::resolve(&path, &r)
        } else if let Some(c) = partials::component_ref_at(line, col) {
            partials::resolve_component(&path, &c)
        } else {
            vec![]
        };
        let locs: Vec<Location> = targets
            .into_iter()
            .filter_map(|t| Url::from_file_path(t).ok())
            .map(|u| Location::new(u, Range::default()))
            .collect();
        Ok(if locs.is_empty() { None } else { Some(GotoDefinitionResponse::Array(locs)) })
    }

    async fn document_link(&self, p: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        let uri = p.text_document.uri;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let Ok(path) = uri.to_file_path() else { return Ok(None) };
        let mut links = Vec::new();
        for (i, line) in doc.text.lines().enumerate() {
            for r in partials::render_refs(line) {
                if let Some(target) = partials::resolve(&path, &r).into_iter().next() {
                    if let Ok(t) = Url::from_file_path(&target) {
                        links.push(DocumentLink {
                            range: util::line_range(&doc.text, i, r.start, r.end),
                            target: Some(t),
                            tooltip: Some(target.to_string_lossy().into_owned()),
                            data: None,
                        });
                    }
                }
            }
        }
        Ok(Some(links))
    }

    async fn hover(&self, p: HoverParams) -> Result<Option<Hover>> {
        let uri = p.text_document_position_params.text_document.uri;
        let pos = p.text_document_position_params.position;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let line = Self::line_at(&doc.text, pos.line);
        let col = Self::byte_col(line, pos.character);
        if let Ok(path) = uri.to_file_path() {
            if let Some(r) = partials::render_ref_at(line, col) {
                let found = partials::resolve(&path, &r);
                let md = if found.is_empty() {
                    format!("**{}** `{}`\n\nNo matching template found.", kind_name(r.kind), r.name)
                } else {
                    let list: Vec<String> = found.iter().map(|f| format!("- `{}`", f.display())).collect();
                    format!("**{}** `{}`\n\n{}", kind_name(r.kind), r.name, list.join("\n"))
                };
                return Ok(Some(Hover { contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: md }), range: None }));
            }
        }
        let role = line_roles(doc.lang, &doc.text).get(pos.line as usize).copied().unwrap_or(LineRole::Start);
        if role == LineRole::Text {
            return Ok(None);
        }
        if let Some(md) = haml::hover(line, col).filter(|_| doc.lang == TemplateLang::Haml && role == LineRole::Start) {
            return Ok(Some(Hover { contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: md }), range: None }));
        }
        // Helper signature hover.
        let word = word_at(line, col);
        if let Some(h) = helpers_for(doc.lang).find(|h| h.name == word) {
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: format!("```ruby\n{}\n```", h.detail) }),
                range: None,
            }));
        }
        Ok(None)
    }

    async fn folding_range(&self, p: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let Some(doc) = self.docs.get(&p.text_document.uri) else { return Ok(None) };
        // A provider's ranges replace VS Code's indentation folding; `None` falls back to it. That fallback is
        // what folds Slim, and the host language of `.js.erb`/`.yml.erb`/… where ERB blocks alone would regress.
        match doc.lang {
            TemplateLang::Haml => Ok(Some(outline::haml_folding_ranges(&doc.text))),
            TemplateLang::Erb if html_host(&p.text_document.uri) => Ok(Some(outline::folding_ranges(&doc.text))),
            _ => Ok(None),
        }
    }

    async fn document_symbol(&self, p: DocumentSymbolParams) -> Result<Option<DocumentSymbolResponse>> {
        let uri = p.text_document.uri;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let symbols = match doc.lang {
            TemplateLang::Erb => outline::document_symbols(&doc.text, html_host(&uri)),
            TemplateLang::Haml => outline::haml_document_symbols(&doc.text),
            TemplateLang::Slim => return Ok(None),
        };
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn completion(&self, p: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = p.text_document_position.text_document.uri;
        let pos = p.text_document_position.position;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let line = Self::line_at(&doc.text, pos.line);
        let col = Self::byte_col(line, pos.character);
        let before = &line[..col.min(line.len())];

        // 1. Partial names inside render "..."
        if let Some(prefix) = partials::render_string_prefix(line, col) {
            let Ok(path) = uri.to_file_path() else { return Ok(None) };
            let items: Vec<CompletionItem> = partials::partial_candidates(&path)
                .into_iter()
                .filter(|(name, _)| name.starts_with(&prefix) || prefix.is_empty())
                .map(|(name, file)| CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::FILE),
                    detail: Some(file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()),
                    insert_text: Some(name),
                    ..Default::default()
                })
                .collect();
            return Ok(Some(CompletionResponse::Array(items)));
        }

        // 2. Form builder methods after `f.` / `form.` / `ff.`
        if let Some(idx) = before.rfind('.') {
            let recv = before[..idx].rsplit(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
            let typed = &before[idx + 1..];
            if matches!(recv, "f" | "form" | "ff" | "builder" | "fields") && typed.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let items = helpers::FORM_BUILDER.iter().filter(|m| m.starts_with(typed)).map(|m| CompletionItem {
                    label: m.to_string(),
                    kind: Some(CompletionItemKind::METHOD),
                    detail: Some("ActionView::Helpers::FormBuilder".into()),
                    ..Default::default()
                }).collect();
                return Ok(Some(CompletionResponse::Array(items)));
            }
        }

        let role = line_roles(doc.lang, &doc.text).get(pos.line as usize).copied().unwrap_or(LineRole::Start);
        if let Some(context) = haml::completion_context(line, col).filter(|_| doc.lang == TemplateLang::Haml && role == LineRole::Start) {
            return Ok(Some(CompletionResponse::Array(haml_completions(context))));
        }

        // 3. Rails helpers, only when we're in a Ruby context.
        let ruby = match role {
            LineRole::Start => in_ruby_context(doc.lang, line, col),
            LineRole::Ruby => !ruby::comment_before(before),
            LineRole::Text => false,
        };
        if !ruby { return Ok(None); }
        let typed = before.rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == '?')).next().unwrap_or("");
        if typed.is_empty() { return Ok(None); }
        let items: Vec<CompletionItem> = helpers_for(doc.lang).filter(|h| h.name.starts_with(typed)).map(|h| CompletionItem {
            label: h.name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(h.detail.to_string()),
            insert_text: Some(h.snippet.to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        }).collect();
        Ok(Some(CompletionResponse::Array(items)))
    }
}

fn helpers_for(lang: TemplateLang) -> impl Iterator<Item = &'static helpers::Helper> {
    let haml_only = if lang == TemplateLang::Haml { helpers::HAML_HELPERS } else { &[] };
    helpers::HELPERS.iter().chain(haml_only)
}

fn haml_completions(context: haml::Completion) -> Vec<CompletionItem> {
    use haml::{AttrPlace, Completion};
    let item = |label: String, kind, detail: &str, snippet: Option<String>| CompletionItem {
        label,
        kind: Some(kind),
        detail: Some(detail.to_string()),
        insert_text_format: snippet.is_some().then_some(InsertTextFormat::SNIPPET),
        insert_text: snippet,
        ..Default::default()
    };
    match context {
        Completion::Filter(typed) => indented::HAML_FILTERS.iter().filter(|name| name.starts_with(typed))
            .map(|f| item(f.to_string(), CompletionItemKind::MODULE, "Haml filter", None)).collect(),
        Completion::Tag(typed) => helpers::HTML_TAGS.iter().filter(|name| name.starts_with(typed))
            .map(|t| item(t.to_string(), CompletionItemKind::PROPERTY, "HTML element", None)).collect(),
        Completion::Attribute { typed, place, rocket } => {
            let names: Vec<String> = match place {
                AttrPlace::Hash => helpers::HTML_ATTRIBUTES.iter().chain(&["data", "aria"]).map(|a| a.to_string()).collect(),
                AttrPlace::DataHash => helpers::DATA_KEYS.iter().map(|k| k.to_string()).collect(),
                AttrPlace::AriaHash => helpers::ARIA_KEYS.iter().map(|k| k.to_string()).collect(),
                AttrPlace::Html => helpers::HTML_ATTRIBUTES.iter().map(|a| a.to_string())
                    .chain(helpers::ARIA_KEYS.iter().map(|k| format!("aria-{k}")))
                    .chain(helpers::DATA_KEYS.iter().map(|k| format!("data-{}", k.replace('_', "-"))))
                    .collect(),
            };
            names.into_iter().filter(|n| n.starts_with(typed)).map(|name| {
                // The typed `:` of an old-style key stays; the snippet replaces only the name after it.
                let key = if name.contains('-') { format!("\"{name}\"") } else { name.clone() };
                let separator = if rocket { " =>" } else { ":" };
                let snippet = match place {
                    AttrPlace::Html => format!("{name}=\"$1\""),
                    AttrPlace::Hash if name == "data" || name == "aria" => format!("{key}{separator} {{$1}}"),
                    _ => format!("{key}{separator} $1"),
                };
                item(name, CompletionItemKind::PROPERTY, "HTML attribute", Some(snippet))
            }).collect()
        }
    }
}

fn kind_name(k: partials::RenderKind) -> &'static str {
    match k { partials::RenderKind::Partial => "Partial", partials::RenderKind::Template => "Template", partials::RenderKind::Layout => "Layout" }
}

fn word_at(line: &str, col: usize) -> &str {
    let is_w = |c: char| c.is_alphanumeric() || c == '_' || c == '?' || c == '!';
    let start = line[..col.min(line.len())].rfind(|c: char| !is_w(c)).map(|i| i + 1).unwrap_or(0);
    let end = line[col.min(line.len())..].find(|c: char| !is_w(c)).map(|i| i + col).unwrap_or(line.len());
    &line[start..end]
}

fn in_ruby_context(lang: TemplateLang, line: &str, col: usize) -> bool {
    let before = &line[..col.min(line.len())];
    match lang {
        TemplateLang::Erb => {
            let open = before.rfind("<%");
            let close = before.rfind("%>");
            matches!((open, close), (Some(o), None) | (Some(o), Some(_)) if close.map(|c| c < o).unwrap_or(true))
                && !before[open.unwrap()..].starts_with("<%#")
                && !ruby::comment_before(&before[open.unwrap() + 2..])
        }
        TemplateLang::Haml => haml::in_ruby(line, col),
        TemplateLang::Slim => {
            let t = before.trim_start();
            t.starts_with('-') || t.starts_with('=') || t.starts_with("!=") || t.starts_with("&=")
                || before.contains("#{") || before.contains("= ") || before.contains('{') || before.contains('(')
        }
    }
}

#[tokio::main]
async fn main() {
    // `ruby-templates-ls --check <file>...` prints diagnostics and exits (handy for CI and debugging).
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|a| a == "--check").unwrap_or(false) {
        let mut failed = false;
        for f in &args[1..] {
            let Ok(text) = std::fs::read_to_string(f) else { eprintln!("cannot read {}", f); failed = true; continue };
            let Ok(uri) = Url::from_file_path(std::fs::canonicalize(f).unwrap_or_else(|_| PathBuf::from(f))) else { continue };
            let Some(lang) = TemplateLang::from_uri_or_id(&uri, None) else { eprintln!("{}: unknown template type", f); failed = true; continue };
            let diags = diagnostics(&uri, &Doc { lang, text });
            for d in &diags {
                let sev = match d.severity { Some(DiagnosticSeverity::ERROR) => "error", Some(DiagnosticSeverity::WARNING) => "warning", _ => "info" };
                println!("{}:{}:{}: {}: {}", f, d.range.start.line + 1, d.range.start.character + 1, sev, d.message);
                if sev == "error" { failed = true; }
            }
        }
        std::process::exit(if failed { 1 } else { 0 });
    }
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend { client, docs: DashMap::new() });
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_host_by_file_name() {
        for (path, html) in [
            ("/v/users/show.html.erb", true),
            ("/v/users/show.html+phone.erb", true),
            ("/v/users/create.turbo_stream.erb", true),
            ("/v/old.rhtml", true),
            ("/v/users/index.js.erb", false),
            ("/v/mailer/welcome.text.erb", false),
            ("/v.html/config.yml.erb", false),
            ("/v.html/nginx.erb", false),
        ] {
            assert_eq!(html_host(&Url::parse(&format!("file://{path}")).unwrap()), html, "{path}");
        }
    }

    #[test]
    fn host_specific_erb_ids() {
        let untitled = Url::parse("untitled:Untitled-1").unwrap();
        assert_eq!(TemplateLang::from_uri_or_id(&untitled, Some("erb-js")), Some(TemplateLang::Erb));
        assert_eq!(TemplateLang::from_uri_or_id(&untitled, Some("erbx")), None);
        let js_erb = Url::parse("file:///app/views/users/index.js.erb").unwrap();
        assert_eq!(TemplateLang::from_uri_or_id(&js_erb, None), Some(TemplateLang::Erb));
    }

    /// `file line:col new_text` for each edit, sorted.
    fn rename(renames: &[(PathBuf, PathBuf)]) -> Vec<String> {
        let read = |p: &Path| std::fs::read_to_string(p).ok();
        let mut got: Vec<String> = partial_rename_edits(renames, read)
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
        assert_eq!(rename(&[(card.clone(), views.join("users/_tile.html.erb"))]), [
            "box.html.haml 1:12 users/tile",
            "index.html.erb 0:12 tile",
            "index.html.erb 0:33 users/tile",
            "show.html.slim 0:10 tile",
        ]);
        assert_eq!(rename(&[(card.clone(), views.join("shared/_card.html.erb"))]), [
            "box.html.haml 1:12 shared/card",
            "index.html.erb 0:12 shared/card",
            "index.html.erb 0:33 shared/card",
            "show.html.slim 0:10 shared/card",
        ]);
        assert!(rename(&[(card.clone(), views.join("users/card.html.erb"))]).is_empty());
        // Nothing names a partial directly in app/views.
        assert!(rename(&[(card, views.join("_card.html.erb"))]).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn partial_rename_needs_every_candidate_to_move() {
        let (root, views) = views_tree("variants", &[
            ("users/_card.html.erb", ""),
            ("users/_card.html+phone.erb", ""),
            ("users/index.html.erb", "<%= render \"card\" %>\n<%= render partial: \"users/card\" %>\n"),
            ("application/_badge.html.erb", ""),
            ("users/_badge.html.erb", ""),
            ("users/show.html.erb", "<%= render \"badge\" %>\n"),
        ]);
        let (html, phone) = (views.join("users/_card.html.erb"), views.join("users/_card.html+phone.erb"));
        assert!(rename(&[(phone.clone(), views.join("users/_tile.html+phone.erb"))]).is_empty());
        let both = [(html.clone(), views.join("shared/_card.html.erb")), (phone.clone(), views.join("shared/_card.html+phone.erb"))];
        assert_eq!(rename(&both), ["index.html.erb 0:12 shared/card", "index.html.erb 1:21 shared/card"]);
        // Moving both, but to different names, leaves the render alone.
        assert!(rename(&[both[0].clone(), (phone, views.join("shared/_other.html+phone.erb"))]).is_empty());
        // `users/_badge` shadows `application/_badge` for users/show.
        assert!(rename(&[(views.join("application/_badge.html.erb"), views.join("application/_pill.html.erb"))]).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn thor_templates_skip_haml_structure() {
        let text = "%table\n  %tr\n<% attributes.each do |attribute| -%>\n    %th <%= attribute.human_name %>\n<% end -%>\n";
        let doc = || Doc { lang: TemplateLang::Haml, text: text.to_string() };
        let uri = |name: &str| Url::parse(&format!("file:///app/lib/templates/haml/scaffold/{name}")).unwrap();
        assert!(diagnostics(&uri("index.html.haml.tt"), &doc()).is_empty());
        assert!(!diagnostics(&uri("index.html.haml"), &doc()).is_empty());
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
            "<p>we render \"prose\"</p>\n",
        );
        let diags = unresolved_renders(TemplateLang::Erb, &file, text);
        let got: Vec<(String, Range)> = diags.iter().map(|d| (d.message.clone(), d.range)).collect();
        let at = |l, s, e| Range::new(Position::new(l, s), Position::new(l, e));
        assert_eq!(got, vec![
            ("Partial `users/missing` not found".to_string(), at(0, 42, 55)),
            ("Partial `users/nope` not found".to_string(), at(1, 13, 23)),
        ]);
        assert!(diags.iter().all(|d| d.severity == Some(DiagnosticSeverity::WARNING)));

        let layouts = root.join("app/views/layouts");
        std::fs::create_dir_all(&layouts).unwrap();
        assert!(unresolved_renders(TemplateLang::Erb, &layouts.join("application.html.erb"), "<%= render \"header\" %>").is_empty());
        assert!(unresolved_renders(TemplateLang::Erb, Path::new("/tmp/x.html.erb"), "<%= render \"users/nope\" %>").is_empty());
        let haml = users.join("show.html.haml");
        assert!(unresolved_renders(TemplateLang::Haml, &haml, "%p (we render \"users/x\")").is_empty());
        assert_eq!(unresolved_renders(TemplateLang::Haml, &haml, "= render \"users/x\"").len(), 1);
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
        let lines: Vec<u32> = unresolved_renders(TemplateLang::Haml, &haml, haml_text).iter().map(|d| d.range.start.line).collect();
        assert_eq!(lines, vec![0, 1, 2, 3, 4, 5]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
