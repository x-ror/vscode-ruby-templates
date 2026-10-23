mod erb;
mod helpers;
mod indented;
mod partials;
mod ruby;
mod util;

use dashmap::DashMap;
use std::path::PathBuf;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TemplateLang { Erb, Haml, Slim }

impl TemplateLang {
    fn from_uri_or_id(uri: &Url, language_id: Option<&str>) -> Option<Self> {
        match language_id {
            Some("erb") => return Some(Self::Erb),
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

impl Backend {
    fn diagnostics_for(&self, doc: &Doc) -> Vec<Diagnostic> {
        match doc.lang {
            TemplateLang::Erb => erb::diagnostics(&doc.text),
            TemplateLang::Haml => indented::diagnostics(&doc.text, indented::Lang::Haml),
            TemplateLang::Slim => indented::diagnostics(&doc.text, indented::Lang::Slim),
        }
    }

    async fn publish(&self, uri: Url) {
        let diags = match self.docs.get(&uri) {
            Some(doc) => self.diagnostics_for(&doc),
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
                    trigger_characters: Some(vec!["\"".into(), "'".into(), ".".into(), "/".into()]),
                    resolve_provider: Some(false),
                    ..Default::default()
                }),
                document_link_provider: Some(DocumentLinkOptions { resolve_provider: Some(false), work_done_progress_options: Default::default() }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
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
        // Helper signature hover.
        let word = word_at(line, col);
        if let Some(h) = helpers::HELPERS.iter().find(|h| h.name == word) {
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: format!("```ruby\n{}\n```", h.detail) }),
                range: None,
            }));
        }
        Ok(None)
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

        // 3. Rails helpers, only when we're in a Ruby context.
        if !in_ruby_context(doc.lang, line, col) { return Ok(None); }
        let typed = before.rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == '?')).next().unwrap_or("");
        if typed.is_empty() { return Ok(None); }
        let items: Vec<CompletionItem> = helpers::HELPERS.iter().filter(|h| h.name.starts_with(typed)).map(|h| CompletionItem {
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
            matches!((open, close), (Some(o), None) | (Some(o), Some(_)) if close.map(|c| c < o).unwrap_or(true)) && !before[open.unwrap()..].starts_with("<%#")
        }
        TemplateLang::Haml | TemplateLang::Slim => {
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
            let Some(lang) = TemplateLang::from_uri_or_id(&uri, None) else { eprintln!("{}: unknown template type", f); continue };
            let doc = Doc { lang, text };
            let diags = match lang {
                TemplateLang::Erb => erb::diagnostics(&doc.text),
                TemplateLang::Haml => indented::diagnostics(&doc.text, indented::Lang::Haml),
                TemplateLang::Slim => indented::diagnostics(&doc.text, indented::Lang::Slim),
            };
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
