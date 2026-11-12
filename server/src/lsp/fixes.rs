//! Quick fixes for the rules that already know the answer: the name or text a diagnostic carries replaces what it flags, an
//! attribute is added, a translation key, a controller, a target or a method is created. What a fix needs travels in the
//! diagnostic's `data`, set where the rule reports.

use super::{off_server_task, Backend};
use crate::checks::Doc;
use crate::erb::html_lint::code::IMG_MISSING_ALT;
use crate::rails::attribute_lints::{PERMANENT_REQUIRES_ID, UNKNOWN_CONTROLLER, UNKNOWN_TARGET_OR_ACTION};
use crate::rails::missing_translation::I18N_MISSING_KEY;
use crate::rails::project::Project;
use crate::rails::unknown_route::UNKNOWN_ROUTE;
use crate::rails::{i18n, locale_edit, paths, stimulus_edit};
use crate::util::{self, data_edit, data_str, LineIndex};
use std::collections::HashMap;
use std::path::Path;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;

/// The empty value a new translation gets, for the user to fill in.
const NEW_TRANSLATION: &str = "\"\"";

/// Whether the quick fixes of `d` read files, which is done off the server task.
fn reads_files(d: &Diagnostic) -> bool {
    matches!(util::code(d), Some(I18N_MISSING_KEY | UNKNOWN_CONTROLLER | UNKNOWN_TARGET_OR_ACTION))
}

fn quick_fix(title: String, d: &Diagnostic, edit: WorkspaceEdit, preferred: bool) -> CodeAction {
    CodeAction {
        title,
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![d.clone()]),
        edit: Some(edit),
        is_preferred: Some(preferred),
        ..Default::default()
    }
}

fn changes_in(uri: &Uri, edits: Vec<TextEdit>) -> WorkspaceEdit {
    WorkspaceEdit { changes: Some(HashMap::from([(uri.clone(), edits)])), ..Default::default() }
}

/// The fixes that edit the document itself: the name a rule suggests, in place of what it flagged, and the attribute a rule adds.
pub(super) fn document_fixes(uri: &Uri, doc: &Doc, diagnostics: &[Diagnostic]) -> Vec<CodeActionOrCommand> {
    let replace = |d: &Diagnostic, title: String, new_text: String| {
        quick_fix(title, d, changes_in(uri, vec![TextEdit::new(d.range, new_text)]), true)
    };
    diagnostics
        .iter()
        .filter_map(|d| {
            let action = match util::code(d)? {
                UNKNOWN_ROUTE => {
                    let route = data_str(d, "replace")?;
                    replace(d, format!("Replace with `{route}`"), route.to_string())
                }
                I18N_MISSING_KEY => {
                    let key = data_str(d, "suggest")?;
                    let range = util::position_to_offset(&doc.text, d.range.start)?..util::position_to_offset(&doc.text, d.range.end)?;
                    let quote = doc.text.get(range)?.chars().next().filter(|quote| matches!(quote, '"' | '\''))?;
                    replace(d, format!("Use `{key}`"), format!("{quote}{key}{quote}"))
                }
                UNKNOWN_CONTROLLER | UNKNOWN_TARGET_OR_ACTION => {
                    let name = data_str(d, "suggest")?;
                    replace(d, format!("Use `{name}`"), name.to_string())
                }
                IMG_MISSING_ALT => {
                    let edit = data_edit(d)?;
                    let title = format!("Add `{}`", edit.new_text.trim_matches([',', ' ', '(', ')']));
                    quick_fix(title, d, changes_in(uri, vec![edit]), false)
                }
                PERMANENT_REQUIRES_ID => quick_fix("Add `id`".into(), d, changes_in(uri, vec![data_edit(d)?]), false),
                _ => return None,
            };
            Some(CodeActionOrCommand::CodeAction(action))
        })
        .collect()
}

impl Backend {
    /// The fixes that read or write other files: a translation key in a locale file, a new controller, a target or a method in
    /// a controller. Reads the files off the server task.
    pub(super) async fn file_fixes(&self, path: &Path, diagnostics: &[Diagnostic]) -> Result<Vec<CodeActionOrCommand>> {
        if !diagnostics.iter().any(reads_files) {
            return Ok(vec![]);
        }
        let (path, project, can_create) = (path.to_path_buf(), self.project_for(path), self.can_create_files());
        let diagnostics: Vec<Diagnostic> = diagnostics.iter().filter(|d| reads_files(d)).cloned().collect();
        off_server_task(move || {
            let fixes = diagnostics.iter().filter_map(|d| match util::code(d)? {
                I18N_MISSING_KEY => add_translation(&project, &path, d, can_create),
                UNKNOWN_CONTROLLER => create_controller(&path, d, can_create),
                _ => add_member(&path, d),
            });
            fixes.map(CodeActionOrCommand::CodeAction).collect()
        })
        .await
    }

    /// The client takes files created and edited in one workspace edit.
    fn can_create_files(&self) -> bool {
        self.file_operations.get() == Some(&true)
    }
}

/// "Add `users.index.title` to config/locales/en.yml": the key goes into the file that holds the closest group of it, else
/// into `<locale>.yml`, which is created when it isn't there.
fn add_translation(project: &Project, path: &Path, d: &Diagnostic, can_create: bool) -> Option<CodeAction> {
    let (key, locale) = (data_str(d, "key")?, data_str(d, "locale")?);
    let segments: Vec<&str> = key.split('.').collect();
    let catalog = i18n::catalog(project, path).filter(|catalog| catalog.default_locale() == locale)?;
    let file = match catalog.file_near(key) {
        Some(file) => file.to_path_buf(),
        None => i18n::locales_dir(path)?.join(format!("{locale}.yml")),
    };
    let (text, creates) = match std::fs::read_to_string(&file) {
        Ok(text) => (text, false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && can_create => (String::new(), true),
        Err(_) => return None,
    };
    let body = text.strip_prefix(util::BOM).unwrap_or(&text);
    let keys: Vec<&str> = std::iter::once(locale).chain(segments).collect();
    if keys.iter().any(|name| name.is_empty()) {
        return None;
    }
    let (offset, inserted) = locale_edit::add_key(body, &keys, NEW_TRANSLATION)?;
    let edit = TextEdit::new(LineIndex::new(body).range(offset, offset), inserted);
    let title = format!("Add `{key}` to {}", paths::rails_relative(&file, path));
    let edit = if creates {
        files_edit(vec![Change::Create(&file), Change::Edit(&file, vec![edit])])?
    } else {
        changes_in(&util::file_uri(&file)?, vec![edit])
    };
    Some(quick_fix(title, d, edit, false))
}

/// "Create app/javascript/controllers/serch_controller.js", with its lines in the manifest of an app that lists its controllers.
fn create_controller(path: &Path, d: &Diagnostic, can_create: bool) -> Option<CodeAction> {
    if !can_create {
        return None;
    }
    let new = stimulus_edit::new_controller(path, data_str(d, "controller")?)?;
    let stub = TextEdit::new(Range::default(), stimulus_edit::CONTROLLER_STUB.into());
    let mut changes = vec![Change::Create(&new.file), Change::Edit(&new.file, vec![stub])];
    if let Some(manifest) = &new.manifest {
        let index = LineIndex::new(&manifest.source);
        let edits = manifest.insertions.iter().map(|(offset, text)| TextEdit::new(index.range(*offset, *offset), text.clone()));
        changes.push(Change::Edit(&manifest.file, edits.collect()));
    }
    Some(quick_fix(format!("Create {}", paths::rails_relative(&new.file, path)), d, files_edit(changes)?, false))
}

/// "Add `results` to static targets" and "Add method `submit()`" in the controller the template names, when it is read with confidence.
fn add_member(path: &Path, d: &Diagnostic) -> Option<CodeAction> {
    let (controller, name) = (data_str(d, "controller")?, data_str(d, "name")?);
    let file = stimulus_edit::editable_controller(path, controller)?;
    let source = std::fs::read_to_string(&file).ok()?;
    let shown = paths::rails_relative(&file, path);
    let (title, insertions) = match data_str(d, "kind")? {
        "target" => (format!("Add `{name}` to static targets in {shown}"), stimulus_edit::add_target(&source, name)?),
        "method" => (format!("Add method `{name}()` to {shown}"), vec![stimulus_edit::add_method(&source, name)?]),
        _ => return None,
    };
    let index = LineIndex::new(&source);
    let edits = insertions.into_iter().map(|(offset, text)| TextEdit::new(index.range(offset, offset), text)).collect();
    Some(quick_fix(title, d, changes_in(&util::file_uri(&file)?, edits), false))
}

enum Change<'a> {
    Create(&'a Path),
    Edit(&'a Path, Vec<TextEdit>),
}

/// A workspace edit that creates and edits files, in this order.
fn files_edit(changes: Vec<Change>) -> Option<WorkspaceEdit> {
    let operations = changes
        .into_iter()
        .map(|change| {
            Some(match change {
                Change::Create(file) => {
                    let options = CreateFileOptions { overwrite: Some(false), ignore_if_exists: Some(false) };
                    DocumentChangeOperation::Op(ResourceOp::Create(CreateFile {
                        uri: util::file_uri(file)?,
                        options: Some(options),
                        annotation_id: None,
                    }))
                }
                Change::Edit(file, edits) => DocumentChangeOperation::Edit(TextDocumentEdit {
                    text_document: OptionalVersionedTextDocumentIdentifier { uri: util::file_uri(file)?, version: None },
                    edits: edits.into_iter().map(OneOf::Left).collect(),
                }),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(WorkspaceEdit { document_changes: Some(DocumentChanges::Operations(operations)), ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::TemplateLang;
    use serde_json::json;

    fn diagnostic(code: &str, range: Range, data: serde_json::Value) -> Diagnostic {
        util::with_data(util::coded(util::warning(range, "x"), code), data)
    }

    fn range(line: u32, from: u32, to: u32) -> Range {
        Range::new(Position::new(line, from), Position::new(line, to))
    }

    fn doc(text: &str) -> Doc {
        Doc { lang: TemplateLang::Erb, plain_erb: true, percent_lines: false, text: text.into(), version: 1 }
    }

    /// The title, preferred flag and the result of the edit of each action offered for `diagnostics` in `text`.
    fn fixed(text: &str, diagnostics: &[Diagnostic]) -> Vec<(String, bool, String)> {
        let uri: Uri = "file:///app/views/x.html.erb".parse().unwrap();
        let actions = document_fixes(&uri, &doc(text), diagnostics);
        actions
            .into_iter()
            .map(|action| {
                let CodeActionOrCommand::CodeAction(action) = action else { panic!("a command") };
                let edits = action.edit.unwrap().changes.unwrap().remove(&uri).unwrap();
                assert_eq!(action.diagnostics.as_ref().map(Vec::len), Some(1));
                assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
                (action.title, action.is_preferred == Some(true), util::edited(text, &edits[0]))
            })
            .collect()
    }

    #[test]
    fn names_the_rules_suggest_replace_what_they_flag() {
        let route = diagnostic("Rails/UnknownRoute", range(0, 15, 24), json!({ "replace": "users_path" }));
        assert_eq!(
            fixed("<%= link_to x, usrs_path %>", &[route]),
            [("Replace with `users_path`".into(), true, "<%= link_to x, users_path %>".into())]
        );
        let double = diagnostic("Rails/I18nMissingKey", range(0, 6, 14), json!({ "key": "a.titel", "locale": "en", "suggest": ".title" }));
        assert_eq!(fixed("<%= t(\".titel\") %>", &[double]), [("Use `.title`".into(), true, "<%= t(\".title\") %>".into())]);
        let single = diagnostic("Rails/I18nMissingKey", range(0, 6, 14), json!({ "suggest": ".title" }));
        assert_eq!(fixed("<%= t('.titel') %>", &[single])[0].2, "<%= t('.title') %>");
        let controller = diagnostic("Stimulus/UnknownController", range(0, 22, 27), json!({ "controller": "serch", "suggest": "search" }));
        assert_eq!(
            fixed("<div data-controller=\"serch\">", &[controller]),
            [("Use `search`".into(), true, "<div data-controller=\"search\">".into())]
        );
        let target = diagnostic("Stimulus/UnknownTargetOrAction", range(0, 23, 30), json!({ "kind": "target", "suggest": "results" }));
        assert_eq!(fixed("<p data-search-target=\"resluts\">", &[target])[0].2, "<p data-search-target=\"results\">");
    }

    #[test]
    fn nothing_is_replaced_without_a_name_to_use() {
        let none = |code: &str, data: serde_json::Value| fixed("<%= t(\".titel\") %>", &[diagnostic(code, range(0, 6, 14), data)]);
        assert!(none("Rails/UnknownRoute", json!({})).is_empty());
        assert!(none("Rails/I18nMissingKey", json!({ "suggest": null })).is_empty());
        assert!(none("Stimulus/UnknownController", json!({ "suggest": null })).is_empty());
        assert!(none("Stimulus/UnknownTargetOrAction", json!({ "controller": "search" })).is_empty());
        // The key isn't a string literal there: no quote to keep.
        assert!(
            fixed("<%= t(:titel) %>", &[diagnostic("Rails/I18nMissingKey", range(0, 6, 12), json!({ "suggest": ".title" }))]).is_empty()
        );
        assert!(none("Html/ImgAlt", json!({})).is_empty());
        assert!(none("Turbo/PermanentRequiresId", json!(null)).is_empty());
    }

    #[test]
    fn attributes_are_added_by_the_edit_the_rule_made() {
        let edit = |at: u32, new_text: &str| json!({ "edit": TextEdit::new(range(0, at, at), new_text.into()) });
        let img = diagnostic("Html/ImgAlt", range(0, 1, 5), edit(4, " alt=\"\""));
        assert_eq!(fixed("<img src=x>", &[img]), [("Add `alt=\"\"`".into(), false, "<img alt=\"\" src=x>".into())]);
        let image_tag = diagnostic("Html/ImgAlt", range(0, 4, 13), edit(21, ", alt: \"\""));
        assert_eq!(fixed("<%= image_tag \"a.png\" %>", &[image_tag])[0].0, "Add `alt: \"\"`");
        let haml = diagnostic("Html/ImgAlt", range(0, 1, 4), edit(4, "(alt=\"\")"));
        assert_eq!(fixed("%img", &[haml])[0].0, "Add `alt=\"\"`");
        let permanent = diagnostic("Turbo/PermanentRequiresId", range(0, 5, 25), edit(4, " id=\"\""));
        assert_eq!(
            fixed("<div data-turbo-permanent>", &[permanent]),
            [("Add `id`".into(), false, "<div id=\"\" data-turbo-permanent>".into())]
        );
    }

    /// An app with `files`, and the path of its view.
    fn app(files: &[(&str, &str)]) -> (std::path::PathBuf, std::path::PathBuf) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rt-fixes-{}-{n}", std::process::id()));
        util::write_files(&root, files);
        let view = root.join("app/views/users/index.html.erb");
        (root, view)
    }

    fn missing_key(key: &str) -> Diagnostic {
        diagnostic("Rails/I18nMissingKey", Range::default(), json!({ "key": key, "locale": "en", "suggest": null }))
    }

    /// The files an action edits, with what the edit made of each one's text, and the files it creates.
    fn effects(action: &CodeAction, root: &Path) -> (Vec<(String, String)>, Vec<String>) {
        let shown = |uri: &Uri| util::slash_path(util::file_path(uri).unwrap().strip_prefix(root).unwrap());
        let edit = action.edit.clone().unwrap();
        let (mut edited, mut created) = (Vec::new(), Vec::new());
        let mut on_disk = |uri: &Uri, edits: &[TextEdit], base: Option<String>| {
            let text = base.unwrap_or_else(|| std::fs::read_to_string(util::file_path(uri).unwrap()).unwrap_or_default());
            // Edits are all for the original text: from the last one back.
            let mut sorted: Vec<&TextEdit> = edits.iter().collect();
            sorted.sort_by_key(|edit| std::cmp::Reverse((edit.range.start.line, edit.range.start.character)));
            edited.push((shown(uri), sorted.into_iter().fold(text, |text, edit| util::edited(&text, edit))));
        };
        for (uri, edits) in edit.changes.iter().flatten() {
            on_disk(uri, edits, None);
        }
        let Some(DocumentChanges::Operations(operations)) = edit.document_changes else { return (edited, created) };
        for operation in operations {
            match operation {
                DocumentChangeOperation::Op(ResourceOp::Create(create)) => created.push(shown(&create.uri)),
                DocumentChangeOperation::Edit(edit) => {
                    let edits: Vec<TextEdit> = edit
                        .edits
                        .into_iter()
                        .map(|edit| match edit {
                            OneOf::Left(edit) => edit,
                            OneOf::Right(annotated) => annotated.text_edit,
                        })
                        .collect();
                    let created_here = created.contains(&shown(&edit.text_document.uri));
                    on_disk(&edit.text_document.uri, &edits, created_here.then(String::new));
                }
                _ => panic!("another operation"),
            }
        }
        (edited, created)
    }

    #[test]
    fn a_translation_goes_into_the_file_with_the_closest_group_of_its_key() {
        let (root, view) = app(&[
            ("config/application.rb", ""),
            ("config/locales/en.yml", "en:\n  layouts:\n    menu: Menu\n"),
            ("config/locales/models/users.en.yml", "en:\n  users:\n    show:\n      name: Name\n"),
            ("config/locales/fr.yml", "fr:\n  users:\n    index:\n      title: Titre\n"),
        ]);
        let project = Project::unknown();
        let action = |key: &str, can_create| add_translation(&project, &view, &missing_key(key), can_create);
        let sibling = action("users.show.age", false).unwrap();
        assert_eq!(sibling.title, "Add `users.show.age` to config/locales/models/users.en.yml");
        assert_eq!(sibling.is_preferred, Some(false));
        assert_eq!(
            effects(&sibling, &root),
            (
                vec![("config/locales/models/users.en.yml".into(), "en:\n  users:\n    show:\n      name: Name\n      age: \"\"\n".into())],
                vec![]
            )
        );
        let nested = action("users.edit.title", false).unwrap();
        assert_eq!(effects(&nested, &root).0[0].1, "en:\n  users:\n    show:\n      name: Name\n    edit:\n      title: \"\"\n");
        let other = action("layouts.footer", false).unwrap();
        assert_eq!(
            effects(&other, &root).0[0],
            ("config/locales/en.yml".into(), "en:\n  layouts:\n    menu: Menu\n    footer: \"\"\n".into())
        );
        // No group of the key has an entry: the default locale's own file, here `en.yml`, which exists.
        assert_eq!(
            effects(&action("pages.home.title", false).unwrap(), &root).0[0].1,
            "en:\n  layouts:\n    menu: Menu\n  pages:\n    home:\n      title: \"\"\n"
        );
        assert!(action("users..title", false).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_translation_creates_the_default_locales_file_when_there_is_none() {
        let (root, view) =
            app(&[("config/application.rb", ""), ("config/locales/models/users.en.yml", "en:\n  users:\n    show:\n      name: Name\n")]);
        let project = Project::unknown();
        let action = |can_create| add_translation(&project, &view, &missing_key("pages.home.title"), can_create);
        assert!(action(false).is_none(), "the client can't create files");
        let created = action(true).unwrap();
        assert_eq!(created.title, "Add `pages.home.title` to config/locales/en.yml");
        assert_eq!(
            effects(&created, &root),
            (
                vec![("config/locales/en.yml".into(), "en:\n  pages:\n    home:\n      title: \"\"\n".into())],
                vec!["config/locales/en.yml".into()]
            )
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_translation_is_not_added_where_the_file_cannot_take_it() {
        let (root, view) = app(&[("config/application.rb", ""), ("config/locales/en.yml", "en:\n  users:\n    index: { title: Users }\n")]);
        let project = Project::unknown();
        assert!(add_translation(&project, &view, &missing_key("users.index.heading"), true).is_none());
        let other_locale = diagnostic("Rails/I18nMissingKey", Range::default(), json!({ "key": "a.b", "locale": "fr" }));
        assert!(add_translation(&project, &view, &other_locale, true).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn unknown_controller(name: &str) -> Diagnostic {
        diagnostic("Stimulus/UnknownController", Range::default(), json!({ "controller": name, "suggest": null }))
    }

    #[test]
    fn a_controller_is_created_with_its_stub_and_its_manifest_lines() {
        let manifest = "import { application } from \"./application\"\nimport HelloController from \"./hello_controller\"\napplication.register(\"hello\", HelloController)\n";
        let (root, view) = app(&[
            ("app/javascript/controllers/hello_controller.js", "export default class extends Controller {\n}\n"),
            ("app/javascript/controllers/index.js", manifest),
        ]);
        assert!(create_controller(&view, &unknown_controller("serch"), false).is_none(), "the client can't create files");
        let action = create_controller(&view, &unknown_controller("serch"), true).unwrap();
        assert_eq!(action.title, "Create app/javascript/controllers/serch_controller.js");
        assert_eq!(action.is_preferred, Some(false));
        let (edited, created) = effects(&action, &root);
        assert_eq!(created, ["app/javascript/controllers/serch_controller.js"]);
        assert_eq!(edited[0], ("app/javascript/controllers/serch_controller.js".into(), stimulus_edit::CONTROLLER_STUB.into()));
        assert_eq!(
            edited[1],
            (
                "app/javascript/controllers/index.js".into(),
                format!("{}import SerchController from \"./serch_controller\"\napplication.register(\"hello\", HelloController)\napplication.register(\"serch\", SerchController)\n", &manifest[..manifest.find("application.register").unwrap()])
            )
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_target_and_a_method_are_added_to_the_controller_the_template_names() {
        let source = "import { Controller } from \"@hotwired/stimulus\"\n\nexport default class extends Controller {\n  static targets = [\"query\"]\n\n  submit() {\n  }\n}\n";
        let (root, view) = app(&[("app/javascript/controllers/search_controller.js", source)]);
        let member = |kind: &str, name: &str| {
            diagnostic(
                "Stimulus/UnknownTargetOrAction",
                Range::default(),
                json!({ "controller": "search", "kind": kind, "name": name, "suggest": null }),
            )
        };
        let target = add_member(&view, &member("target", "results")).unwrap();
        assert_eq!(target.title, "Add `results` to static targets in app/javascript/controllers/search_controller.js");
        assert_eq!(effects(&target, &root).0[0].1, source.replace("[\"query\"]", "[\"query\", \"results\"]"));
        let method = add_member(&view, &member("method", "reset")).unwrap();
        assert_eq!(method.title, "Add method `reset()` to app/javascript/controllers/search_controller.js");
        assert_eq!(effects(&method, &root).0[0].1, source.replace("  }\n}\n", "  }\n\n  reset() {\n  }\n}\n"));
        assert!(add_member(&view, &member("target", "bad name")).is_none());
        let other =
            diagnostic("Stimulus/UnknownTargetOrAction", Range::default(), json!({ "controller": "other", "kind": "target", "name": "x" }));
        assert!(add_member(&view, &other).is_none(), "no such controller");
        std::fs::remove_dir_all(root).unwrap();
    }
}
