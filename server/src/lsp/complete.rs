//! Completion in the language server: render names, and what fits at the cursor in a template.

use super::Backend;
use crate::checks::Doc;
use crate::code_context::{block_locals, code_at, in_ruby_context, line_roles};
use crate::completion::{
    block_local_completions, component_completions, component_prefix, has_arguments, helper_item, helpers_for, i18n_completions,
    ivar_completions, local_completions, receiver_completions, render_completions, render_local_completions, slot_setter_completions,
    stimulus_completions,
};
use crate::haml_indent::LineRole;
use crate::lang::TemplateLang;
use crate::project::Project;
use crate::{
    erb, erb_complete, haml, i18n, rails_paths, render_refs, routes, ruby, schema, slim_complete, slim_parser, slots, stimulus, util,
};
use std::cell::OnceCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;

impl Backend {
    /// The partials and templates a `render` string in `path` can name, this app's and its engines'.
    pub(super) async fn render_path_completions(
        &self,
        path: &Path,
        kind: render_refs::RenderKind,
        prefix: &str,
    ) -> Result<Vec<CompletionItem>> {
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
    pub(super) fn completions_at(
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
        let render_key = render_refs::local_key_before(&doc.text, line_start + col, doc.lang, ruby)
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
}
