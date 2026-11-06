//! Partial locals in the language server: where a local comes from, who passes it, the quick fixes for a render's
//! strict-locals mismatch, and the rename and references of a strict local.

use super::{is_builtin_helper, off_server_task, Backend};
use crate::checks::Doc;
use crate::lang::is_partial;
use crate::rails::project::Project;
use crate::rails::render_call::HamlPipes;
use crate::rails::{helper_defs, ivars, partials, paths, render_refs};
use crate::ruby::strict_locals;
use crate::util::line_location;
use crate::{code_context, partial_rename, ruby, util};
use std::collections::HashMap;
use std::path::Path;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;

/// A partial's strict local, and where the partial declares, reads and assigns it (template byte ranges).
pub(super) struct StrictLocal {
    pub(super) name: String,
    /// The span under the cursor.
    pub(super) range: Range,
    pub(super) declaration: Option<(usize, usize)>,
    pub(super) uses: Vec<(usize, usize)>,
}

/// A partial's strict local: required or its default, and how many `render` calls pass it.
pub(super) fn local_hover(name: &str, default: Option<&str>, passes: usize) -> String {
    let declared = default.map_or_else(|| "required".to_string(), |d| format!("default {}", util::inline_code(d)));
    let passed = match passes {
        0 => "No `render` passes it".to_string(),
        1 => "Passed by 1 `render` call".to_string(),
        n => format!("Passed by {n} `render` calls"),
    };
    format!("`{name}`: local, {declared}\n\n{passed}")
}

/// Whether `name` can be a Ruby local variable's name.
fn is_local_name(name: &str) -> bool {
    helper_defs::name_at(name, 0) == Some(name) && !name.ends_with(['?', '!']) && !ruby::KEYWORDS.contains(&name)
}

impl Backend {
    /// Where the `@ivar` or partial local under `col` comes from: its assignments in the controller, or the
    /// `render` calls passing the local (else the strict-locals comment declaring it).
    pub(super) async fn variable_definitions(&self, text: &str, path: &Path, line: &str, col: usize) -> Result<Vec<Location>> {
        if let Some(name) = ivars::name_at(line, col) {
            return Ok(ivars::for_view(&self.project_for(path), path)
                .into_iter()
                .filter(|a| a.name == name)
                .filter_map(|a| line_location(&a.file, a.line))
                .collect());
        }
        let is_partial = paths::is_partial_path(path);
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
        let Some(views) = paths::views_root(path) else { return Ok(vec![]) };
        let (open, partial, local) = (self.open_texts(), path.to_path_buf(), local.to_string());
        let extra_engines = self.engines_at(path);
        off_server_task(move || {
            let sources = partial_rename::sources(&views, &open);
            partial_rename::local_passes(&project, &partial, &local, &sources, &extra_engines)
        })
        .await
    }

    /// Where the template `uri` (`doc`) defines the local `name` read at `offset`: a block variable or an assignment.
    pub(super) async fn local_definition_range(
        &self,
        uri: &Uri,
        project: &Project,
        doc: &Doc,
        offset: usize,
        name: &str,
    ) -> Result<Option<Range>> {
        let text = doc.text.as_str();
        let (lang, source, engines) = (doc.lang, text.to_string(), self.engines(uri));
        let found = off_server_task(move || code_context::local_definition(lang, &source, offset, &engines)).await?;
        // A partial's strict local that the template also assigns (`user ||= …`) is defined by its callers.
        let strict_local = || is_partial(uri) && project.strict_locals(text).is_some_and(|sig| sig.declares(name));
        let Some(found) = found.filter(|found| found.parameter || !strict_local()) else { return Ok(None) };
        let index = util::LineIndex::new(text);
        Ok(Some(index.range(found.start, found.end)))
    }

    /// `local_passes` counted, from the `local_callers` cache.
    pub(super) async fn local_caller_count(&self, project: Project, path: &Path, local: &str) -> Result<usize> {
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
    pub(super) fn strict_locals_fixes(&self, project: &Project, path: &Path, doc: &Doc, d: &Diagnostic) -> Vec<CodeAction> {
        let text = &doc.text;
        let (line_start, line, start) = util::line_at(text, d.range.start);
        let pipes = HamlPipes::new(doc.lang);
        let Some((r, call)) =
            render_refs::render_calls_in(text, line_start, line, &pipes, project.prism_version(), |r| r.start == start).pop()
        else {
            return vec![];
        };
        let Some(passed) = render_refs::passed_locals(&call, &r) else { return vec![] };
        let Some(uri) = crate::util::file_uri(path) else { return vec![] };
        // Several variants or formats: which one Rails picks depends on the request, as the check says.
        let mut candidates = Vec::new();
        for partial in partials::first_folder_only(partials::resolve(project, path, &r)) {
            let Some(partial_uri) = crate::util::file_uri(&partial) else { return vec![] };
            let text = util::open_or_read(self.docs.get(&partial_uri).map(|doc| doc.text.clone()), &partial).unwrap_or_default();
            let (Some(sig), Some(comment)) = (project.strict_locals(&text), strict_locals::find(&text, project.strict_locals_close()))
            else {
                return vec![];
            };
            candidates.push((partial, partial_uri, text, sig, comment));
        }
        let sigs: Vec<&strict_locals::Signature> = candidates.iter().map(|(.., sig, _)| sig).collect();
        let missing = strict_locals::missing_in_all(&sigs, &passed);
        let unknown = strict_locals::unknown_to_all(&sigs, &passed);
        let base = call.base(line_start);
        let at = |text: &str, offset: usize| util::offset_to_position(text, offset);
        let here = |(range, new): render_refs::ByteEdit| {
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
        if let Some(edit) = (!missing.is_empty()).then(|| render_refs::add_locals_edit(&call, &r, &missing)).flatten() {
            // Not preferred: `nil` silences the warning without passing what the partial needs.
            out.push(action(format!("Pass {} to the render", quoted(&missing)), here(edit), false));
        }
        for name in unknown {
            if let Some(edit) = render_refs::remove_local_edit(&call, &r, name) {
                out.push(action(format!("Remove `{name}` from the render"), here(edit), false));
            }
            // Every candidate, or the variant left alone still raises the warning.
            let mut changes = HashMap::new();
            for (_, partial_uri, partial_text, _, comment) in &candidates {
                let (range, new) = comment.declare(name, render_refs::NEW_LOCAL_VALUE);
                let range = Range::new(at(partial_text, range.start), at(partial_text, range.end));
                changes.insert(partial_uri.clone(), vec![TextEdit::new(range, new)]);
            }
            let first = &candidates[0].0;
            let shown = first.file_name().map_or_else(String::new, |f| f.to_string_lossy().into_owned());
            let others = partials::and_others(candidates.len() - 1);
            out.push(action(format!("Declare `{name}` in the locals of `{shown}`{others}"), changes, false));
        }
        out
    }

    /// The strict local under `pos` of the partial `uri` and its word's range: a read in code, or its parameter in the
    /// `locals:` comment; not a hash key of that name, nor a block variable hiding it.
    pub(super) fn strict_local_at(&self, uri: &Uri, doc: &Doc, pos: Position) -> Option<StrictLocal> {
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
        uses.extend(code_context::local_uses(doc.lang, &doc.text, close, name, &self.engines(uri)).spans);
        let &(start, end) = declaration.iter().chain(&uses).find(|&&(start, _)| start == at)?;
        let index = util::LineIndex::new(&doc.text);
        Some(StrictLocal { name: name.to_string(), range: index.range(start, end), declaration, uses })
    }

    /// Where the partial `path` (text `text`) declares and uses `local`, then the `render` calls passing it.
    pub(super) async fn local_references(&self, path: &Path, text: &str, local: &StrictLocal, declaration: bool) -> Result<Vec<Location>> {
        let Some(uri) = crate::util::file_uri(path) else { return Ok(vec![]) };
        let index = util::LineIndex::new(text);
        let span = |(start, end): (usize, usize)| Location::new(uri.clone(), index.range(start, end));
        let declared = local.declaration.filter(|_| declaration);
        let mut out: Vec<Location> = declared.into_iter().chain(local.uses.iter().copied()).map(span).collect();
        out.extend(self.local_passes(self.project_for(path), path, &local.name).await?);
        Ok(out)
    }

    /// Whether a partial's strict local can take the name `new`: a local's name nothing in the partial declares, reads or
    /// calls, so no read of the renamed local means something else.
    pub(super) fn free_local_name(&self, uri: &Uri, doc: &Doc, project: &Project, new: &str) -> bool {
        let close = project.strict_locals_close();
        let declared = project.strict_locals(&doc.text).is_some_and(|sig| sig.declares(new));
        let in_defaults = strict_locals::find(&doc.text, close).is_some_and(|comment| !comment.parameter_reads(new).is_empty());
        let uses = code_context::local_uses(doc.lang, &doc.text, close, new, &self.engines(uri));
        is_local_name(new) && !declared && !in_defaults && uses.spans.is_empty() && !uses.shadowed
    }
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
}
