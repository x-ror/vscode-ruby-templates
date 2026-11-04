//! Resolve `render "partial"` and friends to files; list partials for completion.

use crate::ivars;
use crate::prism;
use crate::project::Project;
use crate::rails_paths::{app_root, template_format, template_stem, template_suffix, template_variant, views_root};
use crate::render_call::{self, Call, HamlPipes, Pair, MAX_CALL_LINES};
use crate::ruby::{self, CodeEnd};
use crate::strict_locals::{self, Close};
use regex::{Captures, Regex};
use std::borrow::Cow;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

static RENDER_STR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:render|json\.partial!)\s*\(?\s*(?::?(partial|template|layout)\s*(?::|=>)\s*)?["']([\w./-]+)["']"#).unwrap()
});
// `render formats: [:json], partial: "y"`: the first key, then (`top_level_keys`) the later ones that name a file.
// After a bare `render "y", layout: …` the hash is just locals.
static RENDER_OPTIONS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\brender\b\s*\(?\s*:?(\w+)\s*(?::|=>)").unwrap());
// At the start of the arguments too: `json.comments partial: "…"` has no key before it.
static OPTION_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:^|,)\s*:?(partial|template|layout)\s*(?::|=>)\s*["']([\w./-]+)["']"#).unwrap());
// The block that makes a lone `layout:` a partial: `do`, `do |x|` or `{ |x|` after it.
static BLOCK_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bdo\b\s*(?:\||[-=]?\s*(?:%>|$))|\{\s*\|").unwrap());
// Jbuilder's `json.array! @posts, partial: "posts/post", as: :post` and `json.comments @c, partial: "…"`.
// A Haml/Slim element before its code (`#main=`, `%div.a#b=`): its `#` starts an id, not a comment.
static ELEMENT_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:%[\w:-]+|[.#][\w-]+)(?:[.#][\w-]+)*").unwrap());
static JSON_CALL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bjson\.\w+!?[\s(]").unwrap());
// Each parse of a candidate costs its length; a call has a few file-naming keys, so a longer run is noise.
const MAX_KEY_CANDIDATES: usize = 8;
static RENDER_COMPONENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"render\s*\(?\s*((?:[A-Z]\w*::)*[A-Z]\w*Component)\b").unwrap());
static RENDER_MODEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"render\s*\(?\s*(@\w+(?:\.\w+)?)").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderKind {
    Partial,
    Template,
}

impl RenderKind {
    pub fn name(self) -> &'static str {
        match self {
            RenderKind::Partial => "Partial",
            RenderKind::Template => "Template",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RenderRef {
    pub kind: RenderKind,
    pub name: String,
    /// Byte range of the string literal (without quotes) inside the line.
    pub start: usize,
    pub end: usize,
    /// Byte start of the call (`render`, `json.partial!`, `json.array!`) the string belongs to; 0 when it began on an earlier line.
    pub call: usize,
    /// The `layout:` of a `render partial:|template:` call; it receives that call's locals.
    pub from_layout: bool,
    /// The call's literal `formats:`, `variants:` and `handlers:`, which narrow the files Rails picks.
    pub lookup: Lookup,
}

/// The lookup details a `render partial:|template:` call names; `None` where it names none (or not literally).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Lookup {
    pub formats: Option<Vec<String>>,
    pub variants: Option<Vec<String>>,
    pub handlers: Option<Vec<String>>,
}

/// Every `render "..."` reference on a line, in order.
/// ActionView 7.0 to 8.1: a view's `render layout: "x"` (with a block, or next to `partial:`) renders the `_x` partial;
/// next to `template:` the layout is a template path.
pub fn render_refs(line: &str) -> Vec<RenderRef> {
    let mut refs = refs_without_lookups(line);
    // Only the options form has lookup options.
    let lookups: Vec<_> = refs
        .iter()
        .map(|r| {
            Some(if RenderForm::of(line, r).options {
                read_call_with_lookup(line, r, false, ruby_prism::Version::Latest).1
            } else {
                Lookup::default()
            })
        })
        .collect();
    set_lookups(&mut refs, &lookups);
    refs
}

fn render_kind(key: &str) -> RenderKind {
    if key == "template" {
        RenderKind::Template
    } else {
        RenderKind::Partial
    }
}

fn render_ref(line: &str, call: usize, kind: RenderKind, name: Range<usize>, from_layout: bool) -> RenderRef {
    RenderRef { kind, name: line[name.clone()].to_string(), start: name.start, end: name.end, call, from_layout, lookup: Lookup::default() }
}

/// Whether `at` in `line` is Ruby code, not inside a string or comment; ERB text before the last `<%` is not code to scan.
fn is_code_at(line: &str, at: usize) -> bool {
    let before = &line[..at];
    if let Some(open) = before.rfind("<%") {
        return ruby::code_end(&before[open + 2..]) == CodeEnd::Code;
    }
    let before = &before[ELEMENT_PREFIX.find(before).map_or(0, |element| element.end())..];
    // Haml/Slim text before an open `#{` isn't Ruby: `p It's #{render …`.
    ruby::code_end(before) == CodeEnd::Code
        || before.rfind("#{").is_some_and(|open| !before[open..].contains('}') && ruby::code_end(&before[open + 2..]) == CodeEnd::Code)
}

/// Whether the `render`/`json.` call matched at `at` has no receiver: `markdown.render "x"`, `@r.render "x"` and `md_render "x"` are
/// other methods. `self.render` is the view's own.
fn is_receiverless(line: &str, at: usize) -> bool {
    let before = &line[..at];
    let before = before.strip_suffix("self.").unwrap_or(before);
    !before.ends_with(|c: char| c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '@' | '$'))
}

/// Bracket depth at the end of `code`, strings and comments aside.
fn bracket_depth(code: &str) -> isize {
    ruby::strip(code)
        .chars()
        .map(|c| match c {
            '(' | '[' | '{' => 1,
            ')' | ']' | '}' => -1,
            _ => 0,
        })
        .sum()
}

/// Each match of `heads` in code, with the text after it up to its `%>` or the next match outside its brackets: a call's
/// own arguments (a `render(…)` nested in them doesn't end them).
fn call_tails<'l>(line: &'l str, heads: &'l Regex) -> Vec<(Captures<'l>, Range<usize>)> {
    let heads: Vec<_> = heads
        .captures_iter(line)
        .filter(|head| head.get(0).is_some_and(|m| is_receiverless(line, m.start()) && is_code_at(line, m.start())))
        .collect();
    let ranges = heads.iter().enumerate().map(|(i, head)| {
        let start = head.get(0).map_or(line.len(), |m| m.end());
        let tag_end = line[start..].find("%>").map_or(line.len(), |end| start + end);
        let next = heads[i + 1..].iter().filter_map(|next| next.get(0)).map(|next| next.start()).take_while(|&next| next < tag_end);
        let end = next.into_iter().find(|&next| bracket_depth(&line[start..next]) <= 0).unwrap_or(tag_end);
        start..end
    });
    let ranges: Vec<_> = ranges.collect();
    heads.into_iter().zip(ranges).collect()
}

/// The file-naming keys of a call, `tail` being its arguments, as (key, name span in `tail`): those that are top-level, so
/// what precedes them is balanced and in one statement; a nested `{ partial: … }` or a string holding `, partial: …` is skipped.
fn top_level_keys(tail: &str) -> impl Iterator<Item = (&str, Range<usize>)> {
    // A `%>` ends the call, and keeps the parsed prefixes short.
    let tail = tail.find("%>").map_or(tail, |end| &tail[..end]);
    let keys: Vec<_> = OPTION_KEY.captures_iter(tail).filter_map(|cap| Some((cap.get(0)?.start(), cap))).collect();
    let nested = inside_brackets(tail, keys.iter().map(|(start, _)| *start));
    // Clearly nested candidates are skipped without using up the bound; Prism judges the rest.
    let candidates = keys.into_iter().zip(nested).filter(|(_, nested)| !nested).map(|(key, _)| key);
    candidates.take(MAX_KEY_CANDIDATES).filter_map(move |(start, cap)| {
        let before = &tail[..start];
        (!ruby::comment_before(before) && prism::is_one_statement(before)).then_some((cap.get(1)?.as_str(), cap.get(2)?.range()))
    })
}

/// Whether each of `offsets` (ascending) is clearly inside brackets, in one pass. Never true once a `%`, `?`, `/`, `` ` `` or `#`
/// (outside a `'` string) has been seen: this scan can't tell a literal, interpolation or comment from an operator there, and a
/// wrong "nested" would hide a real key.
fn inside_brackets(code: &str, offsets: impl IntoIterator<Item = usize>) -> Vec<bool> {
    let (mut depth, mut unsure, mut quote) = (0isize, false, None);
    let mut chars = code.char_indices().peekable();
    offsets
        .into_iter()
        .map(|offset| {
            while let Some((_, c)) = chars.next_if(|&(at, _)| at < offset) {
                match (quote, c) {
                    (Some(_), '\\') => {
                        chars.next();
                    }
                    (Some(q), c) if c == q => quote = None,
                    (Some('"'), '#') => unsure = true,
                    (Some(_), _) => {}
                    (None, '"' | '\'') => quote = Some(c),
                    (None, '(' | '[' | '{') => depth += 1,
                    (None, ')' | ']' | '}') => depth -= 1,
                    (None, '%' | '?' | '/' | '#' | '`') => unsure = true,
                    _ => {}
                }
            }
            !unsure && depth > 0
        })
        .collect()
}

/// Whether the call `head` + `args` (cut at its `%>`) takes a block right after `after_layout`: a `do` or `{ |x|` in the call's
/// own code, not one in a string or a nested call.
fn opens_block(head: &str, args: &str, after_layout: usize) -> bool {
    let args = args.find("%>").map_or(args, |end| &args[..end]);
    BLOCK_OPEN.find_iter(&args[after_layout..]).take(MAX_KEY_CANDIDATES).any(|opener| {
        let at = after_layout + opener.start();
        let before = args[..at].trim_end();
        let call_closed = head.contains('(') && before.ends_with(')');
        let before = if call_closed { &before[..before.len() - 1] } else { before };
        // `do` binds to the call; a `{` only when nothing but the layout's value or the call's `)` precedes it.
        let binds_to_call = opener.as_str().starts_with("do") || call_closed || before.ends_with(['"', '\'']);
        binds_to_call && ruby::code_end(before) == CodeEnd::Code && prism::is_one_statement(before)
    })
}

/// The refs in `render` options before or after the file's name: `render formats: [:json], partial: "y"`, `render
/// partial: "y", layout: "x"` and a block `render formats: [:json], layout: "x" do`.
fn option_refs(line: &str) -> Vec<RenderRef> {
    let mut refs = vec![];
    for (head, tail) in call_tails(line, &RENDER_OPTIONS) {
        let (Some(call), Some(first)) = (head.get(0), head.get(1)) else { continue };
        let named_first = matches!(first.as_str(), "partial" | "template");
        let (mut name, mut layout) = (None, None);
        for (key, span) in top_level_keys(&line[tail.clone()]) {
            let span = tail.start + span.start..tail.start + span.end;
            if key == "layout" {
                layout.get_or_insert(span);
            } else if !named_first {
                name.get_or_insert((key, span));
            }
            if layout.is_some() && (named_first || name.is_some()) {
                break;
            }
        }
        let kind = if named_first { Some(render_kind(first.as_str())) } else { name.as_ref().map(|(key, _)| render_kind(key)) };
        if let Some((key, span)) = name {
            refs.push(render_ref(line, call.start(), render_kind(key), span, false));
        }
        let Some(layout) = layout else { continue };
        // A layout with no file next to it renders the partial itself, only with a block, like `render layout: "x" do`.
        if kind.is_none() && !opens_block(call.as_str(), &line[tail.clone()], layout.end - tail.start) {
            continue;
        }
        refs.push(render_ref(line, call.start(), kind.unwrap_or(RenderKind::Partial), layout, kind.is_some()));
    }
    refs
}

fn refs_without_lookups(line: &str) -> Vec<RenderRef> {
    // Every pattern below needs one of these; most lines have neither, and the regexes cost per line.
    if !line.contains("render") && !line.contains("partial") {
        return vec![];
    }
    let calls = RENDER_STR.captures_iter(line).filter_map(|cap| {
        let (call, name) = (cap.get(0)?, cap.get(2)?);
        if !is_receiverless(line, call.start()) {
            return None;
        }
        let key = cap.get(1).map_or("", |key| key.as_str());
        // `render layout: "x"` names a partial only with a block or a `partial:`/`template:` next to it.
        let (quoted, after) = (&line[name.start() - 1..], &line[name.end() + 1..]);
        if key == "layout" && !opens_block(call.as_str(), quoted, name.len() + 2) && !top_level_keys(after).any(|(key, _)| key != "layout")
        {
            return None;
        }
        Some(render_ref(line, call.start(), render_kind(key), name.range(), false))
    });
    let json = call_tails(line, &JSON_CALL).into_iter().filter_map(|(call, tail)| {
        let (_, name) = top_level_keys(&line[tail.clone()]).find(|(key, _)| *key == "partial")?;
        Some(render_ref(line, call.get(0)?.start(), RenderKind::Partial, tail.start + name.start..tail.start + name.end, false))
    });
    let mut refs: Vec<RenderRef> = calls.chain(option_refs(line)).chain(json).collect();
    refs.sort_by_key(|r| r.start);
    // `json.partial! partial: "x"` matches both RENDER_STR and the Jbuilder path.
    refs.dedup_by_key(|r| r.start);
    refs
}

/// `render_refs` for the line of `text` at `line_start`, keeping the refs `keep` accepts, with lookups read from the whole
/// call: `formats:` may sit on a later line. Nothing is parsed for a line with no ref to keep. `pipes`: of `text`.
pub fn render_refs_in(
    text: &str,
    line_start: usize,
    line: &str,
    pipes: &HamlPipes,
    ruby: ruby_prism::Version,
    keep: impl FnMut(&RenderRef) -> bool,
) -> Vec<RenderRef> {
    render_calls_in(text, line_start, line, pipes, ruby, keep).into_iter().map(|(r, _)| r).collect()
}

/// A ref keyed on a line that continues a `render` call from an earlier line, and the same ref where that line starts.
struct Continued {
    on_line: RenderRef,
    call_line_start: usize,
    on_call_line: RenderRef,
}

/// The line of `text` before `line_start` that starts the `render` or `json.` call it continues after trailing commas or an open paren.
fn call_line_before(text: &str, line_start: usize, pipes: &HamlPipes) -> Option<usize> {
    let mut start = line_start;
    // `window_end` reads MAX_CALL_LINES lines from the call line, so the continued line must be within them.
    for _ in 0..MAX_CALL_LINES - 1 {
        let before = text[..start].strip_suffix('\n')?;
        let previous = before.rfind('\n').map_or(0, |at| at + 1);
        let above = before[previous..].trim_end();
        let above = if pipes.apply(text, previous) { above.strip_suffix('|').unwrap_or(above).trim_end() } else { above };
        // The trailing comma or open paren is also what keeps this to the few lines that continue a call.
        if !above.ends_with([',', '(']) {
            return None;
        }
        let line_above = &before[previous..];
        let call_at = [line_above.find("render"), JSON_CALL.find(line_above).map(|m| m.start())].into_iter().flatten().min();
        // A `render (` in a trailing comment doesn't open a call.
        if call_at.is_some_and(|at| is_code_at(line_above, at)) {
            return Some(previous);
        }
        start = previous;
    }
    None
}

/// The refs on the line at `line_start` whose `render` or `json.` call began on an earlier line:
/// `render formats: [:json],` and, on the next line, `partial: "users/user"`.
fn continued_refs(text: &str, line_start: usize, line: &str, pipes: &HamlPipes) -> Vec<Continued> {
    // A file name is quoted; a bare one may follow an open paren, like `render(` and `"users/user"`.
    if !line.contains(['"', '\'']) {
        return vec![];
    }
    let Some(call_line_start) = call_line_before(text, line_start, pipes) else { return vec![] };
    let shift = line_start - call_line_start;
    let joined = match pipes.apply(text, call_line_start) {
        true => render_call::haml_group_joined(&text[call_line_start..]),
        false => text[call_line_start..].into(),
    };
    let Some(joined) = joined.get(..shift + line.len()) else { return vec![] };
    refs_without_lookups(joined)
        .into_iter()
        .filter(|r| r.call < shift && r.start >= shift)
        .map(|on_call_line| Continued {
            on_line: RenderRef { start: on_call_line.start - shift, end: on_call_line.end - shift, call: 0, ..on_call_line.clone() },
            call_line_start,
            on_call_line,
        })
        .collect()
}

/// `render_refs_in` with each ref's call, read once. A ref on a continuation line has its `call` at 0, and its call's
/// offsets start on the call's line (see `RenderCall::shift`).
pub fn render_calls_in<'a>(
    text: &'a str,
    line_start: usize,
    line: &str,
    pipes: &HamlPipes,
    ruby: ruby_prism::Version,
    mut keep: impl FnMut(&RenderRef) -> bool,
) -> Vec<(RenderRef, RenderCall<'a>)> {
    let mut refs = refs_without_lookups(line);
    let own = refs.len();
    let continued = continued_refs(text, line_start, line, pipes);
    refs.extend(continued.iter().map(|c| c.on_line.clone()));
    let kept: Vec<bool> = refs.iter().map(&mut keep).collect();
    if !kept.contains(&true) {
        return vec![];
    }
    let haml = pipes.apply(text, line_start);
    // A kept `layout:` takes its partial's lookup, so that call is read too.
    // A continued ref's `call` is 0 in its own line, so it pairs only with refs from the same origin.
    let needed = |i: usize| {
        kept[i]
            || refs.iter().zip(&kept).enumerate().any(|(j, (r, &k))| k && r.from_layout && r.call == refs[i].call && (j < own) == (i < own))
    };
    let read = |i: usize| match i.checked_sub(own).and_then(|j| continued.get(j)) {
        Some(c) => {
            let (call, lookup) =
                read_call_with_lookup(&text[c.call_line_start..], &c.on_call_line, pipes.apply(text, c.call_line_start), ruby);
            (RenderCall { shift: line_start - c.call_line_start, ..call }, lookup)
        }
        None => read_call_with_lookup(&text[line_start..], &refs[i], haml, ruby),
    };
    let (calls, lookups): (Vec<_>, Vec<_>) = (0..refs.len())
        .map(|i| match needed(i) {
            true => {
                let (call, lookup) = read(i);
                (Some(call), Some(lookup))
            }
            false => (None, None),
        })
        .unzip();
    let (own_refs, after_call) = refs.split_at_mut(own);
    set_lookups(own_refs, &lookups[..own]);
    set_lookups(after_call, &lookups[own..]);
    refs.into_iter().zip(calls).zip(kept).filter_map(|((r, call), k)| k.then_some((r, call?))).collect()
}

/// Give each ref the lookup of its call, from `lookups[ref]` (`None`: not needed); a `layout:` takes its partial's (the
/// nearest one before it, else after), since Rails resolves both with the same details; a `layout:` alone reads its own call.
fn set_lookups(refs: &mut [RenderRef], lookups: &[Option<Lookup>]) {
    for (r, lookup) in refs.iter_mut().zip(lookups).filter(|(r, _)| !r.from_layout) {
        if let Some(lookup) = lookup {
            r.lookup = lookup.clone();
        }
    }
    for i in 0..refs.len() {
        if !refs[i].from_layout {
            continue;
        }
        let layout = &refs[i];
        let partners = refs.iter().filter(|r| !r.from_layout && r.call == layout.call);
        let partner = partners.clone().rfind(|r| r.start < layout.start).or_else(|| partners.clone().next());
        refs[i].lookup = match partner {
            Some(partner) => partner.lookup.clone(),
            None => lookups[i].clone().unwrap_or_default(),
        };
    }
}

/// The `formats:`, `variants:` and `handlers:` of `r`'s call in `call`, before or after the name. Only the options form has
/// them: in `render "x", formats: [:json]` the hash is the partial's locals.
fn lookup(parsed: &Call, form: &RenderForm) -> Lookup {
    if !form.options {
        return Lookup::default();
    }
    let option = |name: &str| parsed.pairs.iter().find(|pair| pair.key == name).and_then(|pair| literal_names(pair.value));
    Lookup { formats: option("formats"), variants: option("variants"), handlers: option("handlers") }
}

/// The names in a literal `:json`, `"json"`, `[:json, :html]` or `%i[json html]`; `None` for anything computed.
fn literal_names(value: &str) -> Option<Vec<String>> {
    let value = value.trim();
    let words = value.strip_prefix("%i").or_else(|| value.strip_prefix("%w"));
    let is_name = |name: &str| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if let Some(list) = words {
        let names = list.trim_start_matches(['[', '(']).trim_end_matches([']', ')']).split_whitespace();
        return names.map(|name| is_name(name).then(|| name.to_string())).collect();
    }
    let list = value.strip_prefix('[').and_then(|list| list.strip_suffix(']')).unwrap_or(value);
    list.split(',')
        .map(|item| {
            let item = item.trim();
            // A bare word is a variable; only a symbol or a string names a value.
            let name = item.strip_prefix(':').or_else(|| item.strip_prefix(['"', '\'']).and_then(|rest| rest.strip_suffix(['"', '\''])))?;
            is_name(name).then(|| name.to_string())
        })
        .collect()
}

/// Find a render reference whose string spans `col` (byte column) on `line`, the line of `text` at `line_start`.
pub fn render_ref_at(text: &str, line_start: usize, line: &str, haml: bool, ruby: ruby_prism::Version, col: usize) -> Option<RenderRef> {
    let spans_col = |r: &RenderRef| col >= r.start.saturating_sub(1) && col <= r.end + 1;
    render_refs_in(text, line_start, line, &HamlPipes::new(haml), ruby, spans_col).into_iter().next()
}

pub fn component_ref_at(line: &str, col: usize) -> Option<String> {
    for cap in RENDER_COMPONENT.captures_iter(line) {
        let m = cap.get(1)?;
        if is_receiverless(line, cap.get(0)?.start()) && col >= m.start() && col <= m.end() {
            return Some(m.as_str().to_string());
        }
    }
    None
}

/// `render @user` / `@users` / `@post.comments`: byte range of the expression and the name its partial follows.
/// Longer chains and calls (`@posts.page(1)`, `@post.comments.order(:id)`) name a query, not a model.
pub fn model_renders(line: &str) -> Vec<(usize, usize, &str)> {
    if !line.contains("render") {
        return vec![];
    }
    RENDER_MODEL
        .captures_iter(line)
        .filter_map(|cap| {
            let m = cap.get(1)?;
            if !is_receiverless(line, cap.get(0)?.start()) || line[m.end()..].starts_with(['.', '(']) {
                return None;
            }
            Some((m.start(), m.end(), m.as_str().rsplit(['.', '@']).next()?))
        })
        .collect()
}

/// `user`/`users` → app/views/users/_user.* (`to_partial_path`); nothing when the inflection finds no file.
/// A model that only exists namespaced (`app/models/admin/user.rb`) looks in `admin/users`. A view under a namespace
/// directory gets it prefixed first, as Rails does with `prefix_partial_path_with_controller_namespace`.
pub fn resolve_model(project: &Project, current: &Path, name: &str) -> Vec<PathBuf> {
    let Some(views) = views_root(current) else { return vec![] };
    let singular = singularize(name);
    let plural = pluralize(&singular);
    let view_dir = current.parent().and_then(|dir| dir.strip_prefix(&views).ok()).unwrap_or(Path::new(""));
    let mut namespaces = model_namespaces(project, &views, current, &singular);
    namespaces.push(PathBuf::new());
    for namespace in namespaces {
        let prefixed = views.join(prefixed_with_view_namespace(view_dir, &namespace)).join(&plural);
        // Rails raises MissingTemplate rather than trying the model's own folder, but only for a controller's action views.
        let dirs = if project.prefixes_partial_paths && !is_partial_file(current) && is_controller_folder(&views, view_dir) {
            vec![prefixed]
        } else {
            vec![prefixed, views.join(namespace.join(&plural))]
        };
        for dir in dirs {
            let mut found: Vec<PathBuf> = matching_templates(project, &dir, &format!("_{singular}"))
                .into_iter()
                .filter(|p| renders_format(project, current, p))
                .collect();
            if !found.is_empty() {
                sort_like_action_view(project, current, &Lookup::default(), &mut found);
                return found;
            }
        }
    }
    vec![]
}

fn is_partial_file(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with('_'))
}

/// Is `views/<view_dir>` the folder of an existing controller (`admin/users` for `admin/users_controller.rb`)?
fn is_controller_folder(views: &Path, view_dir: &Path) -> bool {
    let Some(app) = views.parent() else { return false };
    app.join("controllers").join(format!("{}_controller.rb", view_dir.display())).is_file()
}

/// The model's namespace behind the namespace directories of `view_dir` (all but its last part) that it doesn't start with
/// (ActionView's `merge_prefix_into_object_path`): `admin/orders` + `` → `admin`, `shop/orders` + `admin` → `shop/admin`.
fn prefixed_with_view_namespace(view_dir: &Path, model_namespace: &Path) -> PathBuf {
    let dirs: Vec<_> = view_dir.components().collect();
    let Some((_, view_namespace)) = dirs.split_last() else { return model_namespace.to_path_buf() };
    let mut model_parts = model_namespace.components();
    let prefix: PathBuf = view_namespace.iter().map_while(|dir| (Some(*dir) != model_parts.next()).then_some(dir)).collect();
    prefix.join(model_namespace)
}

/// The namespaces of `app/models/<namespace>/<singular>.rb`, those holding the view `current` first; none when a top-level
/// `app/models/<singular>.rb` exists, since the variable is likely that one. `concerns/` is no namespace.
fn model_namespaces(project: &Project, views: &Path, current: &Path, singular: &str) -> Vec<PathBuf> {
    let Some(models) = views.parent().map(|app| app.join("models")) else { return vec![] };
    let file = format!("{singular}.rb");
    if models.join(&file).is_file() {
        return vec![];
    }
    let mut namespaces: Vec<PathBuf> = project
        .namespaced_models(&models)
        .iter()
        .filter(|path| path.file_name().is_some_and(|name| name == file.as_str()))
        .filter_map(|path| Some(path.parent()?.to_path_buf()))
        .collect();
    let view_dir = current.parent().and_then(|dir| dir.strip_prefix(views).ok());
    let outside_view = |namespace: &Path| !view_dir.is_some_and(|dir| dir.starts_with(namespace));
    namespaces.sort_by(|a, b| (outside_view(a), a).cmp(&(outside_view(b), b)));
    namespaces
}

/// Where a missing `dir/name` reference would live: `app/views/dir/_name.<format>+<variant>.<ext>` with the first of the
/// call's `formats:`, `variants:` and `handlers:`, else the view's format (variant dropped) and template language.
pub fn new_template_path(project: &Project, current: &Path, r: &RenderRef) -> Option<PathBuf> {
    let (Some(dir), base) = split_render_name(&r.name) else { return None };
    if base.is_empty() || dir.split('/').any(|part| part == "..") {
        return None;
    }
    let first = |names: &Option<Vec<String>>| names.as_ref().and_then(|names| names.first()).cloned();
    let ext = first(&r.lookup.handlers).or_else(|| Some(current.extension()?.to_str()?.to_string()))?;
    let prefix = if r.kind == RenderKind::Partial { "_" } else { "" };
    let format = first(&r.lookup.formats).or_else(|| template_format(|f| project.is_format(f), current).map(str::to_string));
    let format = format.map(|format| format!(".{format}")).unwrap_or_default();
    let variant = first(&r.lookup.variants).map(|variant| format!("+{variant}")).unwrap_or_default();
    let file = format!("{prefix}{base}{format}{variant}.{ext}");
    Some(views_root(current)?.join(dir).join(file))
}

/// The `resolve` results Rails would actually pick from: it stops at the first folder with a match, so later
/// ones (`application/`) are shadowed.
pub fn first_folder_only(mut found: Vec<PathBuf>) -> Vec<PathBuf> {
    let first = found.first().and_then(|f| f.parent()).map(Path::to_path_buf);
    found.retain(|f| f.parent() == first.as_deref());
    found
}

/// The formats Rails looks a partial up in for a view: its own, plus HTML from JS (`LookupContext#formats=`) and
/// from turbo_stream (its request's Accept header is `text/vnd.turbo-stream.html, text/html`).
fn lookup_formats(view: &str) -> impl Iterator<Item = &str> {
    std::iter::once(view).chain(matches!(view, "js" | "turbo_stream").then_some("html"))
}

/// Format-less files match any format.
fn renders_format(project: &Project, current: &Path, candidate: &Path) -> bool {
    let is_format = |f: &str| project.is_format(f);
    match (template_format(is_format, current), template_format(is_format, candidate)) {
        (Some(view), Some(partial)) => lookup_formats(view).any(|format| format == partial),
        _ => true,
    }
}

// Suffix-matched like ActiveSupport's irregulars, so `sales_person` → `sales_people`.
const IRREGULAR: [(&str, &str); 10] = [
    ("person", "people"),
    ("child", "children"),
    ("movie", "movies"),
    ("status", "statuses"),
    ("alias", "aliases"),
    ("cookie", "cookies"),
    ("hero", "heroes"),
    ("quiz", "quizzes"),
    ("cache", "caches"),
    ("analysis", "analyses"),
];
const UNCOUNTABLE: [&str; 9] = ["equipment", "information", "rice", "money", "species", "series", "fish", "sheep", "news"];

/// Matched on the last `_` segment so `prices` isn't read as `rice`.
fn uncountable(word: &str) -> bool {
    UNCOUNTABLE.contains(&word.rsplit('_').next().unwrap_or(word))
}

fn ends_with_consonant_y(word: &str) -> bool {
    word.strip_suffix('y').and_then(|w| w.chars().last()).is_some_and(|c| !"aeiou".contains(c))
}

pub fn pluralize(word: &str) -> String {
    if uncountable(word) {
        return word.to_string();
    }
    if let Some((stem, plural)) = IRREGULAR.iter().find_map(|(s, p)| Some((word.strip_suffix(s)?, p))) {
        return format!("{stem}{plural}");
    }
    if ends_with_consonant_y(word) {
        return format!("{}ies", &word[..word.len() - 1]);
    }
    if ["s", "x", "z", "ch", "sh"].iter().any(|e| word.ends_with(e)) {
        return format!("{word}es");
    }
    format!("{word}s")
}

pub fn singularize(word: &str) -> String {
    if uncountable(word) {
        return word.to_string();
    }
    for (singular, plural) in IRREGULAR {
        if let Some(stem) = word.strip_suffix(plural) {
            return format!("{stem}{singular}");
        }
        if word.ends_with(singular) {
            return word.to_string();
        }
    }
    if let Some(stem) = word.strip_suffix("ies").filter(|s| ends_with_consonant_y(&format!("{s}y"))) {
        return format!("{stem}y");
    }
    if let Some(stem) = word.strip_suffix("es").filter(|s| ["x", "ch", "ss", "sh", "zz"].iter().any(|e| s.ends_with(e))) {
        return stem.to_string();
    }
    // `ss|us|is` end singulars (`address`, `bus`, `axis`); their plurals took `es` above or just miss.
    match word.strip_suffix('s') {
        Some(stem) if !["ss", "us", "is"].iter().any(|e| word.ends_with(e)) => stem.to_string(),
        _ => word.to_string(),
    }
}

/// All template files matching `dir/<prefix>name.*`.
pub fn matching_templates(project: &Project, dir: &Path, basename: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let fname = p.file_name().and_then(|f| f.to_str()).unwrap_or("");
            template_suffix(|f| project.is_format(f), fname, basename).is_some()
        })
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found
}

/// A render name split into its folder and base name; the folder drops a leading `/` (`/users/card` is
/// `users` under app/views).
pub fn split_render_name(name: &str) -> (Option<&str>, &str) {
    match name.rsplit_once('/') {
        Some((dir, base)) => (Some(dir.trim_start_matches('/')), base),
        None => (None, name),
    }
}

/// Resolve a render reference to candidate files.
pub fn resolve(project: &Project, current: &Path, r: &RenderRef) -> Vec<PathBuf> {
    let views = views_root(current);
    let (dir, base) = split_render_name(&r.name);
    let basename = match r.kind {
        RenderKind::Partial => format!("_{base}"),
        RenderKind::Template => base.to_string(),
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    match (&dir, &views, r.kind) {
        (Some(d), Some(v), _) => dirs.push(v.join(d)),
        (Some(d), None, _) => {
            if let Some(p) = current.parent() {
                dirs.push(p.join(d));
            }
        }
        (None, _, _) => {
            if let Some(p) = current.parent() {
                dirs.push(p.to_path_buf());
            }
            if let Some(v) = &views {
                let inherited = ivars::view_prefixes(project, current).unwrap_or_default();
                for prefix in inherited.iter().map(String::as_str).chain(["application"]) {
                    let dir = v.join(prefix);
                    if !dirs.contains(&dir) {
                        dirs.push(dir);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        let mut found: Vec<PathBuf> =
            matching_templates(project, &d, &basename).into_iter().filter(|p| looked_up(project, current, p, &r.lookup)).collect();
        sort_like_action_view(project, current, &r.lookup, &mut found);
        out.extend(found);
    }
    out
}

/// Orders candidates like ActionView's `TemplateDetails#sort_key_for`: by format, variant, then handler (the locale,
/// unknown here, isn't ordered). With no `variants:` the view's own is wanted, as in Rails.
fn sort_like_action_view(project: &Project, current: &Path, lookup: &Lookup, found: &mut [PathBuf]) {
    let view_format = template_format(|f| project.is_format(f), current);
    let view_variant = template_variant(current);
    found.sort_by_key(|p| {
        let format = template_format(|f| project.is_format(f), p);
        let format_rank = match (&lookup.formats, view_format) {
            (Some(_), _) => rank(listed(&lookup.formats), format),
            (None, Some(view)) => rank(lookup_formats(view), format),
            (None, None) => usize::from(format != Some("html")) + usize::from(format.is_none()),
        };
        let wanted_variants = listed(&lookup.variants).chain(view_variant.filter(|_| lookup.variants.is_none()));
        (format_rank, rank(wanted_variants, template_variant(p)), rank(listed(&lookup.handlers), p.extension().and_then(|e| e.to_str())))
    });
}

fn listed(names: &Option<Vec<String>>) -> impl Iterator<Item = &str> {
    names.iter().flatten().map(String::as_str)
}

/// The place of `value` among the wanted `names`: Rails tries them in order, a template without that detail after
/// them all.
fn rank<'a>(names: impl Iterator<Item = &'a str>, value: Option<&str>) -> usize {
    let mut wanted = 0;
    for name in names {
        if value == Some(name) {
            return wanted;
        }
        wanted += 1;
    }
    wanted + usize::from(value.is_some())
}

/// Whether Rails could pick `candidate` for a render in `current` with the call's `lookup`: its format among
/// `formats:` (else one `current` renders), its variant among `variants:`, its handler among `handlers:`.
fn looked_up(project: &Project, current: &Path, candidate: &Path, lookup: &Lookup) -> bool {
    // A template without that detail (`_card.erb` has no format) is the fallback for every value.
    let among = |names: &Option<Vec<String>>, value: Option<&str>| {
        names.as_ref().is_none_or(|names| value.is_none_or(|value| names.iter().any(|name| name == value)))
    };
    let format_ok = if lookup.formats.is_some() {
        among(&lookup.formats, template_format(|f| project.is_format(f), candidate))
    } else {
        renders_format(project, current, candidate)
    };
    format_ok
        && among(&lookup.variants, template_variant(candidate))
        && among(&lookup.handlers, candidate.extension().and_then(|e| e.to_str()))
}

/// `Users::CardComponent` → app/components/users/card_component.rb (+ sidecar templates).
pub fn resolve_component(project: &Project, current: &Path, name: &str) -> Vec<PathBuf> {
    let Some(app) = app_root(current) else { return vec![] };
    let rel: PathBuf = name.split("::").map(snake_case).collect();
    let base = app.join("components").join(&rel);
    let mut out = Vec::new();
    let rb = base.with_extension("rb");
    if rb.exists() {
        out.push(rb);
    }
    if let (Some(dir), Some(stem)) = (base.parent(), base.file_name().and_then(|s| s.to_str())) {
        out.extend(matching_templates(project, dir, stem).into_iter().filter(|p| p.extension().map(|e| e != "rb").unwrap_or(true)));
    }
    out
}

/// ViewComponent classes under app/components, by the constant `render` takes (`admin/user_card_component.rb` →
/// `Admin::UserCardComponent`), with their file.
pub fn component_classes(current: &Path) -> Vec<(String, PathBuf)> {
    let Some(app) = app_root(current) else { return vec![] };
    let dir = app.join("components");
    let mut out: Vec<(String, PathBuf)> = WalkDir::new(&dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = e.path().strip_prefix(&dir).ok()?.to_str()?.replace('\\', "/");
            let name = rel.strip_suffix("_component.rb")?;
            let constant: Vec<String> = format!("{name}_component").split('/').map(camel_case).collect();
            Some((constant.join("::"), e.into_path()))
        })
        .collect();
    out.sort();
    out
}

/// `user_card` → `UserCard`.
fn camel_case(s: &str) -> String {
    s.split('_')
        .flat_map(|w| {
            let mut chars = w.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>())
        })
        .collect()
}

/// ActiveSupport's `underscore` of one constant: `UI` → `ui`, `HTMLCard` → `html_card`, `Card2Component` → `card2_component`.
pub fn snake_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, &ch) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(i + 1);
        // `([a-z\d])([A-Z])` and `([A-Z\d]+)([A-Z][a-z])`: a word starts after a lowercase letter or digit, or
        // where an acronym meets a capitalized word.
        let starts_word = ch.is_ascii_uppercase()
            && prev.is_some_and(|p| {
                p.is_ascii_lowercase() || p.is_ascii_digit() || (p.is_ascii_uppercase() && next.is_some_and(char::is_ascii_lowercase))
            });
        if starts_word {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

/// Names `render` can complete for `kind`: partials (same-dir bare names, then `dir/name` under app/views)
/// or non-partial templates as `dir/name`. `files` lists app/views, or the current directory outside one.
pub fn render_candidates(project: &Project, current: &Path, kind: RenderKind, files: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let views = views_root(current);
    let partial = kind == RenderKind::Partial;
    let same_dir = current
        .parent()
        .filter(|_| partial)
        .into_iter()
        .flat_map(|d| templates_in(project, d, files, true))
        .filter(|(name, _)| !name.contains('/'));
    let nested = views.iter().flat_map(|v| templates_in(project, v, files, partial)).filter(|(name, _)| name.contains('/'));
    // Same filter as `resolve`: a name it would not find isn't worth offering.
    let mut out: Vec<_> = same_dir.chain(nested).filter(|(_, p)| renders_format(project, current, p)).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

/// Every partial and template among `files` under `views`, by the name `render` uses for it.
pub fn named_templates(project: &Project, views: &Path, files: &[PathBuf]) -> Vec<(String, RenderKind, PathBuf)> {
    let partials = templates_in(project, views, files, true).map(|(name, p)| (name, RenderKind::Partial, p));
    let templates = templates_in(project, views, files, false).map(|(name, p)| (name, RenderKind::Template, p));
    partials.chain(templates).collect()
}

/// Partials (`_x.*`) or non-partial templates among `files` under `root`, named `dir/x` relative to `root`.
fn templates_in<'a>(
    project: &'a Project,
    root: &'a Path,
    files: &'a [PathBuf],
    partial: bool,
) -> impl Iterator<Item = (String, PathBuf)> + 'a {
    files.iter().filter_map(move |p| {
        let name = template_name(project, p, partial)?;
        let rel_dir = p.parent()?.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
        let full = if rel_dir.is_empty() { name } else { format!("{rel_dir}/{name}") };
        Some((full, p.clone()))
    })
}

fn template_name(project: &Project, p: &Path, partial: bool) -> Option<String> {
    let fname = p.file_name()?.to_str()?;
    let stem = match fname.strip_prefix('_') {
        Some(stem) if partial => stem,
        None if !partial => fname,
        _ => return None,
    };
    let name = template_stem(|f| project.is_format(f), stem);
    (!name.is_empty()).then(|| name.to_string())
}

/// Is the cursor inside the string of a `render "..."` call?
/// Returns what the string names (see `render_refs` for `layout:`) and the typed prefix.
pub fn render_string_prefix(line: &str, col: usize) -> Option<(RenderKind, String)> {
    // The string closed after a placeholder, and a block not yet typed, read as a finished call.
    const PLACEHOLDER: &str = "x";
    let before = line.get(..col.min(line.len()))?;
    let probe = format!("{before}{PLACEHOLDER}\" do");
    let r = refs_without_lookups(&probe).into_iter().find(|r| r.end == before.len() + PLACEHOLDER.len())?;
    Some((r.kind, r.name.strip_suffix(PLACEHOLDER)?.to_string()))
}

/// A template's `locals: (…)` magic comment.
#[derive(Debug, PartialEq)]
pub struct StrictLocals {
    /// The parameter list as written, without the parentheses.
    pub params: String,
    /// `(name, default)`; no default means required.
    pub locals: Vec<(String, Option<String>)>,
    /// `**rest` accepts any other local.
    pub rest: bool,
}

pub fn strict_locals(text: &str, close: Close, ruby: ruby_prism::Version) -> Option<StrictLocals> {
    let comment = strict_locals::find(text, close)?;
    let params = if comment.blank() { "" } else { comment.parameters.trim() };
    let (locals, rest) = prism::keyword_parameters(params, ruby)?;
    Some(StrictLocals { params: params.to_string(), locals, rest })
}

pub fn file_strict_locals(file: &Path, close: Close, ruby: ruby_prism::Version) -> Option<StrictLocals> {
    strict_locals(&std::fs::read_to_string(file).ok()?, close, ruby)
}

/// A `render` call read with Prism, once, from the text that starts at its line; offsets are into that text.
pub struct RenderCall<'a> {
    form: RenderForm,
    parsed: Option<Call<'a>>,
    /// How far before the ref's line the offsets start: the ref sits on a line that continues the call from an earlier one.
    shift: usize,
    /// Where the `render` starts, in the same offsets.
    call_at: usize,
}

impl RenderCall<'_> {
    /// Where the offsets of the edits and spans read from this call start, for the ref's line at `line_start`.
    pub fn base(&self, line_start: usize) -> usize {
        line_start - self.shift
    }

    /// Where the `render` starts, for the ref's line at `line_start`.
    pub fn call_start(&self, line_start: usize) -> usize {
        self.base(line_start) + self.call_at
    }
}

/// Local names passed by a literal `render "x", a: 1` or `render partial: "x", locals: { a: 1 }`; `None` for anything
/// less simple (splats, variables, options that add locals).
pub fn passed_locals<'a>(call: &RenderCall<'a>, r: &RenderRef) -> Option<Vec<&'a str>> {
    if r.kind != RenderKind::Partial {
        return None;
    }
    passed_names(call.parsed.as_ref()?, &call.form)
}

/// Where the literal partial render `call` spells the key of the local `name` (`name:` or `:name =>`), from `call.base`.
pub fn local_key_span(call: &RenderCall, name: &str) -> Option<(usize, usize)> {
    let parsed = call.parsed.as_ref().filter(|parsed| parsed.readable)?;
    let key = local_pairs(parsed, &call.form)?.iter().find(|pair| pair.key == name)?;
    Some((key.key_span.start, key.key_span.end))
}

/// A replacement for the bytes `range` of a text, from `RenderCall::base`.
pub type ByteEdit = (std::ops::Range<usize>, String);

/// The value a quick fix gives a local it adds, for the author to replace.
pub const NEW_LOCAL_VALUE: &str = "nil";

/// The edit to the literal render `call` that passes `names` too.
pub fn add_locals_edit(call: &RenderCall, r: &RenderRef, names: &[&str]) -> Option<ByteEdit> {
    // An edit after a Ruby comment would land in the comment.
    let parsed = call.parsed.as_ref().filter(|parsed| parsed.readable && !parsed.commented)?;
    let form = &call.form;
    let new = names.iter().map(|n| format!("{n}: {NEW_LOCAL_VALUE}")).collect::<Vec<_>>().join(", ");
    let end = parsed.pairs.iter().map(|pair| pair.whole.end).fold(r.end + call.shift + 1, usize::max);
    if !form.locals_in_hash(parsed) {
        return Some((end..end, format!(", {new}")));
    }
    if parsed.pairs.iter().any(|pair| changes_locals(pair.key, form)) {
        return None;
    }
    let Some(locals) = parsed.pairs.iter().find(|pair| pair.key == "locals") else {
        return Some((end..end, format!(", locals: {{ {new} }}")));
    };
    let hash = locals.hash.as_ref()?;
    Some(match hash.items.as_ref()?.last() {
        Some(last) => (last.whole.end..last.whole.end, format!(", {new}")),
        None => (hash.inner.clone(), format!(" {new} ")),
    })
}

/// The edit to the literal render `call` that stops passing `name`.
pub fn remove_local_edit(call: &RenderCall, r: &RenderRef, name: &str) -> Option<ByteEdit> {
    let parsed = call.parsed.as_ref().filter(|parsed| parsed.readable)?;
    let form = &call.form;
    let pairs = local_pairs(parsed, form)?;
    let i = pairs.iter().position(|pair| pair.key == name)?;
    let range = match (form.locals_in_hash(parsed), i, pairs.get(i + 1)) {
        // `, name: value`, from the end of what comes before.
        (false, 0, _) => r.end + 1..pairs[0].whole.end,
        (false, i, _) | (true, i @ 1.., _) => pairs[i - 1].whole.end..pairs[i].whole.end,
        (true, _, Some(next)) => pairs[0].whole.start..next.whole.start,
        (true, _, None) => pairs[0].whole.clone(),
    };
    Some((range, String::new()))
}

/// The pairs that name the locals passed: the `locals:` hash's, or the call's own; `None` when that hash isn't literal.
fn local_pairs<'c, 'a>(call: &'c Call<'a>, form: &RenderForm) -> Option<&'c [Pair<'a>]> {
    if !form.locals_in_hash(call) {
        return Some(&call.pairs);
    }
    match call.pairs.iter().find(|pair| pair.key == "locals") {
        Some(locals) => locals.hash.as_ref()?.items.as_deref(),
        None => Some(&[]),
    }
}

/// How a partial render spells its locals.
struct RenderForm {
    /// `render partial: "x", locals: {…}` rather than `render "x", a: 1`.
    options: bool,
    /// `json.partial!`, which also reads a `locals:` hash next to the name, and takes its own options out of the locals.
    jbuilder: bool,
}

impl RenderForm {
    fn of(line: &str, r: &RenderRef) -> Self {
        RenderForm {
            options: line[..r.start.saturating_sub(1)].trim_end().ends_with([':', '>']),
            jbuilder: line[r.call..].starts_with("json."),
        }
    }

    fn locals_in_hash(&self, call: &Call) -> bool {
        self.options || (self.jbuilder && call.pairs.iter().any(|pair| pair.key == "locals"))
    }
}

/// Options that pass locals of their own: the object or each collection item under the partial's name, or `as:`.
const ADDS_LOCALS: [&str; 3] = ["object", "collection", "as"];

/// Does option `key` change the locals the partial gets? Jbuilder also takes `cached:` out of them.
fn changes_locals(key: &str, form: &RenderForm) -> bool {
    ADDS_LOCALS.contains(&key) || (form.jbuilder && key == "cached")
}

/// The local names `call` passes; `None` unless every key is literal.
fn passed_names<'a>(call: &Call<'a>, form: &RenderForm) -> Option<Vec<&'a str>> {
    if !call.readable || (form.locals_in_hash(call) && call.pairs.iter().any(|pair| changes_locals(pair.key, form))) {
        return None;
    }
    let names: Vec<&str> = local_pairs(call, form)?.iter().map(|pair| pair.key).collect();
    if form.jbuilder && names.iter().any(|name| changes_locals(name, form)) {
        return None;
    }
    Some(names)
}

fn read_call_with_lookup<'a>(text: &'a str, r: &RenderRef, haml: bool, ruby: ruby_prism::Version) -> (RenderCall<'a>, Lookup) {
    let form = RenderForm::of(text, r);
    let parsed = render_call::read(text, r.call, r.start, haml, ruby);
    let lookup = parsed.as_ref().map(|parsed| lookup(parsed, &form)).unwrap_or_default();
    (RenderCall { form, parsed, shift: 0, call_at: r.call }, lookup)
}

/// The partial ref, the locals already passed and the name typed so far where a local's name may be typed at `offset`;
/// the render call may start up to `MAX_CALL_LINES` lines higher. `haml`: `text` is Haml. `ruby`: the grammar to read the call with
/// (and its lookup options), asked for once a partial ref is found.
pub fn local_key_before(
    text: &str,
    offset: usize,
    haml: bool,
    ruby: impl FnOnce() -> ruby_prism::Version,
) -> Option<(RenderRef, Vec<String>, &str)> {
    let line_starts = text[..offset].rmatch_indices('\n').map(|(i, _)| i + 1).chain([0]).take(MAX_CALL_LINES);
    // Starts above the first one that sees the partial ref see the same ref, so they can't read the call any better.
    let (start, r) = line_starts.into_iter().find_map(|start| Some((start, partial_ref_before(&text[start..offset])?)))?;
    let haml = HamlPipes::new(haml).apply(text, start);
    let ruby = ruby();
    let (names, typed, lookup) = local_key_after(&text[start..], offset - start, &r, haml, ruby)?;
    Some((RenderRef { start: r.start + start, end: r.end + start, call: r.call + start, lookup, ..r }, names, typed))
}

const CURSOR_KEY: &str = "cursor_key__";

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn partial_ref_before(before: &str) -> Option<RenderRef> {
    refs_without_lookups(before).into_iter().rev().find(|r| r.kind == RenderKind::Partial && !r.from_layout && r.end < before.len())
}

/// The locals already passed (before or after the cursor), the name typed so far and the call's lookup where a local's name
/// may be typed at `col` in `text`, which starts at the render call's line.
fn local_key_after<'a>(
    text: &'a str,
    col: usize,
    r: &RenderRef,
    haml: bool,
    ruby: ruby_prism::Version,
) -> Option<(Vec<String>, &'a str, Lookup)> {
    // The blanking keeps offsets, so the typed word is taken from `text`.
    let joined = if haml { render_call::haml_group_joined(text) } else { Cow::Borrowed(text) };
    let blanked = joined.as_ref();
    let line = blanked.get(..col)?;
    let args = line.get(r.end + 1..)?;
    if args.contains("%>") {
        return None;
    }
    let head = args.trim_end_matches(is_word_char);
    let typed = &args[head.len()..];
    let head = head.trim_end();
    let form = RenderForm::of(line, r);
    // In the options form keys are typed inside `locals: {`; next to the name, only Jbuilder reads that hash.
    let open = open_brace(head);
    match open {
        Some(at) if (form.options || form.jbuilder) && without_locals_key(&head[..at]).is_some() => {}
        None if !form.options => {}
        _ => return None,
    }
    let after_open = head.ends_with('{');
    let listed = if after_open { head } else { head.strip_suffix(',')? };
    let closed = if open.is_some() { format!("{listed}}}") } else { listed.to_string() };
    let call_start = line.get(r.call..=r.end)?;
    // The call from its name on, with a key after the cursor: the call has it only if the cursor is still among its arguments.
    let source = format!("{call_start}{closed}, {CURSOR_KEY}: 1");
    // The same with the rest of the call after the cursor, to count the locals passed there.
    let rest_end = text[col..].match_indices('\n').nth(MAX_CALL_LINES - 1).map_or(text.len(), |(at, _)| col + at);
    let rest = &text[col..rest_end];
    let after_word = rest.trim_start_matches(is_word_char);
    // The word being typed goes on after the cursor: it is dropped, unless a `:` follows, when it is a key to keep (but not to count).
    let (tail, editing) = if typed.is_empty() {
        (rest.to_string(), String::new())
    } else if after_word.starts_with(':') && !after_word.starts_with("::") {
        (format!("{typed}{rest}"), format!("{typed}{}", &rest[..rest.len() - after_word.len()]))
    } else {
        (after_word.to_string(), String::new())
    };
    let separator = if after_open { " " } else { ", " };
    // A key that follows the cursor without a comma is the next argument.
    let joint = if tail.trim_start().starts_with(|c: char| is_word_char(c) || matches!(c, ':' | '"' | '\'')) { "," } else { "" };
    let spliced = format!("{call_start}{listed}{separator}{CURSOR_KEY}: 1{joint}{tail}");
    let name_at = r.start - r.call;
    // The probe and, right after it, the key being edited are not passed locals; a duplicate of that key elsewhere is.
    let others = |mut names: Vec<&str>| {
        let at = names.iter().position(|name| *name == CURSOR_KEY)?;
        names.remove(at);
        if !editing.is_empty() && names.get(at) == Some(&editing.as_str()) {
            names.remove(at);
        }
        Some(names.into_iter().map(str::to_string).collect::<Vec<_>>())
    };
    // The whole call, when it reads, decides: the probe is a local there or nowhere.
    if let Some(call) = render_call::read(&spliced, 0, name_at, haml, ruby).filter(|call| call.readable) {
        return Some((others(passed_names(&call, &form)?)?, &text[col - typed.len()..col], lookup(&call, &form)));
    }
    // Cut at the cursor, the probe may also sit beside a hash the call ignores.
    let passed = |source: &str| {
        let call = render_call::read(source, 0, name_at, haml, ruby)?;
        let names = passed_names(&call, &form)?;
        let lookup = lookup(&call, &form);
        if !names.contains(&CURSOR_KEY) && call.pairs.iter().any(|pair| pair.key == CURSOR_KEY) {
            return Some((names.into_iter().map(str::to_string).collect(), lookup));
        }
        Some((others(names)?, lookup))
    };
    let (names, lookup) = passed(&source).or_else(|| passed(&format!("{source})")))?;
    Some((names, &text[col - typed.len()..col], lookup))
}

/// Where the last `{` that `text` leaves open is, braces in string literals aside.
fn open_brace(text: &str) -> Option<usize> {
    let (mut open, mut quote, mut escaped) = (Vec::new(), None, false);
    for (i, c) in text.char_indices() {
        match (quote, c) {
            _ if escaped => escaped = false,
            (Some(_), '\\') => escaped = true,
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '{') => open.push(i),
            (None, '}') => {
                open.pop();
            }
            _ => {}
        }
    }
    open.pop()
}

/// `before` less a trailing `locals:` or `:locals =>`.
fn without_locals_key(before: &str) -> Option<&str> {
    let before = before.trim_end();
    before.strip_suffix("locals:").or_else(|| before.strip_suffix("=>")?.trim_end().strip_suffix(":locals"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rails_paths::files_under;

    const LATEST: ruby_prism::Version = ruby_prism::Version::Latest;

    /// The call of `r`, read from `text`, which starts at `r`'s line; `haml`: `text` is Haml.
    fn read_call<'a>(text: &'a str, r: &RenderRef, haml: bool) -> RenderCall<'a> {
        read_call_with_lookup(text, r, haml, LATEST).0
    }

    #[test]
    fn local_edits_on_render_calls() {
        let apply = |line: &str, edit: Option<ByteEdit>| {
            edit.map(|(range, new)| {
                let mut out = line.to_string();
                out.replace_range(range, &new);
                out
            })
        };
        let add = |line: &str, names: &[&str]| {
            let r = &render_refs(line)[0];
            apply(line, add_locals_edit(&read_call(line, r, false), r, names))
        };
        let remove = |line: &str, name: &str| {
            let r = &render_refs(line)[0];
            apply(line, remove_local_edit(&read_call(line, r, false), r, name))
        };
        let s = |line: &str| Some(line.to_string());
        assert_eq!(add(r#"<%= render "card" %>"#, &["user"]), s(r#"<%= render "card", user: nil %>"#));
        assert_eq!(add(r#"<%= render("card", size: 1) %>"#, &["a", "b"]), s(r#"<%= render("card", size: 1, a: nil, b: nil) %>"#));
        assert_eq!(
            add(r#"= render partial: "card", locals: { size: 1 }"#, &["user"]),
            s(r#"= render partial: "card", locals: { size: 1, user: nil }"#)
        );
        assert_eq!(add(r#"= render partial: "card", locals: {}"#, &["user"]), s(r#"= render partial: "card", locals: { user: nil }"#));
        assert_eq!(add(r#"= render partial: "card""#, &["user"]), s(r#"= render partial: "card", locals: { user: nil }"#));
        assert_eq!(add(r#"= render partial: "card", object: @u"#, &["user"]), None, "the object is a local of its own");
        assert_eq!(add(r#"<%= render "card", size: 1 # TODO %>"#, &["user"]), None, "it would land in the comment");
        assert_eq!(remove(r#"<%= render "card", colour: 1 %>"#, "colour"), s(r#"<%= render "card" %>"#));
        assert_eq!(remove(r#"<%= render "card", user: u, colour: 1 %>"#, "colour"), s(r#"<%= render "card", user: u %>"#));
        assert_eq!(
            remove(r#"= render partial: "card", locals: { colour: 1, user: u }"#, "colour"),
            s(r#"= render partial: "card", locals: { user: u }"#)
        );
        assert_eq!(
            remove(r#"= render partial: "card", locals: { user: u, colour: 1 }"#, "colour"),
            s(r#"= render partial: "card", locals: { user: u }"#)
        );
        assert_eq!(remove(r#"<%= render "card", user: u %>"#, "colour"), None);
        assert_eq!(remove(r#"<%= render "café", title: "é", colour: 1 %>"#, "colour"), s(r#"<%= render "café", title: "é" %>"#));
        assert_eq!(add("= render \"card\",\n  size: 1", &["user"]), s("= render \"card\",\n  size: 1, user: nil"));
    }

    #[test]
    fn local_keys_after_a_partial_name() {
        let key = |line: &str| {
            let col = line.find('|').unwrap();
            let line = line.replace('|', "");
            local_key_before(&line, col, false, || LATEST).map(|(r, passed, typed)| (r.name, passed.join(" "), typed.to_string()))
        };
        let card = |passed: &str, typed: &str| Some(("card".to_string(), passed.to_string(), typed.to_string()));
        assert_eq!(key(r#"<%= render "card", | %>"#), card("", ""));
        assert_eq!(key(r#"<%= render("card", user: @u, si|) %>"#), card("user", "si"));
        assert_eq!(key(r#"= render partial: "card", locals: { user: @u, |"#), card("user", ""));
        assert_eq!(key(r#"= render partial: "card", locals: {|"#), card("", ""));
        assert_eq!(key(r#"<%= render "card", user: |"#), None, "a value, not a key");
        assert_eq!(key(r#"<%= render "card" %> |"#), None);
        assert_eq!(key(r#"<%= render "card", *opts, |"#), None);
        assert_eq!(key(r#"<%= render partial: "card", |"#), None);
        assert_eq!(key(r#"<%= render template: "users/show", |"#), None);
        assert_eq!(key(r#"<%= render "a", body: render("b"), |"#), None, "b's call is closed");
        assert_eq!(key(r#"<%= foo(render("card"), |"#), None);
        assert_eq!(key(r#"= render(partial: "card") + helper(locals: { |"#), None);
        assert_eq!(key(r#"= render partial: "card", layout: "box", locals: { |"#), card("", ""));
        assert_eq!(key(r#"<%= render :partial => "card", :locals => { :user => u, | } %>"#), card("user", ""));
        assert_eq!(key(r#"json.partial! "card", locals: { user: u, |"#), card("user", ""));
        assert_eq!(key(r#"json.partial! "card", |"#), card("", ""));
        assert_eq!(key(r#"json.partial! "card", collection: @c, as: :c, |"#), None, "Jbuilder's own options");
        assert_eq!(key(r#"<%= render "card", title: "{", | %>"#), card("title", ""), "a brace in a string");
        assert_eq!(key(r#"= render partial: "card", locals: { t: "}", |"#), card("t", ""));
    }

    #[test]
    fn new_template_paths() {
        let views = Path::new("/r/app/views");
        let path = |current: &str, line: &str| new_template_path(&Project::unknown(), &views.join(current), &render_refs(line)[0]);
        assert_eq!(path("users/show.html+phone.slim", r#"= render "shared/card""#), Some(views.join("shared/_card.html.slim")));
        assert_eq!(path("users/index.json.jbuilder", r#"json.partial! "users/user""#), Some(views.join("users/_user.json.jbuilder")));
        assert_eq!(path("users/show.de.erb", r#"render template: "a/b""#), Some(views.join("a/b.erb")));
        assert_eq!(path("users/show.html.erb", r#"render template: "a/b", layout: "layouts/lt""#), Some(views.join("a/b.html.erb")));
        assert_eq!(
            new_template_path(
                &Project::unknown(),
                &views.join("users/show.html.erb"),
                &render_refs(r#"render template: "a/b", layout: "layouts/lt""#)[1]
            ),
            Some(views.join("layouts/lt.html.erb"))
        );
        assert_eq!(path("home/show.html.erb", r#"render "/users/x""#), Some(views.join("users/_x.html.erb")));
        assert_eq!(
            path("users/show.html.erb", r#"render partial: "users/card", formats: [:json, :html]"#),
            Some(views.join("users/_card.json.erb"))
        );
        assert_eq!(
            path("users/show.html.erb", r#"render partial: "users/card", variants: [:phone], handlers: [:haml]"#),
            Some(views.join("users/_card.html+phone.haml"))
        );
        assert_eq!(path("users/show.html.erb", r#"render partial: "users/card", formats: []"#), Some(views.join("users/_card.html.erb")));
        assert_eq!(path("users/show.html.erb", r#"render "card""#), None);
        assert_eq!(path("users/show.html.erb", r#"render "../../config/x""#), None);
        assert_eq!(path("users/show.html.erb", r#"render "shared/""#), None);
        assert_eq!(new_template_path(&Project::unknown(), Path::new("/tmp/x.html.erb"), &render_refs(r#"render "a/b""#)[0]), None);
    }

    #[test]
    fn jbuilder_refs() {
        let names = |line: &str| render_refs(line).into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(names(r#"json.partial! "users/user", user: u"#), ["users/user"]);
        assert_eq!(names(r#"json.partial!("users/user")"#), ["users/user"]);
        assert_eq!(names(r#"json.partial! partial: 'users/user', locals: { user: u }"#), ["users/user"]);
        assert_eq!(names(r#"json.array! @users, partial: "users/user", as: :user"#), ["users/user"]);
        assert_eq!(names(r#"json.array!(@users, :partial => "users/user", :as => :user)"#), ["users/user"]);
        assert_eq!(names(r#"json.comments @post.comments, partial: "comments/comment", as: :comment"#), ["comments/comment"]);
        assert_eq!(names(r#"xml << render(partial: "items/item", locals: { i: i })"#), ["items/item"]);
        assert_eq!(names(r#"<%= render partial: "x" %>"#), ["x"]);
    }

    #[test]
    fn refs() {
        let l = r#"<%= render "users/card", user: u %>"#;
        let r = render_ref_at(l, 0, l, false, LATEST, 14).unwrap();
        assert_eq!(r.name, "users/card");
        assert_eq!(r.kind, RenderKind::Partial);
        let row = "= render partial: 'row', collection: @x";
        let r = render_ref_at(row, 0, row, false, LATEST, 20).unwrap();
        assert_eq!(r.name, "row");
        let tpl = "= render template: 'a/b'";
        assert_eq!(render_ref_at(tpl, 0, tpl, false, LATEST, 22).unwrap().kind, RenderKind::Template);
        assert_eq!(component_ref_at("= render Users::CardComponent.new(user: u)", 12).unwrap(), "Users::CardComponent");
        assert_eq!(render_string_prefix(r#"<%= render "us"#, 14).unwrap(), (RenderKind::Partial, "us".to_string()));
        assert_eq!(render_string_prefix(r#"render template: "a/"#, 20).unwrap(), (RenderKind::Template, "a/".to_string()));
        assert_eq!(render_string_prefix(r#"render partial: "x", layout: 'ad"#, 32).unwrap(), (RenderKind::Partial, "ad".to_string()));
        assert_eq!(render_string_prefix(r#"render layout: "ad"#, 18).unwrap(), (RenderKind::Partial, "ad".to_string()));
        assert_eq!(render_string_prefix(r#"render template: "a", layout: "la"#, 33).unwrap(), (RenderKind::Template, "la".to_string()));
        assert!(render_string_prefix(r#"render partial: "x", locals: { layout: "ad"#, 41).is_none());
        assert!(render_string_prefix(r#"render "x", layout: "ad"#, 23).is_none());
        assert!(render_string_prefix(r#"page_layout "ad"#, 15).is_none());
        assert_eq!(render_string_prefix(r#"json.partial! "po"#, 17).unwrap(), (RenderKind::Partial, "po".to_string()));
        assert_eq!(render_string_prefix(r#"render formats: [:json], layout: "sh"#, 36).unwrap(), (RenderKind::Partial, "sh".to_string()));
        assert_eq!(
            render_string_prefix(r##"json.tags @t, label: "#x", partial: "sh"##, 39).unwrap(),
            (RenderKind::Partial, "sh".to_string())
        );
        assert_eq!(render_string_prefix(r#"json.partial! partial: 'po"#, 26).unwrap(), (RenderKind::Partial, "po".to_string()));
        assert_eq!(snake_case("CardComponent"), "card_component");
        for (constant, file) in
            [("UI", "ui"), ("HTMLCardComponent", "html_card_component"), ("Card2Component", "card2_component"), ("APIv2", "ap_iv2")]
        {
            assert_eq!(snake_case(constant), file, "{constant}");
        }
        let refs = |l: &str| render_refs(l).into_iter().map(|r| (r.kind, r.name)).collect::<Vec<_>>();
        let p = |n: &str| (RenderKind::Partial, n.to_string());
        assert_eq!(refs(r#"<%= render layout: "box" do %>"#), [p("box")]);
        assert_eq!(refs(r#"<%= render partial: "row", layout: "box", locals: { a: 1 } %>"#), [p("row"), p("box")]);
        assert_eq!(
            refs(r#"<%= render template: "a/b", layout: "layouts/x" %>"#),
            [(RenderKind::Template, "a/b".to_string()), (RenderKind::Template, "layouts/x".to_string())]
        );
        assert_eq!(refs(r#"<%= render "row", layout: "box" %>"#), [p("row")]);
        assert_eq!(refs(r#"<%= render partial: "row", locals: { a: 1, layout: "gone" } %>"#), [p("row")]);
        assert_eq!(refs(r#"<%= render partial: "row", locals: [{ a: 1 }], layout: "box" %>"#), [p("row"), p("box")]);
        assert!(render_refs(r#"render partial: "row", layout: "box""#)[1].from_layout);
        // Pre-Ruby-1.9 hash rockets, common in old views.
        assert_eq!(refs(r#"<%= render :partial => "row", :locals => { :a => 1 } %>"#), [p("row")]);
        assert_eq!(refs(r#"render(:template => "a/b")"#), [(RenderKind::Template, "a/b".to_string())]);
        assert_eq!(refs(r#"render(:partial => "row", :layout => "box")"#), [p("row"), p("box")]);
        assert_eq!(render_string_prefix(r#"render :partial => "ro"#, 22).unwrap(), (RenderKind::Partial, "ro".to_string()));
        assert_eq!(render_string_prefix(r#"render :partial => "x", :layout => "a"#, 37).unwrap(), (RenderKind::Partial, "a".to_string()));
    }

    #[test]
    fn calls_on_another_receiver_are_not_renders() {
        for line in [
            r#"<%= markdown.render("hi") %>"#,
            r#"<%= @renderer.render "summary" %>"#,
            r#"<%= md_render "intro" %>"#,
            r#"= presenter.render "summary""#,
            r#"<%= obj.render formats: [:html], partial: "x" %>"#,
            r#"<%= obj&.render partial: "x" %>"#,
            r#"<%= Foo::render "x" %>"#,
        ] {
            assert!(render_refs(line).is_empty(), "{line}");
        }
        assert!(component_ref_at("= md.render Users::CardComponent.new", 20).is_none());
        assert!(model_renders("<%= md.render @user %>").is_empty());
        assert_eq!(render_refs(r#"<%= self.render "x" %>"#).len(), 1);
        assert_eq!(render_refs(r#"<%= myself.render "x" %>"#).len(), 0);
        assert_eq!(render_refs(r#"json.partial! "x""#).len(), 1);
        assert_eq!(render_refs(r#"<%= foo(render("x")) %>"#).len(), 1);
    }

    #[test]
    fn a_render_in_a_block_keeps_its_lookup() {
        for line in [
            r#"xml.feed { render partial: "post", formats: [:html] }"#,
            r#"<%= content_tag(:div) { render partial: "post", formats: [:html] } %>"#,
            r#"= content_tag(:div) { render partial: "post", formats: [:html] }"#,
        ] {
            let found = render_refs_in(line, 0, line, &HamlPipes::new(false), LATEST, |_| true);
            assert_eq!(found[0].lookup.formats, Some(vec!["html".to_string()]), "{line}");
        }
    }

    #[test]
    fn file_keys_after_other_keys_and_hash_noise() {
        let refs = |l: &str| render_refs(l).into_iter().map(|r| (r.name, r.from_layout)).collect::<Vec<_>>();
        let one = |name: &str| vec![(name.to_string(), false)];
        // Jbuilder: a `#` inside a string or interpolation is not a comment.
        assert_eq!(refs(r##"json.comments @c, as: "#{prefix}_comment", partial: "comments/comment""##), one("comments/comment"));
        assert_eq!(refs(r##"json.tags @tags, label: "#tag", partial: "tags/tag""##), one("tags/tag"));
        assert!(refs(r#"json.tags @tags # partial: "tags/tag""#).is_empty());
        assert!(refs(r##"json.tags @tags, label: "#tag" # partial: "tags/tag""##).is_empty());
        // A nested or quoted key doesn't hide the top-level one after it.
        assert_eq!(refs(r#"render locals: { partial: "x" }, partial: "card""#), one("card"));
        assert_eq!(refs(r#"render locals: { a: 1, partial: "x" }, partial: "card""#), one("card"));
        assert_eq!(refs(r#"render formats: "a,partial: 'x'", partial: "card""#), one("card"));
        assert_eq!(
            refs(r#"render partial: "row", locals: { a: 1, layout: "x" }, layout: "box""#),
            [("row".to_string(), false), ("box".to_string(), true)]
        );
        assert!(refs(r#"render locals: { a: 1, partial: "x" }"#).is_empty());
        assert!(refs(r#"render partial: row, locals: { a: 1, layout: "x" }"#).is_empty());
        // A block layout next to other options is the partial itself, with the call's formats.
        let layout = &render_refs(r#"render formats: [:json], layout: "shared/wrap" do"#)[0];
        assert_eq!((layout.name.as_str(), layout.from_layout, layout.kind), ("shared/wrap", false, RenderKind::Partial));
        assert_eq!(layout.lookup.formats, Some(vec!["json".to_string()]));
        assert_eq!(
            refs(r#"render formats: [:json], partial: "row", layout: "box""#),
            [("row".to_string(), false), ("box".to_string(), true)]
        );
        assert!(refs(r#"render formats: [:json], locals: { layout: "x" }"#).is_empty());
    }

    #[test]
    fn option_refs_span_the_name_and_stop_at_the_tag_end() {
        let spans = |l: &str| {
            render_refs(l)
                .into_iter()
                .map(|r| (l[r.start..r.end].to_string(), l[r.call..].starts_with("render"), r.from_layout))
                .collect::<Vec<_>>()
        };
        let named = |name: &str, from_layout| (name.to_string(), true, from_layout);
        // Multibyte text before the key moves the byte offsets, not the name.
        assert_eq!(spans(r#"<%= render locals: { a: "é", partial: "x" }, partial: "card" %>"#), [named("card", false)]);
        assert_eq!(spans(r#"<%= render a: "日本", partial: "x", layout: "box" %>"#), [named("x", false), named("box", true)]);
        // Each call keeps its own keys: neither borrows a key from the next tag.
        assert_eq!(
            spans(r#"<%= render "a" %><%= render partial: "b", layout: "c" %>"#),
            [named("a", false), named("b", false), named("c", true)]
        );
        assert_eq!(spans(r#"<%= render x: 1 %><%= render y: 2, partial: "b" %>"#), [named("b", false)]);
        assert!(spans(r#"<%= render x: 1 %><%= foo a, partial: "b" %>"#).is_empty());
        // Haml and Slim lines, with and without parentheses.
        assert_eq!(spans(r#"= render(formats: [:json], partial: "w")"#), [named("w", false)]);
        assert_eq!(spans(r#"= render(partial: "w", layout: "l")"#), [named("w", false), named("l", true)]);
        assert_eq!(spans(r#"= render :layout => "l", :partial => "p""#), [named("l", false), named("p", false)]);
    }

    #[test]
    fn lone_layout_is_a_partial_only_with_a_block() {
        let names = |l: &str| refs_without_lookups(l).into_iter().map(|r| r.name).collect::<Vec<_>>();
        for line in [
            r#"<%= render inline: "<p>x</p>", layout: "shared/box" %>"#,
            r#"<%= render html: "x", layout: "shared/box" %>"#,
            r#"<%= render plain: "x", layout: "shared/box" %>"#,
            r#"<%= render locals: { a: 1 }, layout: "shared/box" %>"#,
            r#"<%= render collection: @x, layout: "shared/box" %>"#,
            r#"= render(collection: @x, layout: "shared/box")"#,
            r#"<%= render locals: { do: 1 }, layout: "shared/box" %>"#,
            r#"<%= render layout: "shared/box", locals: { a: 1 } %>"#,
        ] {
            assert!(names(line).is_empty(), "{line}");
        }
        for line in [
            r#"<%= render html: "x", layout: "shared/box" %><% @items.each do |i| %>"#,
            r#"<%= render collection: @x, layout: "shared/box", cb: proc { |a| a } %>"#,
            r#"<%= render collection: @x, layout: "shared/box", foo: "do |" %>"#,
            r#"<%= render collection: @x, layout: "shared/box", foo: "do %>"#,
            r#"<%= render html: "x", layout: "shared/box" %><%= link_to "x" do %>"#,
        ] {
            assert!(names(line).is_empty(), "{line}");
        }
        for line in [
            r#"<%= render locals: { a: 1 }, layout: "shared/box" do %>"#,
            r#"<%= render collection: @x, layout: "shared/box" do |x| %>"#,
            r#"<%= render(locals: { a: 1 }, layout: "shared/box") do -%>"#,
            r#"<%= render locals: { a: 1 }, layout: "shared/box" do |x| x.y %>"#,
            r#"<%= render locals: { a: 1 }, layout: "shared/box", formats: [:html] do %>"#,
            r#"= render locals: { a: 1 }, layout: "shared/box" do"#,
            r#"= render locals: { a: 1 }, layout: "shared/box" { |f| f }"#,
        ] {
            assert_eq!(names(line), ["shared/box"], "{line}");
        }
    }

    #[test]
    fn call_heads_in_keys_strings_and_comments_do_not_cut_the_call() {
        let found = |l: &str| render_refs(l).into_iter().map(|r| (r.name, r.from_layout)).collect::<Vec<_>>();
        let n = |name: &str, from_layout| (name.to_string(), from_layout);
        assert_eq!(found(r#"<%= render partial: "a", renderer: :x, layout: "b" %>"#), [n("a", false), n("b", true)]);
        let json = render_refs(r#"<%= render formats: [:json], rendered: 1, render_mode: 2, partial: "x" %>"#);
        assert_eq!((json[0].name.as_str(), json[0].call, json[0].lookup.formats.clone()), ("x", 4, Some(vec!["json".to_string()])));
        assert_eq!(found(r#"<%= render a: 1, b: "render x: 1", partial: "x" %>"#), [n("x", false)]);
        assert_eq!(found(r#"<%= render partial: "a", title: "render a: 1", layout: "b" %>"#), [n("a", false), n("b", true)]);
        assert_eq!(found(r#"json.x @y, label: "json.y z", partial: "p""#), [n("p", false)]);
        assert_eq!(found(r#"<p>it's</p><%= render a: 1, partial: "x" %>"#), [n("x", false)]);
        assert_eq!(found(r#"p It's #{render formats: [:json], partial: "x"}"#), [n("x", false)]);
        // A Haml/Slim id is not a comment.
        assert_eq!(found(r#"#main= render partial: "row", layout: "box""#), [n("row", false), n("box", true)]);
        assert_eq!(found(r#"%div.a#main= render formats: [:json], partial: "x""#), [n("x", false)]);
        assert_eq!(found(r#"#main = render formats: [:json], partial: "x""#), [n("x", false)]);
        let line = r#"<%= render a: 1 %>(<%= render b: 2 %>"#;
        assert!(call_tails(line, &RENDER_OPTIONS).iter().all(|(_, tail)| line[tail.end..].starts_with("%>")));
    }

    #[test]
    fn a_call_nested_in_the_arguments_does_not_end_them() {
        let names = |l: &str| refs_without_lookups(l).into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(names(r#"<%= render locals: { x: render(partial: "f") }, partial: "item" %>"#), ["f", "item"]);
        assert_eq!(names(r#"json.array! @posts, locals: { user: json.current_user }, partial: "posts/post""#), ["posts/post"]);
        assert_eq!(names(r#"render(a: 1, partial: "p"); render(b: 2, partial: "q")"#), ["p", "q"]);
    }

    #[test]
    fn json_keys_belong_to_their_own_call() {
        let names = |l: &str| refs_without_lookups(l).into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(names(r#"json.x @y, locals: { partial: "n" }, partial: "a""#), ["a"]);
        assert_eq!(names(r#"json.a 1; json.b @b, partial: "x""#), ["x"]);
        assert_eq!(names(r#"json.a 1, as: :z; json.b @b, partial: "x""#), ["x"]);
        assert_eq!(names(r#"json.comments partial: "comments/comment", collection: @c"#), ["comments/comment"]);
        assert!(names(r#"json.a @a; json.b @b"#).is_empty());
    }

    #[test]
    fn pathological_lines_cost_a_bounded_number_of_parses() {
        let parses = |line: &str| {
            render_call::PARSES.with(|count| count.set(0));
            let refs = refs_without_lookups(line).len();
            (refs, render_call::PARSES.with(|count| count.get()))
        };
        let nested = format!("render k0: 1{}", (1..500).map(|i| format!(", k{i}: 1, layout: \"x{i}\"")).collect::<String>());
        assert!(parses(&nested).1 <= MAX_KEY_CANDIDATES, "keys of one call");
        let joined = (0..100).map(|i| format!("render a: {i}, partial: \"p{i}\"")).collect::<Vec<_>>().join("; ");
        let (refs, parses) = parses(&joined);
        assert_eq!(refs, 100);
        assert!(parses <= 2 * refs, "{parses} parses for {refs} calls");
    }

    #[test]
    fn nested_keys_cost_one_parse_however_many() {
        render_call::PARSES.with(|count| count.set(0));
        let nested = (0..5000).map(|i| format!(", layout: \"n{i}\"")).collect::<String>();
        let line = format!(r#"render formats: [:json], locals: {{ a: 1{nested} }}, layout: "box" do"#);
        let names: Vec<_> = refs_without_lookups(&line).into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["box"]);
        assert!(render_call::PARSES.with(|count| count.get()) <= 2);
    }

    #[test]
    fn literals_the_depth_scan_cannot_read_do_not_hide_keys() {
        let names = |l: &str| refs_without_lookups(l).into_iter().map(|r| r.name).collect::<Vec<_>>();
        for literal in [r"%q{x {y} z}", r"/\(/", "?(", "%w|(|", r"%r{\{}"] {
            assert_eq!(names(&format!(r#"render formats: [:json], locals: {{ s: {literal} }}, partial: "a""#)), ["a"], "{literal}");
            assert_eq!(names(&format!(r#"json.x @y, sep: {literal}, partial: "a""#)), ["a"], "{literal}");
        }
        assert_eq!(names(r#"render formats: [:json], locals: { re: /\(/ }, partial: "a""#), ["a"]);
        assert_eq!(names(r#"render partial: "a", locals: { s: %q{x {y} z} }, layout: "box" do"#), ["a", "box"]);
        assert_eq!(names(r##"json.x @y, sep: "#{a("[")}", partial: "a""##), ["a"]);
        assert_eq!(names(r##"render formats: [:json], locals: { t: "#{a("(")}" }, partial: "a""##), ["a"]);
        assert_eq!(names("json.x @y, sep: `(`, partial: \"a\""), ["a"]);
    }

    #[test]
    fn many_nested_keys_before_the_real_one_do_not_hide_it() {
        let nested = (0..10).map(|i| format!(", layout: \"n{i}\"")).collect::<String>();
        let line = format!(r#"<%= render formats: [:json], locals: {{ a: 1{nested} }}, layout: "box" do %>"#);
        let names: Vec<_> = refs_without_lookups(&line).into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["box"]);
    }

    #[test]
    fn jbuilder_keys_follow_literals_holding_a_hash_sign() {
        let names = |l: &str| refs_without_lookups(l).into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(names(r#"json.x @y, sep: ?#, partial: "a""#), ["a"]);
        assert_eq!(names(r#"json.x @y, sep: %q{#}, partial: "a""#), ["a"]);
        assert_eq!(names(r#"json.x @y, sep: %w[# a], partial: "a""#), ["a"]);
        assert!(names(r#"json.x @y, sep: 1 # , partial: "a""#).is_empty());
    }

    #[test]
    fn template_with_layout_makes_the_layout_a_template() {
        let refs = render_refs(r#"render formats: [:json], template: "t", layout: "l""#);
        let got: Vec<_> = refs.iter().map(|r| (r.name.as_str(), r.kind, r.from_layout)).collect();
        assert_eq!(got, [("t", RenderKind::Template, false), ("l", RenderKind::Template, true)]);
    }

    #[test]
    fn jbuilder_partial_key_is_the_first_outside_a_comment() {
        let names = |l: &str| render_refs(l).into_iter().map(|r| (r.name, r.start)).collect::<Vec<_>>();
        assert_eq!(names(r#"json.partial! "x""#), [("x".to_string(), 15)]);
        assert_eq!(names(r#"json.array! @a, partial: "x", as: :y"#), [("x".to_string(), 26)]);
        assert_eq!(names(r#"json.a @a, partial: "x", partial: "y""#), [("x".to_string(), 21)]);
        // Offsets are bytes; "é" takes two.
        assert_eq!(names(r#"json.a @a, b: "é", partial: "x""#), [("x".to_string(), 30)]);
        assert!(names(r#"json.a @a, b: 1 # c, partial: "z""#).is_empty());
    }

    #[test]
    fn inflection_round_trips() {
        for (singular, plural) in [
            ("user", "users"),
            ("category", "categories"),
            ("day", "days"),
            ("address", "addresses"),
            ("box", "boxes"),
            ("match", "matches"),
            ("person", "people"),
            ("sales_person", "sales_people"),
            ("child", "children"),
            ("movie", "movies"),
            ("status", "statuses"),
            ("news", "news"),
            ("line_item", "line_items"),
            ("price", "prices"),
            ("cookie", "cookies"),
            ("hero", "heroes"),
            ("quiz", "quizzes"),
            ("cache", "caches"),
            ("analysis", "analyses"),
            ("daily_news", "daily_news"),
        ] {
            assert_eq!(pluralize(singular), plural);
            assert_eq!(singularize(plural), singular);
            assert_eq!(singularize(singular), singular);
        }
        for singular in ["bus", "virus", "axis"] {
            assert_eq!(singularize(singular), singular);
        }
    }

    #[test]
    fn model_render_partials() {
        let root = std::env::temp_dir().join(format!("rt-models-{}", std::process::id()));
        let views = root.join("app/views");
        for f in ["users/_user.html.erb", "comments/_comment.html.erb", "people/_person.html.haml", "categories/_category.html.erb"] {
            let p = views.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let current = views.join("posts/show.html.erb");
        let line = "<%= render @user %> <%= render(@users) %> <%= render @post.comments, cached: true %> <%= render @people %> <%= render @categories %> <%= render @post.author %> <%= render @user.name %>";
        let refs = model_renders(line);
        assert_eq!(refs.iter().map(|r| r.2).collect::<Vec<_>>(), ["user", "users", "comments", "people", "categories", "author", "name"]);
        assert_eq!(&line[refs[2].0..refs[2].1], "@post.comments");
        let found: Vec<Vec<PathBuf>> = refs.iter().map(|r| resolve_model(&Project::unknown(), &current, r.2)).collect();
        assert_eq!(found[0], [views.join("users/_user.html.erb")]);
        assert_eq!(found[1], found[0]);
        assert_eq!(found[2], [views.join("comments/_comment.html.erb")]);
        assert_eq!(found[3], [views.join("people/_person.html.haml")]);
        assert_eq!(found[4], [views.join("categories/_category.html.erb")]);
        assert!(found[5..].iter().all(Vec::is_empty));
        for query in [
            "render @products.page(params[:page])",
            "render @post.comments.order(:id)",
            "render @users.map(&:x)",
            "render @users.first.posts",
        ] {
            assert!(model_renders(query).is_empty(), "{query}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn model_render_partials_of_namespaced_models() {
        let root = std::env::temp_dir().join(format!("rt-ns-models-{}", std::process::id()));
        let views = root.join("app/views");
        for f in [
            "app/models/admin/user.rb",
            "app/models/admin/post.rb",
            "app/models/post.rb",
            "app/models/billing/invoice.rb",
            "app/views/admin/users/_user.html.erb",
            "app/views/users/_user.html.erb",
            "app/views/admin/posts/_post.html.erb",
            "app/views/posts/_post.html.erb",
            "app/views/billing/invoices/_invoice.html.erb",
            "app/views/billing/reports/show.html.erb",
            "app/views/admin/users/index.html.erb",
            "app/views/home/index.html.erb",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let found = |view: &str, name: &str| resolve_model(&Project::unknown(), &views.join(view), name);
        assert_eq!(found("home/index.html.erb", "users"), [views.join("admin/users/_user.html.erb")]);
        assert_eq!(found("admin/users/index.html.erb", "user"), [views.join("admin/users/_user.html.erb")]);
        assert_eq!(found("billing/reports/show.html.erb", "invoices"), [views.join("billing/invoices/_invoice.html.erb")]);
        // Rails prefixes the view's namespace, whether the model is `Post` or `Admin::Post`.
        assert_eq!(found("admin/users/index.html.erb", "posts"), [views.join("admin/posts/_post.html.erb")]);
        assert_eq!(found("home/index.html.erb", "posts"), [views.join("posts/_post.html.erb")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_partial_under_a_controller_folder_falls_back_to_the_model_folder() {
        let root = std::env::temp_dir().join(format!("rt-model-partial-fallback-{}", std::process::id()));
        crate::util::write_files(
            &root,
            &[
                ("app/controllers/admin/users_controller.rb", ""),
                ("app/views/admin/users/index.html.erb", ""),
                ("app/views/admin/users/_row.html.erb", ""),
                ("app/views/posts/_post.html.erb", ""),
            ],
        );
        let views = root.join("app/views");
        let found = |view: &str| resolve_model(&Project::unknown(), &views.join(view), "posts");
        assert!(found("admin/users/index.html.erb").is_empty(), "an action view gets no fallback");
        assert_eq!(found("admin/users/_row.html.erb"), [views.join("posts/_post.html.erb")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn namespaced_models_in_several_namespaces_prefer_the_views_own() {
        let root = std::env::temp_dir().join(format!("rt-ns-two-{}", std::process::id()));
        let views = root.join("app/views");
        for f in [
            "app/models/admin/user.rb",
            "app/models/shop/user.rb",
            "app/views/admin/users/_user.html.erb",
            "app/views/shop/users/_user.html.erb",
            "app/views/shop/orders/show.html.erb",
            "app/views/admin/orders/show.html.erb",
            "app/views/home/index.html.erb",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let found = |view: &str| resolve_model(&Project::unknown(), &views.join(view), "user");
        assert_eq!(found("shop/orders/show.html.erb"), [views.join("shop/users/_user.html.erb")]);
        assert_eq!(found("admin/orders/show.html.erb"), [views.join("admin/users/_user.html.erb")]);
        assert_eq!(found("home/index.html.erb"), [views.join("admin/users/_user.html.erb")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn model_partials_get_the_views_namespace_prefixed_like_rails() {
        let root = std::env::temp_dir().join(format!("rt-ns-prefix-{}", std::process::id()));
        let views = root.join("app/views");
        for f in [
            "app/models/admin/user.rb",
            "app/models/concerns/taggable.rb",
            "app/models/concerns/user.rb",
            "app/views/shop/admin/users/_user.html.erb",
            "app/views/admin/users/_user.html.erb",
            "app/views/shop/orders/show.html.erb",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let found = |view: &str| resolve_model(&Project::unknown(), &views.join(view), "user");
        assert_eq!(found("shop/orders/show.html.erb"), [views.join("shop/admin/users/_user.html.erb")]);
        let prefixed = |view_dir: &str, namespace: &str| prefixed_with_view_namespace(Path::new(view_dir), Path::new(namespace));
        assert_eq!(prefixed("a/b/c", "a/x"), Path::new("a/x"));
        assert_eq!(prefixed("a/b/c", ""), Path::new("a/b"));
        assert_eq!(prefixed("home", "admin"), Path::new("admin"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn model_partials_of_namespaced_views_fall_back_only_when_rails_would() {
        let root = std::env::temp_dir().join(format!("rt-ns-fallback-{}", std::process::id()));
        let views = root.join("app/views");
        let controller = "class Admin::UsersController < ApplicationController\nend\n";
        crate::util::write_files(
            &root,
            &[
                ("app/views/posts/_post.html.erb", ""),
                ("app/views/admin/users/index.html.erb", ""),
                ("app/views/shared/forms/show.html.erb", ""),
                ("app/views/posts/partials/_list.html.erb", ""),
                ("app/controllers/admin/users_controller.rb", controller),
            ],
        );
        let own = [views.join("posts/_post.html.erb")];
        let found = |view: &str| {
            let current = views.join(view);
            resolve_model(&Project::for_template(&current), &current, "posts")
        };
        assert!(found("admin/users/index.html.erb").is_empty(), "Rails raises MissingTemplate for admin/posts/_post");
        assert_eq!(found("shared/forms/show.html.erb"), own, "shared partials have no controller namespace");
        assert_eq!(found("posts/partials/_list.html.erb"), own, "a folder that is no controller's has no namespace");
        std::fs::create_dir_all(root.join("config/initializers")).unwrap();
        let setting = "Rails.application.config.action_view.prefix_partial_path_with_controller_namespace = false\n";
        std::fs::write(root.join("config/initializers/action_view.rb"), format!("# {setting}")).unwrap();
        assert!(found("admin/users/index.html.erb").is_empty(), "a comment doesn't turn it off");
        std::fs::write(root.join("config/initializers/action_view.rb"), setting).unwrap();
        assert_eq!(found("admin/users/index.html.erb"), own);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bare_names_resolve_through_the_controllers_parents() {
        let root = std::env::temp_dir().join(format!("rt-bare-prefixes-{}", std::process::id()));
        crate::util::write_files(
            &root,
            &[
                ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
                ("app/controllers/admin/base_controller.rb", "class Admin::BaseController < ApplicationController\nend\n"),
                ("app/controllers/admin/users_controller.rb", "class Admin::UsersController < Admin::BaseController\nend\n"),
                ("app/views/admin/users/index.html.erb", ""),
                ("app/views/admin/base/_nav.html.erb", ""),
                ("app/views/application/_flash.html.erb", ""),
            ],
        );
        let views = root.join("app/views");
        let current = views.join("admin/users/index.html.erb");
        let found = |name: &str| resolve(&Project::unknown(), &current, &render_refs(&format!("render \"{name}\"")).remove(0));
        assert_eq!(found("nav"), [views.join("admin/base/_nav.html.erb")]);
        assert_eq!(found("flash"), [views.join("application/_flash.html.erb")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_leading_slash_folder_from_views_root() {
        let root = std::env::temp_dir().join(format!("rt-slash-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        std::fs::write(users.join("_card.html.erb"), "").unwrap();
        let card = render_refs(r#"render "/users/card""#).remove(0);
        let found = resolve(&Project::unknown(), &root.join("app/views/home/index.html.erb"), &card);
        assert_eq!(found, [users.join("_card.html.erb")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_by_view_format() {
        let root = std::env::temp_dir().join(format!("rt-formats-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        for f in ["_user.html.erb", "_user.json.jbuilder", "_user.html+phone.erb", "_user.erb"] {
            std::fs::write(users.join(f), "").unwrap();
        }
        let views = root.join("app/views");
        let found = |view: &str| resolved_names(&Project::unknown(), &views, &format!("users/{view}"), r#"render "users/user""#);
        assert_eq!(found("index.html.erb"), ["_user.html.erb", "_user.html+phone.erb", "_user.erb"]);
        assert_eq!(found("show.html+phone.erb"), ["_user.html+phone.erb", "_user.html.erb", "_user.erb"]);
        assert_eq!(found("show.erb"), ["_user.html.erb", "_user.html+phone.erb", "_user.json.jbuilder", "_user.erb"]);
        let render = render_refs(r#"render "users/user""#).remove(0);
        for view in ["index.html.erb", "show.html+phone.erb", "show.erb", "index.json.jbuilder"] {
            let current = users.join(view);
            assert_eq!(resolve_model(&Project::unknown(), &current, "users"), resolve(&Project::unknown(), &current, &render));
        }
        assert_eq!(found("index.json.jbuilder"), ["_user.json.jbuilder", "_user.erb"]);
        assert_eq!(found("create.js.erb"), found("index.html.erb"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn turbo_stream_view_renders_turbo_stream_then_html_partials() {
        let root = std::env::temp_dir().join(format!("rt-turbo-{}", std::process::id()));
        let cards = root.join("app/views/cards");
        std::fs::create_dir_all(&cards).unwrap();
        let views = root.join("app/views");
        let found = |view: &str| resolved_names(&Project::unknown(), &views, &format!("cards/{view}"), r#"render "cards/card""#);
        std::fs::write(cards.join("_card.js.erb"), "").unwrap();
        assert!(found("create.turbo_stream.erb").is_empty());
        assert_eq!(found("create.js.erb"), ["_card.js.erb"]);
        std::fs::write(cards.join("_card.html.erb"), "").unwrap();
        assert_eq!(found("create.turbo_stream.erb"), ["_card.html.erb"]);
        assert_eq!(found("create.js.erb"), ["_card.js.erb", "_card.html.erb"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn partials_are_ordered_by_the_views_formats_then_variants_then_handlers() {
        let root = std::env::temp_dir().join(format!("rt-partial-order-{}", std::process::id()));
        let cards = root.join("app/views/cards");
        std::fs::create_dir_all(&cards).unwrap();
        let views = root.join("app/views");
        let found = |view: &str, line: &str| resolved_names(&Project::unknown(), &views, &format!("cards/{view}"), line);
        let write = |names: &[&str]| {
            for entry in std::fs::read_dir(&cards).unwrap() {
                std::fs::remove_file(entry.unwrap().path()).unwrap();
            }
            for name in names {
                std::fs::write(cards.join(name), "").unwrap();
            }
        };
        let plain = r#"render "cards/card""#;

        write(&["_card.html.erb", "_card.turbo_stream.erb", "_card.erb", "_card.js.erb"]);
        assert_eq!(found("create.turbo_stream.erb", plain), ["_card.turbo_stream.erb", "_card.html.erb", "_card.erb"]);
        let html = r#"render partial: "cards/card", formats: [:html]"#;
        assert_eq!(found("create.turbo_stream.erb", html), ["_card.html.erb", "_card.erb"]);
        let js = r#"render partial: "cards/card", formats: :js"#;
        assert_eq!(found("create.turbo_stream.erb", js), ["_card.js.erb", "_card.erb"]);

        write(&["_a.atom.builder", "_a.html.erb"]);
        assert_eq!(found("x.erb", r#"render "cards/a""#), ["_a.html.erb", "_a.atom.builder"]);

        write(&["_e.erb", "_e.js.erb"]);
        assert_eq!(found("x.js.erb", r#"render "cards/e""#), ["_e.js.erb", "_e.erb"]);

        write(&["_c.html+phone.erb", "_c.js.erb"]);
        let phone = r#"render partial: "cards/c", variants: [:phone]"#;
        assert_eq!(found("x.js.erb", phone), ["_c.js.erb", "_c.html+phone.erb"]);

        write(&["_h.js.haml", "_h.html.erb", "_h.js.erb"]);
        assert_eq!(found("x.js.erb", r#"render partial: "cards/h", handlers: [:erb, :haml]"#), ["_h.js.erb", "_h.js.haml", "_h.html.erb"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn app_registered_format_is_not_part_of_the_name() {
        let root = std::env::temp_dir().join(format!("rt-registered-format-{}", std::process::id()));
        let users = root.join("app/views/users");
        std::fs::create_dir_all(&users).unwrap();
        for f in ["_card.html.erb", "_card.mobile.erb"] {
            std::fs::write(users.join(f), "").unwrap();
        }
        let views = root.join("app/views");
        let found = |view: &str| {
            resolved_names(&Project::for_template(&users.join(view)), &views, &format!("users/{view}"), r#"render "users/card""#)
        };
        assert_eq!(found("show.mobile.erb"), ["_card.html.erb"]);
        assert_eq!(found("show.html.erb"), ["_card.html.erb"]);

        let initializers = root.join("config/initializers");
        std::fs::create_dir_all(&initializers).unwrap();
        std::fs::write(initializers.join("mime_types.rb"), "Mime::Type.register_alias \"text/html\", :mobile\n").unwrap();
        assert_eq!(found("show.mobile.erb"), ["_card.mobile.erb"]);
        assert_eq!(found("show.html.erb"), ["_card.html.erb"]);
        assert_eq!(found("show.erb"), ["_card.html.erb", "_card.mobile.erb"]);
        let project = Project::for_template(&users.join("show.mobile.erb"));
        assert_eq!(template_stem(|f| project.is_format(f), "_card.mobile.erb"), "_card");
        assert_eq!(template_format(|f| project.is_format(f), &users.join("_card.mobile.erb")), Some("mobile"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn candidates_per_render_kind() {
        let root = std::env::temp_dir().join(format!("rt-candidates-{}", std::process::id()));
        let views = root.join("app/views");
        for f in [
            "users/index.html.erb",
            "users/index.json.jbuilder",
            "users/_card.html.erb",
            "users/_row.json.jbuilder",
            "layouts/application.html.erb",
            "layouts/admin/base.html.erb",
            "layouts/_nav.html.erb",
            "layouts/.keep",
        ] {
            let p = views.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let current = views.join("users/show.html.erb");
        let files = files_under(&views, usize::MAX);
        let names = |kind| render_candidates(&Project::unknown(), &current, kind, &files).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names(RenderKind::Partial), ["card", "layouts/nav", "users/card"]);
        assert_eq!(names(RenderKind::Template), ["layouts/admin/base", "layouts/application", "users/index"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    // Ruby 3.3 rejects a line starting with `&&` after an expression; the latest grammar takes it.
    #[test]
    fn passed_locals_follow_the_ruby_version() {
        let text = "<%= render \"card\", a: (1\n && 2), b: 1 %>";
        let r = &render_refs(text.lines().next().unwrap())[0];
        let passed = |ruby| passed_locals(&read_call_with_lookup(text, r, false, ruby).0, r);
        assert_eq!(passed(LATEST), Some(vec!["a", "b"]));
        assert_eq!(passed(ruby_prism::Version::CRuby3_3), None);
    }

    #[test]
    fn strict_locals_signature_follows_the_ruby_version() {
        let text = "<%# locals: (a: (1\n && 2)) %>";
        let signature = |ruby| strict_locals(text, Close::default(), ruby);
        assert!(signature(LATEST).is_some());
        assert!(signature(ruby_prism::Version::CRuby3_3).is_none());
    }

    #[test]
    fn strict_locals_signature() {
        let sig =
            strict_locals("<%# locals: (title:, items: [1, 2], label: t(\"a, b\"), **options) -%>\n<p>", Close::default(), LATEST).unwrap();
        assert_eq!(sig.params, "title:, items: [1, 2], label: t(\"a, b\"), **options");
        assert_eq!(
            sig.locals,
            vec![
                ("title".to_string(), None),
                ("items".to_string(), Some("[1, 2]".to_string())),
                ("label".to_string(), Some("t(\"a, b\")".to_string())),
            ]
        );
        assert!(sig.rest);
        let none = strict_locals("<%# locals: () -%>", Close::default(), LATEST).unwrap();
        assert!(none.locals.is_empty() && !none.rest);
        assert_eq!(
            strict_locals("-# locals: (user: nil)", Close::default(), LATEST).unwrap().locals,
            vec![("user".to_string(), Some("nil".to_string()))]
        );
        assert_eq!(
            strict_locals("<%# locals: (a: ')') %>", Close::default(), LATEST).unwrap().locals,
            vec![("a".to_string(), Some("')'".to_string()))]
        );
        let no_rest = strict_locals("<%# locals: (**nil) %>", Close::default(), LATEST).unwrap();
        assert_eq!((no_rest.locals.len(), no_rest.rest), (0, false));
        // Lazy: stops at the `)` that closes the tag, not the last one on the line.
        assert_eq!(strict_locals("<%# locals: (a:) %> <%= f(1) %>", Close::default(), LATEST).unwrap().params, "a:");
        let multi = strict_locals("<%# locals: (\n  a:,\n  b: [1,\n 2]\n) -%>", Close::default(), LATEST).unwrap();
        assert_eq!(multi.locals, vec![("a".to_string(), None), ("b".to_string(), Some("[1,\n 2]".to_string()))]);
        // ActionView needs whitespace before the parenthesis.
        assert!(strict_locals("<%# locals:(a:) %>", Close::default(), LATEST).is_none());
        assert!(strict_locals("<%# just a comment %>", Close::default(), LATEST).is_none());
    }

    #[test]
    fn calls_span_continued_lines() {
        assert_eq!(passed_in("= render \"card\",\n  user: u,\n  size: 1\np next"), strs(&["user", "size"]));
        assert_eq!(passed_in("<%= render(\"card\",\n user: u) %>\nx"), strs(&["user"]));
        assert_eq!(passed_in("<%= render partial: \"card\", locals: {\n a: \"),\" } %>"), strs(&["a"]));
        assert_eq!(passed_in("<%= render \"card\" %>, x\ny"), strs(&[]));
        assert_eq!(passed_in("= render \"card\"\n= x,"), strs(&[]));
        assert_eq!(passed_in("= render(\"card\", f("), None, "never closed");
    }

    #[test]
    fn a_leading_dot_line_continues_the_last_argument() {
        let text = "<%= render \"card\", a: items\n      .first %>";
        assert_eq!(passed_in(text), strs(&["a"]));
        let r = &render_refs(text.lines().next().unwrap())[0];
        let call = read_call(text, r, false);
        let apply = |(range, new): ByteEdit| {
            let mut out = text.to_string();
            out.replace_range(range, &new);
            out
        };
        assert_eq!(apply(add_locals_edit(&call, r, &["b"]).unwrap()), "<%= render \"card\", a: items\n      .first, b: nil %>");
        assert_eq!(apply(remove_local_edit(&call, r, "a").unwrap()), "<%= render \"card\" %>");
        assert_eq!(passed_in("<%= render \"card\", a: items,\n  b: 1\n  &.first %>"), strs(&["a", "b"]));
        assert_eq!(added("<%= render \"card\", a: 1\n..3 %>", "b").unwrap(), "<%= render \"card\", a: 1, b: nil\n..3 %>");
        // On its own line inside a `<% %>` tag, the call continues too.
        let own = "<%\n  render \"card\", a: items\n    .first\n%>";
        let own_ref = &render_refs("  render \"card\", a: items")[0];
        let own_call = read_call(&own[3..], own_ref, false);
        assert_eq!(passed_locals(&own_call, own_ref), Some(vec!["a"]));
        let mut edited = own[3..].to_string();
        let (range, new) = add_locals_edit(&own_call, own_ref, &["b"]).unwrap();
        edited.replace_range(range, &new);
        assert_eq!(edited, "  render \"card\", a: items\n    .first, b: nil\n%>");
        // In Haml and Slim a leading-dot line is a `.class` element, not Ruby.
        assert_eq!(added("= render \"card\", a: 1\n.footer\n  %p hi", "b").unwrap(), "= render \"card\", a: 1, b: nil\n.footer\n  %p hi");
        assert_eq!(added("== render \"card\", a: 1\n.row", "b").unwrap(), "== render \"card\", a: 1, b: nil\n.row");
    }

    #[test]
    fn a_ref_on_a_continuation_line_edits_the_call_from_its_own_line() {
        let text = "<p>é</p>\r\n<%= render formats: [:json], locals: { user: u },\r\n  partial: \"users/card\" %>\r\n";
        let line_start = text.find("  partial").unwrap();
        let line = text[line_start..].lines().next().unwrap();
        let (r, call) = render_calls_in(text, line_start, line, &HamlPipes::new(false), LATEST, |_| true).remove(0);
        assert_eq!(passed_locals(&call, &r), Some(vec!["user"]));
        let base = call.base(line_start);
        assert_eq!(base, text.find("<%= render").unwrap());
        let (at, end) = local_key_span(&call, "user").unwrap();
        assert_eq!(&text[base + at..base + end], "user");
        let (range, new) = add_locals_edit(&call, &r, &["size"]).unwrap();
        assert_eq!((&text[base + range.start..base + range.end], new.as_str()), ("", ", size: nil"));
        assert_eq!(text[..base + range.start].lines().last(), Some("<%= render formats: [:json], locals: { user: u"));
        let (range, _) = remove_local_edit(&call, &r, "user").unwrap();
        assert_eq!(&text[base + range.start..base + range.end], "user: u");
        let no_locals = "<%= render formats: [:json],\n  partial: \"users/card\", locals: { a: 1, size: 1 } %>\n";
        let line_start = no_locals.find("  partial").unwrap();
        let line = no_locals[line_start..].lines().next().unwrap();
        let (r, call) = render_calls_in(no_locals, line_start, line, &HamlPipes::new(false), LATEST, |_| true).remove(0);
        let (range, _) = remove_local_edit(&call, &r, "size").unwrap();
        assert_eq!(&no_locals[call.base(line_start) + range.start..call.base(line_start) + range.end], ", size: 1");
    }

    #[test]
    fn local_edits_on_a_continuation_line_land_in_erb_and_haml_calls() {
        let edit = |text: &str, haml: bool, change: &dyn Fn(&RenderCall, &RenderRef) -> Option<ByteEdit>| {
            let line_start = text.find("  partial: \"").unwrap();
            let line = text[line_start..].lines().next().unwrap();
            let (r, call) = render_calls_in(text, line_start, line, &HamlPipes::new(haml), LATEST, |_| true).remove(0);
            let base = call.base(line_start);
            let (range, new) = change(&call, &r).unwrap();
            format!("{}{new}{}", &text[..base + range.start], &text[base + range.end..])
        };
        let remove = |text: &str, haml| edit(text, haml, &|call, r| remove_local_edit(call, r, "user"));
        let add = |text: &str, haml| edit(text, haml, &|call, r| add_locals_edit(call, r, &["size"]));
        let erb = |locals: &str| format!("<%= render formats: [:json],\r\n  partial: \"users/card\"{locals} %>\r\n");
        let haml = |locals: &str| format!("= render formats: [:json], |\r\n  partial: \"users/card\"{locals} |\r\n");
        // The only local goes with its hash's contents; the others keep their separators.
        assert_eq!(remove(&erb(", locals: { user: u }"), false), erb(", locals: {  }"));
        assert_eq!(remove(&erb(", locals: { a: 1, user: u }"), false), erb(", locals: { a: 1 }"));
        assert_eq!(remove(&haml(", locals: { user: u, b: 1 }"), true), haml(", locals: { b: 1 }"));
        assert_eq!(remove(&haml(", locals: { a: 1, user: u }"), true), haml(", locals: { a: 1 }"));
        assert_eq!(add(&erb(""), false), erb(", locals: { size: nil }"));
        assert_eq!(add(&haml(""), true), haml(", locals: { size: nil }"));
    }

    #[test]
    fn local_keys_on_a_continued_line() {
        let key = |text: &str| {
            let offset = text.find('|').unwrap();
            let text = text.replace('|', "");
            local_key_before(&text, offset, false, || LATEST)
                .map(|(r, passed, typed)| (r.name, r.start, passed.join(" "), typed.to_string()))
        };
        let card = |passed: &str, typed: &str| Some(("card".to_string(), 12, passed.to_string(), typed.to_string()));
        assert_eq!(key("p\n= render \"card\",\n  user: u,\n  si|"), card("user", "si"));
        assert_eq!(key("p\n= render(\"card\",\n  |)"), card("", ""));
        assert_eq!(key("p\n= render \"card\", user: u\n= link_to x, |"), None, "the call ended on its line");
    }

    #[test]
    fn local_key_lookup_follows_the_ruby_version() {
        let text = "<%= render partial: \"card\", formats: [:json], a: (1\n && 2), locals: { } %>";
        let offset = text.find("{ ").unwrap() + 2;
        let formats = |ruby| local_key_before(text, offset, false, || ruby).map(|(r, ..)| r.lookup.formats);
        assert_eq!(formats(LATEST), Some(Some(vec!["json".to_string()])));
        assert_eq!(formats(ruby_prism::Version::CRuby3_3), None);
        let unclosed = "<%= render partial: \"card\", formats: [:json], locals: { ";
        assert_eq!(
            local_key_before(unclosed, unclosed.len(), false, || LATEST).map(|(r, ..)| r.lookup.formats),
            Some(Some(vec!["json".to_string()]))
        );
    }

    #[test]
    fn haml_group_locals_after_the_cursor_are_counted_up_to_max_call_lines_below_it() {
        let rows = |prefix: &str, from: usize, to: usize| (from..to).map(|i| format!("  {prefix}{i}: 1, |\n")).collect::<String>();
        let head = "= render \"card\", |\n".to_string() + &rows("a", 0, 3);
        let text = format!("{head}  x, |\n{}  z: 1 |\np x", rows("b", 0, 15));
        let offset = head.len() + 3;
        let passed = local_key_before(&text, offset, true, || LATEST).map(|(_, passed, _)| passed.len());
        assert_eq!(passed, Some(3 + 15 + 1));
    }

    #[test]
    fn literal_render_locals() {
        fn span<'a>(line: &'a str, name: &str) -> Option<(usize, &'a str)> {
            local_key_span(&read_call(line, &render_refs(line)[0], false), name).map(|(s, e)| (s, &line[s..e]))
        }
        let line = r#"<%= render "card", title: "user: Bob", user: u %>"#;
        assert_eq!(span(line, "user"), Some((39, "user")), "not the text inside the string");
        let line = r#"<%= render partial: "card", locals: { :user => u } %>"#;
        assert_eq!(span(line, "user"), Some((39, "user")));
        assert_eq!(span(r#"<%= render "card", size: 1 %>"#, "user"), None);

        fn passed(line: &str) -> Option<Vec<&str>> {
            passed_of(line)
        }
        assert_eq!(passed(r#"<%= render "card", title: "a, b", n: f(1, 2) %>"#), Some(vec!["title", "n"]));
        assert_eq!(passed(r#"<%= render("card", user:) -%>"#), Some(vec!["user"]));
        assert_eq!(passed(r#"<%= render "card" %>"#), Some(vec![]));
        assert_eq!(passed(r#"<%= render partial: "card", locals: { a: 1, b: [2] } %>"#), Some(vec!["a", "b"]));
        assert_eq!(passed(r#"= render(partial: "card")"#), Some(vec![]));
        assert_eq!(passed(r#"json.partial! "posts/post", post: p"#), Some(vec!["post"]));
        assert_eq!(passed(r#"json.partial! partial: "posts/post", locals: { post: p }"#), Some(vec!["post"]));
        assert_eq!(passed(r#"json.partial! "posts/post", collection: @posts, as: :post"#), None);
        assert_eq!(passed(r#"json.array! @posts, partial: "posts/post", as: :post"#), None);
        assert_eq!(passed(r#"<%= render :partial => "card", :locals => { :a => 1, b: 2 } %>"#), Some(vec!["a", "b"]));
        assert_eq!(passed(r#"<%= render "card", :a => 1 %>"#), Some(vec!["a"]));
        assert_eq!(passed(r#"<%= render partial: "card", layout: "box", locals: { a: 1 } %>"#), Some(vec!["a"]));
        assert_eq!(passed(r#"<%= render partial: "card", formats: [:html] %>"#), Some(vec![]));
        assert_eq!(passed(r#"<%= render partial: "card", object: @u %>"#), None, "the object arrives as a local");
        assert_eq!(passed(r#"<%= render partial: "card", collection: @us, locals: { a: 1 } %>"#), None);
        assert_eq!(passed(r#"<%= render "card", collection: @cards %>"#), Some(vec!["collection"]));
        assert_eq!(passed(r#"json.partial! "posts/post", locals: { post: @post }"#), Some(vec!["post"]));
        assert_eq!(passed(r#"<%= @x.to_json.html_safe %><%= render "card", as: 1 %>"#), Some(vec!["as"]));
        assert_eq!(passed(r#"<%= render "card" do %>"#), Some(vec![]), "the block is the partial's content");
        assert_eq!(passed(r#"<%= render("card", a: 1) { |x| %>"#), Some(vec!["a"]));
        assert_eq!(passed(r#"<%= content_tag :div, render("card", a: 1), class: "x" %>"#), Some(vec!["a"]));
        assert_eq!(passed(r#"= render "card", a: 1 if show?"#), Some(vec!["a"]));
        for skip in [
            r#"<%= render partial: "card", collection: @cards %>"#,
            r#"<%= render "card", **opts %>"#,
            r#"<%= render "card", locals %>"#,
            r#"<%= render "card", Card::DEFAULTS %>"#,
            r#"<%= render "card", "a" => 1 %>"#,
            r#"<%= render "card", a: 1,"#,
            r#"<%= render template: "a/b" %>"#,
        ] {
            assert_eq!(passed(skip), None, "{skip}");
        }
    }

    #[test]
    fn component_classes_by_constant() {
        let root = std::env::temp_dir().join(format!("rt-components-{}", std::process::id()));
        for f in [
            "app/components/admin/user_card_component.rb",
            "app/components/button_component.rb",
            "app/components/button_component.html.erb",
            "app/components/helpers.rb",
            "app/components/über_component.rb",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        std::fs::create_dir_all(root.join("app/views/home")).unwrap();
        let names: Vec<String> = component_classes(&root.join("app/views/home/index.html.erb")).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["Admin::UserCardComponent", "ButtonComponent", "ÜberComponent"]);
        let from_component: Vec<String> =
            component_classes(&root.join("app/components/button_component.html.erb")).into_iter().map(|(n, _)| n).collect();
        assert_eq!(from_component, names);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn component_haml_sidecar() {
        let root = std::env::temp_dir().join(format!("rt-component-{}", std::process::id()));
        let components = root.join("app/components/users");
        std::fs::create_dir_all(&components).unwrap();
        std::fs::create_dir_all(root.join("app/views/users")).unwrap();
        for f in ["card_component.rb", "card_component.html.haml"] {
            std::fs::write(components.join(f), "").unwrap();
        }
        let found = resolve_component(&Project::unknown(), &root.join("app/views/users/show.html.haml"), "Users::CardComponent");
        assert_eq!(found, [components.join("card_component.rb"), components.join("card_component.html.haml")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Files of `dir` under a fresh temp `app/views`, and a resolver from `view` (path under app/views) to file names.
    fn partial_tree(tag: &str, files: &[&str]) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("rt-{tag}-{}", std::process::id()));
        let views = root.join("app/views");
        for f in files {
            let p = views.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        (root, views)
    }

    fn resolved_names(project: &Project, views: &Path, view: &str, render: &str) -> Vec<String> {
        let r = render_refs(render).remove(0);
        resolve(project, &views.join(view), &r).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn locale_only_partial_resolves_from_any_format() {
        let (root, views) = partial_tree("locale-only", &["shared/_greeting.de.erb"]);
        for view in ["users/show.html.erb", "users/index.json.jbuilder", "users/show.erb"] {
            assert_eq!(resolved_names(&Project::unknown(), &views, view, r#"render "shared/greeting""#), ["_greeting.de.erb"], "{view}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn locale_and_format_partial_follows_view_format() {
        let (root, views) = partial_tree("locale-format", &["users/_card.en.html.erb", "users/_clip.js.erb", "users/_event.ics.erb"]);
        assert_eq!(resolved_names(&Project::unknown(), &views, "users/show.html.erb", r#"render "card""#), ["_card.en.html.erb"]);
        assert!(resolved_names(&Project::unknown(), &views, "users/index.json.jbuilder", r#"render "card""#).is_empty());
        // `js` has locale shape but is a format: it must not read as format-less and match JSON views.
        assert!(resolved_names(&Project::unknown(), &views, "users/index.json.jbuilder", r#"render "clip""#).is_empty());
        assert_eq!(resolved_names(&Project::unknown(), &views, "users/index.js.erb", r#"render "clip""#), ["_clip.js.erb"]);
        assert!(resolved_names(&Project::unknown(), &views, "users/index.json.jbuilder", r#"render "event""#).is_empty());
        assert_eq!(resolved_names(&Project::unknown(), &views, "users/index.ics.erb", r#"render "event""#), ["_event.ics.erb"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn partial_in_the_views_root() {
        let (root, views) = partial_tree("views-root", &["_note.html.erb", "users/_note.html.erb"]);
        assert_eq!(resolved_names(&Project::unknown(), &views, "users/show.html.erb", r#"render "/note""#), ["_note.html.erb"]);
        let found = resolve(&Project::unknown(), &views.join("users/show.html.erb"), &render_refs(r#"render "/note""#).remove(0));
        assert_eq!(found, [views.join("_note.html.erb")], "the root one, not the folder's");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lookup_options_pick_the_files() {
        let files = ["users/_card.html.erb", "users/_card.html+phone.erb", "users/_card.json.jbuilder", "users/_card.html.haml"];
        let (root, views) = partial_tree("lookup", &files);
        let names = |render: &str| resolved_names(&Project::unknown(), &views, "users/show.html.erb", render);
        assert_eq!(names(r#"render partial: "card", formats: [:json]"#), ["_card.json.jbuilder"]);
        assert_eq!(names(r#"render partial: "card", formats: :json, handlers: [:jbuilder]"#), ["_card.json.jbuilder"]);
        assert_eq!(names(r#"render partial: "card", handlers: %i[haml]"#), ["_card.html.haml"]);
        assert_eq!(names(r#"render partial: "card", variants: [:phone]"#), ["_card.html+phone.erb", "_card.html.erb", "_card.html.haml"]);
        assert_eq!(
            names(r#"render partial: "card", variants: [:tablet]"#),
            ["_card.html.erb", "_card.html.haml"],
            "only the variant-less ones"
        );
        // In the short form the hash is locals, and a computed value names nothing literal.
        assert_eq!(names(r#"render "card", formats: [:json]"#), ["_card.html.erb", "_card.html.haml", "_card.html+phone.erb"]);
        assert_eq!(
            names(r#"render partial: "card", formats: request_formats"#),
            ["_card.html.erb", "_card.html.haml", "_card.html+phone.erb"]
        );
        assert!(names(r#"render partial: "card", formats: [:xml]"#).is_empty(), "Rails raises Missing partial");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lookup_options_apply_to_render_template() {
        let (root, views) = partial_tree("lookup-template", &["users/card.html.erb", "users/card.json.jbuilder"]);
        let names = |render: &str| resolved_names(&Project::unknown(), &views, "users/show.html.erb", render);
        assert_eq!(names(r#"render template: "users/card", formats: [:json]"#), ["card.json.jbuilder"]);
        assert_eq!(names(r#"render template: "users/card", handlers: [:erb]"#), ["card.html.erb"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lookup_handlers_can_exclude_the_format_match() {
        let (root, views) = partial_tree("lookup-handlers", &["users/_card.json.jbuilder", "users/_card.html.erb"]);
        let names = |render: &str| resolved_names(&Project::unknown(), &views, "users/show.html.erb", render);
        assert!(names(r#"render partial: "card", formats: [:json], handlers: [:erb]"#).is_empty());
        assert_eq!(names(r#"render partial: "card", formats: %i(json html), handlers: %i(erb)"#), ["_card.html.erb"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lookup_is_read_around_the_name_and_the_statement() {
        let json = Some(vec!["json".to_string()]);
        let formats = |line: &str| render_refs(line)[0].lookup.formats.clone();
        assert_eq!(formats(r#"<%= render partial: "a", formats: [:json]; render partial: "b" %>"#), json);
        assert_eq!(formats(r#"<%= render partial: "a"; render partial: "b", formats: [:json] %>"#), None);
        assert_eq!(formats(r#"render formats: [:json], partial: "card""#), json);
        assert_eq!(formats(r#"render(formats: [:json], :partial => "card")"#), json);
        assert_eq!(formats(r#"render layout: "wrap", formats: [:json] do |x|"#), json);
        assert_eq!(formats(r#"render partial: "card", locals: { formats: 1 }"#), None);
    }

    #[test]
    fn lookup_keeps_options_before_the_name_when_the_rest_is_unparsed() {
        let json = Some(vec!["json".to_string()]);
        let formats = |line: &str| render_refs(line)[0].lookup.formats.clone();
        assert_eq!(formats(r#"render formats: [:json], partial: "card" if show?"#), json);
        assert_eq!(formats(r#"render formats: [:json], partial: "card" do"#), json);
    }

    #[test]
    fn a_first_key_that_only_starts_with_partial_is_not_the_name() {
        let refs = render_refs(r#"render partial_name: 1, partial: "card""#);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "card");
    }

    #[test]
    fn semicolons_inside_nested_interpolated_strings_do_not_cut_the_arguments() {
        let locals = |line: &str| passed_of(line).map(|names| names.join(","));
        assert_eq!(locals(r##"render "card", a: "#{f("a;b")}", b: 1"##).as_deref(), Some("a,b"));
        assert_eq!(locals(r##"render "card", a: "#{f("}")} ;", b: 1"##).as_deref(), Some("a,b"));
    }

    #[test]
    fn interpolations_do_not_end_the_call() {
        fn locals(text: &str) -> Option<Vec<&str>> {
            let r = render_refs(text.lines().next().unwrap()).remove(0);
            passed_locals(&read_call(text, &r, false), &r)
        }
        assert_eq!(locals("<%= render \"card\", a: \"#{f(\"}\")}\", b: 1 %>"), Some(vec!["a", "b"]));
        assert_eq!(locals("= render \"card\", a: \"#{t(\"(\")}\", b: 1\n= c: 2\n"), Some(vec!["a", "b"]));
        assert_eq!(locals("<%= render(\"card\", a: \"#{b}\") { |y| y } %>"), Some(vec!["a"]));
    }

    #[test]
    fn slashes_in_strings_are_not_regex_literals() {
        assert_eq!(passed_of(r#"<% render "card", url: "/users"; foo %>"#), Some(vec!["url"]));
    }

    #[test]
    fn an_open_quote_is_counted_in_the_call_lines() {
        let text = format!("= render \"card\", a: \"x\n{}", "y\n".repeat(MAX_CALL_LINES * 2));
        assert_eq!(passed_in(&text), None, "an unclosed call is not read");
    }

    #[test]
    fn blocks_do_not_hide_passed_locals() {
        let locals = |line: &str| passed_of(line).map(|names| names.join(","));
        assert_eq!(locals(r#"render layout: "wrap", locals: { a: 1 } do |x|"#).as_deref(), Some("a"));
        assert_eq!(locals(r#"render("card", a: 1) { |y| y }"#).as_deref(), Some("a"));
        assert_eq!(locals(r#"render partial: "card", locals: { a: 1 }; foo"#).as_deref(), Some("a"));
    }

    #[test]
    fn semicolons_inside_strings_and_hashes_do_not_cut_the_arguments() {
        let locals = |line: &str| passed_of(line).map(|names| names.join(","));
        assert_eq!(locals(r#"render "card", a: "x;y", b: 1"#).as_deref(), Some("a,b"));
        assert_eq!(locals(r#"render partial: "card", locals: { a: ";", b: 2 }"#).as_deref(), Some("a,b"));
        assert_eq!(locals(r#"render "card", a: (1; 2).size, b: 1"#).as_deref(), Some("a,b"));
    }

    #[test]
    fn hash_braces_are_not_taken_for_blocks() {
        let locals = |line: &str| passed_of(line).map(|names| names.join(","));
        for line in [
            r#"render "card", tags: post.tags.map { |t| t.name }, size: :md"#,
            r#"render "card", f: lambda { x }, b: 2"#,
            r#"render "card", f: proc { }, b: 2"#,
            r#"render "card", a: x || {}, b: 1"#,
        ] {
            assert_eq!(locals(line).map(|names| names.split(',').count()), Some(2), "{line}");
        }
        assert_eq!(locals(r#"render "card", :a => { x: 1 }, b: 2"#).as_deref(), Some("a,b"));
        assert_eq!(locals(r#"render partial: "card", locals: {a: 1}"#).as_deref(), Some("a"));
        assert_eq!(locals(r#"render partial: "card", locals: { a: 1 } do |x|"#).as_deref(), Some("a"));
    }

    #[test]
    fn keys_in_the_next_statement_are_not_refs() {
        assert!(render_refs(r#"- render plain: "ok"; helper "id", partial: "row""#).is_empty());
        assert!(render_refs(r#"<% render partial: "a"; wrap x, layout: "b" %>"#).iter().all(|r| !r.from_layout));
    }

    #[test]
    fn percent_literals_before_the_name_are_refs() {
        let refs = render_refs(r#"render formats: %i[json], partial: "card""#);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "card");
        assert_eq!(refs[0].lookup.formats, Some(vec!["json".to_string()]));
    }

    #[test]
    fn options_before_the_name_are_a_ref_with_spans_and_edits() {
        let line = r#"<%= render formats: [:json], partial: "users/card", locals: { a: 1 } %>"#;
        let refs = render_refs(line);
        assert_eq!(refs.len(), 1);
        let r = &refs[0];
        assert_eq!(&line[r.start..r.end], "users/card");
        assert_eq!(r.call, line.find("render").unwrap());
        let call = read_call(line, r, false);
        assert_eq!(passed_locals(&call, r).as_deref(), Some(&["a"][..]));
        let (range, text) = add_locals_edit(&call, r, &["b"]).unwrap();
        assert_eq!(format!("{}{}{}", &line[..range.start], text, &line[range.end..]), line.replace("a: 1", "a: 1, b: nil"));
        let (range, _) = remove_local_edit(&call, r, "a").unwrap();
        assert_eq!(&line[range], "a: 1");
        let (at, end) = local_key_span(&call, "a").unwrap();
        assert_eq!(&line[at..end], "a");
    }

    #[test]
    fn options_before_the_name_ignore_unbalanced_or_nested_partial_keys() {
        assert!(render_refs(r#"render locals: { partial: "x" }, id: 1"#).is_empty());
        assert!(render_refs(r#"render foo: [1, partial: "x""#).is_empty());
        assert_eq!(render_refs(r#"render partial: "a", partial: "b""#).len(), 1);
    }

    /// The refs `render_calls_in` finds on each line of `text`, as `line:name:start-end:call:layout?:formats`.
    fn refs_by_line(text: &str, haml: bool) -> Vec<String> {
        let pipes = HamlPipes::new(haml);
        let mut found = vec![];
        for (i, (line_start, line)) in crate::util::lines_with_offsets(text).enumerate() {
            for (r, _) in render_calls_in(text, line_start, line, &pipes, LATEST, |_| true) {
                assert_eq!(&line[r.start..r.end], r.name);
                let formats = r.lookup.formats.map(|formats| formats.join(",")).unwrap_or_default();
                found.push(format!("{i}:{}:{}-{}:{}:{}:{formats}", r.name, r.start, r.end, r.call, r.from_layout));
            }
        }
        found
    }

    #[test]
    fn options_continued_after_a_trailing_comma() {
        let erb = "<%= render formats: [:json],\n           partial: \"users/user\" %>\n";
        assert_eq!(refs_by_line(erb, false), ["1:users/user:21-31:0:false:json"]);
        let erb = "<%= render partial: \"row\",\n           layout: \"box\" %>\n";
        assert_eq!(refs_by_line(erb, false), ["0:row:21-24:4:false:", "1:box:20-23:0:true:"]);
        let haml = "= render formats: [:json],\n  layout: \"shared/wrap\" do\n  p x\n";
        assert_eq!(refs_by_line(haml, true), ["1:shared/wrap:11-22:0:false:json"]);
        // No render above, or a line that ends the call: nothing continues.
        assert!(refs_by_line("<%= link_to \"x\", a,\n  partial: \"users/user\" %>\n", false).is_empty());
        assert!(refs_by_line("<%= render formats: [:json] %>\n  partial: \"users/user\"\n", false).is_empty());
    }

    #[test]
    fn jbuilder_options_continued_on_a_later_line() {
        let refs = |text: &str| {
            let found = refs_by_line(text, false);
            found.iter().map(|r| r.split(':').take(2).collect::<Vec<_>>().join(":")).collect::<Vec<_>>()
        };
        assert_eq!(refs("json.partial! partial: \"users/user\",\n  locals: { user: @user }\n"), ["0:users/user"]);
        assert_eq!(refs("json.partial! \"users/user\",\n  user: u\n"), ["0:users/user"]);
        assert_eq!(refs("json.partial!(\n  partial: \"users/user\", locals: { user: u })\n"), ["1:users/user"]);
        assert_eq!(refs("json.array! @posts,\n  partial: \"posts/post\", as: :post\n"), ["1:posts/post"]);
        assert_eq!(refs("json.comments @c,\n  partial: \"comments/comment\"\n"), ["1:comments/comment"]);
        assert_eq!(refs("render(\n  partial: \"users/user\")\n"), ["1:users/user"]);
        assert!(refs("json.name user.name,\n  other: 1\n").is_empty());
    }

    #[test]
    fn open_paren_continues_only_render_and_json_calls() {
        let names =
            |text: &str| refs_by_line(text, false).iter().map(|r| r.split(':').take(2).collect::<Vec<_>>().join(":")).collect::<Vec<_>>();
        assert!(names("<%= link_to(\n  \"x\", partial: \"y\") %>\n").is_empty());
        assert!(names("<%= my_json.map(\n  partial: \"x\") %>\n").is_empty());
        assert!(names("<%= foo # render (\n  partial: \"x\" %>\n").is_empty());
        assert_eq!(names("<%= render(\n  \"users/user\") %>\n"), ["1:users/user"]);
        assert_eq!(names("json.partial!(\n  \"users/user\", user: u)\n"), ["1:users/user"]);
        assert_eq!(names("<%= render(\n  partial: \"users/user\", locals: { a: { b: 1 } }) %>\n"), ["1:users/user"]);
    }

    #[test]
    fn element_ids_and_html_before_a_call_are_not_comments() {
        let names = |text: &str, haml| {
            refs_by_line(text, haml).iter().map(|r| r.split(':').take(2).collect::<Vec<_>>().join(":")).collect::<Vec<_>>()
        };
        assert_eq!(names("#main= render formats: [:json],\n  partial: \"users/user\"\n", true), ["1:users/user"]);
        assert_eq!(names("%div#main= render formats: [:json],\n  partial: \"users/user\"\n", true), ["1:users/user"]);
        assert_eq!(names("#main = render(\n  \"users/user\")\n", true), ["1:users/user"]);
        assert_eq!(names("<a href=#top><%= render(\n  \"users/user\") %>\n", false), ["1:users/user"]);
    }

    #[test]
    fn continued_call_does_not_repeat_refs_of_its_first_line() {
        let refs = refs_by_line("<%= render partial: \"a\",\n  layout: \"c\", as: :x %>\n", false);
        let names: Vec<_> = refs.iter().map(|r| r.split(':').take(2).collect::<Vec<_>>().join(":")).collect();
        assert_eq!(names, ["0:a", "1:c"]);
    }

    #[test]
    fn continuation_refs_need_a_render_call_within_reach_and_a_trailing_comma() {
        // A call line that ended its call has no continuation; CRLF lines continue like LF ones.
        let quoted = "<%= render formats: [:json] %>\n<%= x \"a,\"\n  partial: \"users/user\" %>\n";
        assert!(refs_by_line(quoted, false).is_empty());
        let ended = "<%= render formats: [:json]\n  partial: \"users/user\" %>\n";
        assert!(refs_by_line(ended, false).is_empty());
        let crlf = "<%= render formats: [:json],\r\n  partial: \"users/user\" %>\r\n";
        assert_eq!(refs_by_line(crlf, false), ["1:users/user:12-22:0:false:json"]);
        // A call line further than MAX_CALL_LINES lines above is out of reach.
        let filler = |n: usize| "  a: 1,\n".repeat(n);
        let reach = |n: usize| format!("<%= render formats: [:json],\n{}  partial: \"users/user\" %>\n", filler(n));
        let last = |text: String| refs_by_line(&text, false).pop();
        assert_eq!(last(reach(MAX_CALL_LINES - 2)), Some(format!("{}:users/user:12-22:0:false:json", MAX_CALL_LINES - 1)));
        assert_eq!(last(reach(MAX_CALL_LINES - 1)), None);
    }

    #[test]
    fn continuation_refs_in_haml_with_pipes_and_trailing_commas() {
        let haml = "= render formats: [:json], |\n  partial: \"users/user\" |\np x\n";
        assert_eq!(refs_by_line(haml, true), ["1:users/user:12-22:0:false:json"]);
    }

    #[test]
    fn options_before_the_name_in_haml() {
        let r = &render_refs(r#"= render formats: [:json], partial: "card""#)[0];
        assert_eq!(r.name, "card");
        assert_eq!(r.lookup.formats, Some(vec!["json".to_string()]));
    }

    #[test]
    fn semicolon_inside_a_literal_is_not_a_statement_end() {
        let locals = |line: &str| passed_of(line).map(|names| names.join(","));
        for line in
            [r#"= render "card", sep: /;/, b: 1"#, r#"= render "card", sep: ?;, b: 1"#, r#"= render "card", sep: a / b, c: 1; x / y"#]
        {
            assert_eq!(locals(line).map(|names| names.split(',').count()), Some(2), "{line}");
            assert!(added(line, "x").is_some(), "{line}");
        }
        let line = r#"<% render "a", x: 1; render "b" %>"#;
        assert_eq!(passed_of(line).as_deref(), Some(&["x"][..]));
    }

    #[test]
    fn a_call_is_not_read_past_max_call_lines() {
        let text = format!("= render \"card\", a: (\n{}", "1,\n".repeat(MAX_CALL_LINES * 2));
        assert_eq!(passed_in(&text), None, "an unclosed call is not read");
    }

    #[test]
    fn render_ref_at_reads_lookups_from_later_lines() {
        let text = "<%= render partial: \"users/card\",\n  formats: [:json] %>\n";
        let line = text.lines().next().unwrap();
        let r = render_ref_at(text, 0, line, false, LATEST, 22).unwrap();
        assert_eq!(r.lookup.formats, Some(vec!["json".to_string()]));
    }

    #[test]
    fn layout_next_to_partial_takes_the_lookup() {
        let json = Some(vec!["json".to_string()]);
        for line in
            [r#"render partial: "card", layout: "wrap", formats: [:json]"#, r#"render partial: "card", formats: [:json], layout: "wrap""#]
        {
            let lookups: Vec<_> = render_refs(line).iter().map(|r| (r.name.clone(), r.lookup.formats.clone())).collect();
            assert_eq!(lookups, [("card".to_string(), json.clone()), ("wrap".to_string(), json.clone())], "{line}");
        }
    }

    #[test]
    fn a_kept_layout_alone_still_gets_its_partners_lookup_and_call() {
        let text = "p\n<%= render partial: \"card\", layout: \"wrap\",\n  formats: [:json], locals: { a: 1 } %>\n";
        let line_start = 2;
        let line = text[line_start..].lines().next().unwrap();
        let found = render_calls_in(text, line_start, line, &HamlPipes::new(false), LATEST, |r| r.from_layout);
        assert_eq!(found.len(), 1);
        let (layout, call) = &found[0];
        assert_eq!((layout.name.as_str(), layout.lookup.formats.clone()), ("wrap", Some(vec!["json".to_string()])));
        assert_eq!(passed_locals(call, layout), Some(vec!["a"]));
    }

    #[test]
    fn locals_after_a_method_chain_line_are_passed() {
        assert_eq!(passed_in("<%= render \"card\", size: items\n    .first,\n  user: u %>"), strs(&["size", "user"]));
    }

    #[test]
    fn block_layout_reads_its_own_call_lookup() {
        let refs = render_refs(r#"render layout: "shared/wrap", formats: [:json] do"#);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].lookup.formats, Some(vec!["json".to_string()]));
    }

    #[test]
    fn refs_sharing_a_call_keep_their_own_lookup() {
        let formats = |line: &str| {
            render_refs(line).iter().map(|r| (r.name.clone(), r.lookup.formats.clone().unwrap_or_default())).collect::<Vec<_>>()
        };
        let json = vec!["json".to_string()];
        assert_eq!(
            formats(r#"json.partial! "a"; json.c @c, partial: "b", formats: [:json]"#),
            [("a".to_string(), vec![]), ("b".to_string(), json.clone())]
        );
        assert_eq!(
            formats(r#"<%= render partial: "a", formats: [:json] %> <%= render partial: "b", layout: "w" %>"#),
            [("a".to_string(), json), ("b".to_string(), vec![]), ("w".to_string(), vec![])]
        );
    }

    #[test]
    fn lookup_orders_by_the_given_formats_and_variants() {
        let files = ["users/_card.html.erb", "users/_card.json.jbuilder", "users/_card.erb", "users/_card.en.json.jbuilder"];
        let (root, views) = partial_tree("lookup-order", &files);
        let names = |render: &str| resolved_names(&Project::unknown(), &views, "users/show.html.erb", render);
        assert_eq!(
            names(r#"render partial: "card", formats: [:json, :html]"#),
            ["_card.en.json.jbuilder", "_card.json.jbuilder", "_card.html.erb", "_card.erb"]
        );
        assert_eq!(names(r#"render partial: "card", formats: [:html, :json]"#)[..2], ["_card.html.erb", "_card.en.json.jbuilder"]);
        assert_eq!(names(r#"render partial: "card", formats: [:json]"#), ["_card.en.json.jbuilder", "_card.json.jbuilder", "_card.erb"]);
        std::fs::remove_dir_all(&root).unwrap();

        let files = ["users/_card.html+tablet.erb", "users/_card.html+phone.erb", "users/_card.html.erb"];
        let (root, views) = partial_tree("lookup-variant-order", &files);
        let names = |render: &str| resolved_names(&Project::unknown(), &views, "users/show.html.erb", render);
        assert_eq!(
            names(r#"render partial: "card", variants: [:phone, :tablet]"#),
            ["_card.html+phone.erb", "_card.html+tablet.erb", "_card.html.erb"]
        );
        assert_eq!(names(r#"render partial: "card", variants: [:tablet, :phone]"#)[..2], ["_card.html+tablet.erb", "_card.html+phone.erb"]);
        std::fs::remove_dir_all(&root).unwrap();

        let (root, views) = partial_tree("lookup-handler-order", &["users/_card.json.erb", "users/_card.json.jbuilder"]);
        let names = |render: &str| resolved_names(&Project::unknown(), &views, "users/show.html.erb", render);
        assert_eq!(
            names(r#"render partial: "card", formats: [:json], handlers: [:jbuilder, :erb]"#),
            ["_card.json.jbuilder", "_card.json.erb"]
        );
        assert_eq!(
            names(r#"render partial: "card", formats: [:json], handlers: [:erb, :jbuilder]"#),
            ["_card.json.erb", "_card.json.jbuilder"]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn literal_lookup_names() {
        assert_eq!(literal_names(":json"), Some(vec!["json".to_string()]));
        assert_eq!(literal_names(r#"["json", :html]"#), Some(vec!["json".to_string(), "html".to_string()]));
        assert_eq!(literal_names("%w(phone tablet)"), Some(vec!["phone".to_string(), "tablet".to_string()]));
        assert_eq!(literal_names("[:json, fmt]"), None, "a variable in the list");
        assert_eq!(literal_names("request.format"), None);
    }

    #[test]
    fn variant_named_partial_is_not_the_bare_name() {
        let (root, views) = partial_tree("variant-name", &["users/_user.v2.html.erb", "users/_user.html.erb"]);
        assert_eq!(resolved_names(&Project::unknown(), &views, "users/show.html.erb", r#"render "user""#), ["_user.html.erb"]);
        assert_eq!(resolved_names(&Project::unknown(), &views, "users/show.html.erb", r#"render "user.v2""#), ["_user.v2.html.erb"]);
        let only_v2 = partial_tree("variant-only", &["users/_user.v2.html.erb"]);
        assert!(resolved_names(&Project::unknown(), &only_v2.1, "users/show.html.erb", r#"render "user""#).is_empty());
        assert!(resolve_model(&Project::unknown(), &only_v2.1.join("users/show.html.erb"), "user").is_empty());
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&only_v2.0).unwrap();
    }

    #[test]
    fn completion_names_drop_locale_format_and_keep_variant_looking_stems() {
        let (root, views) = partial_tree(
            "names",
            &["users/_user.v2.html.erb", "users/_card.en.html.erb", "users/_row.html+phone.erb", "users/index.de.html.erb"],
        );
        let current = views.join("users/show.html.erb");
        let files = files_under(&views, usize::MAX);
        let names = |kind| render_candidates(&Project::unknown(), &current, kind, &files).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names(RenderKind::Partial), ["card", "row", "user.v2", "users/card", "users/row", "users/user.v2"]);
        assert_eq!(names(RenderKind::Template), ["users/index"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn render_ref_at_covers_quotes_but_not_beyond() {
        let line = r#"<%= render "users/card" %>"#;
        let open_quote = line.find('"').unwrap();
        let end = open_quote + 1 + "users/card".len();
        assert!(render_ref_at(line, 0, line, false, LATEST, open_quote - 1).is_none());
        assert!(render_ref_at(line, 0, line, false, LATEST, open_quote).is_some());
        assert!(render_ref_at(line, 0, line, false, LATEST, end + 1).is_some());
        assert!(render_ref_at(line, 0, line, false, LATEST, end + 2).is_none());
    }

    #[test]
    fn component_ref_at_covers_exactly_the_class_name() {
        let line = "= render Users::CardComponent.new";
        let start = line.find("Users").unwrap();
        let end = start + "Users::CardComponent".len();
        assert!(component_ref_at(line, start - 1).is_none());
        assert!(component_ref_at(line, start).is_some());
        assert!(component_ref_at(line, end).is_some());
        assert!(component_ref_at(line, end + 1).is_none());
    }

    /// The names `passed_locals` reads from the first call in `text`, which starts at its first line.
    fn passed_in(text: &str) -> Option<Vec<String>> {
        passed_of(text).map(|names| names.into_iter().map(str::to_string).collect())
    }

    fn passed_of(text: &str) -> Option<Vec<&str>> {
        passed_for(text, false)
    }

    fn passed_for(text: &str, haml: bool) -> Option<Vec<&str>> {
        let r = render_refs(text.lines().next().unwrap()).remove(0);
        passed_locals(&read_call(text, &r, haml), &r)
    }

    /// `passed_in` for a Haml template.
    fn passed_in_haml(text: &str) -> Option<Vec<String>> {
        passed_for(text, true).map(|names| names.into_iter().map(str::to_string).collect())
    }

    /// `text` with `name` passed to its first call.
    fn added(text: &str, name: &str) -> Option<String> {
        added_for(text, name, false)
    }

    fn added_for(text: &str, name: &str, haml: bool) -> Option<String> {
        let r = render_refs(text.lines().next().unwrap()).remove(0);
        let (range, new) = add_locals_edit(&read_call(text, &r, haml), &r, &[name])?;
        let mut out = text.to_string();
        out.replace_range(range, &new);
        Some(out)
    }

    fn strs(names: &[&str]) -> Option<Vec<String>> {
        Some(names.iter().map(|name| name.to_string()).collect())
    }

    #[test]
    fn erb_trim_markers_and_raw_output_are_not_ruby() {
        assert_eq!(passed_in(r#"<%== render "card", a: 1 -%>"#), strs(&["a"]));
        assert_eq!(passed_in(r#"<%= render "card", a: 1 =%>"#), strs(&["a"]));
        assert_eq!(passed_in("<%= render \"card\",\n  a: 1 -%>\n<% x %>"), strs(&["a"]));
    }

    #[test]
    fn haml_and_slim_continuations_read_as_one_call() {
        assert_eq!(passed_in("== render \"card\", a: 1,\n  b: 2\np x"), strs(&["a", "b"]));
        assert_eq!(passed_in("= render \"card\", a: 1, \\\n  b: 2\np x"), strs(&["a", "b"]));
        assert_eq!(passed_in("- render \"card\", a: 1\np x"), strs(&["a"]));
    }

    #[test]
    fn haml_pipe_multiline_reads_as_one_call() {
        let text = "= render \"users/card\",  |\n    user: @user,         |\n    size: :md            |\np x";
        assert_eq!(passed_in_haml(text), strs(&["user", "size"]));
        assert_eq!(passed_in_haml("= render \"card\", a: 1, |\n  b: 2 |\np x"), strs(&["a", "b"]));
        assert_eq!(passed_in_haml("= render \"card\", a: 1 |\np x"), strs(&["a"]));
        // Blank lines keep the group open, as in Haml.
        assert_eq!(passed_in_haml("= render \"card\", a: 1, |\n\n  b: 2 |\np x"), strs(&["a", "b"]));
        // The group ends at the first line without the marker.
        assert_eq!(passed_in_haml("= render \"card\", a: 1, |\n  b: 2 |\n  c: 3\np x"), strs(&["a", "b"]));
        assert_eq!(passed_in_haml("= render \"card\", a: x ||\n  y"), strs(&["a"]));
        // Haml joins the lines, so a value or an operator may start on the next one.
        assert_eq!(passed_in_haml("= render \"card\", title: |\n  \"Hi\", b: 1 |\np x"), strs(&["title", "b"]));
        assert_eq!(passed_in_haml("= render \"card\", a: x |\n  + y, b: 1 |\np x"), strs(&["a", "b"]));
        assert_eq!(passed_in_haml("= render \"card\", a: x |\n  && y, b: 1 |\np x"), strs(&["a", "b"]));
    }

    #[test]
    fn haml_filter_bodies_have_no_pipe_groups() {
        let text = ":ruby\n  x = render \"card\", a: b |\n    c, d: 1\np x";
        let line_start = text.find("  x =").unwrap();
        assert!(!HamlPipes::new(true).apply(text, line_start));
        assert!(!HamlPipes::new(true).apply(text, text.find("p x").unwrap()));
        let line = &text[line_start..text.find("\n    c").unwrap()];
        let (r, call) = render_calls_in(text, line_start, line, &HamlPipes::new(true), LATEST, |_| true).remove(0);
        assert_eq!(passed_locals(&call, &r), Some(vec!["a", "d"]));
        let outside = ":ruby\n  x = 1\n= render \"card\", a: b, |\n    d: 1 |\np x";
        let line_start = outside.find("= render").unwrap();
        let (r, call) =
            render_calls_in(outside, line_start, "= render \"card\", a: b, |", &HamlPipes::new(true), LATEST, |_| true).remove(0);
        assert_eq!(passed_locals(&call, &r), Some(vec!["a", "d"]));
        // A `:symbol` line above (attribute hash continuation) is no filter.
        let hash = "%section\n  %div{ :class => \"x\",\n  :id => \"y\" }\n    = render \"card\", a: 1, |\n      b: 2 |\n";
        let line_start = hash.find("    = render").unwrap();
        let line = "    = render \"card\", a: 1, |";
        let (r, call) = render_calls_in(hash, line_start, line, &HamlPipes::new(true), LATEST, |_| true).remove(0);
        assert_eq!(passed_locals(&call, &r), Some(vec!["a", "b"]));
    }

    #[test]
    fn haml_structure_pass_runs_once_per_document_and_only_for_pipe_lines() {
        let passes = || render_call::STRUCTURE_PASSES.with(|passes| passes.get());
        let render_all = |text: &str| {
            let pipes = HamlPipes::new(true);
            for (start, line) in crate::util::lines_with_offsets(text) {
                render_calls_in(text, start, line, &pipes, LATEST, |_| true);
            }
        };
        let before = passes();
        render_all(&"= render \"card\", a: 1\n".repeat(50));
        assert_eq!(passes(), before);
        render_all(&"= render \"card\", a: 1, |\n  b: 2 |\n".repeat(50));
        assert_eq!(passes(), before + 1);
    }

    #[test]
    fn pipes_outside_a_haml_group_stay_ruby() {
        // Bitwise or at a line end in ERB, also when the tag opened on an earlier line.
        assert_eq!(passed_in("<%= render \"card\", a: x |\n y, b: 1 %>"), strs(&["a", "b"]));
        let text = "<%=\n  render \"card\", a: x |\n  y, b: 1 %>";
        let (line_start, line) = (4, "  render \"card\", a: x |");
        let (r, call) = render_calls_in(text, line_start, line, &HamlPipes::new(false), LATEST, |_| true).remove(0);
        assert_eq!(passed_locals(&call, &r), Some(vec!["a", "b"]));
        assert_eq!(passed_in("<%= render \"card\", a: 1 ||\n  b %>"), strs(&["a"]));
        // The same in a Ruby file.
        assert_eq!(passed_in("  render \"card\", a: x |\n  y, b: 1"), strs(&["a", "b"]));
        assert_eq!(passed_in("  render \"card\", a: h.map { |k, v |\n v }, b: 1"), strs(&["a", "b"]));
        // A block param list is not a marker.
        assert_eq!(passed_in_haml("= render \"card\", a: items.map do |x|\n x\nend\np x"), strs(&["a"]));
        // A Slim text line after a call is not part of it.
        assert_eq!(passed_in("= render \"card\", a: 1\n| b |\np x"), strs(&["a"]));
        // The group ends at the first plain line, even if text there ends in a pipe.
        assert_eq!(passed_in_haml("= render \"card\", a: 1 |\n  b: 2\np x |"), strs(&["a"]));
    }

    #[test]
    fn jbuilder_bitwise_or_at_line_end_stays_ruby() {
        assert_eq!(passed_in("json.partial! \"card\", locals: { b: c +\n d }"), strs(&["b"]));
        assert_eq!(passed_in("json.partial! \"card\", locals: { b: c |\n d }"), strs(&["b"]));
    }

    #[test]
    fn erb_completion_keeps_a_line_end_pipe_as_ruby() {
        let text = "<%= render \"card\", z|a: x |\n y, b: 1 %>";
        let col = text.find('|').unwrap();
        let text = text.replacen('|', "", 1);
        let passed = local_key_before(&text, col, false, || LATEST).map(|(_, passed, _)| passed.join(" "));
        assert_eq!(passed.as_deref(), Some("b"));
    }

    #[test]
    fn haml_completion_inside_a_pipe_group() {
        let complete = |text: &str| {
            let col = text.find('^').unwrap();
            let text = text.replacen('^', "", 1);
            local_key_before(&text, col, true, || LATEST).map(|(_, passed, typed)| (passed.join(" "), typed.to_string()))
        };
        let expect = |passed: &str, typed: &str| Some((passed.to_string(), typed.to_string()));
        assert_eq!(complete("= render \"card\", a: 1, |\n  ^ |\np x"), expect("a", ""));
        assert_eq!(complete("= render \"card\", a: 1, |\n  b: 2, |\n  z^ |\np x"), expect("a b", "z"));
        assert_eq!(complete("= render \"card\", |\n  ^ |\np x"), expect("", ""));
        assert_eq!(complete("= render \"card\", a: 1, |\r\n  é^ |\r\np x"), expect("a", "é"));
    }

    #[test]
    fn haml_pipe_multiline_with_crlf_and_multibyte() {
        let text = "= render \"card\", a: 1, |\r\n  b: 2 |\r\np x";
        assert_eq!(passed_in_haml(text), strs(&["a", "b"]));
        assert_eq!(added_for(text, "c", true).as_deref(), Some("= render \"card\", a: 1, |\r\n  b: 2, c: nil |\r\np x"));
        let text = "= render \"card\", a: \"é日\", |\n  b: \"日\" |\np x";
        assert_eq!(added_for(text, "c", true).as_deref(), Some("= render \"card\", a: \"é日\", |\n  b: \"日\", c: nil |\np x"));
    }

    #[test]
    fn local_edits_keep_haml_pipe_markers_valid() {
        let remove = |text: &str, name: &str| {
            let r = &render_refs(text.lines().next().unwrap())[0];
            let (range, new) = remove_local_edit(&read_call(text, r, true), r, name)?;
            let mut out = text.to_string();
            out.replace_range(range, &new);
            Some(out)
        };
        let text = "= render \"card\", a: 1, |\n  b: 2, |\n  c: 3 |\np x";
        assert_eq!(added_for(text, "d", true).as_deref(), Some("= render \"card\", a: 1, |\n  b: 2, |\n  c: 3, d: nil |\np x"));
        assert_eq!(remove(text, "c").as_deref(), Some("= render \"card\", a: 1, |\n  b: 2 |\np x"));
        assert_eq!(remove(text, "b").as_deref(), Some("= render \"card\", a: 1, |\n  c: 3 |\np x"));
        assert_eq!(remove(text, "a").as_deref(), Some("= render \"card\", |\n  b: 2, |\n  c: 3 |\np x"));
        let text = "= render \"card\", title: |\n  \"Hi\", b: 1 |\np x";
        assert_eq!(added_for(text, "c", true).as_deref(), Some("= render \"card\", title: |\n  \"Hi\", b: 1, c: nil |\np x"));
        assert_eq!(remove(text, "b").as_deref(), Some("= render \"card\", title: |\n  \"Hi\" |\np x"));
        let text = "= render \"card\", title: |\n  \"Hi\" |\np x";
        assert_eq!(added_for(text, "c", true).as_deref(), Some("= render \"card\", title: |\n  \"Hi\", c: nil |\np x"));
        let text = "= render \"card\", a: x |\n  + y, b: 1 |\np x";
        assert_eq!(added_for(text, "c", true).as_deref(), Some("= render \"card\", a: x |\n  + y, b: 1, c: nil |\np x"));
        assert_eq!(remove(text, "b").as_deref(), Some("= render \"card\", a: x |\n  + y |\np x"));
    }

    #[test]
    fn jbuilder_calls_read_their_own_arguments() {
        assert_eq!(passed_in(r#"json.array! @x, partial: "a/b", locals: { z: 1 }"#), strs(&["z"]));
        assert_eq!(passed_in(r#"json.array! @x, partial: "a/b", as: :x"#), None);
        assert_eq!(passed_in(r#"json.partial! "a/b", z: 1; json.c @c, partial: "c/d", y: 2"#), strs(&["z"]));
        let line = r#"json.partial! "a/b", z: 1; json.c @c, partial: "c/d", locals: { y: 2 }"#;
        let refs = render_refs(line);
        assert_eq!(refs.len(), 2);
        assert_eq!(passed_locals(&read_call(line, &refs[1], false), &refs[1]), Some(vec!["y"]));
        assert_eq!(passed_locals(&read_call(line, &refs[0], false), &refs[0]), Some(vec!["z"]));
    }

    #[test]
    fn heredoc_argument_does_not_hide_later_locals() {
        assert_eq!(passed_in("<%= render \"card\", body: <<~EOS, b: 1\n  hi\nEOS\n%>"), strs(&["body", "b"]));
    }

    #[test]
    fn render_inside_a_call_with_a_block_opened_in_the_same_tag() {
        let text = "<%= link_to render(\"icon\", a: 1), path do %>\n  x\n<% end %>";
        assert_eq!(passed_in(text), strs(&["a"]));
        let r = &render_refs(text.lines().next().unwrap())[0];
        assert!(add_locals_edit(&read_call(text, r, false), r, &["b"]).is_some());
    }

    #[test]
    fn a_line_ending_in_a_ternary_operator_continues_the_call() {
        let text = "= render \"card\", a: 1, b: x ?\n  1 : 2, c: 2\np next";
        assert_eq!(passed_in(text), strs(&["a", "b", "c"]));
    }

    #[test]
    fn call_followed_by_a_method_chain_or_in_a_ternary() {
        assert_eq!(passed_in(r#"<%= render("card", a: 1).html_safe %>"#), strs(&["a"]));
        assert_eq!(passed_in(r#"<%= show ? render("card", a: 1) : "" %>"#), strs(&["a"]));
        assert_eq!(passed_in(r#"<%= render("card", a: 1).to_s + render("other", b: 2) %>"#), strs(&["a"]));
    }

    #[test]
    fn symbol_keys_with_quotes_and_percent_values() {
        let line = r#"<%= render "card", "data-x": 1, w: %w(a b), i: %i[c d] %>"#;
        let r = &render_refs(line)[0];
        let call = read_call(line, r, false);
        let (at, end) = local_key_span(&call, "data-x").unwrap();
        assert_eq!(&line[at..end], "data-x");
        assert_eq!(passed_locals(&call, r), Some(vec!["data-x", "w", "i"]));
    }

    #[test]
    fn string_keys_and_splats_make_the_call_unreadable() {
        for line in [
            r#"render "card", a: 1, "b" => 2"#,
            r#"render "card", **opts, a: 1"#,
            r#"render partial: "card", locals: { "a" => 1 }"#,
            r#"render partial: "card", locals: { **o }"#,
        ] {
            let r = &render_refs(line)[0];
            let call = read_call(line, r, false);
            assert_eq!(passed_locals(&call, r), None, "{line}");
            assert_eq!(remove_local_edit(&call, r, "a"), None, "{line}");
        }
    }

    #[test]
    fn call_at_the_max_lines_boundary() {
        let call = |lines: usize| {
            let middle = "  a: 1,\n".repeat(lines - 2);
            format!("= render \"card\",\n{middle}  z: 1\np next\n")
        };
        let names = passed_in(&call(MAX_CALL_LINES)).unwrap();
        assert_eq!(names.len(), MAX_CALL_LINES - 1);
        assert_eq!(passed_in(&call(MAX_CALL_LINES + 1)), None, "one line past the window");
    }

    #[test]
    fn multibyte_values_keep_edit_spans_on_byte_offsets() {
        let apply = |line: &str, (range, new): ByteEdit| {
            let mut out = line.to_string();
            out.replace_range(range, &new);
            out
        };
        let line = r#"<%= render "café", título: "é😀", colour: "日本", size: 2 %>"#;
        let r = &render_refs(line)[0];
        let call = read_call(line, r, false);
        assert_eq!(apply(line, remove_local_edit(&call, r, "colour").unwrap()), r#"<%= render "café", título: "é😀", size: 2 %>"#);
        assert_eq!(apply(line, remove_local_edit(&call, r, "título").unwrap()), r#"<%= render "café", colour: "日本", size: 2 %>"#);
        assert_eq!(
            apply(line, add_locals_edit(&call, r, &["x"]).unwrap()),
            r#"<%= render "café", título: "é😀", colour: "日本", size: 2, x: nil %>"#
        );
        let (at, end) = local_key_span(&call, "size").unwrap();
        assert_eq!(&line[at..end], "size");
        let line = r#"= render partial: "café", locals: { t: "日本" }"#;
        let r = &render_refs(line)[0];
        assert_eq!(
            apply(line, add_locals_edit(&read_call(line, r, false), r, &["x"]).unwrap()),
            r#"= render partial: "café", locals: { t: "日本", x: nil }"#
        );
    }

    #[test]
    fn a_comment_in_the_call_blocks_adding_but_a_hash_in_a_string_does_not() {
        let text = "<%= render \"card\",\n  a: 1, # note\n  b: 2 %>";
        let r = &render_refs(text.lines().next().unwrap())[0];
        let call = read_call(text, r, false);
        assert_eq!(passed_locals(&call, r), Some(vec!["a", "b"]));
        assert_eq!(add_locals_edit(&call, r, &["c"]), None);
        assert!(remove_local_edit(&call, r, "b").is_some());
        for line in [r##"<%= render "card", a: "#1", b: "#{x}" %>"##, r#"<%= render "card", a: ?# %>"#] {
            assert!(added(line, "c").is_some(), "{line}");
        }
    }

    #[test]
    fn half_typed_local_keys() {
        let key = |line: &str| {
            let col = line.find('|').unwrap();
            let line = line.replace('|', "");
            local_key_before(&line, col, false, || LATEST).map(|(_, passed, typed)| (passed.join(" "), typed.to_string()))
        };
        let at = |passed: &str, typed: &str| Some((passed.to_string(), typed.to_string()));
        assert_eq!(key(r#"<%= render "card", a| %>"#), at("", "a"));
        assert_eq!(key(r#"<%= render "card", a: 1, b| %>"#), at("a", "b"));
        assert_eq!(key(r#"<%= render partial: "card", locals: { a| } %>"#), at("", "a"));
        assert_eq!(key(r#"<%= render partial: "card", locals: { a: 1, b|"#), at("a", "b"));
        assert_eq!(key(r#"<%= render partial: "card", locals: { a: 1 }, b| %>"#), None, "outside the locals hash");
        assert_eq!(key(r#"<%= render "card" a| %>"#), None, "no comma after the name");
        assert_eq!(key(r#"<%= render "card", title: "a, | %>"#), None, "inside a string");
        assert_eq!(key(r#"<%= render "café", título: 1, tít| %>"#), at("título", "tít"));
        assert_eq!(key(r#"<%= render "card" , | %>"#), at("", ""));
        assert_eq!(key(r#"<%= render "card", a: 1, | , b: 2 %>"#), at("a b", ""));
        assert_eq!(key(r#"<%= render("card", a: 1,| b: 2) %>"#), at("a b", ""));
        assert_eq!(key(r#"<%= render partial: "card", locals: { |, user: u } %>"#), at("user", ""));
        assert_eq!(key(r#"<%= render partial: "card", locals: { a: 1, | user: u } %>"#), at("a user", ""));
        assert_eq!(key(r#"<%= render "card", a| , b: 2 %>"#), at("b", "a"));
        assert_eq!(key(r#"<%= render "card", a|bc, b: 2 %>"#), at("b", "a"), "the cursor is inside a word");
        assert_eq!(key("= render \"card\", a: 1, |\n  b: 2"), at("a b", ""), "a later line");
        assert_eq!(key(r#"<%= render "card", a: 1, | b: (2 %>"#), at("a", ""), "a broken rest keeps what comes before");
        assert_eq!(key(r#"<%= render "card", a: 1, | %> <%= render "x", b: 1 %>"#), at("a", ""), "another tag's call");
        assert_eq!(key(r#"<%= render "card", "a" => 1, b| %>"#), None, "string keys are not locals");
    }

    #[test]
    fn local_keys_after_the_cursor_ignore_what_is_not_the_call() {
        let key = |line: &str| {
            let col = line.find('|').unwrap();
            let line = line.replace('|', "");
            local_key_before(&line, col, false, || LATEST).map(|(_, passed, typed)| (passed.join(" "), typed.to_string()))
        };
        let at = |passed: &str, typed: &str| Some((passed.to_string(), typed.to_string()));
        assert_eq!(key("<%= render \"a\", | %>\n<%= render \"a\", b: 1 %>"), at("", ""), "the next tag's call");
        assert_eq!(key("= render \"a\", |\n= render \"a\", b: 1"), at("", ""), "the next line's call");
        assert_eq!(key("= render \"a\", |\n= render \"a\", cursor_key__: 1"), at("", ""), "a probe-named key is never offered");
        assert_eq!(key("= render \"a\", x: 1, |\n%p text"), at("x", ""), "Haml markup on the next line");
        assert_eq!(key("= render \"a\", x: 1, |\np text"), at("x", ""), "Slim markup on the next line");
        assert_eq!(key(r#"<%= render "a", | b: "}),", c: 2 %>"#), at("b c", ""), "punctuation in a string");
        assert_eq!(key(r#"<%= render("a", x: 1, |)"#), at("x", ""), "right before the closing parenthesis");
        assert_eq!(key(r#"<%= render partial: "a", locals: { | } %>"#), at("", ""), "an empty locals hash");
        assert_eq!(key(r#"<%= render partial: "a", locals: {|"#), at("", ""), "an open locals hash");
        assert_eq!(key(r#"<%= render "a", us|er: u %>"#), at("", "us"), "inside an existing key");
    }

    #[test]
    fn local_keys_after_the_cursor_follow_the_next_key() {
        let key = |line: &str| {
            let col = line.find('|').unwrap();
            let line = line.replace('|', "");
            local_key_before(&line, col, false, || LATEST).map(|(_, passed, typed)| (passed.join(" "), typed.to_string()))
        };
        let at = |passed: &str, typed: &str| Some((passed.to_string(), typed.to_string()));
        assert_eq!(key(r#"<%= render "a", |b: 2, c: 3 %>"#), at("b c", ""), "the next key right at the cursor");
        assert_eq!(key(r#"<%= render "a", x: 1, a|bc: 1, c: 2 %>"#), at("x c", "a"), "inside a key: not counted, the rest is");
        assert_eq!(key(r#"<%= render "a", x: 1, | :b => 2 %>"#), at("x b", ""), "a symbol key");
        assert_eq!(key(r#"<%= render "a", x: 1, | "b": 2 %>"#), at("x b", ""), "a quoted key");
    }

    #[test]
    fn jbuilder_probe_beside_locals_hash_is_not_a_local_position() {
        let line = r#"json.partial! "a", |, locals: { b: 1 }"#;
        let col = line.find('|').unwrap();
        let line = line.replace('|', "");
        let names = local_key_before(&line, col, false, || LATEST).map(|(_, passed, _)| passed);
        assert_eq!(names, None, "top-level keys are ignored when locals: is given");
        let key = |line: &str| {
            let col = line.find('|').unwrap();
            local_key_before(&line.replace('|', ""), col, false, || LATEST).map(|(_, passed, _)| passed.join(" "))
        };
        assert_eq!(key(r#"<%= render partial: "a", locals: { x: 1, | }, collection: c %>"#), None, "collection: after the cursor");
        assert_eq!(key(r#"<%= render partial: "a", collection: c, locals: { x: 1, | } %>"#), None);
        assert_eq!(key(r#"<%= render "card", a: 1, a|: 2 %>"#).as_deref(), Some("a"), "a duplicate of the edited key is passed");
    }

    #[test]
    fn local_key_before_stops_at_the_render_it_found() {
        let markup = "%p x\n".repeat(25);
        let text = format!("{markup}= render \"a\", x: \"open, |\n{}", "  y: 2,\n".repeat(25));
        let offset = text.find('|').unwrap();
        let text = text.replace('|', "");
        let before = render_call::PARSES.with(|parses| parses.get());
        local_key_before(&text, offset, false, || LATEST);
        let parses = render_call::PARSES.with(|parses| parses.get()) - before;
        assert!(parses <= 100, "{parses} parses for one keystroke");
    }

    #[test]
    fn half_typed_local_key_costs_few_parses() {
        let line = r#"<%= render "card", a: 1, b: 2, c , d: 3 %>"#;
        let before = render_call::PARSES.with(|parses| parses.get());
        local_key_before(line, line.find(" ,").unwrap(), false, || LATEST).unwrap();
        let parses = render_call::PARSES.with(|parses| parses.get()) - before;
        assert!(parses <= 6, "{parses} parses for one keystroke");
    }

    #[test]
    fn plain_renders_and_invalid_calls_cost_few_parses() {
        let parses = |run: &dyn Fn()| {
            render_call::PARSES.with(|count| count.set(0));
            run();
            render_call::PARSES.with(|count| count.get())
        };
        assert_eq!(parses(&|| drop(render_refs(r#"<%= render "card", user: u %>"#))), 0, "no lookup options to read");
        let invalid = format!("= render \"card\", a: (1,), b: 2\n{}", "%p x\n".repeat(19));
        let r = &render_refs(invalid.lines().next().unwrap())[0];
        assert!(parses(&|| drop(read_call(&invalid, r, false))) <= 2, "later lines can't fix an invalid call");
    }

    #[test]
    fn locals_and_edits_reuse_the_call_read_with_the_refs() {
        let text = "<%= render \"card\", a: 1, b: 2 %>\n";
        let before = render_call::PARSES.with(|parses| parses.get());
        let (r, call) = render_calls_in(text, 0, text.lines().next().unwrap(), &HamlPipes::new(false), LATEST, |_| true).remove(0);
        assert_eq!(passed_locals(&call, &r), Some(vec!["a", "b"]));
        assert!(add_locals_edit(&call, &r, &["c"]).is_some());
        assert!(remove_local_edit(&call, &r, "a").is_some());
        assert!(local_key_span(&call, "b").is_some());
        assert_eq!(render_call::PARSES.with(|parses| parses.get()) - before, 1, "one Prism read for the render");
    }

    #[test]
    fn checking_a_long_file_of_unclosed_renders_stays_fast() {
        let text = "<%= render \"card\", a: (\n".repeat(2000);
        let lines = text.lines().count();
        let before = render_call::PARSES.with(|parses| parses.get());
        let mut line_start = 0;
        for line in text.lines() {
            render_calls_in(&text, line_start, line, &HamlPipes::new(false), LATEST, |_| true);
            line_start += line.len() + 1;
        }
        let parses = render_call::PARSES.with(|parses| parses.get()) - before;
        // A call is read from at most MAX_CALL_LINES lines; each is parsed, then with each of two closers (and the call alone).
        assert!(parses <= lines * MAX_CALL_LINES * 4, "{parses} parses for {lines} lines");
    }
}
