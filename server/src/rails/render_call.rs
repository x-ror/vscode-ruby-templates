//! Reads a `render` call's arguments with Prism, from the call's text in a template.

use crate::haml::indent::haml_structure;
use crate::haml::parser::is_haml_multiline;
use crate::lang::TemplateLang;
use crate::ruby::RUBY_WHITESPACE;
use ruby_prism::{ArgumentsNode, AssocNode, CallNode, InterpolatedStringNode, Location, Options, StringNode, Version, Visit};
use std::borrow::Cow;
use std::cell::OnceCell;
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
    /// The keyword arguments, but the one that holds the name; those with a non-literal key are left out.
    pub pairs: Vec<Pair<'a>>,
    /// Every argument after the name is such a keyword argument.
    pub readable: bool,
    /// A Ruby comment sits in the call's text.
    pub commented: bool,
}

/// Where the call text starting at `call_at` in `text` may end at most: an ERB `%>`, the `}` closing the block the call sits
/// in (`feed { render … }`), or `MAX_CALL_LINES` lines.
fn window_end(text: &str, call_at: usize) -> usize {
    let end = text[call_at..].match_indices('\n').nth(MAX_CALL_LINES - 1).map_or(text.len(), |(at, _)| call_at + at);
    let end = text[call_at..end].find("%>").map_or(end, |at| call_at + at);
    unmatched_close_brace(&text[call_at..end]).map_or(end, |at| call_at + at)
}

/// The first `}` in `code` that closes a `{` opened before `code`, skipping quoted strings and their interpolations.
fn unmatched_close_brace(code: &str) -> Option<usize> {
    close_of_level(&mut code.char_indices().peekable())
}

type Chars<'a> = std::iter::Peekable<std::str::CharIndices<'a>>;

/// The offset of the `}` that ends the brace level `chars` is in, `None` when the input ends first.
fn close_of_level(chars: &mut Chars) -> Option<usize> {
    while let Some((at, c)) = chars.next() {
        match c {
            '"' | '\'' => skip_string(chars, c)?,
            '{' => {
                close_of_level(chars)?;
            }
            '}' => return Some(at),
            _ => {}
        }
    }
    None
}

/// Consumes a string up to its closing `quote`; `None` when the input ends first.
fn skip_string(chars: &mut Chars, quote: char) -> Option<()> {
    while let Some((_, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            c if c == quote => return Some(()),
            '#' if quote == '"' && chars.next_if(|&(_, next)| next == '{').is_some() => {
                close_of_level(chars)?;
            }
            _ => {}
        }
    }
    None
}

/// The `render` or `json.…` call that starts at `call_at` in `text` and holds the name at `name_at`, read from the fewest
/// lines that make Ruby; `None` when they don't (an unclosed call, or one that isn't the name's). With `haml_multiline`
/// (`HamlPipes::apply` of the call's line) the ` |` that ends the lines of a multiline group is not Ruby. `ruby`: the
/// grammar to read with.
pub fn read(text: &str, call_at: usize, name_at: usize, haml_multiline: bool, ruby: Version) -> Option<Call<'_>> {
    let end = window_end(text, call_at);
    let window = haml_markers_blanked(text, call_at, end, haml_multiline);
    let window = window.trim_end();
    // `-%>` and `=%>` trim the template's whitespace; they are not Ruby.
    let closed_by_tag = text[end..].starts_with("%>");
    let window = if closed_by_tag { window.trim_end_matches(['-', '=']).trim_end() } else { window };
    let mut line_ends = window.match_indices('\n').map(|(at, _)| at).chain([window.len()]);
    let mut line_start = 0;
    let mut call = None;
    for line_end in line_ends.by_ref() {
        line_start = line_end;
        match read_code(text, call_at, name_at, window[..line_end].trim_end(), ruby) {
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
    // A chain line ending in `,` goes on with more arguments on the lines after it.
    let mut open = false;
    for line_end in line_ends {
        let line = window[line_start..line_end].trim();
        line_start = line_end;
        if line.is_empty() {
            continue;
        }
        if !open && !is_method_chain(line) {
            break;
        }
        match read_code(text, call_at, name_at, window[..line_end].trim_end(), ruby) {
            Ok(longer) => (call, open) = (longer, false),
            Err(Failure::EndsEarly) => open = true,
            Err(Failure::Invalid) => break,
        }
    }
    Some(call)
}

/// `text` from the call's line on, as far as a call may reach, with its Haml `|` group joined (see `haml_markers_blanked`).
pub fn haml_group_joined(text: &str) -> Cow<'_, str> {
    haml_markers_blanked(text, 0, window_end(text, 0), true)
}

/// `text[call_at..end]`, with the ` |` and line breaks of a Haml `|` multiline group blanked, as Haml joins its lines, so
/// the group reads as one line of Ruby and offsets stay the text's. Only a call whose own line ends in a ` |` marker is one.
fn haml_markers_blanked(text: &str, call_at: usize, end: usize, haml_multiline: bool) -> Cow<'_, str> {
    let window = &text[call_at..end];
    let first_line = window.lines().next().unwrap_or_default();
    if !haml_multiline || !is_haml_multiline(first_line) {
        return Cow::Borrowed(window);
    }
    let mut in_group = true;
    let lines: Vec<(bool, &str)> = window
        .split_inclusive('\n')
        .map(|line| {
            // Haml skips blank lines inside the group.
            in_group = in_group && (line.trim().is_empty() || is_haml_multiline(line));
            (in_group, line)
        })
        .collect();
    let mut blanked = String::with_capacity(window.len());
    for (i, &(in_group, line)) in lines.iter().enumerate() {
        let code = line.trim_end_matches(RUBY_WHITESPACE).strip_suffix('|').filter(|_| in_group);
        let body_end = line.trim_end_matches(['\r', '\n']).len();
        let joins_next = in_group && lines.get(i + 1).is_some_and(|&(next_in_group, _)| next_in_group);
        match code {
            Some(code) => {
                blanked.push_str(code);
                blanked.push(' ');
                blanked.push_str(&line[code.len() + 1..body_end]);
            }
            None => blanked.push_str(&line[..body_end]),
        }
        let line_break = &line[body_end..];
        blanked.extend(line_break.chars().map(|c| if joins_next { ' ' } else { c }));
    }
    Cow::Owned(blanked)
}

#[cfg(test)]
thread_local! {
    /// How many times this thread has run the Haml structure pass, for tests that bound the work.
    pub static STRUCTURE_PASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether `|` ends a multiline group on the lines of one document: only in Haml, and not in a filter body (`:ruby`),
/// which Haml leaves alone. The structure pass runs once, for the first line ending in ` |`.
pub struct HamlPipes {
    haml: bool,
    /// Per line: outside filter and comment bodies.
    structural: OnceCell<Vec<bool>>,
}

impl HamlPipes {
    pub fn new(lang: TemplateLang) -> Self {
        HamlPipes { haml: lang == TemplateLang::Haml, structural: OnceCell::new() }
    }

    /// Does `|` end a multiline group on the line of `text` at `line_start`?
    pub fn apply(&self, text: &str, line_start: usize) -> bool {
        if !self.haml {
            return false;
        }
        let line = text[line_start..].lines().next().unwrap_or_default();
        if !is_haml_multiline(line) {
            return false;
        }
        let structural = self.structural.get_or_init(|| {
            #[cfg(test)]
            STRUCTURE_PASSES.with(|passes| passes.set(passes.get() + 1));
            let mut structural = vec![false; text.lines().count()];
            for (first, span) in haml_structure(text) {
                let last = (first + span).min(structural.len());
                structural[first..last].fill(true);
            }
            structural
        });
        structural.get(text[..line_start].matches('\n').count()).copied().unwrap_or(false)
    }
}

fn is_method_chain(line: &str) -> bool {
    line.starts_with("&.") || (line.starts_with('.') && !line.starts_with(".."))
}

/// A block whose closer lies in a later template tag is closed here so the call parses.
const BLOCK_CLOSERS: [&str; 2] = ["\nend", "\n}"];

fn read_code<'a>(text: &'a str, call_at: usize, name_at: usize, code: &str, ruby: Version) -> Result<Call<'a>, Failure> {
    let end = (call_at + code.len()).min(text.len());
    let text = if text.is_char_boundary(end) { &text[..end] } else { text };
    match parse(text, call_at, name_at, code, ruby) {
        Err(Failure::EndsEarly) => {}
        plain => return plain.map(|found| found.call),
    }
    BLOCK_CLOSERS
        .iter()
        .find_map(|closer| {
            let found = parse(text, call_at, name_at, &format!("{code}{closer}"), ruby).ok()?;
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

fn parse<'a>(text: &'a str, call_at: usize, name_at: usize, source: &str, ruby: Version) -> Result<Found<'a>, Failure> {
    #[cfg(test)]
    crate::ruby::prism::PARSES.with(|parses| parses.set(parses.get() + 1));
    let options = Options::default().version(ruby).build();
    let result = ruby_prism::parse_with_options(source.as_bytes(), &options);
    // An error in the last token (`x ?`, `a +`) is Prism's complaint about a missing operand, not about the code before it;
    // a trailing comment is no token.
    let mut code = source.trim_end();
    let trailing_comment = result.comments().last().filter(|comment| comment.location().end() as usize >= code.len());
    if let Some(comment) = trailing_comment {
        code = source[..comment.location().start() as usize].trim_end();
    }
    let last_token_at = code.len() - code.rsplit(char::is_whitespace).next().map_or(0, str::len);
    // Prism puts an unterminated string at its opening and a heredoc at its tag (`<<~EOS`): more lines may close them.
    let opens_literal = |at: usize| {
        source[..at].trim_end_matches(['~', '-']).ends_with("<<")
            || source[at..].starts_with(['"', '\'', '%', '/', '`'])
            || source[at..].starts_with(":\"")
            || source[at..].starts_with(":'")
    };
    let ends_early = |error: ruby_prism::Diagnostic| {
        let at = error.location().start() as usize;
        at >= last_token_at || opens_literal(at)
    };
    if result.errors().any(ends_early) {
        return Err(Failure::EndsEarly);
    }
    let found = find(&result, text, call_at, name_at).ok_or(Failure::Invalid)?;
    if result.errors().next().is_none() {
        return Ok(found);
    }
    // The code around the call (`f(render("x"), 1)`) is its own business: the call alone must be valid Ruby.
    #[cfg(test)]
    crate::ruby::prism::PARSES.with(|parses| parses.set(parses.get() + 1));
    let alone = ruby_prism::parse_with_options(&source.as_bytes()[..found.node_end], &options);
    if alone.errors().next().is_some() {
        return Err(Failure::Invalid);
    }
    find(&alone, text, call_at, name_at).ok_or(Failure::Invalid)
}

fn find<'a>(result: &ruby_prism::ParseResult, text: &'a str, call_at: usize, name_at: usize) -> Option<Found<'a>> {
    let mut finder = Finder { text, call_at, name_at, found: None };
    finder.visit(&result.node());
    let mut found = finder.found?;
    found.call.commented = result.comments().next().is_some();
    Some(found)
}

struct Finder<'a> {
    text: &'a str,
    call_at: usize,
    name_at: usize,
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
        named.then_some(Found { call: Call { pairs, readable, commented: false }, node_end, has_block })
    }

    fn pair(&self, assoc: &AssocNode, read_hash: bool) -> Option<Pair<'a>> {
        let key_span = self.range(&assoc.key().as_symbol_node()?.content_loc());
        let key = self.text.get(key_span.clone())?;
        let value = assoc.value();
        let hash = value.as_hash_node().filter(|_| read_hash && key == "locals").map(|hash| Hash {
            inner: self.call_at + hash.opening_loc().end() as usize..self.call_at + hash.closing_loc().start() as usize,
            items: hash.elements().iter().map(|item| self.pair(&item.as_assoc_node()?, false)).collect(),
        });
        Some(Pair { key, key_span, whole: self.range(&assoc.location()), value: self.text.get(self.range(&value.location()))?, hash })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_quoted_symbol_and_eof_block_closer() {
        let quoted_sym = "<%= render \"users/card\",\n  tag: :\"a b\nc\"\n%>";
        let name_at = quoted_sym.find("users/card").unwrap();
        let call = read(quoted_sym, quoted_sym.find("render").unwrap(), name_at, false, Version::Latest).unwrap();
        assert_eq!(call.pairs.iter().map(|p| p.key).collect::<Vec<_>>(), ["tag"]);

        let eof_block = "<%= render \"users/card\", a: 1 do |x|\n";
        let name_at = eof_block.find("users/card").unwrap();
        let call = read(eof_block, eof_block.find("render").unwrap(), name_at, false, Version::Latest).unwrap();
        assert_eq!(call.pairs.iter().map(|p| p.key).collect::<Vec<_>>(), ["a"]);
    }
}
