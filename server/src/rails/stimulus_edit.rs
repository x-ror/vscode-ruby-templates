//! Edits that give a template's Stimulus names what they lack: a new controller file, registered in the manifest that lists
//! them, and a target or method added to a controller's class. Offsets are in the text each edit is for.

use super::paths::app_root;
use super::stimulus::{self, CONTROLLERS_DIR, SCRIPT_EXTENSIONS};
use super::stimulus_controller::{self, blank, matching_brace, CLASS, TARGETS_DECLARATION, TARGETS_LIST};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// What a new controller file holds.
pub const CONTROLLER_STUB: &str = "import { Controller } from \"@hotwired/stimulus\"\n\nexport default class extends Controller {\n}\n";

/// The manifest files bundlers' apps list controllers in, as `stimulus:manifest:update` writes them.
const MANIFESTS: [&str; 2] = ["index.js", "index.ts"];

// `application.register("hello", HelloController)` and `import HelloController from "./hello_controller"`, one to a line.
static REGISTER_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^([ \t]*)(\w+)\.register\([ \t]*(["'])([^"']+)["'][ \t]*,[ \t]*(\w+)[ \t]*\)(;?)[ \t]*\r?$"#).unwrap()
});
static IMPORT_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^import[ \t]+(\w+)[ \t]+from[ \t]+(["'])(\.{1,2}/[^"']+)["'](;?)[ \t]*\r?$"#).unwrap());
// Every name an import statement brings in, from any module.
static IMPORT_CLAUSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^import\s+([^"';]+?)\s+from\s+["']"#).unwrap());
// Controllers that load by themselves: a manifest using them needs no line.
static LOADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:eager|lazy)LoadControllersFrom\b|\b(?:require\.context|import\.meta\.glob(?:Eager)?)\(|definitionsFromContext")
        .unwrap()
});
static STRING_ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""[^"\n]*"|'[^'\n]*'"#).unwrap());

/// An edit of a text: the offset to insert at and what to insert.
pub type Insertion = (usize, String);

/// The controller file to create for the Stimulus identifier `identifier`, and the manifest lines to add for it, if any.
pub struct NewController {
    pub file: PathBuf,
    pub manifest: Option<ManifestEdit>,
}

pub struct ManifestEdit {
    pub file: PathBuf,
    /// The manifest's text, which `insertions` are for.
    pub source: String,
    pub insertions: Vec<Insertion>,
}

/// The new controller for `identifier` in `current`'s app. None where the lint doesn't report names (controllers loaded from
/// elsewhere, a library or a computed name) and for an identifier no file name can spell.
pub fn new_controller(current: &Path, identifier: &str) -> Option<NewController> {
    stimulus::registered(current)?;
    let stem = file_stem(identifier)?;
    let dir = app_root(current)?.join(CONTROLLERS_DIR);
    let extension = if stimulus::mostly_typescript(current) { "ts" } else { "js" };
    let file = dir.join(format!("{stem}_controller.{extension}"));
    if file.exists() {
        return None;
    }
    let listing = MANIFESTS.iter().map(|name| dir.join(name)).find_map(|file| {
        let source = std::fs::read_to_string(&file).ok()?;
        (source.contains(".register(") && !LOADING.is_match(&source)).then_some((file, source))
    });
    // A controller its manifest lists but this can't add to would never load.
    let manifest = match listing {
        Some((file, source)) => Some(ManifestEdit { insertions: manifest_insertions(&source, identifier, &stem)?, file, source }),
        None => None,
    };
    Some(NewController { file, manifest })
}

/// The file `identifier` names below the controllers folder, without `_controller.js`: `admin--date-picker` is
/// `admin/date_picker`.
fn file_stem(identifier: &str) -> Option<String> {
    let valid = |part: &str| !part.is_empty() && !part.starts_with('-') && !part.ends_with('-');
    let spelled = identifier.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    (spelled && identifier.split("--").all(valid)).then(|| identifier.replace("--", "/").replace('-', "_"))
}

/// `date-picker` as `DatePickerController`; none when that starts with a digit.
fn class_name(identifier: &str) -> Option<String> {
    let words = identifier.split('-').filter(|word| !word.is_empty());
    let pascal: String = words
        .flat_map(|word| word.chars().next().map(|first| first.to_ascii_uppercase()).into_iter().chain(word.chars().skip(1)))
        .collect();
    (!pascal.starts_with(|c: char| c.is_ascii_digit())).then(|| format!("{pascal}Controller"))
}

/// The import and the `register` call for `identifier` in a manifest of literal ones: after the last of each, written as they
/// are. None for a manifest with another way of loading controllers, a call that isn't one line, or no relative import to copy.
fn manifest_insertions(source: &str, identifier: &str, stem: &str) -> Option<Vec<Insertion>> {
    if LOADING.is_match(source) {
        return None;
    }
    let registers: Vec<_> = REGISTER_LINE.captures_iter(source).collect();
    let imports: Vec<_> = IMPORT_LINE.captures_iter(source).collect();
    let is_name_char = |c: char| !(c.is_alphanumeric() || matches!(c, '_' | '$'));
    let imported = |class: &str| IMPORT_CLAUSE.captures_iter(source).any(|import| import[1].split(is_name_char).any(|name| name == class));
    let every_call_is_a_line = registers.len() == source.matches(".register(").count();
    let class = class_name(identifier)?;
    if registers.is_empty() || !every_call_is_a_line || !registers.iter().all(|register| imported(&register[5])) {
        return None;
    }
    if registers.iter().any(|register| &register[4] == identifier) || imported(&class) {
        return None;
    }
    let (register, import) = (registers.last()?, imports.iter().rfind(|import| registers.iter().any(|register| register[5] == import[1]))?);
    let eol = if source.contains("\r\n") { "\r\n" } else { "\n" };
    let extension = Path::new(&import[3]).extension().and_then(|extension| extension.to_str()).filter(|e| SCRIPT_EXTENSIONS.contains(e));
    let specifier = format!("./{stem}_controller{}", extension.map_or_else(String::new, |extension| format!(".{extension}")));
    let import_line = format!("import {class} from {quote}{specifier}{quote}{semicolon}", quote = &import[2], semicolon = &import[4]);
    let register_line = format!(
        "{indent}{application}.register({quote}{identifier}{quote}, {class}){semicolon}",
        indent = &register[1],
        application = &register[2],
        quote = &register[3],
        semicolon = &register[6]
    );
    let mut insertions = vec![
        (line_end(source, import.get(0)?.start()), format!("{eol}{import_line}")),
        (line_end(source, register.get(0)?.start()), format!("{eol}{register_line}")),
    ];
    insertions.sort();
    Some(insertions)
}

/// The file of the controller `identifier`, when it can be edited with confidence: one file registers it and its class is read
/// whole, as the lints on its targets and methods read it.
pub fn editable_controller(current: &Path, identifier: &str) -> Option<PathBuf> {
    stimulus_controller::controller(current, identifier)?;
    let [file]: [PathBuf; 1] = stimulus::resolve(current, identifier).try_into().ok()?;
    Some(file)
}

/// The edits that add `name` to the `static targets` of the controller class in `source`: into its list, or a new declaration
/// as the first member. None when the class or the list isn't plain enough to edit.
pub fn add_target(source: &str, name: &str) -> Option<Vec<Insertion>> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')) {
        return None;
    }
    let (code, skeleton) = (blank(source, false), blank(source, true));
    let quote = code.find(['"', '\'']).and_then(|at| code[at..].chars().next()).unwrap_or('"');
    let Some(declaration) = TARGETS_DECLARATION.find(&skeleton) else {
        let semicolon = if uses_semicolons(&code) { ";" } else { "" };
        return first_member(source, &skeleton, &format!("static targets = [{quote}{name}{quote}]{semicolon}"));
    };
    let list = TARGETS_LIST.find(&code).filter(|list| list.start() == declaration.start())?;
    let open = list.start() + list.as_str().find('[')?;
    let close = list.end() - 1;
    let Some(last) = STRING_ITEM.find_iter(&code[open + 1..close]).last() else {
        return Some(vec![(open + 1, format!("{quote}{name}{quote}"))]);
    };
    let (item_start, item_end) = (open + 1 + last.start(), open + 1 + last.end());
    let quote = source[item_start..].chars().next()?;
    let entry = format!("{quote}{name}{quote}");
    let comma_at = code[item_end..close].find(',').filter(|&at| code[item_end..item_end + at].trim().is_empty()).map(|at| item_end + at);
    let line_start = source[..item_start].rfind('\n').map_or(0, |at| at + 1);
    let end_of_line = line_end(source, item_end);
    let alone = source[line_start..item_start].trim().is_empty() && code[item_end..end_of_line].trim_matches([',', ' ', '\t']).is_empty();
    if !alone {
        return Some(vec![match comma_at {
            Some(at) => (at + 1, format!(" {entry},")),
            None => (item_end, format!(", {entry}")),
        }]);
    }
    let indent = &source[line_start..item_start];
    let new_line = format!("{}{indent}{entry}{}", eol(source), if comma_at.is_some() { "," } else { "" });
    Some(match comma_at {
        Some(_) => vec![(end_of_line, new_line)],
        None if item_end == end_of_line => vec![(item_end, format!(",{new_line}"))],
        None => vec![(item_end, ",".to_string()), (end_of_line, new_line)],
    })
}

/// The edit that adds the method stub `name() {}` to the controller class in `source`, before its closing brace. None when the
/// class closes on a line of its own no more than the one it opens on.
pub fn add_method(source: &str, name: &str) -> Option<Insertion> {
    let starts_like_a_name = name.starts_with(|c: char| c.is_ascii_alphabetic() || matches!(c, '_' | '$'));
    if !starts_like_a_name || !name.chars().all(stimulus::is_name_char) {
        return None;
    }
    let skeleton = blank(source, true);
    let class = CLASS.find(&skeleton)?;
    let open = class.end() - 1;
    let close = matching_brace(&skeleton, open)?;
    let line_start = source[..close].rfind('\n').map_or(0, |at| at + 1);
    if line_start <= open || !source[line_start..close].trim().is_empty() {
        return None;
    }
    let body = &skeleton[open + 1..close];
    let indent = member_indent(source, class.start(), body);
    let eol = eol(source);
    let before = &source[..line_start];
    let blank_line = !body.trim().is_empty() && !before.ends_with("\n\n") && !before.ends_with("\n\r\n");
    Some((line_start, format!("{}{indent}{name}() {{{eol}{indent}}}{eol}", if blank_line { eol } else { "" })))
}

/// The edit that puts `member` first in the controller class of `source`, which must open its body on a line of its own.
fn first_member(source: &str, skeleton: &str, member: &str) -> Option<Vec<Insertion>> {
    let class = CLASS.find(skeleton)?;
    let open = class.end() - 1;
    let close = matching_brace(skeleton, open)?;
    let end_of_line = line_end(source, open + 1);
    if end_of_line > close || !skeleton[open + 1..end_of_line].trim().is_empty() {
        return None;
    }
    let body = &skeleton[end_of_line..close];
    let indent = member_indent(source, class.start(), body);
    let eol = eol(source);
    let spacing = if body.trim().is_empty() { "" } else { eol };
    Some(vec![(end_of_line, format!("{eol}{indent}{member}{spacing}"))])
}

/// The indentation of the members of the class that starts at `class_start`, whose body is `body`: its first member's, else one
/// level deeper than the class.
fn member_indent(source: &str, class_start: usize, body: &str) -> String {
    let first = body.lines().find(|line| !line.trim().is_empty());
    if let Some(line) = first {
        return line[..line.len() - line.trim_start().len()].to_string();
    }
    let line_start = source[..class_start].rfind('\n').map_or(0, |at| at + 1);
    let class_indent = &source[line_start..class_start];
    let class_indent = &class_indent[..class_indent.len() - class_indent.trim_start().len()];
    let level = if class_indent.contains('\t') { "\t" } else { "  " };
    format!("{class_indent}{level}")
}

fn uses_semicolons(code: &str) -> bool {
    code.lines().find(|line| line.starts_with("import ")).is_some_and(|line| line.trim_end().ends_with(';'))
}

fn eol(source: &str) -> &'static str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// The offset where the line `from` is on ends, before its line ending.
fn line_end(source: &str, from: usize) -> usize {
    let end = source[from..].find('\n').map_or(source.len(), |at| from + at);
    if source[..end].ends_with('\r') {
        end - 1
    } else {
        end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::write_files;

    /// `source` with `insertions`, which are all for it, made.
    fn apply(source: &str, insertions: &[Insertion]) -> String {
        let mut out = source.to_string();
        for (offset, text) in insertions.iter().rev() {
            out.insert_str(*offset, text);
        }
        out
    }

    fn with_target(source: &str, name: &str) -> Option<String> {
        add_target(source, name).map(|insertions| apply(source, &insertions))
    }

    fn with_method(source: &str, name: &str) -> Option<String> {
        add_method(source, name).map(|insertion| apply(source, &[insertion]))
    }

    const HEADER: &str = "import { Controller } from \"@hotwired/stimulus\"\n\nexport default class extends Controller {\n";

    #[test]
    fn a_target_joins_the_literal_list_in_its_style() {
        let list = |declaration: &str| format!("{HEADER}  {declaration}\n\n  connect() {{\n  }}\n}}\n");
        assert_eq!(
            with_target(&list("static targets = [\"query\", \"results\"]"), "output").unwrap(),
            list("static targets = [\"query\", \"results\", \"output\"]")
        );
        assert_eq!(with_target(&list("static targets = ['query']"), "output").unwrap(), list("static targets = ['query', 'output']"));
        assert_eq!(
            with_target(&list("static targets = [\"query\",]"), "output").unwrap(),
            list("static targets = [\"query\", \"output\",]")
        );
        assert_eq!(with_target(&list("static targets = []"), "output").unwrap(), list("static targets = [\"output\"]"));
        assert_eq!(
            with_target(&list("static targets = [ \"query\" ]"), "my-output").unwrap(),
            list("static targets = [ \"query\", \"my-output\" ]")
        );
        assert_eq!(
            with_target(&list("static targets = [\"query\"] // inputs"), "output").unwrap(),
            list("static targets = [\"query\", \"output\"] // inputs")
        );
    }

    #[test]
    fn a_target_joins_a_list_over_several_lines_on_a_line_of_its_own() {
        let list = |items: &str| format!("{HEADER}  static targets = [\n{items}  ]\n}}\n");
        assert_eq!(
            with_target(&list("    \"query\",\n    \"results\",\n"), "output").unwrap(),
            list("    \"query\",\n    \"results\",\n    \"output\",\n")
        );
        assert_eq!(
            with_target(&list("    \"query\",\n    \"results\"\n"), "output").unwrap(),
            list("    \"query\",\n    \"results\",\n    \"output\"\n")
        );
        assert_eq!(
            with_target(&list("    \"query\", // the box\n    \"results\" // the list\n"), "output").unwrap(),
            list("    \"query\", // the box\n    \"results\", // the list\n    \"output\"\n")
        );
        assert_eq!(with_target(&list("    'query',\r\n"), "output").unwrap().replace("\r\n", "\n"), list("    'query',\n    'output',\n"));
        let wrapped = format!("{HEADER}  static targets = [\"query\",\n                    \"results\"]\n}}\n");
        assert_eq!(
            with_target(&wrapped, "output").unwrap(),
            format!("{HEADER}  static targets = [\"query\",\n                    \"results\", \"output\"]\n}}\n")
        );
    }

    #[test]
    fn a_target_without_a_declaration_makes_one_the_first_member() {
        let with_members = format!("{HEADER}  connect() {{\n  }}\n}}\n");
        assert_eq!(
            with_target(&with_members, "output").unwrap(),
            format!("{HEADER}  static targets = [\"output\"]\n\n  connect() {{\n  }}\n}}\n")
        );
        assert_eq!(with_target(&format!("{HEADER}}}\n"), "output").unwrap(), format!("{HEADER}  static targets = [\"output\"]\n}}\n"));
        let semicolons = "import { Controller } from '@hotwired/stimulus';\n\nexport default class extends Controller {\n    go() {}\n}\n";
        assert_eq!(
            with_target(semicolons, "output").unwrap(),
            "import { Controller } from '@hotwired/stimulus';\n\nexport default class extends Controller {\n    static targets = ['output'];\n\n    go() {}\n}\n"
        );
        let named = "export default class SearchController extends Controller { // search\n\tgo() {}\n}\n";
        assert_eq!(
            with_target(named, "output").unwrap(),
            "export default class SearchController extends Controller { // search\n\tstatic targets = [\"output\"]\n\n\tgo() {}\n}\n"
        );
        assert_eq!(
            with_target(&with_members.replace('\n', "\r\n"), "output").unwrap(),
            format!("{HEADER}  static targets = [\"output\"]\n\n  connect() {{\n  }}\n}}\n").replace('\n', "\r\n")
        );
    }

    #[test]
    fn a_target_is_not_added_where_the_class_or_the_list_is_not_plain() {
        assert_eq!(with_target(&format!("{HEADER}  static targets = targets()\n}}\n"), "output"), None);
        assert_eq!(with_target(&format!("{HEADER}  static targets = [...base, \"a\"]\n}}\n"), "output"), None);
        assert_eq!(with_target("export default class extends Controller { go() {} }\n", "output"), None);
        assert_eq!(with_target("export default class extends Controller {}\n", "output"), None);
        assert_eq!(with_target(&format!("{HEADER}}}\n"), "bad name"), None);
        assert_eq!(with_target(&format!("{HEADER}}}\n"), "bad\"name"), None);
    }

    #[test]
    fn a_method_goes_before_the_closing_brace() {
        let class = |members: &str| format!("{HEADER}{members}}}\n");
        assert_eq!(with_method(&class("  connect() {\n  }\n"), "submit").unwrap(), class("  connect() {\n  }\n\n  submit() {\n  }\n"));
        assert_eq!(with_method(&class(""), "submit").unwrap(), class("  submit() {\n  }\n"));
        assert_eq!(with_method(&class("  connect() {\n  }\n\n"), "submit").unwrap(), class("  connect() {\n  }\n\n  submit() {\n  }\n"));
        assert_eq!(with_method(&class("    go() {}\n"), "$run").unwrap(), class("    go() {}\n\n    $run() {\n    }\n"));
        let tabs = "export default class extends Controller {\n\tgo() {}\n}\n";
        assert_eq!(with_method(tabs, "run").unwrap(), "export default class extends Controller {\n\tgo() {}\n\n\trun() {\n\t}\n}\n");
        let nested = "class Search extends Controller {\n  go() {\n    if (x) {\n    }\n  }\n}";
        assert_eq!(
            with_method(nested, "run").unwrap(),
            "class Search extends Controller {\n  go() {\n    if (x) {\n    }\n  }\n\n  run() {\n  }\n}"
        );
        assert_eq!(
            with_method(&class("  go() {\n  }\n").replace('\n', "\r\n"), "run").unwrap(),
            class("  go() {\n  }\n\n  run() {\n  }\n").replace('\n', "\r\n")
        );
    }

    #[test]
    fn a_method_is_not_added_where_the_class_closes_on_a_line_with_code() {
        assert_eq!(with_method("export default class extends Controller { go() {} }\n", "run"), None);
        assert_eq!(with_method("export default class extends Controller {\n  go() {} }\n", "run"), None);
        assert_eq!(with_method("export default class extends Controller {}\n", "run"), None);
        assert_eq!(with_method(&format!("{HEADER}}}\n"), "1run"), None);
        assert_eq!(with_method(&format!("{HEADER}}}\n"), "a-b"), None);
    }

    const MANIFEST: &str = "// auto-generated\nimport { application } from \"./application\"\n\nimport HelloController from \"./hello_controller\"\napplication.register(\"hello\", HelloController)\n\nimport SearchController from \"./search_controller\"\napplication.register(\"search\", SearchController)\n";

    fn registered_in(manifest: &str, identifier: &str) -> Option<String> {
        let stem = file_stem(identifier)?;
        manifest_insertions(manifest, identifier, &stem).map(|insertions| apply(manifest, &insertions))
    }

    #[test]
    fn a_controller_is_listed_in_a_manifest_of_literal_lines() {
        assert_eq!(
            registered_in(MANIFEST, "date-picker").unwrap(),
            "// auto-generated\nimport { application } from \"./application\"\n\nimport HelloController from \"./hello_controller\"\napplication.register(\"hello\", HelloController)\n\nimport SearchController from \"./search_controller\"\nimport DatePickerController from \"./date_picker_controller\"\napplication.register(\"search\", SearchController)\napplication.register(\"date-picker\", DatePickerController)\n"
        );
        let styled = "import { application } from './application';\nimport HelloController from './hello_controller.js';\n  app.register('hello', HelloController);\n";
        assert_eq!(
            registered_in(styled, "admin--users").unwrap(),
            "import { application } from './application';\nimport HelloController from './hello_controller.js';\nimport AdminUsersController from './admin/users_controller.js';\n  app.register('hello', HelloController);\n  app.register('admin--users', AdminUsersController);\n"
        );
        let crlf = registered_in(&MANIFEST.replace('\n', "\r\n"), "x").unwrap();
        assert!(crlf.contains("import XController from \"./x_controller\"\r\napplication.register(\"search\""), "{crlf:?}");
        assert!(crlf.ends_with("application.register(\"x\", XController)\r\n"), "{crlf:?}");
        let unterminated = "import HelloController from \"./hello_controller\"\napplication.register(\"hello\", HelloController)";
        assert_eq!(
            registered_in(unterminated, "x").unwrap(),
            "import HelloController from \"./hello_controller\"\nimport XController from \"./x_controller\"\napplication.register(\"hello\", HelloController)\napplication.register(\"x\", XController)"
        );
    }

    #[test]
    fn a_manifest_that_loads_or_lists_otherwise_gets_no_lines() {
        let eager = "import { application } from \"./application\"\nimport { eagerLoadControllersFrom } from \"@hotwired/stimulus-loading\"\neagerLoadControllersFrom(\"controllers\", application)\n";
        let mixed = format!("{MANIFEST}eagerLoadControllersFrom(\"controllers\", application)\n");
        let glob = format!("{MANIFEST}application.load(definitionsFromContext(require.context(\".\", true, /_controller\\.js$/)))\n");
        let computed = format!("{MANIFEST}names.forEach((name) => application.register(name, controllers[name]))\n");
        let not_imported = "application.register(\"hello\", HelloController)\n";
        for (label, manifest) in [
            ("eager", eager),
            ("eager and literal", &mixed),
            ("a glob", &glob),
            ("a computed name", &computed),
            ("a controller that isn't imported", not_imported),
            ("no controller yet", "import { application } from \"./application\"\n"),
        ] {
            assert_eq!(registered_in(manifest, "x"), None, "{label}");
        }
        let multiline = "import HelloController from \"./hello_controller\"\napplication.register(\n  \"hello\",\n  HelloController\n)\n";
        assert_eq!(registered_in(multiline, "x"), None, "a call over several lines");
        let spread = "import HelloController from \"./hello_controller\"\napplication.register(\"hello\",\n  HelloController)\n";
        assert_eq!(registered_in(spread, "x"), None, "a call over two lines");
        assert_eq!(registered_in(MANIFEST, "hello"), None, "already listed");
        assert_eq!(registered_in(MANIFEST, "3d"), None, "not a class name");
        assert_eq!(registered_in(MANIFEST, "a--"), None);
    }

    #[test]
    fn a_package_controller_in_the_manifest_is_no_obstacle() {
        let manifest = "import { application } from \"./application\"\nimport { Dropdown, Modal as Dialog } from \"tailwindcss-stimulus-components\"\nimport HelloController from \"./hello_controller\"\napplication.register(\"dropdown\", Dropdown)\napplication.register(\"modal\", Dialog)\napplication.register(\"hello\", HelloController)\n";
        assert_eq!(
            registered_in(manifest, "x").unwrap(),
            "import { application } from \"./application\"\nimport { Dropdown, Modal as Dialog } from \"tailwindcss-stimulus-components\"\nimport HelloController from \"./hello_controller\"\nimport XController from \"./x_controller\"\napplication.register(\"dropdown\", Dropdown)\napplication.register(\"modal\", Dialog)\napplication.register(\"hello\", HelloController)\napplication.register(\"x\", XController)\n"
        );
        let package_only = "import { Dropdown } from \"tailwindcss-stimulus-components\"\napplication.register(\"dropdown\", Dropdown)\n";
        assert_eq!(registered_in(package_only, "x"), None, "no relative import to copy");
    }

    fn app(files: &[(&str, &str)]) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rt-stimulus-edit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        write_files(&root, files);
        root
    }

    #[test]
    fn a_new_controller_is_a_file_of_the_apps_script_kind_and_a_manifest_line() {
        let javascript = "app/javascript/controllers";
        let root = app(&[
            ("app/views/x/index.html.erb", ""),
            (&format!("{javascript}/hello_controller.js"), HEADER),
            (&format!("{javascript}/index.js"), MANIFEST),
        ]);
        let view = root.join("app/views/x/index.html.erb");
        let new = new_controller(&view, "date-picker").unwrap();
        assert_eq!(new.file, root.join(javascript).join("date_picker_controller.js"));
        let manifest = new.manifest.unwrap();
        assert_eq!(manifest.file, root.join(javascript).join("index.js"));
        assert_eq!(manifest.insertions.len(), 2);
        assert!(new_controller(&view, "hello").is_none(), "the file exists");
        assert!(new_controller(&view, "Bad").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_controller_is_typescript_where_most_are_and_not_listed_where_they_load_themselves() {
        let javascript = "app/javascript/controllers";
        let eager = "import { application } from \"./application\"\nimport { eagerLoadControllersFrom } from \"@hotwired/stimulus-loading\"\neagerLoadControllersFrom(\"controllers\", application)\n";
        let root = app(&[
            ("app/views/x/index.html.erb", ""),
            (&format!("{javascript}/a_controller.ts"), HEADER),
            (&format!("{javascript}/b_controller.ts"), HEADER),
            (&format!("{javascript}/c_controller.js"), HEADER),
            (&format!("{javascript}/index.ts"), eager),
        ]);
        let new = new_controller(&root.join("app/views/x/index.html.erb"), "admin--users").unwrap();
        assert_eq!(new.file, root.join(javascript).join("admin/users_controller.ts"));
        assert!(new.manifest.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_controller_is_made_where_its_manifest_lists_controllers_but_cannot_take_one_more() {
        let javascript = "app/javascript/controllers";
        let multiline = "import HelloController from \"./hello_controller\"\napplication.register(\n  \"hello\",\n  HelloController\n)\n";
        let root = app(&[
            ("app/views/x/index.html.erb", ""),
            (&format!("{javascript}/hello_controller.js"), HEADER),
            (&format!("{javascript}/index.js"), multiline),
        ]);
        let view = root.join("app/views/x/index.html.erb");
        assert!(new_controller(&view, "x").is_none(), "a call over several lines");
        std::fs::write(root.join(javascript).join("index.js"), MANIFEST).unwrap();
        assert!(new_controller(&view, "3d-viewer").is_none(), "no class name");
        assert!(new_controller(&view, "x").is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_controller_is_made_where_the_lint_cannot_tell_the_names() {
        let root = app(&[
            ("app/views/x/index.html.erb", ""),
            ("app/javascript/controllers/a_controller.js", HEADER),
            ("config/importmap.rb", "pin_all_from \"app/components\", under: \"controllers\"\n"),
        ]);
        assert!(new_controller(&root.join("app/views/x/index.html.erb"), "x").is_none());
        std::fs::remove_dir_all(root).unwrap();
        let root = app(&[("app/views/x/index.html.erb", "")]);
        assert!(new_controller(&root.join("app/views/x/index.html.erb"), "x").is_none(), "no controllers folder");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_controller_is_edited_only_when_it_is_read_with_confidence() {
        let javascript = "app/javascript/controllers";
        let root = app(&[
            ("app/views/x/index.html.erb", ""),
            (&format!("{javascript}/search_controller.js"), &format!("{HEADER}}}\n")),
            (&format!("{javascript}/odd_controller.js"), &format!("{HEADER}  static targets = names\n}}\n")),
        ]);
        let view = root.join("app/views/x/index.html.erb");
        assert_eq!(editable_controller(&view, "search"), Some(root.join(javascript).join("search_controller.js")));
        assert_eq!(editable_controller(&view, "odd"), None);
        assert_eq!(editable_controller(&view, "missing"), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
