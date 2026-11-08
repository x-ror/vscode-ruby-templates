//! Rails' strict locals (7.1+): the `locals: (…)` magic comment it cuts out of any template's source and turns
//! into the parameters of the method it evals, whatever the handler.

use crate::ruby::prism;
use crate::util::{error, line_text_range, offset_to_position};
use regex::Regex;
use std::collections::HashSet;
use std::ops::Range as ByteRange;
use std::path::Path;
use std::sync::LazyLock;
use tower_lsp_server::ls_types::{Diagnostic, Range};

/// Ruby's `\s`, which `STRICT_LOCALS_REGEX` is written in.
const RUBY_SPACE: [char; 6] = [' ', '\t', '\r', '\n', '\x0C', '\x0B'];
/// `local_assigns` and `output_buffer`, which every template method takes before the locals.
const WRAPPER_PARAMETERS: &str = "local_assigns, output_buffer, ";
const WRAPPER_PARAMETER_COUNT: usize = 2;
/// The first actionview with strict locals; before, the `locals:` line is a plain comment.
pub(crate) const SINCE: &str = "7.1";
/// The first actionview whose `STRICT_LOCALS_REGEX` is `Close::BeforeTagOrLineEnd`'s.
pub(crate) const LAZY_REGEXP_SINCE: &str = "8.1";
/// The method Haml, Slim and Ruby views' comment is checked in: only its parameters come from the template.
const SIGNATURE_DEF: &str = "def _template(";

// The start of `ActionView::Template::STRICT_LOCALS_REGEX`; Rust's regexps have no lookahead for the rest.
static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#[ \t\r\n\x0C\x0B]+locals:[ \t\r\n\x0C\x0B]+\(").unwrap());

/// Which `)` ends the comment, as the project's `STRICT_LOCALS_REGEX` picks it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Close {
    /// actionview 7.1 to 8.0, `\((.*)\)`: the last `)` on the line.
    LastOnLine,
    /// actionview 8.1+, `\((.*?)\)(?=\s*-?%>|\s*$)/m`: the first `)` a tag end or a line end follows.
    #[default]
    BeforeTagOrLineEnd,
}

/// Strict locals as a project's Rails has them: where the comment ends, and the template's name in Rails' messages.
pub struct Rails {
    pub close: Close,
    pub identifier: String,
}

/// How Rails evals a template: the Prism grammar of its project's Ruby, and strict locals where they exist (Rails 7.1
/// or newer). Without them the `locals:` line is a plain comment.
pub(crate) struct RubyContext {
    pub grammar: ruby_prism::Version,
    pub strict_locals: Option<Rails>,
}

impl RubyContext {
    pub(crate) fn outside_project() -> Self {
        RubyContext { grammar: ruby_prism::Version::Latest, strict_locals: None }
    }
}

/// The `locals: (…)` comment Rails cuts out of the source, and the parameters it holds.
pub struct Comment {
    pub range: ByteRange<usize>,
    pub parameters: String,
    /// Where `parameters` starts in the text; none when Rails put `**nil` for a blank list.
    parameters_at: Option<usize>,
    /// Where `locals:` is, which may be on a line after the `#`.
    keyword_at: usize,
}

/// `STRICT_LOCALS_REGEX`'s first match.
pub fn find(text: &str, close: Close) -> Option<Comment> {
    let mut from = 0;
    while let Some(head) = HEAD.find_at(text, from) {
        let after = &text[head.end()..];
        let end = match close {
            Close::LastOnLine => after[..after.find('\n').unwrap_or(after.len())].rfind(')'),
            Close::BeforeTagOrLineEnd => after.match_indices(')').map(|(i, _)| i).find(|&i| closes_comment(&after[i + 1..])),
        };
        if let Some(end) = end.map(|i| head.end() + i) {
            let parameters = text[head.end()..end].trim_end_matches([' ', '\t', '\n', '\x0B', '\x0C', '\r', '\0']);
            // ActiveSupport's `blank?` counts Unicode whitespace, which `rstrip` leaves.
            let blank = parameters.chars().all(char::is_whitespace);
            return Some(Comment {
                range: head.start()..end + 1,
                parameters: if blank { "**nil" } else { parameters }.to_string(),
                parameters_at: (!blank).then_some(head.end()),
                keyword_at: head.start() + head.as_str().find("locals:").unwrap_or(0),
            });
        }
        from = head.start() + 1;
    }
    None
}

/// The lookahead of 8.1's regexp: a tag end or a line end after the `)`.
fn closes_comment(rest: &str) -> bool {
    let after_tag_space = rest.trim_start_matches(RUBY_SPACE);
    let after_line_space = rest.trim_start_matches([' ', '\t', '\r', '\x0C', '\x0B']);
    after_tag_space.strip_prefix('-').unwrap_or(after_tag_space).starts_with("%>")
        || after_line_space.is_empty()
        || after_line_space.starts_with('\n')
}

/// The parameters a template's `locals: (…)` comment declares.
#[derive(Debug, PartialEq)]
pub struct Signature {
    /// The parameter list as written, without the parentheses.
    pub params: String,
    /// `(name, default)`; no default means required.
    pub locals: Vec<(String, Option<String>)>,
    /// `**rest` accepts any other local.
    pub rest: bool,
}

impl Signature {
    pub fn declares(&self, name: &str) -> bool {
        self.locals.iter().any(|(local, _)| local == name)
    }

    /// The required locals `passed` leaves out.
    pub fn missing(&self, passed: &[&str]) -> Vec<&str> {
        self.locals
            .iter()
            .filter(|(name, default)| default.is_none() && !passed.contains(&name.as_str()))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// The locals in `passed` the signature doesn't take, each once.
    pub fn unknown<'a>(&self, passed: &[&'a str]) -> Vec<&'a str> {
        let mut seen = HashSet::new();
        passed.iter().copied().filter(|name| seen.insert(*name) && !self.rest && !self.declares(name)).collect()
    }

    pub fn takes_no_locals(&self) -> bool {
        self.locals.is_empty() && !self.rest
    }
}

/// The required locals `passed` leaves out of every one of `candidates`, the templates a render may pick from.
pub fn missing_in_all<'a>(candidates: &[&'a Signature], passed: &[&str]) -> Vec<&'a str> {
    let Some((first, others)) = candidates.split_first() else { return vec![] };
    let mut missing = first.missing(passed);
    missing.retain(|name| others.iter().all(|sig| sig.missing(passed).contains(name)));
    missing
}

/// The locals in `passed` that none of `candidates` takes.
pub fn unknown_to_all<'a>(candidates: &[&Signature], passed: &[&'a str]) -> Vec<&'a str> {
    let Some((first, others)) = candidates.split_first() else { return vec![] };
    let mut unknown = first.unknown(passed);
    unknown.retain(|name| others.iter().all(|sig| !sig.rest && !sig.declares(name)));
    unknown
}

/// The signature of `text`'s `locals:` comment, if it has one Prism parses.
pub fn signature(text: &str, close: Close, ruby: ruby_prism::Version) -> Option<Signature> {
    find(text, close)?.signature(ruby)
}

pub fn file_signature(file: &Path, close: Close, ruby: ruby_prism::Version) -> Option<Signature> {
    signature(&std::fs::read_to_string(file).ok()?, close, ruby)
}

/// The parameter list of the method Rails compiles a template into.
pub fn method_parameters(comment: Option<&Comment>) -> String {
    match comment {
        Some(comment) if comment.parameters.contains('&') => format!("{WRAPPER_PARAMETERS}{}", comment.parameters),
        Some(comment) => format!("{WRAPPER_PARAMETERS}{}, &_", comment.parameters),
        None => format!("{WRAPPER_PARAMETERS}&_"),
    }
}

impl Comment {
    /// The parameters the comment declares, if Prism parses them.
    pub fn signature(&self, ruby: ruby_prism::Version) -> Option<Signature> {
        let params = if self.blank() { "" } else { self.parameters.trim() };
        let (locals, rest) = prism::keyword_parameters(params, ruby)?;
        Some(Signature { params: params.to_string(), locals, rest })
    }

    /// The byte range to delete from the template to drop the parameter `name`, with its comma.
    pub fn undeclare(&self, name: &str, ruby: ruby_prism::Version) -> Option<ByteRange<usize>> {
        let at = self.parameters_at?;
        let start = self.parameter_at(name)? - at;
        let spans = prism::parameter_spans(&self.parameters, ruby)?;
        let index = spans.iter().position(|&(from, _)| from == start)?;
        let (from, to) = match (index.checked_sub(1).map(|i| spans[i]), spans.get(index + 1)) {
            (_, Some(&(next, _))) => (start, next),
            (Some((_, previous_end)), None) => (previous_end, spans[index].1),
            (None, None) => spans[index],
        };
        Some(at + from..at + to)
    }

    /// The edit to the template that declares `name` with `value` as its default: a byte range and its replacement.
    pub fn declare(&self, name: &str, value: &str) -> (ByteRange<usize>, String) {
        match self.parameters_at {
            Some(at) => {
                let params = &self.parameters;
                let last_at = params.rfind(',').map_or(0, |comma| comma + 1);
                let last = &params[last_at..];
                let local = format!("{name}: {value}");
                // Ruby takes no keyword after `**`: the new one goes before, and replaces a lone `**nil`.
                match last.trim() {
                    "**nil" if last_at == 0 => (at..at + params.len(), local),
                    splat if splat.starts_with("**") => {
                        let before = at + last_at + (last.len() - last.trim_start().len());
                        (before..before, format!("{local}, "))
                    }
                    _ => (at + params.len()..at + params.len(), format!(", {local}")),
                }
            }
            None => {
                let close = self.range.end - 1;
                (close..close, format!("{name}: {value}"))
            }
        }
    }

    /// Where the parameter `name` (`name:` in the list) starts in the template.
    pub fn parameter_at(&self, name: &str) -> Option<usize> {
        Some(self.parameters_at? + crate::ruby::prism::keyword_parameter_at(&self.parameters, name)?)
    }

    /// Where the other parameters' defaults read the parameter `name`, in the template.
    pub fn parameter_reads(&self, name: &str) -> Vec<(usize, usize)> {
        let Some(at) = self.parameters_at else { return vec![] };
        crate::ruby::prism::parameter_reads(&self.parameters, name).into_iter().map(|(start, end)| (at + start, at + end)).collect()
    }

    /// Whether the list was blank, which Rails reads as `**nil`.
    pub fn blank(&self) -> bool {
        self.parameters_at.is_none()
    }

    /// The 0-based line of `locals:`.
    pub fn line(&self, text: &str) -> usize {
        text[..self.keyword_at].matches('\n').count()
    }

    /// Where Prism's error at `start..end` of a method source whose parameter list starts at `list_at` is in `text`:
    /// exact inside the locals the comment wrote, none elsewhere.
    pub fn error_range(&self, text: &str, list_at: usize, start: usize, end: usize) -> Option<Range> {
        let at = self.parameters_at?;
        let first = list_at + WRAPPER_PARAMETERS.len();
        let last = first + self.parameters.len();
        if !(first..=last).contains(&start) {
            return None;
        }
        let position = |offset: usize| offset_to_position(text, at + offset.clamp(first, last) - first);
        Some(Range::new(position(start), position(end)))
    }
}

/// Rails' `ArgumentError` for the parameters of the compiled method that aren't keywords, named for `identifier`.
pub fn non_keyword_message(positional: Vec<String>, block: Option<String>, identifier: &str) -> Option<String> {
    // The wrapper's own `&_` is no one's mistake.
    let non_keyword: Vec<String> = positional.into_iter().skip(WRAPPER_PARAMETER_COUNT).chain(block.filter(|name| name != "_")).collect();
    if non_keyword.is_empty() {
        return None;
    }
    Some(format!(
        "{} set as non-keyword {} for {identifier}. Locals can only be set as keyword arguments.",
        to_sentence(&non_keyword),
        if non_keyword.len() == 1 { "argument" } else { "arguments" }
    ))
}

/// ActiveSupport's `Array#to_sentence` of the names in backticks.
fn to_sentence(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.as_slice() {
        [only] => only.clone(),
        [first, second] => format!("{first} and {second}"),
        [init @ .., last] => format!("{}, and {last}", init.join(", ")),
        [] => String::new(),
    }
}

/// Rails' errors for the `locals:` comment alone; `rails` is given only where strict locals exist.
pub fn diagnostics(text: &str, ruby: ruby_prism::Version, rails: Option<&Rails>) -> Vec<Diagnostic> {
    let Some(rails) = rails else { return Vec::new() };
    let Some(comment) = find(text, rails.close) else { return Vec::new() };
    comment_diagnostics(text, &comment, ruby, rails)
}

/// `diagnostics` for the comment `find` found in `text`.
pub fn comment_diagnostics(text: &str, comment: &Comment, ruby: ruby_prism::Version, rails: &Rails) -> Vec<Diagnostic> {
    let line = line_text_range(text, comment.line(text));
    let source = format!("{SIGNATURE_DEF}{})\nend\n", method_parameters(Some(comment)));
    let (errors, (positional, block)) = prism::errors_and_def_parameters(&source, ruby);
    if !errors.is_empty() {
        let mut seen = HashSet::new();
        return errors
            .into_iter()
            .filter(|e| seen.insert(e.message.clone()))
            .map(|e| error(comment.error_range(text, SIGNATURE_DEF.len(), e.start, e.end).unwrap_or(line), e.message))
            .collect();
    }
    non_keyword_message(positional, block, &rails.identifier).map(|message| error(line, message)).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp_server::ls_types::Position;

    const IDENTIFIER: &str = "app/views/users/_card.html.haml";

    fn rails(close: Close) -> Rails {
        Rails { close, identifier: IDENTIFIER.into() }
    }

    fn found(text: &str, close: Close) -> Vec<Diagnostic> {
        diagnostics(text, ruby_prism::Version::Latest, Some(&rails(close)))
    }

    fn messages(text: &str) -> Vec<(u32, String)> {
        found(text, Close::default()).into_iter().map(|d| (d.range.start.line, d.message)).collect()
    }

    #[test]
    fn declare_adds_a_local_to_the_list() {
        let declared = |text: &str| {
            let (range, new) = find(text, Close::BeforeTagOrLineEnd).unwrap().declare("colour", "nil");
            let mut out = text.to_string();
            out.replace_range(range, &new);
            out
        };
        assert_eq!(declared("<%# locals: (user:) %>"), "<%# locals: (user:, colour: nil) %>");
        assert_eq!(declared("<%# locals: () %>"), "<%# locals: (colour: nil) %>");
        assert_eq!(declared("<%# locals: (**nil) %>"), "<%# locals: (colour: nil) %>");
    }

    #[test]
    fn haml_and_slim_locals_must_be_keywords() {
        for comment in ["-#", "/#"] {
            let text = format!("%p\n{comment} locals: (user, size:)\np x\n");
            let expected = format!("`user` set as non-keyword argument for {IDENTIFIER}. Locals can only be set as keyword arguments.");
            assert_eq!(messages(&text), [(1, expected)], "{comment}");
            let found = messages(&format!("{comment} locals: (a, *rest, &block)\n"));
            assert_eq!(found.len(), 1);
            assert!(found[0].1.starts_with("`a`, `rest`, and `block` set as non-keyword arguments"), "{found:?}");
        }
    }

    #[test]
    fn keyword_locals_are_accepted() {
        for signature in ["(a:)", "(a:, b: 1, **rest)", "()", "(a:, &_)"] {
            assert_eq!(messages(&format!("-# locals: {signature}\n%p\n")), [], "{signature}");
        }
    }

    #[test]
    fn a_syntax_error_is_where_the_signature_has_it() {
        let text = "%p\n  -# locals: (a: 1, b: ] , c:)\n";
        let at = text.find(']').unwrap() - text.find("  -#").unwrap();
        let range = found(text, Close::default())[0].range;
        assert_eq!(range.start, Position::new(1, at as u32));
        let multi = "-# locals: (a:,\n  b: ])\n";
        assert!(found(multi, Close::default()).iter().any(|d| d.range.start.line == 1), "{:?}", found(multi, Close::default()));
    }

    #[test]
    fn the_message_is_on_the_line_of_locals() {
        let expected = format!("`user` set as non-keyword argument for {IDENTIFIER}. Locals can only be set as keyword arguments.");
        assert_eq!(messages("-#\n  locals: (user)\n%p\n"), [(1, expected)]);
    }

    #[test]
    fn rails_before_8_1_ends_the_comment_at_the_last_parenthesis_on_its_line() {
        let text = "-# locals: (a:, b:)) x\n";
        assert!(found(text, Close::default()).is_empty(), "8.1: no `)` before the line end, a plain comment");
        assert!(!found(text, Close::LastOnLine).is_empty(), "7.1: `a:, b:)` is the list");
        // 7.1 doesn't cross lines.
        assert!(find("-# locals: (a:,\n  b:)\n", Close::LastOnLine).is_none());
        assert_eq!(find("-# locals: (a:,\n  b:)\n", Close::default()).unwrap().parameters, "a:,\n  b:");
    }

    #[test]
    fn no_check_without_strict_locals_or_a_closed_comment() {
        assert!(diagnostics("-# locals: (a)\n", ruby_prism::Version::Latest, None).is_empty());
        // Rails' regexp needs the `)` at the end of the line; without it the line is a plain comment.
        assert_eq!(messages("-# locals: (a: 1\n%p\n"), []);
    }

    #[test]
    fn signature_follows_the_ruby_version() {
        let text = "<%# locals: (a: (1\n && 2)) %>";
        let signature = |ruby| signature(text, Close::default(), ruby);
        assert!(signature(ruby_prism::Version::Latest).is_some());
        assert!(signature(ruby_prism::Version::CRuby3_3).is_none());
    }

    #[test]
    fn signature_of_the_comment() {
        let sig = signature(
            "<%# locals: (title:, items: [1, 2], label: t(\"a, b\"), **options) -%>\n<p>",
            Close::default(),
            ruby_prism::Version::Latest,
        )
        .unwrap();
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
        let none = signature("<%# locals: () -%>", Close::default(), ruby_prism::Version::Latest).unwrap();
        assert!(none.locals.is_empty() && !none.rest);
        assert_eq!(
            signature("-# locals: (user: nil)", Close::default(), ruby_prism::Version::Latest).unwrap().locals,
            vec![("user".to_string(), Some("nil".to_string()))]
        );
        assert_eq!(
            signature("<%# locals: (a: ')') %>", Close::default(), ruby_prism::Version::Latest).unwrap().locals,
            vec![("a".to_string(), Some("')'".to_string()))]
        );
        let no_rest = signature("<%# locals: (**nil) %>", Close::default(), ruby_prism::Version::Latest).unwrap();
        assert_eq!((no_rest.locals.len(), no_rest.rest), (0, false));
        // Lazy: stops at the `)` that closes the tag, not the last one on the line.
        assert_eq!(signature("<%# locals: (a:) %> <%= f(1) %>", Close::default(), ruby_prism::Version::Latest).unwrap().params, "a:");
        let multi = signature("<%# locals: (\n  a:,\n  b: [1,\n 2]\n) -%>", Close::default(), ruby_prism::Version::Latest).unwrap();
        assert_eq!(multi.locals, vec![("a".to_string(), None), ("b".to_string(), Some("[1,\n 2]".to_string()))]);
        // ActionView needs whitespace before the parenthesis.
        assert!(signature("<%# locals:(a:) %>", Close::default(), ruby_prism::Version::Latest).is_none());
        assert!(signature("<%# just a comment %>", Close::default(), ruby_prism::Version::Latest).is_none());
    }

    #[test]
    fn missing_and_unknown_locals() {
        let sig = signature("<%# locals: (title:, size: 1) %>", Close::default(), ruby_prism::Version::Latest).unwrap();
        assert!(sig.declares("size") && !sig.declares("colour"));
        assert_eq!(sig.missing(&["size"]), ["title"]);
        assert_eq!(sig.unknown(&["colour", "title", "colour"]), ["colour"]);
        assert!(!sig.takes_no_locals());
    }
}
