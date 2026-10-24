mod erb;
mod helpers;
mod indented;
mod outline;
mod partials;
mod ruby;
mod tags;
mod util;

use dashmap::DashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TemplateLang { Erb, Haml, Slim, Skim }

impl TemplateLang {
    fn from_uri_or_id(uri: &Url, language_id: Option<&str>) -> Option<Self> {
        match language_id {
            Some(id) if id == "erb" || id.starts_with("erb-") => return Some(Self::Erb),
            Some("haml") => return Some(Self::Haml),
            Some("slim") => return Some(Self::Slim),
            Some("skim") => return Some(Self::Skim),
            _ => {}
        }
        let path = uri.path();
        if path.ends_with(".erb") || path.ends_with(".rhtml") { Some(Self::Erb) }
        else if path.ends_with(".haml") { Some(Self::Haml) }
        else if path.ends_with(".slim") { Some(Self::Slim) }
        else if path.ends_with(".skim") { Some(Self::Skim) }
        else { None }
    }
}

struct Doc { lang: TemplateLang, text: String }

/// `initializationOptions.slim`, from the `rubyTemplates.slim.*` settings.
#[derive(Default, Debug, PartialEq)]
struct SlimOptions {
    extra_engines: Vec<String>,
    disabled_diagnostics: Vec<String>,
}

impl SlimOptions {
    /// Reads each field on its own so one malformed field keeps the others; returns what was ignored.
    fn from_init(init: Option<&serde_json::Value>) -> (Self, Vec<String>) {
        let slim = init.and_then(|v| v.get("slim"));
        let mut ignored = Vec::new();
        let mut strings = |name: &str| -> Vec<String> {
            let Some(value) = slim.and_then(|s| s.get(name)) else {
                return vec![];
            };
            let Some(items) = value.as_array() else {
                ignored.push(format!("slim.{name}: expected an array, got {value}"));
                return vec![];
            };
            items
                .iter()
                .filter_map(|item| {
                    let s = item.as_str().map(String::from);
                    if s.is_none() {
                        ignored.push(format!("slim.{name}: expected a string, got {item}"));
                    }
                    s
                })
                .collect()
        };
        let options = Self {
            extra_engines: strings("extraEngines"),
            disabled_diagnostics: strings("disabledDiagnostics"),
        };
        (options, ignored)
    }
}

struct Backend {
    client: Client,
    docs: DashMap<Url, Doc>,
    // Set once in `initialize`; the client restarts the server when these settings change.
    slim: OnceLock<SlimOptions>,
}

fn diagnostics(uri: &Url, doc: &Doc, slim: &SlimOptions) -> Vec<Diagnostic> {
    let mut out = match doc.lang {
        TemplateLang::Erb => erb::diagnostics(&doc.text),
        TemplateLang::Haml => indented::diagnostics(&doc.text, indented::Lang::Haml, &[]),
        TemplateLang::Slim => indented::diagnostics(&doc.text, indented::Lang::Slim, &slim.extra_engines),
        // Skim is Slim syntax with CoffeeScript code, so the Ruby checks would misfire.
        TemplateLang::Skim => indented::diagnostics(&doc.text, indented::Lang::Slim, &slim.extra_engines)
            .into_iter()
            .filter(|d| !matches!(&d.code, Some(NumberOrString::String(c)) if indented::code::RUBY.contains(&c.as_str())))
            .collect(),
    };
    if doc.lang == TemplateLang::Erb && html_host(uri) {
        let partial = uri.path().rsplit('/').next().is_some_and(|f| f.starts_with('_'));
        out.extend(tags::diagnostics(&doc.text, partial));
    }
    if let Ok(path) = uri.to_file_path() {
        out.extend(unresolved_renders(doc.lang, &path, &doc.text));
    }
    if matches!(doc.lang, TemplateLang::Slim | TemplateLang::Skim) {
        out.retain(|d| {
            !matches!(&d.code, Some(NumberOrString::String(c)) if slim.disabled_diagnostics.contains(c))
        });
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
            let ruby_line = match lang {
                TemplateLang::Erb => true,
                _ => { let t = line.trim_start(); ["=", "-", "!=", "&="].iter().any(|p| t.starts_with(p)) }
            };
            if !ruby_line || !in_ruby_context(lang, line, r.start) || !partials::resolve(path, &r).is_empty() {
                continue;
            }
            let message = format!("{} `{}` not found", kind_name(r.kind), r.name);
            let range = util::line_range(text, i, r.start, r.end);
            out.push(util::coded(
                util::warning(range, message),
                "unresolved-render",
            ));
        }
    }
    out
}

impl Backend {
    async fn publish(&self, uri: Url) {
        let diags = match self.docs.get(&uri) {
            Some(doc) => diagnostics(&uri, &doc, self.slim.get_or_init(SlimOptions::default)),
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
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let (slim, ignored) = SlimOptions::from_init(params.initialization_options.as_ref());
        for message in ignored {
            let message = format!("ignoring initializationOptions.{message}");
            self.client.log_message(MessageType::WARNING, message).await;
        }
        let _ = self.slim.set(slim);
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
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
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

    async fn folding_range(&self, p: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let Some(doc) = self.docs.get(&p.text_document.uri) else { return Ok(None) };
        // A provider's ranges replace VS Code's indentation folding; `None` falls back to it. That fallback is
        // what folds Haml/Slim, and the host language of `.js.erb`/`.yml.erb`/… where ERB blocks alone would regress.
        if doc.lang != TemplateLang::Erb || !html_host(&p.text_document.uri) {
            return Ok(None);
        }
        Ok(Some(outline::folding_ranges(&doc.text)))
    }

    async fn document_symbol(&self, p: DocumentSymbolParams) -> Result<Option<DocumentSymbolResponse>> {
        let uri = p.text_document.uri;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        if doc.lang != TemplateLang::Erb {
            return Ok(None);
        }
        Ok(Some(DocumentSymbolResponse::Nested(outline::document_symbols(&doc.text, html_host(&uri)))))
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

/// A `#` (not `#{`) outside a string means the rest of the tag is a Ruby comment.
fn ruby_comment_before(code: &str) -> bool {
    let mut quote = None;
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), '\\') => { chars.next(); }
            (None, '"' | '\'') => quote = Some(c),
            (None, '#') if chars.peek() != Some(&'{') => return true,
            _ => {}
        }
    }
    false
}

fn in_ruby_context(lang: TemplateLang, line: &str, col: usize) -> bool {
    let before = &line[..col.min(line.len())];
    match lang {
        TemplateLang::Erb => {
            let open = before.rfind("<%");
            let close = before.rfind("%>");
            matches!((open, close), (Some(o), None) | (Some(o), Some(_)) if close.map(|c| c < o).unwrap_or(true))
                && !before[open.unwrap()..].starts_with("<%#")
                && !ruby_comment_before(&before[open.unwrap() + 2..])
        }
        TemplateLang::Haml | TemplateLang::Slim | TemplateLang::Skim => {
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
            let diags = diagnostics(&uri, &Doc { lang, text }, &SlimOptions::default());
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
    let (service, socket) = LspService::new(|client| Backend {
        client,
        docs: DashMap::new(),
        slim: OnceLock::new(),
    });
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
    fn slim_options_keep_valid_fields() {
        let init = serde_json::json!({
            "slim": { "extraEngines": "mjml", "disabledDiagnostics": ["bad-dedent", 3] }
        });
        let (options, ignored) = SlimOptions::from_init(Some(&init));
        let expected = SlimOptions {
            extra_engines: vec![],
            disabled_diagnostics: vec!["bad-dedent".into()],
        };
        assert_eq!(options, expected);
        assert_eq!(ignored.len(), 2, "{ignored:?}");
        assert_eq!(SlimOptions::from_init(None), (SlimOptions::default(), vec![]));
    }

    #[test]
    fn skim_skips_ruby_checks() {
        let uri = Url::parse("untitled:Untitled-1").unwrap();
        assert_eq!(TemplateLang::from_uri_or_id(&uri, Some("skim")), Some(TemplateLang::Skim));
        let text = "- else\n= alert(\"hi\"\n- form_with do |f|\n  p\n    a\n   span\n";
        let codes = |lang| -> Vec<String> {
            diagnostics(&uri, &Doc { lang, text: text.into() }, &SlimOptions::default())
                .into_iter()
                .filter_map(|d| match d.code { Some(NumberOrString::String(c)) => Some(c), _ => None })
                .collect()
        };
        let slim = codes(TemplateLang::Slim);
        assert!(indented::code::RUBY.iter().all(|c| slim.iter().any(|s| s == c)), "{slim:?}");
        assert_eq!(codes(TemplateLang::Skim), [indented::code::BAD_DEDENT]);
    }

    #[test]
    fn host_specific_erb_ids() {
        let untitled = Url::parse("untitled:Untitled-1").unwrap();
        assert_eq!(TemplateLang::from_uri_or_id(&untitled, Some("erb-js")), Some(TemplateLang::Erb));
        assert_eq!(TemplateLang::from_uri_or_id(&untitled, Some("erbx")), None);
        let js_erb = Url::parse("file:///app/views/users/index.js.erb").unwrap();
        assert_eq!(TemplateLang::from_uri_or_id(&js_erb, None), Some(TemplateLang::Erb));
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
        std::fs::remove_dir_all(&root).unwrap();
    }
}
