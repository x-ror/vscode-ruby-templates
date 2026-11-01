//! Stimulus identifiers in `data-*` attributes → app/javascript/controllers files.

use crate::rails_paths::app_root;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

// `data-controller="a b"`, `"data-action": "…"`, `"data-controller" => "…"`.
static ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"data-(controller|action)["']?\s*(?:=>|=|:)\s*["']([^"']*)["']"#).unwrap());
static TARGET_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"data-([\w-]+)-target\b").unwrap());
// `data: { controller: "x", action: "…", x_target: "…" }` in helpers and Haml.
static DATA_HASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bdata:\s*\{([^}]*)\}").unwrap());
static HASH_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\b(?:(controller|action):\s*["']([^"']*)["']|(\w+)_target:)"#).unwrap());

// An unfinished `data-controller="…`, `data-action="…`, or `controller:`/`action:` value in a `data:` hash.
static OPEN_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:data-(controller|action)["']?\s*(?:=>|=|:)|\bdata:\s*\{[^}]*\b(controller|action):)\s*["']([^"']*)$"#).unwrap()
});
// An unfinished `data-x-target="…` or `x_target: "…` value in a `data:` hash.
static OPEN_TARGET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:data-([\w-]+)-target["']?\s*(?:=>|=|:)|\bdata:\s*\{[^}]*\b(\w+)_target:)\s*["']([^"']*)$"#).unwrap());
static STATIC_LIST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"static\s+(targets|classes|outlets)\s*=\s*\[([^\]]*)\]").unwrap());
static STATIC_VALUES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"static\s+values\s*=\s*\{").unwrap());
// A Haml or Slim tag head and the attribute wrapper right after it: `%form(`, `div.card{`, `a[`.
static TAG_WRAPPER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[ \t]*[%.#]?[\w:-]+(?:[.#][\w:-]+)*[(\[{]").unwrap());
// A `data:` hash still open where the element's text ends, as in Haml's `%div{ data: { controller: "x", x_` while typing.
static OPEN_DATA_HASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bdata:\s*\{([^}]*)$").unwrap());
static HASH_CONTROLLER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bcontroller:\s*["']([^"']*)["']"#).unwrap());
static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"["']([\w-]+)["']"#).unwrap());
// `name(args) {`, `async name() {`, TS `name(): void {`, or a `name = (e) =>` class field, at the start of a line.
static METHOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^([ \t]+)(?:(?:async|public|protected|override)\s+)*([A-Za-z_$][\w$]*)\s*(?:\((?:[^()]|\([^()]*\))*\)\s*(?::[^{;]*)?\{|=\s*(?:async\s*)?(?:\((?:[^()]|\([^()]*\))*\)|\w+)\s*=>)",
    )
    .unwrap()
});
static CLASS_BODY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bclass\b[^{]*\{").unwrap());
const NOT_ACTIONS: &[&str] =
    &["if", "for", "while", "switch", "catch", "return", "function", "constructor", "initialize", "connect", "disconnect"];

/// What a Stimulus attribute value is naming at the cursor.
#[derive(Debug, PartialEq)]
pub enum Slot {
    Controller,
    /// After `id#` in an action descriptor.
    Method(String),
    /// In a `data-id-target` value.
    Target(String),
}

/// The name being typed at byte `col`: its start, the typed part, and what it names. In an action value the part
/// after `->` and before `#` names a controller, the part after `#` one of its methods.
pub fn completion_prefix(line: &str, col: usize) -> Option<(usize, &str, Slot)> {
    let before = line.get(..col)?;
    if let Some(cap) = OPEN_TARGET.captures(before) {
        let controller = cap.get(1).or(cap.get(2))?.as_str().replace('_', "-");
        let (start, typed) = last_token(cap.get(3)?);
        return typed.chars().all(is_name_char).then_some((start, typed, Slot::Target(controller)));
    }
    let cap = OPEN_VALUE.captures(before)?;
    let (token_start, token) = last_token(cap.get(3)?);
    let is_action = cap.get(1).or(cap.get(2))?.as_str() == "action";
    let skip = if is_action { token.rfind("->").map_or(0, |i| i + 2) } else { 0 };
    let typed = &token[skip..];
    if let Some((controller, method)) = typed.split_once('#').filter(|_| is_action) {
        let named = is_identifier(controller) && method.chars().all(is_name_char);
        return named.then(|| (token_start + skip + controller.len() + 1, method, Slot::Method(controller.to_string())));
    }
    if is_action && typed.contains(':') || !typed.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return None;
    }
    Some((token_start + skip, typed, Slot::Controller))
}

/// The start and text of the space-separated token a value ends with.
fn last_token(value: regex::Match<'_>) -> (usize, &str) {
    let start = value.as_str().rfind(char::is_whitespace).map_or(0, |i| i + 1);
    (value.start() + start, &value.as_str()[start..])
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

fn is_identifier(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The names in a controller's `static targets = [...]`.
pub fn targets(source: &str) -> Vec<String> {
    static_list(source, "targets")
}

/// The names in a controller's `static <kind> = [...]` (`targets`, `classes`, `outlets`).
fn static_list(source: &str, kind: &str) -> Vec<String> {
    let Some(list) = STATIC_LIST.captures_iter(source).find(|cap| &cap[1] == kind).and_then(|cap| cap.get(2)) else { return vec![] };
    QUOTED.captures_iter(list.as_str()).map(|cap| cap[1].to_string()).collect()
}

/// The keys of a controller's `static values = { name: Type, other: { type: String, default: "" } }`.
fn static_values(source: &str) -> Vec<String> {
    let Some(open) = STATIC_VALUES.find(source) else { return vec![] };
    let body = &source[open.end()..];
    let (mut depth, mut key, mut out, mut at) = (1, String::new(), Vec::new(), 0);
    while let Some(c) = body[at..].chars().next() {
        let rest = &body[at..];
        // A comment or a string says nothing about the keys.
        let skipped = if rest.starts_with("//") {
            Some(rest.find('\n').unwrap_or(rest.len()))
        } else if rest.starts_with("/*") {
            Some(rest.find("*/").map_or(rest.len(), |end| end + 2))
        } else if matches!(c, '"' | '\'' | '`') {
            Some(string_len(rest, c))
        } else {
            None
        };
        if let Some(len) = skipped {
            key.clear();
            at += len;
            continue;
        }
        at += c.len_utf8();
        match c {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' if depth == 1 => break,
            '}' | ']' | ')' => depth -= 1,
            ':' if depth == 1 && !key.is_empty() => out.push(std::mem::take(&mut key)),
            c if depth == 1 && is_name_char(c) => key.push(c),
            _ if depth == 1 => key.clear(),
            _ => {}
        }
    }
    out
}

/// The length of the string literal `rest` starts with, quotes included; a backslash escapes the next character.
fn string_len(rest: &str, quote: char) -> usize {
    let mut chars = rest.char_indices().skip(1);
    while let Some((at, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            c if c == quote => return at + c.len_utf8(),
            _ => {}
        }
    }
    rest.len()
}

/// The controllers of the element whose start tag `text` ends in: `data-controller="a b"`, or `controller:` in a `data:` hash.
pub fn element_controllers(text: &str) -> Vec<String> {
    // ERB output names a controller only at runtime, and its quotes would cut the attribute value short.
    let text = &without_erb(text);
    let attributes = ATTR.captures_iter(text).filter(|cap| &cap[1] == "controller").map(|cap| cap[2].to_string());
    let hashes = DATA_HASH.captures_iter(text).chain(OPEN_DATA_HASH.captures_iter(text)).filter_map(|cap| cap.get(1));
    let in_hashes = hashes.flat_map(|hash| HASH_CONTROLLER.captures_iter(hash.as_str()).map(|cap| cap[1].to_string()).collect::<Vec<_>>());
    let mut out: Vec<String> = Vec::new();
    for id in attributes.chain(in_hashes).flat_map(|value| value.split_whitespace().map(str::to_string).collect::<Vec<_>>()) {
        if is_identifier(&id) && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// `text` with each ERB tag, `<%` to `%>`, blanked out.
fn without_erb(text: &str) -> String {
    let mut out = text.to_string();
    for (start, end) in crate::erb::code_spans(text).into_iter().rev() {
        let (from, to) = (start - "<%".len(), (end + 1).min(text.len()));
        out.replace_range(from..to, &" ".repeat(to - from));
    }
    out
}

/// How many lines up an element's attributes may start, wrapped in `(`, `[` or `{` over several lines.
const MAX_ATTRIBUTE_LINES: usize = 20;

/// The text of the indentation-based (Haml, Slim) element the end of `before` is in: from the line that opens the
/// attribute wrapper still unclosed there, else the current line.
pub fn indented_element(before: &str) -> &str {
    let current = before.rfind('\n').map_or(0, |at| at + 1);
    let line_starts = before.rmatch_indices('\n').map(|(at, _)| at + 1).chain([0]).take(MAX_ATTRIBUTE_LINES);
    let opens_wrapper = |start: usize| {
        let wrapper = TAG_WRAPPER.find(&before[start..])?;
        unclosed(&before[start + wrapper.end() - 1..]).then_some(start)
    };
    let start = line_starts.into_iter().find_map(opens_wrapper).unwrap_or(current);
    &before[start..]
}

/// Whether `text` leaves a bracket open, counting none inside quotes.
fn unclosed(text: &str) -> bool {
    let (mut depth, mut quote) = (0i32, None);
    for c in text.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')' | ']' | '}') => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

/// The attributes the controller `id` reads from its element, given its source: `data-id-target`, and a `-value`,
/// `-class` or `-outlet` attribute for each name its class declares.
pub fn element_attributes(id: &str, source: &str) -> Vec<String> {
    let declared =
        [(static_values(source), "value"), (static_list(source, "classes"), "class"), (static_list(source, "outlets"), "outlet")];
    let named = declared.into_iter().flat_map(|(names, suffix)| names.into_iter().map(move |name| (name, suffix)));
    std::iter::once(format!("data-{id}-target"))
        .chain(named.map(|(name, suffix)| format!("data-{id}-{}-{suffix}", dasherize(&name))))
        .collect()
}

/// The Stimulus attributes for the element whose start tag `text` ends in, from its controllers in `current`'s app.
pub fn attributes(current: &Path, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for id in element_controllers(text) {
        let source = resolve(current, &id).first().and_then(|file| std::fs::read_to_string(file).ok()).unwrap_or_default();
        out.extend(element_attributes(&id, &source));
    }
    out
}

/// `fooBar` → `foo-bar`, as Stimulus maps a value or class name to its attribute.
fn dasherize(name: &str) -> String {
    name.chars().fold(String::new(), |mut out, c| {
        if c.is_ascii_uppercase() {
            out.push('-');
        }
        out.push(c.to_ascii_lowercase());
        out
    })
}

/// Methods an action can call: the controller class's members, lifecycle callbacks left out.
pub fn methods(source: &str) -> Vec<String> {
    let Some(class) = CLASS_BODY.find(source) else { return vec![] };
    let mut members = METHOD.captures_iter(&source[class.end()..]).map(|cap| (cap.get(1).map_or(0, |m| m.len()), cap[2].to_string()));
    // Class members share the first member's indentation; deeper matches are nested functions or object literals.
    let Some((indent, first)) = members.next() else { return vec![] };
    let mut out: Vec<String> = std::iter::once(first)
        .chain(members.filter(|(i, _)| *i == indent).map(|(_, name)| name))
        .filter(|name| {
            !NOT_ACTIONS.contains(&name.as_str())
                && !name.ends_with("TargetConnected")
                && !name.ends_with("TargetDisconnected")
                && !name.ends_with("ValueChanged")
        })
        .collect();
    out.dedup();
    out
}

/// The identifiers stimulus-loading registers for the files under app/javascript/controllers.
pub fn controllers(current: &Path) -> Vec<String> {
    let Some(app) = app_root(current) else { return vec![] };
    let dir = app.join("javascript/controllers");
    let mut out: Vec<String> = WalkDir::new(&dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = e.path().strip_prefix(&dir).ok()?.to_str()?.replace('\\', "/");
            let stem = ["_controller.js", "_controller.ts", "-controller.js", "-controller.ts"].iter().find_map(|s| rel.strip_suffix(s))?;
            Some(stem.replace('/', "--").replace('_', "-"))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Every controller identifier on `line`: its byte range and the identifier Stimulus would register.
pub fn identifiers(line: &str) -> Vec<(usize, usize, String)> {
    let named = |(s, e): (usize, usize)| (s, e, line[s..e].to_string());
    let mut out: Vec<_> = ATTR
        .captures_iter(line)
        .flat_map(|cap| cap.get(2).map(|value| value_identifiers(&cap[1], value)).unwrap_or_default())
        .chain(TARGET_ATTR.captures_iter(line).filter_map(|cap| cap.get(1)).map(|m| (m.start(), m.end())))
        .map(named)
        .collect();
    for hash in DATA_HASH.captures_iter(line).filter_map(|cap| cap.get(1)) {
        let at = |(s, e): (usize, usize)| (hash.start() + s, hash.start() + e);
        for cap in HASH_KEY.captures_iter(hash.as_str()) {
            match (cap.get(1), cap.get(2), cap.get(3)) {
                (Some(key), Some(value), _) => out.extend(value_identifiers(key.as_str(), value).into_iter().map(at).map(named)),
                // Rails dasherizes the key: `date_picker_target:` → `data-date-picker-target`.
                (_, _, Some(target)) => {
                    let (s, e) = at((target.start(), target.end()));
                    out.push((s, e, target.as_str().replace('_', "-")));
                }
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// Identifiers in a `controller` value (space-separated) or an `action` value (`event->id#method` descriptors).
fn value_identifiers(key: &str, value: regex::Match) -> Vec<(usize, usize)> {
    let (offset, value) = (value.start(), value.as_str());
    let mut out = Vec::new();
    let mut pos = 0;
    for token in value.split_whitespace() {
        let start = pos + value[pos..].find(token).unwrap_or(0);
        pos = start + token.len();
        let (s, e) = if key == "controller" {
            (0, token.len())
        } else {
            let Some(hash) = token.find('#') else { continue };
            (token[..hash].rfind("->").map_or(0, |i| i + 2), hash)
        };
        if s < e {
            out.push((offset + start + s, offset + start + e));
        }
    }
    out
}

/// `date-picker` → `date{_,-}picker{_,-}controller.{js,ts}`; `admin--users` → `admin/users_…`.
/// Mirrors stimulus-loading, which registers `a/b_c_controller.js` as `a--b-c`, so `_` never appears in an identifier.
pub fn resolve(current: &Path, identifier: &str) -> Vec<PathBuf> {
    if identifier.is_empty() || !identifier.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return vec![];
    }
    let Some(app) = app_root(current) else { return vec![] };
    let dir = app.join("javascript/controllers");
    let rel = identifier.replace("--", "/");
    let mut out = Vec::new();
    for name in [rel.replace('-', "_"), rel] {
        for file in
            ["_controller.js", "_controller.ts", "-controller.js", "-controller.ts"].map(|suffix| dir.join(format!("{name}{suffix}")))
        {
            if file.is_file() && !out.contains(&file) {
                out.push(file);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_controllers_from_attributes_and_data_hashes() {
        assert_eq!(element_controllers(r#"<div class="x" data-controller="search admin--users" data-"#), ["search", "admin--users"]);
        assert_eq!(element_controllers(r#"%div{ data: { controller: "search", sea"#), ["search"]);
        assert_eq!(element_controllers(r#"div data-controller="a" data-controller="a b" "#), ["a", "b"]);
        assert!(element_controllers(r#"<div data-action="click->search#go" "#).is_empty());
        assert_eq!(element_controllers(r#"<div data-controller="<%= name %> search" "#), ["search"], "ERB output names nothing");
        assert!(element_controllers(r#"<div data-controller="<%= admin ? "admin" : "user" %>" data-"#).is_empty(), "quotes in ERB");
    }

    #[test]
    fn element_attributes_from_the_controller_class() {
        let source = r#"export default class extends Controller {
  static targets = ["input", "results"]
  static values = { url: String, perPage: { type: Number, default: 10 } }
  static classes = ["loading"]
  static outlets = ["result-list"]
}"#;
        assert_eq!(
            element_attributes("search", source),
            [
                "data-search-target",
                "data-search-url-value",
                "data-search-per-page-value",
                "data-search-loading-class",
                "data-search-result-list-outlet"
            ]
        );
        assert_eq!(element_attributes("plain", ""), ["data-plain-target"]);
        let commented = "static values = {\n  // note: in seconds\n  delay: Number, /* skip: x */ label: { default: \"a: b\" }\n}";
        assert_eq!(static_values(commented), ["delay", "label"]);
        let escaped = r#"static values = { label: { default: "say \"hi\" {" }, wait: Number }"#;
        assert_eq!(static_values(escaped), ["label", "wait"]);
    }

    #[test]
    fn indented_elements_span_wrapped_attributes() {
        assert_eq!(indented_element("p\ndiv(\n  data-controller=\"a\"\n  data-"), "div(\n  data-controller=\"a\"\n  data-");
        assert_eq!(indented_element("div(a=\"1\")\np data-"), "p data-");
        assert_eq!(indented_element("p data-controller=\"old\" (see below\ndiv data-"), "div data-", "a bracket in text");
        assert_eq!(indented_element("%div{ title: \"(\" }\n%p(data-"), "%p(data-", "a bracket in a value");
        assert_eq!(indented_element("%form{ data: { a: 1 },\n  data-"), "%form{ data: { a: 1 },\n  data-");
    }

    #[test]
    fn identifiers_in_attributes_and_hashes() {
        let ids = |line: &str| identifiers(line).into_iter().map(|(_, _, id)| id).collect::<Vec<_>>();
        assert_eq!(
            ids(r#"<div data-controller="admin--users hello" data-action="click->hello#greet keydown.enter->x#y:prevent z#w">"#),
            ["admin--users", "hello", "hello", "x", "z"]
        );
        assert_eq!(ids(r#"<input data-date-picker-target="name">"#), ["date-picker"]);
        assert_eq!(
            ids(r#"<%= tag.div data: { controller: "hello", action: "hello#go", date_picker_target: "x" } do %>"#),
            ["hello", "hello", "date-picker"]
        );
        let line = r#"<p data: { date_picker_target: "x" }>"#;
        let (s, e, _) = identifiers(line)[0];
        assert_eq!(&line[s..e], "date_picker");
        assert_eq!(ids(r#"%div{ "data-controller" => "hello" }"#), ["hello"]);
        assert_eq!(ids(r#"div data-controller="hello""#), ["hello"]);
        assert!(ids(r#"<%= link_to "x", controller: "users", action: "index" %>"#).is_empty());
        assert!(ids(r#"<div data-action="click->nope">"#).is_empty());
    }

    #[test]
    fn completion_prefixes() {
        let at_end = |line: &str| completion_prefix(line, line.len()).map(|(s, typed, slot)| (s, typed.to_string(), slot));
        let controller = |s, typed: &str| Some((s, typed.to_string(), Slot::Controller));
        assert_eq!(at_end(r#"<div data-controller="he"#), controller(22, "he"));
        assert_eq!(at_end(r#"<div data-controller="hello da"#), controller(28, "da"));
        assert_eq!(at_end(r#"<div data-controller=""#), controller(22, ""));
        assert_eq!(at_end(r#"<div data-action="click->he"#), controller(25, "he"));
        assert_eq!(at_end(r#"<div data-action="he"#), controller(18, "he"));
        assert_eq!(at_end(r#"%div{data: {controller: "x", action: "click->y"#), controller(45, "y"));
        assert_eq!(at_end(r#"div data-controller='a--"#), controller(21, "a--"));
        assert_eq!(at_end(r#"<div data-action="click->hello#gr"#), Some((31, "gr".into(), Slot::Method("hello".into()))));
        assert_eq!(at_end(r#"<a data-action="x#a admin--users#"#), Some((33, "".into(), Slot::Method("admin--users".into()))));
        assert_eq!(at_end(r#"<input data-date-picker-target="a in"#), Some((34, "in".into(), Slot::Target("date-picker".into()))));
        assert_eq!(
            at_end(r#"= f.text_field :x, data: { date_picker_target: ""#),
            Some((48, "".into(), Slot::Target("date-picker".into())))
        );
        for line in [
            r#"<div data-action="click->hello#go:prevent"#,
            r#"<div data-action="click->#go"#,
            r#"<div data-controller="x">"#,
            r#"<%= link_to "x", controller: "us"#,
        ] {
            assert_eq!(at_end(line), None, "{line}");
        }
    }

    #[test]
    fn controller_members() {
        let source = r#"import { Controller } from "@hotwired/stimulus"
const options = {
    onOpen() {},
}
export default class extends Controller {
  static targets = [ "name",
    'output' ]
  static values = { url: String }

  connect() {
    this.items = { open() {} }
    if (this.hasNameTarget) {
      fetch(this.urlValue).then((r) => r.json())
    }
  }

  async greet(event) {
    this.outputTarget.textContent = `Hi ${this.nameTarget.value}`
  }

  reset = () => {
    this.nameTarget.value = ""
  }

  nameTargetConnected(element) {}

  submit(): void {
  }

  search(event = new Event("x", { bubbles: true })) {}
}
"#;
        assert_eq!(targets(source), ["name", "output"]);
        assert_eq!(methods(source), ["greet", "reset", "submit", "search"]);
        assert!(targets("export default class extends Controller {}").is_empty());
    }

    #[test]
    fn resolves_controller_files() {
        let root = std::env::temp_dir().join(format!("rt-stimulus-{}", std::process::id()));
        let controllers = root.join("app/javascript/controllers");
        std::fs::create_dir_all(controllers.join("admin")).unwrap();
        std::fs::create_dir_all(root.join("app/views/users")).unwrap();
        for f in [
            "hello_controller.js",
            "admin/users_controller.ts",
            "date_picker_controller.js",
            "color-picker_controller.js",
            "tab-list-controller.ts",
        ] {
            std::fs::write(controllers.join(f), "").unwrap();
        }
        let current = root.join("app/views/users/index.html.erb");
        assert_eq!(resolve(&current, "hello"), [controllers.join("hello_controller.js")]);
        assert_eq!(resolve(&current, "admin--users"), [controllers.join("admin/users_controller.ts")]);
        assert_eq!(resolve(&current, "date-picker"), [controllers.join("date_picker_controller.js")]);
        assert_eq!(resolve(&current, "color-picker"), [controllers.join("color-picker_controller.js")]);
        assert_eq!(resolve(&current, "tab-list"), [controllers.join("tab-list-controller.ts")]);
        assert_eq!(super::controllers(&current), ["admin--users", "color-picker", "date-picker", "hello", "tab-list"]);
        std::fs::create_dir_all(controllers.join("a")).unwrap();
        std::fs::write(controllers.join("a/b_controller.js"), "").unwrap();
        std::fs::write(root.join("app/x_controller.js"), "").unwrap();
        assert_eq!(resolve(&current, "a--b"), [controllers.join("a/b_controller.js")]);
        // Stimulus registers these files under other identifiers (`date-picker`, `a--b`); paths must stay inside controllers/.
        for id in ["missing", "date_picker", "a/b", "../x", "../../app/x", "Hello", ""] {
            assert!(resolve(&current, id).is_empty(), "{id}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
