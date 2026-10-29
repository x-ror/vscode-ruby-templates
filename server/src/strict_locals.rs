//! Rails' strict locals (7.1+): the `locals: (…)` magic comment it cuts out of any template's source and turns
//! into the parameters of the method it evals, whatever the handler.

use crate::prism;
use crate::project::Project;
use crate::util::{error, line_text_range, offset_to_position};
use regex::Regex;
use std::collections::HashSet;
use std::ops::Range as ByteRange;
use std::sync::LazyLock;
use tower_lsp::lsp_types::{Diagnostic, Range};

/// Ruby's `\s`, which `STRICT_LOCALS_REGEX` is written in.
const RUBY_SPACE: [char; 6] = [' ', '\t', '\r', '\n', '\x0C', '\x0B'];
/// `local_assigns` and `output_buffer`, which every template method takes before the locals.
const WRAPPER_PARAMETERS: &str = "local_assigns, output_buffer, ";
const WRAPPER_PARAMETER_COUNT: usize = 2;
const LAZY_REGEXP_SINCE: &str = "8.1";
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

impl Close {
    /// The latest regexp unless the project's lockfile names an older actionview.
    pub fn of(project: &Project) -> Self {
        match project.gem_version("actionview") {
            Some(version) if !version.at_least(LAZY_REGEXP_SINCE) => Close::LastOnLine,
            _ => Close::BeforeTagOrLineEnd,
        }
    }
}

/// Strict locals as a project's Rails has them: where the comment ends, and the template's name in Rails' messages.
pub struct Rails {
    pub close: Close,
    pub identifier: String,
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

/// The parameter list of the method Rails compiles a template into.
pub fn method_parameters(comment: Option<&Comment>) -> String {
    match comment {
        Some(comment) if comment.parameters.contains('&') => format!("{WRAPPER_PARAMETERS}{}", comment.parameters),
        Some(comment) => format!("{WRAPPER_PARAMETERS}{}, &_", comment.parameters),
        None => format!("{WRAPPER_PARAMETERS}&_"),
    }
}

impl Comment {
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
    use tower_lsp::lsp_types::Position;

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
}
