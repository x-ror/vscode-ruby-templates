//! What a Stimulus controller file declares, for lints that check a template against it: its targets and the methods an
//! action can call. A controller whose class could have either from somewhere not read here is not described at all.

use crate::rails::paths::app_root;
use crate::rails::stimulus::{self, CONTROLLERS_DIR, SCRIPT_EXTENSIONS};
use dashmap::DashMap;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

/// Base controllers followed through `import`, so a chain of them ends before a cycle could be walked.
const MAX_PARENTS: usize = 3;
/// What `Controller` itself has, so an action may name it.
const BASE_METHODS: [&str; 4] = ["initialize", "connect", "disconnect", "dispatch"];

// A class header, with the modifiers and names around `extends` (`class Name extends Base<T> `).
static CLASS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bclass\b([\w\s$<>,.]*)\{").unwrap());
static STIMULUS_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"import\s*(?:\w+\s*,\s*)?\{[^}]*\bController\b[^}]*\}\s*from\s*["'](?:@hotwired/)?stimulus["']"#).unwrap()
});
static TARGETS_DECLARATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bstatic\s+(?:(?:get|readonly|override|declare|public)\s+)*targets\b").unwrap());
static TARGETS_LIST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\bstatic\s+(?:(?:readonly|override|public)\s+)*targets\s*=\s*\[\s*(?:(?:"[^"\n]*"|'[^'\n]*')\s*,?\s*)*\]"#).unwrap()
});
// Methods and targets that come in without a declaration in the class.
static UNREADABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\.prototype\b|\.targets\s*=[^=]|\bObject\.(?:assign|defineProperty|defineProperties)\s*\(\s*this\b|\bthis\s*\[[^\]]*\]\s*=[^=]",
    )
    .unwrap()
});
// A helper handed the controller (`useClickOutside(this)`, `Modal.install(this, …)`) may add methods to it.
static THIS_ARGUMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[(,]\s*this\s*[,)]").unwrap());
static ASSIGNED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bthis\.([A-Za-z_$][\w$]*)\s*=[^=>]").unwrap());

#[derive(Debug)]
pub struct Members {
    pub targets: Vec<String>,
    pub methods: Vec<String>,
}

impl Members {
    pub fn has_method(&self, name: &str) -> bool {
        BASE_METHODS.contains(&name) || self.methods.iter().any(|method| method == name)
    }
}

/// `read` per controller file: a template names the same few controllers over and over.
static MEMBERS: LazyLock<DashMap<PathBuf, Option<Arc<Members>>>> = LazyLock::new(DashMap::new);

/// Drops what `controller` remembers, when a watched file may have changed it.
pub fn clear_cache() {
    MEMBERS.clear();
}

/// The members of the controller `identifier` in `current`'s app, when exactly one file registers it, the app registers its
/// controllers where they can be seen, and the file can be read with confidence.
pub fn controller(current: &Path, identifier: &str) -> Option<Arc<Members>> {
    let [file]: [PathBuf; 1] = stimulus::resolve(current, identifier).try_into().ok()?;
    stimulus::registered(current)?;
    if let Some(found) = MEMBERS.get(&file) {
        return found.clone();
    }
    let found = read(&file, MAX_PARENTS).map(Arc::new);
    MEMBERS.insert(file, found.clone());
    found
}

/// The members `file` declares and inherits from the controllers it extends; `None` when that can't be told.
fn read(file: &Path, parents_left: usize) -> Option<Members> {
    let source = std::fs::read_to_string(file).ok()?;
    let code = blank(&source, false);
    let skeleton = blank(&source, true);
    if UNREADABLE.is_match(&skeleton) || passes_this(&skeleton) {
        return None;
    }
    let mut classes = CLASS.captures_iter(&skeleton);
    let (Some(class), None) = (classes.next(), classes.next()) else { return None };
    let parent = parent_of(class.get(1)?.as_str())?;
    let open = class.get(0)?.end() - "{".len();
    let body = &skeleton[open + "{".len()..matching_brace(&skeleton, open)?];
    let mut methods = member_names(body)?;
    if methods.iter().any(|name| name == "targets") && !TARGETS_DECLARATION.is_match(&skeleton) {
        return None;
    }
    methods.extend(ASSIGNED.captures_iter(&skeleton).map(|assigned| assigned[1].to_string()));
    let mut members = Members { targets: declared_targets(&code, &skeleton)?, methods };
    if parent != "Controller" || !STIMULUS_IMPORT.is_match(&code) {
        let inherited = read(&imported_file(&code, &parent, file)?, parents_left.checked_sub(1)?)?;
        members.targets.extend(inherited.targets);
        members.methods.extend(inherited.methods);
    }
    Some(members)
}

/// Whether the code hands `this` to a call, other than `.bind(this)`.
fn passes_this(skeleton: &str) -> bool {
    THIS_ARGUMENT.find_iter(skeleton).any(|call| !skeleton[..call.start()].ends_with(".bind"))
}

/// The class a header (`Name extends Base<T> `) extends; none for a class that extends nothing or whose header says more.
fn parent_of(header: &str) -> Option<String> {
    let (before, after) = header.split_once("extends")?;
    let after = after.trim();
    let name_len = after.find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '$'))).unwrap_or(after.len());
    let (name, rest) = after.split_at(name_len);
    let rest = rest.trim_start();
    let plain = !name.is_empty() && before.split_whitespace().count() <= 1;
    (plain && (rest.is_empty() || rest.starts_with('<') || rest.starts_with("implements"))).then(|| name.to_string())
}

/// The targets of `static targets = [...]`; `None` when the declaration is anything but a list of strings.
fn declared_targets(code: &str, skeleton: &str) -> Option<Vec<String>> {
    let mut declarations = TARGETS_DECLARATION.find_iter(skeleton);
    let Some(declaration) = declarations.next() else { return Some(Vec::new()) };
    let literal = TARGETS_LIST.find(code).is_some_and(|list| list.start() == declaration.start());
    (literal && declarations.next().is_none()).then(|| stimulus::targets(code))
}

/// The file the class `name` is imported from, when it is another controller of the app: a relative path or `controllers/…`.
fn imported_file(code: &str, name: &str, from: &Path) -> Option<PathBuf> {
    let name = regex::escape(name);
    let import = Regex::new(&format!(r#"import\s+(?:{name}|\{{[^}}]*\b{name}\b[^}}]*\}})\s*from\s*["']([^"']+)["']"#)).ok()?;
    let specifier = import.captures(code)?.get(1)?.as_str();
    let base = match specifier.strip_prefix("controllers/") {
        Some(rest) => app_root(from)?.join(CONTROLLERS_DIR).join(rest),
        None if specifier.starts_with('.') => from.parent()?.join(specifier),
        None => return None,
    };
    let with_extension = SCRIPT_EXTENSIONS.iter().map(|extension| PathBuf::from(format!("{}.{extension}", base.display())));
    std::iter::once(base.clone()).chain(with_extension).find(|candidate| candidate.is_file())
}

/// The names the class body defines at its own level: methods, accessors and fields. `None` for a name written as a computed
/// key, a string or a decorator, which can't be read here. A name followed by `(`, `=`, `;` or `:` counts, so an initializer
/// may add names that no member has; that only makes more names known.
fn member_names(body: &str) -> Option<Vec<String>> {
    let (mut names, mut depth, mut previous) = (Vec::new(), 0usize, None::<char>);
    let mut chars = body.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            '[' if depth == 0 && starts_member(previous) && !body[..at].ends_with(|p: char| p.is_alphanumeric() || "_$]".contains(p)) => {
                return None;
            }
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth = depth.saturating_sub(1),
            '@' if depth == 0 => return None,
            '"' | '\'' | '`' if depth == 0 => {
                let quoted = body[at + 1..].find(c).map_or(body.len(), |close| at + 1 + close + 1);
                if body[quoted..].trim_start().starts_with('(') {
                    return None;
                }
                while chars.next_if(|&(i, _)| i < quoted).is_some() {}
            }
            c if depth == 0 && (c.is_alphabetic() || matches!(c, '_' | '$')) => {
                let mut end = at + c.len_utf8();
                while let Some((i, next)) = chars.next_if(|&(_, next)| next.is_alphanumeric() || matches!(next, '_' | '$')) {
                    end = i + next.len_utf8();
                }
                let (name, rest) = (&body[at..end], skip_generics(body[end..].trim_start()));
                let optional = rest.strip_prefix(['?', '!']).unwrap_or(rest);
                let defines =
                    optional.starts_with(['(', ';', ':']) || optional.strip_prefix('=').is_some_and(|after| !after.starts_with(['=', '>']));
                if defines && previous != Some('.') {
                    names.push(name.to_string());
                }
            }
            _ => {}
        }
        if !c.is_whitespace() {
            previous = Some(c);
        }
    }
    Some(names)
}

/// `rest` without a leading `<…>` (a TypeScript method's type parameters).
fn skip_generics(rest: &str) -> &str {
    if !rest.starts_with('<') {
        return rest;
    }
    let mut depth = 0usize;
    for (at, c) in rest.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return rest[at + 1..].trim_start();
                }
            }
            _ => {}
        }
    }
    rest
}

/// Whether a member can begin after `previous`, the last character before it: `[` there is a computed name, not an index.
fn starts_member(previous: Option<char>) -> bool {
    previous.is_none_or(|p| p.is_alphanumeric() || matches!(p, '_' | '$' | '}' | ';' | '*'))
}

/// The byte of the `}` that closes the `{` at `open`.
fn matching_brace(code: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (at, c) in code[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + at);
                }
            }
            _ => {}
        }
    }
    None
}

/// `source` with its comments, and with `strings` the text of its strings, regular expressions and template literals, replaced
/// by spaces: the same bytes in the same places, so what is left has only code in it. The quotes stay.
fn blank(source: &str, strings: bool) -> String {
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    let mut last_code = None::<char>;
    while at < source.len() {
        let rest = &source[at..];
        let c = rest.chars().next().unwrap_or_default();
        let (len, is_comment, is_literal) = if rest.starts_with("//") {
            (rest.find('\n').unwrap_or(rest.len()), true, false)
        } else if rest.starts_with("/*") {
            (rest.find("*/").map_or(rest.len(), |end| end + "*/".len()), true, false)
        } else if c == '`' {
            (template_len(rest), false, true)
        } else if matches!(c, '"' | '\'') {
            (stimulus::string_len(rest, c), false, true)
        } else if c == '/' && (last_code.is_none_or(|p| "(,=:[!&|?{};<>+-*%~^".contains(p)) || ends_with_keyword(&out)) {
            (regex_len(rest), false, true)
        } else {
            (c.len_utf8(), false, false)
        };
        let piece = &rest[..len];
        if is_comment || (is_literal && strings) {
            let enclosed = is_literal && len > 1 && piece.chars().next() == piece.chars().next_back();
            for (i, ch) in piece.char_indices() {
                let edge = enclosed && (i == 0 || i + ch.len_utf8() == len);
                if ch == '\n' || edge {
                    out.push(ch);
                } else {
                    out.extend(std::iter::repeat_n(' ', ch.len_utf8()));
                }
            }
        } else {
            out.push_str(piece);
        }
        if !is_comment {
            last_code = piece.chars().rev().find(|ch| !ch.is_whitespace()).or(last_code);
        }
        at += len;
    }
    out
}

/// Whether `code` ends in a keyword after which an expression, so a regular expression, can begin.
fn ends_with_keyword(code: &str) -> bool {
    let code = code.trim_end();
    let start = code.rfind(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '$'))).map_or(0, |at| at + 1);
    const KEYWORDS: [&str; 12] = ["return", "typeof", "case", "yield", "await", "void", "delete", "in", "of", "new", "else", "do"];
    KEYWORDS.contains(&&code[start..]) || &code[start..] == "instanceof"
}

/// The length of the template literal `rest` starts with, `${ }` included, which may hold strings and templates of its own.
fn template_len(rest: &str) -> usize {
    let mut at = 1;
    while let Some(c) = rest[at..].chars().next() {
        match c {
            '\\' => at += c.len_utf8() + rest[at + 1..].chars().next().map_or(0, char::len_utf8),
            '`' => return at + 1,
            '$' if rest[at..].starts_with("${") => at += interpolation_len(&rest[at..]),
            _ => at += c.len_utf8(),
        }
    }
    rest.len()
}

/// The length of the `${ … }` `rest` starts with.
fn interpolation_len(rest: &str) -> usize {
    let (mut at, mut depth) = ("${".len(), 1);
    while let Some(c) = rest[at..].chars().next() {
        match c {
            '`' => at += template_len(&rest[at..]),
            '"' | '\'' => at += stimulus::string_len(&rest[at..], c),
            '{' => {
                depth += 1;
                at += 1;
            }
            '}' => {
                depth -= 1;
                at += 1;
                if depth == 0 {
                    return at;
                }
            }
            _ => at += c.len_utf8(),
        }
    }
    rest.len()
}

/// The length of the regular expression literal `rest` starts with, to its closing `/` on the same line; `/` alone when none.
fn regex_len(rest: &str) -> usize {
    let mut class = false;
    let mut chars = rest.char_indices().skip(1);
    while let Some((at, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '[' => class = true,
            ']' => class = false,
            '/' if !class => return at + 1,
            '\n' => break,
            _ => {}
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members_of(source: &str) -> Option<(Vec<String>, Vec<String>)> {
        let dir = std::env::temp_dir().join(format!("rt-stimulus-members-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("search_controller.js");
        std::fs::write(&file, source).unwrap();
        let found = read(&file, MAX_PARENTS);
        std::fs::remove_dir_all(&dir).unwrap();
        found.map(|found| (found.targets, found.methods))
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn members_are_the_targets_and_the_methods_of_the_class() {
        let source = r#"import { Controller } from "@hotwired/stimulus"
// static targets = ["commented"]
export default class extends Controller {
  static targets = ["query", 'results']
  static values = { url: String }

  connect() {
    this.timeout = null
    const html = `<li>${items.map((i) => `<b>${i}</b>`).join("")}</li>` // submit() {
    if (/[{"]/.test(html)) { this.element.hidden = true }
  }

  async submit(event) {
    fetch("/search?q=" + this.queryTarget.value, { method: "POST" })
  }

  clear = () => {
    this.resultsTarget.innerHTML = ""
  }

  get count() { return 1 }
  static create() {}
  #secret() {}
}
"#;
        let (targets, methods) = members_of(source).unwrap();
        assert_eq!(targets, ["query", "results"]);
        for method in ["connect", "submit", "clear", "count", "create", "timeout"] {
            assert!(methods.contains(&method.to_string()), "{method} in {methods:?}");
        }
        for not_a_method in ["fetch", "if", "html", "test", "commented"] {
            assert!(!methods.contains(&not_a_method.to_string()), "{not_a_method} in {methods:?}");
        }
        assert_eq!(
            members_of("import { Controller } from \"stimulus\"\nclass SearchController extends Controller {}\n").unwrap().0,
            names(&[])
        );
        let typed = "import { Controller } from \"@hotwired/stimulus\"\nexport default class SearchController extends Controller<HTMLElement> {\n  static targets = [\"a\"]\n  declare readonly aTarget: HTMLElement\n  go(): void {}\n}\n";
        let (targets, methods) = members_of(typed).unwrap();
        assert_eq!((targets, methods.contains(&"go".to_string())), (names(&["a"]), true));
    }

    #[test]
    fn modifiers_generics_and_regexes_do_not_hide_members() {
        let header = "import { Controller } from \"@hotwired/stimulus\"\nexport default class extends Controller {\n";
        let read = |body: &str| members_of(&format!("{header}{body}\n}}"));
        for modifiers in ["readonly", "override", "public", "public readonly"] {
            let (targets, _) = read(&format!("  static {modifiers} targets = [\"a\"]")).unwrap();
            assert_eq!(targets, ["a"], "{modifiers}");
        }
        assert!(read("  static targets = foo\n  static readonly values = {}").is_none());
        assert!(read("  static async targets = [\"a\"]").is_none(), "a targets member that can't be read");
        let (_, methods) =
            read("  handle<T extends Event>(event: T): void {}\n  async load<T>() {}\n  other<A, B = Map<A, B>>(a) {}").unwrap();
        for method in ["handle", "load", "other"] {
            assert!(methods.contains(&method.to_string()), "{method} in {methods:?}");
        }
        let (_, methods) =
            read("  isQuote(c) { return /'/.test(c) }\n  later() {}\n  more() { x = a > /\"/.test(b) }\n  last() {}").unwrap();
        for method in ["isQuote", "later", "more", "last"] {
            assert!(methods.contains(&method.to_string()), "{method} in {methods:?}");
        }
        assert!(read("  connect() { this.f = this.f.bind(this) }").is_some(), "bind(this) adds nothing");
    }

    #[test]
    fn controllers_that_could_have_members_unseen_are_not_described() {
        let header = "import { Controller } from \"@hotwired/stimulus\"\n";
        for body in [
            "export default class extends Controller {\n  static targets = [...Base.targets, \"a\"]\n}",
            "export default class extends Controller {\n  static targets = names\n}",
            "export default class extends Controller {\n  static get targets() { return [\"a\"] }\n}",
            "export default class extends Controller {\n  static targets: string[] = [\"a\"]\n}",
            "export default class extends Controller {\n  [name]() {}\n}",
            "export default class extends Controller {\n  static [key] = 1\n}",
            "export default class extends Controller {\n  'my-method'() {}\n}",
            "export default class extends Controller {\n  @action go() {}\n}",
            "export default class extends Controller {}\nObject.assign(SearchController.prototype, Mixin)",
            "export default class extends Controller {\n  connect() { Object.assign(this, mixin) }\n}",
            "export default class extends Controller {\n  connect() { this[name] = fn }\n}",
            "export default class extends Controller {\n  connect() { useClickOutside(this) }\n}",
            "export default class extends Controller {\n  connect() { useTransition(this, { element: x }) }\n}",
            "export default class extends Controller {\n  connect() { Modal.install(a, this) }\n}",
            "export default class extends mixin(Controller) {}",
            "export default class extends Controller {}\nclass Other extends Controller {}",
            "export default class {}",
        ] {
            assert_eq!(members_of(&format!("{header}{body}")), None, "{body}");
        }
        assert_eq!(members_of("export default class extends Controller {}"), None, "Controller is not imported from Stimulus");
        assert_eq!(members_of("import { Controller } from \"./mine\"\nexport default class extends Controller {}"), None);
        assert_eq!(members_of("import { Controller as C } from \"@hotwired/stimulus\"\nexport default class extends C {}"), None);
        assert!(members_of(&format!("{header}export default class extends Controller {{\n  static targets = []\n}}")).is_some());
    }

    #[test]
    fn a_controller_inherits_from_the_base_controller_it_imports() {
        let root = std::env::temp_dir().join(format!("rt-stimulus-inherit-{}", std::process::id()));
        let controllers = root.join("app/javascript/controllers");
        let view = root.join("app/views/users/index.html.erb");
        std::fs::create_dir_all(controllers.join("admin")).unwrap();
        let stimulus = "import { Controller } from \"@hotwired/stimulus\"\n";
        let write = |file: &str, source: &str| std::fs::write(controllers.join(file), source).unwrap();
        write(
            "application_controller.js",
            &format!("{stimulus}export default class extends Controller {{\n  static targets = [\"flash\"]\n  close() {{}}\n}}"),
        );
        write("search_controller.js", "import ApplicationController from \"./application_controller\"\nexport default class extends ApplicationController {\n  static targets = [\"query\"]\n  go() {}\n}");
        write(
            "admin/users_controller.js",
            "import Base from \"controllers/application_controller.js\"\nexport default class extends Base {}",
        );
        write("lonely_controller.js", "import Base from \"lib/base\"\nexport default class extends Base {}");
        write("loop_controller.js", "import Loop from \"./loop_controller\"\nexport default class extends Loop {}");
        write("index.js", "import { application } from \"./application\"\napplication.register(\"x\", X)");
        std::fs::write(controllers.join("../application.js"), "export const application = Application.start()").unwrap();

        let search = controller(&view, "search").unwrap();
        assert_eq!(search.targets, ["query", "flash"]);
        assert!(search.has_method("go") && search.has_method("close") && search.has_method("connect") && !search.has_method("nope"));
        assert!(controller(&view, "admin--users").unwrap().has_method("close"));
        assert!(controller(&view, "lonely").is_none(), "a base controller from a package");
        assert!(controller(&view, "loop").is_none(), "a controller that is its own base");
        assert!(controller(&view, "missing").is_none());
        write("two_controller.js", &format!("{stimulus}export default class extends Controller {{}}"));
        write("two_controller.ts", &format!("{stimulus}export default class extends Controller {{}}"));
        assert!(controller(&view, "two").is_none(), "two files register one name");
        std::fs::remove_dir_all(&root).unwrap();
        let hidden = std::env::temp_dir().join(format!("rt-stimulus-hidden-{}", std::process::id()));
        std::fs::create_dir_all(hidden.join("app/javascript/controllers")).unwrap();
        std::fs::write(
            hidden.join("app/javascript/controllers/search_controller.js"),
            format!("{stimulus}export default class extends Controller {{}}"),
        )
        .unwrap();
        std::fs::write(hidden.join("app/javascript/loader.js"), "application.register(computed(), X)").unwrap();
        assert!(controller(&hidden.join("app/views/users/index.html.erb"), "search").is_none(), "controllers registered unseen");
        std::fs::remove_dir_all(&hidden).unwrap();
    }
}
