//! Reads a `render` call's arguments with Prism, from the call's text in a template.

use ruby_prism::{ArgumentsNode, AssocNode, CallNode, InterpolatedStringNode, Location, StringNode, Visit};
use std::ops::Range;

/// How many lines up a multi-line render call may start from the cursor, and how many lines it may span.
pub const MAX_CALL_LINES: usize = 20;

/// A `key: value` or `:key => value` argument; offsets are into the text the call was read from.
pub struct Pair<'a> {
    pub key: &'a str,
    pub key_span: Range<usize>,
    pub whole: Range<usize>,
    pub value: &'a str,
    /// The `{…}` value of `locals`, the only key whose hash is read.
    pub hash: Option<Hash<'a>>,
}

pub struct Hash<'a> {
    /// Between the braces.
    pub inner: Range<usize>,
    /// `None` when an item isn't a literal `key: value`.
    pub items: Option<Vec<Pair<'a>>>,
}

pub struct Call<'a> {
    /// Includes heredoc bodies, and stops at the code that was parsed.
    pub end: usize,
    /// The keyword arguments, but the one that holds the name; those with a non-literal key are left out.
    pub pairs: Vec<Pair<'a>>,
    /// Every argument after the name is such a keyword argument.
    pub readable: bool,
    /// A Ruby comment sits in the call's text.
    pub commented: bool,
}

/// Where the call text starting at `call_at` in `text` may end at most: an ERB `%>` or `MAX_CALL_LINES` lines.
pub fn window_end(text: &str, call_at: usize) -> usize {
    let end = text[call_at..].match_indices('\n').nth(MAX_CALL_LINES - 1).map_or(text.len(), |(at, _)| call_at + at);
    text[call_at..end].find("%>").map_or(end, |at| call_at + at)
}

/// The `render` or `json.…` call that starts at `call_at` in `text` and holds the name at `name_at`, read from the fewest
/// lines that make Ruby; `None` when they don't (an unclosed call, or one that isn't the name's).
pub fn read(text: &str, call_at: usize, name_at: usize) -> Option<Call<'_>> {
    let end = window_end(text, call_at);
    let window = text[call_at..end].trim_end();
    // `-%>` and `=%>` trim the template's whitespace; they are not Ruby.
    let closed_by_tag = text[end..].starts_with("%>");
    let window = if closed_by_tag { window.trim_end_matches(['-', '=']).trim_end() } else { window };
    let mut line_ends = window.match_indices('\n').map(|(at, _)| at).chain([window.len()]);
    let mut line_start = 0;
    let mut call = None;
    for line_end in line_ends.by_ref() {
        line_start = line_end;
        match read_code(text, call_at, name_at, window[..line_end].trim_end()) {
            Ok(found) => {
                call = Some(found);
                break;
            }
            // Later lines can't make an invalid call valid.
            Err(Failure::Invalid) => return None,
            Err(Failure::EndsEarly) => {}
        }
    }
    let mut call = call?;
    // A `.method` on the next line goes on with the call's last argument; in Haml and Slim that line is a `.class` element.
    // The text starts at the call's line, so an ERB call has its `<%` there or its `%>` after it.
    if !closed_by_tag && !text[..call_at].contains("<%") {
        return Some(call);
    }
    for line_end in line_ends {
        let line = window[line_start..line_end].trim();
        line_start = line_end;
        if line.is_empty() {
            continue;
        }
        if !is_method_chain(line) {
            break;
        }
        match read_code(text, call_at, name_at, window[..line_end].trim_end()) {
            Ok(longer) => call = longer,
            Err(_) => break,
        }
    }
    Some(call)
}

fn is_method_chain(line: &str) -> bool {
    line.starts_with("&.") || (line.starts_with('.') && !line.starts_with(".."))
}

#[cfg(test)]
thread_local! {
    /// How many times this thread has run Prism over a call, for tests that bound the work.
    pub static PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A block whose closer lies in a later template tag is closed here so the call parses.
const BLOCK_CLOSERS: [&str; 2] = ["\nend", "\n}"];

fn read_code<'a>(text: &'a str, call_at: usize, name_at: usize, code: &str) -> Result<Call<'a>, Failure> {
    match parse(text, call_at, name_at, code.len(), code) {
        Err(Failure::EndsEarly) => {}
        plain => return plain.map(|found| found.call),
    }
    BLOCK_CLOSERS
        .iter()
        .find_map(|closer| {
            let found = parse(text, call_at, name_at, code.len(), &format!("{code}{closer}")).ok()?;
            // The closer ends the call's block, or lies after the call.
            (found.has_block || found.node_end <= code.len()).then_some(found.call)
        })
        .ok_or(Failure::EndsEarly)
}

struct Found<'a> {
    call: Call<'a>,
    node_end: usize,
    has_block: bool,
}

enum Failure {
    /// The call may go on past the source, so more lines or a closer could make it Ruby.
    EndsEarly,
    Invalid,
}

fn parse<'a>(text: &'a str, call_at: usize, name_at: usize, code_len: usize, source: &str) -> Result<Found<'a>, Failure> {
    #[cfg(test)]
    PARSES.with(|parses| parses.set(parses.get() + 1));
    let result = ruby_prism::parse(source.as_bytes());
    // An error in the last token (`x ?`, `a +`) is Prism's complaint about a missing operand, not about the code before it;
    // a trailing comment is no token.
    let mut code = source.trim_end();
    let trailing_comment = result.comments().last().filter(|comment| comment.location().end() as usize >= code.len());
    if let Some(comment) = trailing_comment {
        code = source[..comment.location().start() as usize].trim_end();
    }
    let last_token_at = code.len() - code.rsplit(char::is_whitespace).next().map_or(0, str::len);
    // Prism puts an unterminated string at its opening and a heredoc at its tag (`<<~EOS`): more lines may close them.
    let opens_literal =
        |at: usize| source[..at].trim_end_matches(['~', '-']).ends_with("<<") || source[at..].starts_with(['"', '\'', '%', '/', '`']);
    let ends_early = |error: ruby_prism::Diagnostic| {
        let at = error.location().start() as usize;
        at >= last_token_at || opens_literal(at)
    };
    if result.errors().any(ends_early) {
        return Err(Failure::EndsEarly);
    }
    let found = find(&result, text, call_at, name_at, code_len).ok_or(Failure::Invalid)?;
    if result.errors().next().is_none() {
        return Ok(found);
    }
    // The code around the call (`f(render("x"), 1)`) is its own business: the call alone must be valid Ruby.
    #[cfg(test)]
    PARSES.with(|parses| parses.set(parses.get() + 1));
    let alone = ruby_prism::parse(&source.as_bytes()[..found.node_end]);
    if alone.errors().next().is_some() {
        return Err(Failure::Invalid);
    }
    find(&alone, text, call_at, name_at, code_len).ok_or(Failure::Invalid)
}

fn find<'a>(result: &ruby_prism::ParseResult, text: &'a str, call_at: usize, name_at: usize, code_len: usize) -> Option<Found<'a>> {
    let mut finder = Finder { text, call_at, name_at, code_len, found: None };
    finder.visit(&result.node());
    let mut found = finder.found?;
    found.call.commented = result.comments().next().is_some();
    Some(found)
}

struct Finder<'a> {
    text: &'a str,
    call_at: usize,
    name_at: usize,
    code_len: usize,
    found: Option<Found<'a>>,
}

impl<'pr> Visit<'pr> for Finder<'_> {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        if self.found.is_none() && is_render(node) {
            self.found = self.read_call(node);
        }
        if self.found.is_none() {
            ruby_prism::visit_call_node(self, node);
        }
    }
}

/// Where the last heredoc body in `arguments` ends; the call's own location stops at its last token, before the bodies.
fn heredocs_end(arguments: &ArgumentsNode) -> usize {
    struct Ends(usize);
    impl<'pr> Visit<'pr> for Ends {
        fn visit_string_node(&mut self, node: &StringNode<'pr>) {
            self.0 = self.0.max(node.closing_loc().map_or(0, |closing| closing.end() as usize));
        }
        fn visit_interpolated_string_node(&mut self, node: &InterpolatedStringNode<'pr>) {
            self.0 = self.0.max(node.closing_loc().map_or(0, |closing| closing.end() as usize));
            ruby_prism::visit_interpolated_string_node(self, node);
        }
    }
    let mut ends = Ends(0);
    ends.visit_arguments_node(arguments);
    ends.0
}

fn is_render(node: &CallNode) -> bool {
    match node.receiver() {
        None => node.name().as_slice() == b"render",
        Some(receiver) => receiver.as_call_node().is_some_and(|json| json.receiver().is_none() && json.name().as_slice() == b"json"),
    }
}

impl<'a> Finder<'a> {
    fn range(&self, location: &Location) -> Range<usize> {
        self.call_at + location.start() as usize..self.call_at + location.end() as usize
    }

    fn read_call(&self, node: &CallNode) -> Option<Found<'a>> {
        let arguments = node.arguments()?;
        let node_end = heredocs_end(&arguments).max(node.location().end() as usize);
        let name_at = self.name_at;
        let (mut pairs, mut readable, mut named) = (vec![], true, false);
        for argument in arguments.arguments().iter() {
            let Some(hash) = argument.as_keyword_hash_node() else {
                let span = self.range(&argument.location());
                named |= span.contains(&name_at);
                readable &= span.start <= name_at;
                continue;
            };
            for element in hash.elements().iter() {
                let Some(assoc) = element.as_assoc_node() else {
                    readable = false;
                    continue;
                };
                if self.range(&assoc.value().location()).contains(&name_at) {
                    named = true;
                } else if let Some(pair) = self.pair(&assoc, true) {
                    pairs.push(pair);
                } else {
                    readable = false;
                }
            }
        }
        let has_block = node.block().is_some_and(|block| block.as_block_node().is_some());
        let end = self.call_at + node_end.min(self.code_len);
        named.then_some(Found { call: Call { end, pairs, readable, commented: false }, node_end, has_block })
    }

    fn pair(&self, assoc: &AssocNode, read_hash: bool) -> Option<Pair<'a>> {
        let key_span = self.range(&assoc.key().as_symbol_node()?.content_loc());
        let key = &self.text[key_span.clone()];
        let value = assoc.value();
        let hash = value.as_hash_node().filter(|_| read_hash && key == "locals").map(|hash| Hash {
            inner: self.call_at + hash.opening_loc().end() as usize..self.call_at + hash.closing_loc().start() as usize,
            items: hash.elements().iter().map(|item| self.pair(&item.as_assoc_node()?, false)).collect(),
        });
        Some(Pair { key, key_span, whole: self.range(&assoc.location()), value: self.text.get(self.range(&value.location()))?, hash })
    }
}
