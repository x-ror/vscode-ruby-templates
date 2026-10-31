//! Completion items for the contexts the language server recognises: render names, i18n keys, Stimulus values,
//! instance variables, component slots and classes, receivers' methods, helpers and partial locals.

use crate::lang::TemplateLang;
use crate::partials::RenderRef;
use crate::project::Project;
use crate::strict_locals::Close;
use crate::{helpers, i18n, ivars, partials, slots, stimulus, util};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, CompletionTextEdit, InsertTextFormat, Range, TextEdit};

pub(crate) fn block_local_completions(names: Vec<String>, typed: &str) -> Vec<CompletionItem> {
    names
        .into_iter()
        .filter(|n| n.starts_with(typed))
        .map(|n| util::completion_item(n, CompletionItemKind::VARIABLE, "block variable"))
        .collect()
}

/// The current template's own strict locals, as completions.
pub(crate) fn local_completions(text: &str, typed: &str, close: Close) -> Vec<CompletionItem> {
    let Some(sig) = partials::strict_locals(text, close) else { return vec![] };
    sig.locals.into_iter().filter(|(name, _)| name.starts_with(typed)).map(|(name, default)| local_item(name, default)).collect()
}

/// The strict locals of the partial `r` renders that the call doesn't pass yet, inserted as `name: `.
pub(crate) fn render_local_completions(
    project: &Project,
    path: &Path,
    r: &RenderRef,
    passed: &[String],
    typed: &str,
) -> Vec<CompletionItem> {
    let close = Close::of(project);
    let Some(sig) = partials::resolve(project, path, r).first().and_then(|file| partials::file_strict_locals(file, close)) else {
        return vec![];
    };
    sig.locals
        .into_iter()
        .filter(|(name, _)| name.starts_with(typed) && !passed.contains(name))
        .map(|(name, default)| CompletionItem { insert_text: Some(format!("{name}: ")), ..local_item(name, default) })
        .collect()
}

fn local_item(name: String, default: Option<String>) -> CompletionItem {
    let detail = default.map_or_else(|| "local (required)".to_string(), |d| format!("local = {d}"));
    util::completion_item(name, CompletionItemKind::VARIABLE, detail)
}

/// Partial, template or layout names for the `render "…"` string being typed.
pub(crate) fn render_completions(
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
pub(crate) fn i18n_completions(project: &Project, locales: &i18n::Locales, path: &Path, prefix: &str, range: Range) -> Vec<CompletionItem> {
    i18n::completions(project, locales, path, prefix)
        .into_iter()
        .map(|(key, value)| CompletionItem {
            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(range, key.clone()))),
            ..util::completion_item(key, CompletionItemKind::CONSTANT, value)
        })
        .collect()
}

/// Stimulus controllers, or a controller's action methods or targets, for a `data-*` value; each replaces `range`.
pub(crate) fn stimulus_completions(path: &Path, typed: &str, slot: stimulus::Slot, range: Range) -> Vec<CompletionItem> {
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
pub(crate) fn ivar_completions(project: &Project, path: &Path, typed: &str, range: Range) -> Vec<CompletionItem> {
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
pub(crate) fn slot_setter_completions(project: &Project, path: &Path, before: &str) -> Option<Vec<CompletionItem>> {
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
pub(crate) fn component_prefix(before: &str) -> Option<&str> {
    static OPEN: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\brender\s*\(?\s*((?:[A-Z]\w*::)*[A-Z]\w*)$").unwrap());
    Some(OPEN.captures(before)?.get(1)?.as_str())
}

/// ViewComponent classes matching `typed`, inserted as `Name.new(…)`.
pub(crate) fn component_completions(path: &Path, typed: &str, range: Range) -> Vec<CompletionItem> {
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
pub(crate) fn receiver_completions(before: &str) -> Option<Vec<CompletionItem>> {
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

pub(crate) fn helper_item(h: &helpers::Helper) -> CompletionItem {
    CompletionItem {
        insert_text: Some(h.snippet.to_string()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..util::completion_item(h.name, CompletionItemKind::FUNCTION, h.detail)
    }
}

pub(crate) fn helpers_for(lang: TemplateLang) -> impl Iterator<Item = &'static helpers::Helper> {
    let haml_only = if lang == TemplateLang::Haml { helpers::HAML_HELPERS } else { &[] };
    helpers::HELPERS.iter().chain(haml_only)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_setters_leave_other_receivers_alone() {
        let before = "<%= render CardComponent.new do |f| %><% end %>\n<%= form_with model: @user do |f| %>\n  <%= f.te";
        assert!(slot_setter_completions(&Project::unknown(), Path::new("/nowhere/app/views/users/new.html.erb"), before).is_none());
        assert!(receiver_completions(before).is_some_and(|items| !items.is_empty()));
    }

    #[test]
    fn local_completions_list_strict_locals() {
        let card = "<%# locals: (user:, size: :md) -%>\n<p><%= user.name %></p>\n";
        let items: Vec<(String, Option<String>)> =
            local_completions(card, "", Close::default()).into_iter().map(|c| (c.label, c.detail)).collect();
        assert_eq!(
            items,
            vec![("user".to_string(), Some("local (required)".to_string())), ("size".to_string(), Some("local = :md".to_string())),]
        );
        assert_eq!(local_completions(card, "si", Close::default()).len(), 1);
        assert!(local_completions("<p>", "", Close::default()).is_empty());
    }
}
