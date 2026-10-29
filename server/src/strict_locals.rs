//! Rails' strict locals (7.1+): the `locals: (…)` magic comment it cuts out of any template's source and turns
//! into the parameters of the method it evals, whatever the handler.

use regex::Regex;
use std::ops::Range as ByteRange;
use std::sync::LazyLock;

/// Ruby's `\s`, which `STRICT_LOCALS_REGEX` is written in.
const RUBY_SPACE: [char; 6] = [' ', '\t', '\r', '\n', '\x0C', '\x0B'];
/// `local_assigns` and `output_buffer`, which every template method takes first.
const WRAPPER_PARAMETERS: usize = 2;

// The start of `ActionView::Template::STRICT_LOCALS_REGEX`; Rust's regexps have no lookahead for the rest.
static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#[ \t\r\n\x0C\x0B]+locals:[ \t\r\n\x0C\x0B]+\(").unwrap());

/// The `locals: (…)` comment Rails cuts out of the source, and the parameters it holds.
pub struct Comment {
    pub range: ByteRange<usize>,
    pub parameters: String,
}

/// `STRICT_LOCALS_REGEX`'s first match: `# locals: (` up to the first `)` that a tag end or a line end follows.
pub fn find(text: &str) -> Option<Comment> {
    let mut from = 0;
    while let Some(head) = HEAD.find_at(text, from) {
        let closes_comment = |close: &usize| {
            let rest = &text[close + 1..];
            let after_tag_space = rest.trim_start_matches(RUBY_SPACE);
            let after_line_space = rest.trim_start_matches([' ', '\t', '\r', '\x0C', '\x0B']);
            after_tag_space.strip_prefix('-').unwrap_or(after_tag_space).starts_with("%>")
                || after_line_space.is_empty()
                || after_line_space.starts_with('\n')
        };
        let close = text[head.end()..].match_indices(')').map(|(i, _)| head.end() + i).find(closes_comment);
        if let Some(close) = close {
            let parameters = text[head.end()..close].trim_end_matches([' ', '\t', '\n', '\x0B', '\x0C', '\r', '\0']);
            // ActiveSupport's `blank?` counts Unicode whitespace, which `rstrip` leaves.
            let parameters = if parameters.chars().all(char::is_whitespace) { "**nil" } else { parameters };
            return Some(Comment { range: head.start()..close + 1, parameters: parameters.to_string() });
        }
        from = head.start() + 1;
    }
    None
}

/// The parameter list of the method Rails compiles a template into.
pub fn method_parameters(comment: Option<&Comment>) -> String {
    match comment {
        Some(comment) if comment.parameters.contains('&') => format!("local_assigns, output_buffer, {}", comment.parameters),
        Some(comment) => format!("local_assigns, output_buffer, {}, &_", comment.parameters),
        None => "local_assigns, output_buffer, &_".to_string(),
    }
}

/// Rails' `ArgumentError` for the parameters of the compiled method that aren't keywords, named for `identifier`.
pub fn non_keyword_message(positional: Vec<String>, block: Option<String>, identifier: &str) -> Option<String> {
    // The wrapper's own `&_` is no one's mistake.
    let non_keyword: Vec<String> = positional.into_iter().skip(WRAPPER_PARAMETERS).chain(block.filter(|name| name != "_")).collect();
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

/// The 0-based line `comment` starts on.
pub fn line(text: &str, comment: &Comment) -> usize {
    text[..comment.range.start].matches('\n').count()
}
