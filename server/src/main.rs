mod assets;
mod erb;
mod helpers;
mod indented;
mod outline;
mod partials;
mod ruby;
mod stimulus;
mod tags;
mod util;

use dashmap::DashMap;
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
        TemplateLang::Haml => indented::diagnostics(&doc.text, indented::Lang::Haml),
        TemplateLang::Slim => indented::diagnostics(&doc.text, indented::Lang::Slim),
    };
    if doc.lang == TemplateLang::Erb && html_host(uri) {
        let partial = uri.path().rsplit('/').next().is_some_and(|f| f.starts_with('_'));
        out.extend(tags::diagnostics(&doc.text, partial));
    }
    if let Ok(path) = uri.to_file_path() {
        let spans = erb::code_spans(&doc.text);
        out.extend(unresolved_renders(doc.lang, &path, &doc.text, &spans));
        out.extend(strict_locals_mismatches(doc.lang, &path, &doc.text, &spans));
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
fn unresolved_renders(lang: TemplateLang, path: &Path, text: &str, spans: &[(usize, usize)]) -> Vec<Diagnostic> {
    let Some(views) = partials::views_root(path) else { return vec![] };
    let mut out = Vec::new();
    for (i, (line_start, line)) in util::lines_with_offsets(text).enumerate() {
        if !ruby_line(lang, line) { continue; }
        for r in partials::render_refs(line) {
            if !in_app_views_dir(&views, &r.name) || !in_ruby_context(lang, text, spans, line_start + r.start) || !partials::resolve(path, &r).is_empty() {
                continue;
            }
            out.push(util::warning(util::line_range(text, i, r.start, r.end), format!("{} `{}` not found", kind_name(r.kind), r.name)));
        }
    }
    out
}

/// Literal one-line renders whose locals don't fit the partial's `locals: (…)` signature.
fn strict_locals_mismatches(lang: TemplateLang, path: &Path, text: &str, spans: &[(usize, usize)]) -> Vec<Diagnostic> {
    let Some(views) = partials::views_root(path) else { return vec![] };
    let mut out = Vec::new();
    for (i, (line_start, line)) in util::lines_with_offsets(text).enumerate() {
        if !ruby_line(lang, line) { continue; }
        for r in partials::render_refs(line) {
            // A partial's layout gets the partial's locals, which this check can't see.
            if r.from_layout || !in_app_views_dir(&views, &r.name) || !in_ruby_context(lang, text, spans, line_start + r.start) { continue; }
            let Some(passed) = partials::passed_locals(line, &r) else { continue };
            // Several variants/formats: which one Rails picks depends on the request.
            let found = partials::resolve(path, &r);
            let [file] = found.as_slice() else { continue };
            let Some(sig) = file_signature(file) else { continue };
            let missing: Vec<&str> = sig.locals.iter().filter(|(n, d)| d.is_none() && !passed.contains(&n.as_str())).map(|(n, _)| n.as_str()).collect();
            let unknown: Vec<&str> = passed.into_iter().filter(|p| !sig.rest && !sig.locals.iter().any(|(n, _)| n == p)).collect();
            let range = util::line_range(text, i, r.start, r.end);
            if !missing.is_empty() {
                out.push(util::warning(range, format!("Partial `{}` requires locals: {}", r.name, code_list(&missing))));
            }
            if !unknown.is_empty() {
                out.push(util::warning(range, format!("Partial `{}` does not accept locals: {}", r.name, code_list(&unknown))));
            }
        }
    }
    out
}

fn code_list(names: &[&str]) -> String {
    names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
}

fn file_signature(file: &Path) -> Option<partials::StrictLocals> {
    partials::strict_locals(&std::fs::read_to_string(file).ok()?)
}

// ponytail: only `dir/name` with an existing app/views/dir is checked. Bare names resolve via the
// controller's prefix and its ancestors, and missing dirs are usually gem/engine views (devise/…);
// neither is knowable without reading controllers or the bundle.
fn in_app_views_dir(views: &Path, name: &str) -> bool {
    name.rsplit_once('/').is_some_and(|(dir, _)| views.join(dir).is_dir())
}

/// ERB lines are scanned per tag; Haml/Slim lines only when they start with a Ruby marker.
fn ruby_line(lang: TemplateLang, line: &str) -> bool {
    match lang {
        TemplateLang::Erb => true,
        _ => { let t = line.trim_start(); ["=", "-", "!=", "&="].iter().any(|p| t.starts_with(p)) }
    }
}

fn render_hover(path: &Path, r: &partials::RenderRef) -> String {
    let found = partials::resolve(path, r);
    if found.is_empty() {
        return format!("**{}** `{}`\n\nNo matching template found.", kind_name(r.kind), r.name);
    }
    let list: Vec<String> = found.iter().map(|f| format!("- `{}`", f.display())).collect();
    let mut md = format!("**{}** `{}`\n\n{}", kind_name(r.kind), r.name, list.join("\n"));
    if let Some(sig) = file_signature(&found[0]) {
        md.push_str(&format!("\n\n```ruby\nlocals: ({})\n```", sig.params));
    }
    md
}

/// Byte ranges on `line` that lead to files by convention: `render @model`, Stimulus identifiers, asset paths.
/// With `col`, only the ones under it are resolved.
fn convention_targets(path: &Path, line: &str, col: Option<usize>) -> Vec<(usize, usize, Vec<PathBuf>)> {
    let at = |s: usize, e: usize| col.is_none_or(|c| (s..=e).contains(&c));
    let models = partials::model_renders(line).into_iter().filter(|&(s, e, _)| at(s, e))
        .map(|(s, e, name)| (s, e, partials::resolve_model(path, name)));
    let controllers = stimulus::identifiers(line).into_iter().filter(|&(s, e, _)| at(s, e))
        .map(|(s, e, id)| (s, e, stimulus::resolve(path, &id)));
    let assets = assets::asset_refs(line).into_iter().filter(|r| at(r.start, r.end))
        .map(|r| (r.start, r.end, assets::resolve(path, &r)));
    models.chain(controllers).chain(assets).filter(|(_, _, found)| !found.is_empty()).collect()
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
    sig.locals.into_iter().filter(|(name, _)| name.starts_with(typed)).map(|(name, default)| CompletionItem {
        label: name,
        kind: Some(CompletionItemKind::VARIABLE),
        detail: Some(default.map_or_else(|| "local (required)".to_string(), |d| format!("local = {d}"))),
        ..Default::default()
    }).collect()
}

impl Backend {
    async fn publish(&self, uri: Url) {
        let diags = match self.docs.get(&uri) {
            Some(doc) => diagnostics(&uri, &doc),
            None => vec![],
        };
        self.client.publish_diagnostics(uri, diags, None).await;
    }

    /// The line's byte offset in `text` and its content.
    fn line_at(text: &str, line: u32) -> (usize, &str) {
        util::lines_with_offsets(text).nth(line as usize).unwrap_or((text.len(), ""))
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
        let (_, line) = Self::line_at(&doc.text, pos.line);
        let col = Self::byte_col(line, pos.character);

        let targets: Vec<PathBuf> = if let Some(r) = partials::render_ref_at(line, col) {
            partials::resolve(&path, &r)
        } else if let Some(c) = partials::component_ref_at(line, col) {
            partials::resolve_component(&path, &c)
        } else {
            convention_targets(&path, line, Some(col)).into_iter().next().map(|(_, _, found)| found).unwrap_or_default()
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
            let renders = partials::render_refs(line).into_iter().map(|r| (r.start, r.end, partials::resolve(&path, &r)));
            for (start, end, found) in renders.chain(convention_targets(&path, line, None)) {
                let Some(target) = found.into_iter().next() else { continue };
                let Ok(t) = Url::from_file_path(&target) else { continue };
                links.push(DocumentLink {
                    range: util::span_range(i, line, start, end),
                    target: Some(t),
                    tooltip: Some(target.to_string_lossy().into_owned()),
                    data: None,
                });
            }
        }
        Ok(Some(links))
    }

    async fn hover(&self, p: HoverParams) -> Result<Option<Hover>> {
        let uri = p.text_document_position_params.text_document.uri;
        let pos = p.text_document_position_params.position;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let (_, line) = Self::line_at(&doc.text, pos.line);
        let col = Self::byte_col(line, pos.character);
        let markdown = |value: String| Ok(Some(Hover { contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value }), range: None }));
        if doc.lang == TemplateLang::Erb {
            if let Some(md) = erb::tag_doc(line, col) {
                return markdown(md.to_string());
            }
        }
        if let Ok(path) = uri.to_file_path() {
            if let Some(r) = partials::render_ref_at(line, col) {
                return markdown(render_hover(&path, &r));
            }
        }
        let word = word_at(line, col);
        if let Some(h) = helpers::HELPERS.iter().find(|h| h.name == word) {
            return markdown(helper_hover(h));
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
        let (line_start, line) = Self::line_at(&doc.text, pos.line);
        let col = Self::byte_col(line, pos.character);
        let before = &line[..col.min(line.len())];

        // 1. Partial/template/layout names inside render "..."
        if let Some((kind, prefix)) = partials::render_string_prefix(line, col) {
            let Ok(path) = uri.to_file_path() else { return Ok(None) };
            let items: Vec<CompletionItem> = partials::render_candidates(&path, kind)
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
        if !in_ruby_context(doc.lang, &doc.text, &erb::code_spans(&doc.text), line_start + col) { return Ok(None); }
        let typed = before.rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == '?')).next().unwrap_or("");
        if typed.is_empty() { return Ok(None); }
        let mut items = local_completions(&doc.text, typed);
        items.extend(helpers::HELPERS.iter().filter(|h| h.name.starts_with(typed)).map(|h| CompletionItem {
            label: h.name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(h.detail.to_string()),
            insert_text: Some(h.snippet.to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        }));
        Ok(Some(CompletionResponse::Array(items)))
    }
}

fn kind_name(k: partials::RenderKind) -> &'static str {
    match k { partials::RenderKind::Partial => "Partial", partials::RenderKind::Template => "Template" }
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

/// Is byte `offset` of `text` in Ruby: inside an ERB code tag (`spans` from `erb::code_spans`, may cross lines)
/// or on a Haml/Slim Ruby line.
fn in_ruby_context(lang: TemplateLang, text: &str, spans: &[(usize, usize)], offset: usize) -> bool {
    let offset = offset.min(text.len());
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    match lang {
        // A Ruby `#` comment ends at the newline, so only this line of the tag matters.
        TemplateLang::Erb => erb::span_at(spans, offset).is_some_and(|code| !ruby_comment_before(&text[code.max(line_start)..offset])),
        TemplateLang::Haml | TemplateLang::Slim => {
            let before = &text[line_start..offset];
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
        assert!(helper_hover(link_to).ends_with("[API docs](https://api.rubyonrails.org/classes/ActionView/Helpers/UrlHelper.html#method-i-link_to)"));
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
        let diags = unresolved_renders(TemplateLang::Erb, &file, text, &erb::code_spans(text));
        let got: Vec<(String, Range)> = diags.iter().map(|d| (d.message.clone(), d.range)).collect();
        let at = |l, s, e| Range::new(Position::new(l, s), Position::new(l, e));
        assert_eq!(got, vec![
            ("Partial `users/missing` not found".to_string(), at(0, 42, 55)),
            ("Partial `users/nope` not found".to_string(), at(1, 13, 23)),
            ("Partial `users/frame` not found".to_string(), at(4, 20, 31)),
        ]);
        let card = partials::render_refs(r#"<%= render layout: "card" do %>"#);
        assert_eq!(partials::resolve(&file, &card[0]), vec![users.join("_card.html.erb")]);
        assert!(diags.iter().all(|d| d.severity == Some(DiagnosticSeverity::WARNING)));

        let layouts = root.join("app/views/layouts");
        std::fs::create_dir_all(&layouts).unwrap();
        assert!(unresolved_renders(TemplateLang::Erb, &layouts.join("application.html.erb"), "<%= render \"header\" %>", &erb::code_spans("<%= render \"header\" %>")).is_empty());
        assert!(unresolved_renders(TemplateLang::Erb, Path::new("/tmp/x.html.erb"), "<%= render \"users/nope\" %>", &erb::code_spans("<%= render \"users/nope\" %>")).is_empty());
        let haml = users.join("show.html.haml");
        assert!(unresolved_renders(TemplateLang::Haml, &haml, "%p (we render \"users/x\")", &[]).is_empty());
        assert_eq!(unresolved_renders(TemplateLang::Haml, &haml, "= render \"users/x\"", &[]).len(), 1);
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
        let md = render_hover(&file, &partials::render_ref_at(line, 22).unwrap());
        assert!(md.ends_with("```ruby\nlocals: (user:, size: :md)\n```"), "{md}");
        assert!(!render_hover(&file, &partials::render_ref_at(r#"render "plain""#, 9).unwrap()).contains("locals:"));

        let items: Vec<(String, Option<String>)> = local_completions(card, "").into_iter().map(|c| (c.label, c.detail)).collect();
        assert_eq!(items, vec![
            ("user".to_string(), Some("local (required)".to_string())),
            ("size".to_string(), Some("local = :md".to_string())),
        ]);
        assert_eq!(local_completions(card, "si").len(), 1);
        assert!(local_completions("<p>", "").is_empty());

        let text = concat!(
            "<%= render \"users/card\", user: u %> <%= render \"users/plain\", x: 1 %> <%= render \"users/any\", user: u, x: 1 %>\n",
            "<%= render \"users/card\", size: :lg, colour: 1 %>\n",
            "<%= render partial: \"users/card\", locals: { user: u } %> <%= render \"users/card\", **opts %>\n",
            "<%= render \"users/card\", Card::DEFAULTS %> <%= render \"card\" %> <%= render \"users/multi\" %>\n",
            "<%= render partial: \"users/plain\", locals: { a: 1 }, layout: \"users/card\" %>\n",
        );
        let got: Vec<(String, Range)> = strict_locals_mismatches(TemplateLang::Erb, &file, text, &erb::code_spans(text)).into_iter().map(|d| (d.message, d.range)).collect();
        let at = |l, s, e| Range::new(Position::new(l, s), Position::new(l, e));
        assert_eq!(got, vec![
            ("Partial `users/card` requires locals: `user`".to_string(), at(1, 12, 22)),
            ("Partial `users/card` does not accept locals: `colour`".to_string(), at(1, 12, 22)),
        ]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
