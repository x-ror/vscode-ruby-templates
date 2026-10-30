//! A port of `Haml::Parser` (Haml 7.5.1, `lib/haml/parser.rb`): the first error Haml raises, at the line it names,
//! what each line is to Haml, and enough of the compiled nodes to tell whether two templates compile alike.

use crate::generated_ruby::ruby_string_body;
use crate::slim_parser::VOID_ELEMENTS;
use regex::Regex;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::LazyLock;

pub const BOM: char = '\u{feff}';
const AT_START: &str = "Indenting at the beginning of the document is illegal.";
const UNBALANCED: &str = "Unbalanced brackets.";
const ILLEGAL_ELEMENT: &str = "Illegal element: classes and ids must have values.";
const NO_END: &str = "You don't need to use \"- end\" in Haml. Un-indent to close a block:\n- if foo?\n  %strong Foo!\n- else\n  Not foo.\n%p This line is un-indented, so it isn't part of the \"if\" block";
const SELF_CLOSING_CONTENT: &str = "Self-closing tags can't have content.";
const NESTING_PLAIN: &str = "Illegal nesting: nesting within plain text is illegal.";
const NESTING_SELF_CLOSING: &str = "Illegal nesting: nesting within a self-closing tag is illegal.";
const NESTING_CONTENT: &str = "Illegal nesting: nesting within a tag that already has content is illegal.";
const NESTING_HEADER: &str = "Illegal nesting: nesting within a header command is illegal.";
const MIXED_INDENTATION: &str = "Indentation can't use both tabs and spaces.";
/// Ruby's `\s` and `rstrip` characters.
const RUBY_WHITESPACE: [char; 7] = [' ', '\t', '\n', '\x0b', '\x0c', '\r', '\0'];
/// Ruby's regexp `\s`.
const REGEXP_SPACE: [char; 6] = [' ', '\t', '\n', '\x0b', '\x0c', '\r'];
/// The filters Haml 6 and 7 register (`Haml::Filters`).
pub const HAML_FILTERS: &[&str] = &[
    "plain",
    "escaped",
    "preserve",
    "cdata",
    "javascript",
    "css",
    "ruby",
    "erb",
    "markdown",
    "sass",
    "scss",
    "less",
    "coffee",
    "coffeescript",
];
/// Haml's `MID_BLOCK_KEYWORDS`: a `-` line starting with one continues the block above instead of closing it.
const MID_BLOCK_KEYWORDS: &[&str] = &["else", "elsif", "rescue", "ensure", "end", "when", "in"];
/// Haml's `BLOCK_KEYWORD_REGEX`, applied to the code after the `-`/`=` prefix.
static BLOCK_KEYWORD: LazyLock<Regex> = LazyLock::new(|| {
    // ASCII `\w`/`\s` like Ruby's; Ruby's `\b` is Unicode-aware like this one.
    Regex::new(r"(?-u:^\s*(?:(else|elsif|rescue|ensure|end|when|in)|(?:\w+(?:,\s*\w+)*\s*=\s*)?(if|begin|case|unless)))\b").unwrap()
});
static BLOCK_WITH_SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"do\s*\|\s*[^|]*\s+\|$").unwrap());

/// A parse error: Haml's message and the 0-based line it reports.
#[derive(Debug, PartialEq, Eq)]
pub struct Error {
    pub message: String,
    pub line: usize,
}

/// Why the parse stopped: the end of the template (Ruby's `StopIteration`, which ends Haml's loop) or an error.
enum Halt {
    Done,
    Error(Error),
}

fn halt(message: impl Into<String>, line: usize) -> Halt {
    Halt::Error(Error { message: message.into(), line })
}

#[derive(Debug, Default, Clone)]
struct Line {
    whitespace: Option<String>,
    text: String,
    full: String,
    index: usize,
    end_of_document: bool,
    tabs: Option<usize>,
}

impl Line {
    fn indent(&self) -> usize {
        self.whitespace.as_ref().map_or(0, String::len)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Current,
    Next,
}

struct Parser {
    template: VecDeque<Line>,
    line: Line,
    line_defined: bool,
    next_line: Line,
    indentation: Option<String>,
    flat: bool,
    flat_spaces: Option<String>,
    template_tabs: usize,
    /// One entry per open block (Haml's `@parent` chain), which says what closing it undoes.
    open_blocks: Vec<Open>,
    tab_up: bool,
    /// The open `if`/`case`/`unless` lines, for `else`, `elsif` and `when` (`@script_level_stack`).
    script_levels: Vec<ScriptLevel>,
    void_content: Vec<VoidContent>,
    roles: Vec<Role>,
    /// The lines whose depth Haml read (`Line#tabs`).
    tabs_read: Vec<bool>,
    compiled: Vec<Compiled>,
    /// The texts of the lines joined into the node being read.
    joined: Vec<String>,
    /// The line of the filter or `-#` comment whose body is being read.
    flat_owner: usize,
    /// The flat section is a `-#` comment, which Haml compiles to nothing.
    flat_comment: bool,
    /// The first filter Haml has no compiler for; it fails at compile time, so any parser error comes first.
    unknown_filter: Option<Error>,
    /// Whether to build `pieces`, which only the Ruby check uses.
    ruby: bool,
    pieces: Vec<Piece>,
    /// Where each physical line starts in the template.
    line_starts: Vec<usize>,
    /// The lines whose text `|` multiline joined: they are no longer a slice of the template.
    pipe_joined: Vec<bool>,
    /// How the Ruby of the line being read opens a block.
    line_block: Option<RubyBlock>,
    /// The line being read is a node Haml pushes although the document ends in the middle of it (an attribute list).
    counted_at_end: bool,
    /// What the body of the filter just read compiles to.
    filter: FilterBody,
    /// Where the body of the open filter starts in `compiled`.
    filter_start: usize,
}

/// What a filter's body is to the Ruby Haml generates.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FilterBody {
    None,
    Ruby,
    /// Text that may hold `#{}`.
    Text,
}

/// One open block: its node, how many lines Haml pushed into it, and what its Ruby needs.
struct Open {
    node: Node,
    line: usize,
    children: usize,
    ruby: Option<RubyBlock>,
}

/// A `-` or `=` line, or a tag with code: what the lines that nest under it do to its Ruby.
#[derive(Clone, Copy)]
struct RubyBlock {
    /// The index of its `Output` piece.
    output: Option<usize>,
    /// Haml gives it an `end` and a block when lines nest under it; a tag's code is dropped instead.
    ends: bool,
    /// A `case` line, whose first `when` takes the lines that nest under it.
    case: bool,
    when: bool,
    /// The first `when` of a `case`: Haml moves its lines to the `case`, so it has no `end`.
    adopted: bool,
}

/// What a line opens, as far as closing it matters.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Node {
    Other,
    /// A filter or `-#` comment: its end closes the flat section.
    FlatSection,
    /// An `if`, `case` or `unless` line: its end closes its entry in `script_levels`.
    Branch,
}

struct ScriptLevel {
    tabs: usize,
    seen_when: bool,
}

/// Content in a void element, which Haml compiles to an end tag (`<br>x</br>`): a WHATWG parse error.
#[derive(Debug, PartialEq, Eq)]
pub struct VoidContent {
    pub line: usize,
    pub name: String,
    /// Nested lines, rather than content on the tag's own line.
    pub nested: bool,
}

/// What a physical line is to Haml. Reliable when the template has no error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// An empty or whitespace-only line Haml skips, and any line past the last one it reads.
    Blank,
    /// A line Haml reads as a node, `depth` levels deep (`Line#tabs`).
    Structural { depth: usize },
    /// A line joined into the one at `owner`: `|` multiline, Ruby after a comma, an attribute group that goes on.
    /// Its indentation is stripped, so it can move freely, except that Haml checks it against the indentation unit
    /// when it read the line's depth (`counted`): then it has to be a multiple of the unit.
    Inner { owner: usize, counted: bool },
    /// A line of the filter or `-#` comment body at `owner`, a content line: `prefix_len` bytes of its whitespace are
    /// the body's indentation (the flat prefix), the rest is text. Also the blank lines inside the body.
    FlatBody { owner: usize, prefix_len: usize },
}

/// A run of the Ruby Haml compiles, on template line `line`: at byte `offset` of the template (without its BOM) when it
/// is a verbatim slice of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    pub line: usize,
    pub offset: Option<usize>,
}

impl Chunk {
    pub(crate) fn synthetic(text: &str, line: usize) -> Chunk {
        Chunk { text: text.to_string(), line, offset: None }
    }
}

/// The Ruby Haml generates for the template, as far as its structure and syntax go.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    /// A `-` line, or the body of a `:ruby` filter.
    Statement(Vec<Chunk>),
    /// A `=` line or tag content: the value is written. `block` when lines nest under it.
    Output { code: Vec<Chunk>, block: bool },
    /// Text with `#{}`, as the Ruby string literal Haml makes of it.
    Text(Vec<Chunk>),
    /// What Haml passes to `AttributeBuilder.build` at run time: an object reference, then the hash of the `()` attributes
    /// and the code of the `{}` hash.
    Attributes { object: Vec<Chunk>, arguments: Vec<Chunk> },
    /// The `end` of the block the line opened, which has lines nested under it.
    End(usize),
    /// A tag's code that Haml leaves out because lines nest under the tag.
    Dropped,
}

/// What Haml makes of one node, as far as the compiled source depends on it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Compiled {
    Node { index: usize, tabs: usize, text: String, joined: Vec<String> },
    Flat { index: usize, text: String },
}

pub struct Parsed {
    /// The first error Haml raises for the template, at the line it reports.
    pub error: Option<Error>,
    /// Only filled when the template compiles.
    pub void_content: Vec<VoidContent>,
    /// One per physical line of the template (without a BOM).
    pub roles: Vec<Role>,
    /// The Ruby Haml generates, in order. Only complete when the template has no error.
    pub pieces: Vec<Piece>,
    compiled: Vec<Compiled>,
}

pub fn parse(text: &str) -> Parsed {
    parse_as(text, false)
}

/// `parse`, and the `pieces` of Ruby Haml generates, which cost more than the rest.
pub fn parse_with_ruby(text: &str) -> Parsed {
    parse_as(text, true)
}

fn parse_as(text: &str, ruby: bool) -> Parsed {
    let mut parser = Parser::new(text.strip_prefix(BOM).unwrap_or(text), ruby);
    let error = match parser.run() {
        Err(Halt::Error(error)) => Some(error),
        Err(Halt::Done) => {
            parser.close_all();
            parser.unknown_filter.take()
        }
    };
    let void_content = if error.is_some() { Vec::new() } else { parser.void_content };
    let mut roles = parser.roles;
    for (role, &counted) in roles.iter_mut().zip(&parser.tabs_read) {
        if let Role::Inner { owner, .. } = *role {
            *role = Role::Inner { owner, counted };
        }
    }
    Parsed { error, void_content, roles, pieces: parser.pieces, compiled: parser.compiled }
}

/// Whether Haml compiles `before` and `after` to the same source: both are free of errors and read the same nodes, at
/// the same lines and depths, with the same texts. Indentation (other than the depth and a filter body's text past its
/// prefix), line endings, trailing whitespace and the empty lines at the end don't matter to Haml.
pub fn same_compilation(before: &str, after: &str) -> bool {
    let (before, after) = (parse(before), parse(after));
    before.error.is_none() && after.error.is_none() && before.compiled == after.compiled
}

/// The lines of `text` as Haml (and the LSP) split them, at `\r\n`, `\r` and `\n`, each with its line ending
/// (empty on a last line without one).
pub fn lines_with_endings(text: &str) -> Vec<(&str, &str)> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut rest = text;
    loop {
        let end = rest.find(['\r', '\n']).unwrap_or(rest.len());
        let tail = &rest[end..];
        let ending = if tail.starts_with("\r\n") { 2 } else { tail.len().min(1) };
        lines.push((&rest[..end], &tail[..ending]));
        if ending == 0 {
            return lines;
        }
        rest = &tail[ending..];
    }
}

pub fn physical_lines(text: &str) -> Vec<&str> {
    lines_with_endings(text).into_iter().map(|(line, _)| line).collect()
}

fn split_lines(text: &str) -> Vec<Line> {
    let lines = physical_lines(text.trim_end_matches(RUBY_WHITESPACE));
    lines
        .into_iter()
        .enumerate()
        .map(|(index, full)| {
            let indent_len = full.len() - full.trim_start_matches([' ', '\t']).len();
            Line {
                whitespace: (indent_len > 0).then(|| full[..indent_len].to_string()),
                text: full[indent_len..].trim_end_matches(RUBY_WHITESPACE).to_string(),
                full: full.to_string(),
                index,
                ..Line::default()
            }
        })
        .collect()
}

/// Where each of the first `count` lines starts in `text`.
fn line_starts(text: &str, count: usize) -> Vec<usize> {
    let mut start = 0;
    lines_with_endings(text)
        .into_iter()
        .take(count)
        .map(|(line, ending)| {
            let at = start;
            start += line.len() + ending.len();
            at
        })
        .collect()
}

fn human_indentation(indentation: &str) -> String {
    let noun = match (indentation.contains('\t'), indentation.contains(' ')) {
        (false, _) => "space",
        (true, false) => "tab",
        (true, true) => return format!("{indentation:?}"),
    };
    let plural = if indentation.len() == 1 { "" } else { "s" };
    format!("{} {noun}{plural}", indentation.len())
}

impl Parser {
    fn new(text: &str, ruby: bool) -> Parser {
        let mut template: VecDeque<Line> = split_lines(text).into();
        let index = template.len();
        let lines = index;
        let line_starts = if ruby { line_starts(text, lines) } else { Vec::new() };
        template.push_back(Line { text: "-#".into(), full: "-#".into(), index, end_of_document: true, ..Line::default() });
        Parser {
            template,
            line: Line::default(),
            line_defined: false,
            next_line: Line::default(),
            indentation: None,
            flat: false,
            flat_spaces: None,
            template_tabs: 0,
            open_blocks: Vec::new(),
            tab_up: false,
            script_levels: Vec::new(),
            void_content: Vec::new(),
            roles: vec![Role::Blank; lines],
            tabs_read: vec![false; lines],
            compiled: Vec::new(),
            joined: Vec::new(),
            flat_owner: 0,
            flat_comment: false,
            unknown_filter: None,
            ruby,
            pieces: Vec::new(),
            line_starts,
            pipe_joined: vec![false; lines],
            line_block: None,
            counted_at_end: false,
            filter: FilterBody::None,
            filter_start: 0,
        }
    }

    /// Haml's main loop; it only ends by running out of lines or by an error.
    fn run(&mut self) -> Result<Infallible, Halt> {
        self.next_line()?;
        self.advance();
        if self.tabs(Which::Current)? != 0 {
            return Err(halt(AT_START, self.line.index));
        }
        loop {
            self.next_line()?;
            if !self.line.text.is_empty() {
                self.process_indent()?;
            }
            if self.flat {
                self.read_flat_line();
                self.advance();
                continue;
            }
            self.tab_up = false;
            self.joined.clear();
            self.line_block = None;
            self.counted_at_end = false;
            let read = if self.line.text.is_empty() { Ok(Node::Other) } else { self.process_line() };
            // The last node counts even when the document ends in the middle of it.
            self.record_node();
            if self.counted_at_end && matches!(read, Err(Halt::Done)) {
                self.push_node();
            }
            let node = read?;
            let ruby = self.push_node();
            if self.block_opened()? || self.tab_up {
                self.template_tabs += 1;
                self.open_blocks.push(Open { node, line: self.line.index, children: 0, ruby });
            }
            if !self.flat {
                let (next, current) = (self.tabs(Which::Next)?, self.tabs(Which::Current)?);
                if next > current + 1 {
                    let message = format!("The line was indented {} levels deeper than the previous line.", next - current);
                    return Err(halt(message, self.next_line.index));
                }
            }
            self.advance();
        }
    }

    /// Haml's `push`: the line just read becomes a child of the block it stands in. Returns its Ruby block, if it opens one.
    fn push_node(&mut self) -> Option<RubyBlock> {
        if self.line.text.is_empty() {
            return None;
        }
        let mut ruby = self.line_block.take();
        let Some(parent) = self.open_blocks.last_mut() else { return ruby };
        parent.children += 1;
        let first = parent.children == 1;
        if let (true, Some(RubyBlock { output: Some(piece), ends, .. })) = (first, parent.ruby) {
            match self.pieces.get_mut(piece) {
                Some(slot) if !ends => *slot = Piece::Dropped,
                Some(Piece::Output { block, .. }) => *block = true,
                _ => {}
            }
        }
        if let (true, Some(block), Some(RubyBlock { case: true, .. })) = (first, ruby.as_mut(), parent.ruby) {
            block.adopted = block.when;
        }
        ruby
    }

    fn set_role(&mut self, index: usize, role: Role) {
        if let Some(slot) = self.roles.get_mut(index) {
            *slot = role;
        }
    }

    /// A line of a filter or `-#` comment body, whose text is what follows the flat prefix.
    fn read_flat_line(&mut self) {
        let prefix = self.flat_spaces.as_deref().unwrap_or("");
        let (text, prefix_len) = match self.line.full.strip_prefix(prefix) {
            Some(text) if !self.flat_comment => (text.to_string(), prefix.len()),
            Some(_) => (String::new(), prefix.len()),
            None => (String::new(), 0),
        };
        let index = self.line.index;
        self.set_role(index, Role::FlatBody { owner: self.flat_owner, prefix_len });
        self.compiled.push(Compiled::Flat { index, text });
    }

    fn record_node(&mut self) {
        if self.line.text.is_empty() {
            return;
        }
        let (index, tabs) = (self.line.index, self.line.tabs.unwrap_or(0));
        self.set_role(index, Role::Structural { depth: tabs });
        let comment = self.line.text.starts_with("-#");
        let text = if comment { String::new() } else { self.line.text.clone() };
        let node = Compiled::Node { index, tabs, text, joined: std::mem::take(&mut self.joined) };
        self.compiled.push(node);
    }

    /// A line that was read as part of the one at `owner`.
    fn join_line(&mut self, line_index: usize, owner: usize, text: &str) {
        let role = if text.is_empty() { Role::Blank } else { Role::Inner { owner, counted: false } };
        self.set_role(line_index, role);
        self.joined.push(text.to_string());
    }

    fn advance(&mut self) {
        self.line = std::mem::take(&mut self.next_line);
        self.line_defined = true;
    }

    /// `Line#tabs`: computed on first use, since it can raise and sets the document's indentation.
    fn tabs(&mut self, which: Which) -> Result<usize, Halt> {
        if let Some(tabs) = self.slot(which).tabs {
            return Ok(tabs);
        }
        let line = std::mem::take(self.slot(which));
        let tabs = self.compute_tabs(&line);
        if let Some(read) = self.tabs_read.get_mut(line.index) {
            *read = true;
        }
        *self.slot(which) = line;
        let tabs = tabs?;
        self.slot(which).tabs = Some(tabs);
        Ok(tabs)
    }

    fn slot(&mut self, which: Which) -> &mut Line {
        match which {
            Which::Current => &mut self.line,
            Which::Next => &mut self.next_line,
        }
    }

    fn compute_tabs(&mut self, line: &Line) -> Result<usize, Halt> {
        let Some(whitespace) = line.whitespace.as_ref().filter(|_| !line.text.is_empty()) else { return Ok(0) };
        let Some(indentation) = &self.indentation else {
            self.indentation = Some(whitespace.clone());
            if whitespace.contains('\t') && whitespace.contains(' ') {
                return Err(halt(MIXED_INDENTATION, line.index));
            }
            if self.flat {
                self.flat_spaces = Some(whitespace.repeat(self.template_tabs + 1));
            }
            return Ok(1);
        };
        let tabs = whitespace.len() / indentation.len();
        if *whitespace == indentation.repeat(tabs) {
            return Ok(tabs);
        }
        if self.flat && whitespace.starts_with(self.flat_spaces.as_deref().unwrap_or("")) {
            return Ok(self.template_tabs + 1);
        }
        let message = format!(
            "Inconsistent indentation: {} used for indentation, but the rest of the document was indented using {}.",
            human_indentation(whitespace),
            human_indentation(indentation)
        );
        Err(halt(message, line.index))
    }

    fn closes_flat(&self, line: &Line) -> bool {
        !line.text.is_empty() && !line.full.starts_with(self.flat_spaces.as_deref().unwrap_or(""))
    }

    /// Reads the next line that matters into `next_line`: blank lines are skipped and `|` lines joined, except in flat sections.
    fn next_line(&mut self) -> Result<(), Halt> {
        loop {
            let mut line = self.template.pop_front().ok_or(Halt::Done)?;
            if self.line_defined {
                self.tabs(Which::Current)?;
            }
            let in_flat_section = self.flat && !self.closes_flat(&line) && !(self.line_defined && self.closes_flat(&self.line));
            if !(in_flat_section || self.is_filter_body(&line)) {
                if line.text.is_empty() {
                    continue;
                }
                self.join_multiline(&mut line);
            }
            self.next_line = line;
            return Ok(());
        }
    }

    /// A line indented deeper than the filter line before it, which no blank-line or `|` handling touches.
    fn is_filter_body(&self, line: &Line) -> bool {
        if !self.line_defined || !self.line.text.starts_with(':') {
            return false;
        }
        let own_indent = self.line.full.len() - self.line.full.trim_start_matches(REGEXP_SPACE).len();
        line.full.strip_prefix(&self.line.full[..own_indent]).is_some_and(|rest| rest.starts_with(REGEXP_SPACE))
    }

    /// `handle_multiline`: the lines that follow a line ending in ` |` and end in ` |` too are part of it.
    fn join_multiline(&mut self, line: &mut Line) {
        if !is_haml_multiline(&line.text) {
            return;
        }
        line.text.pop();
        if let Some(joined) = self.pipe_joined.get_mut(line.index) {
            *joined = true;
        }
        while let Some(next) = self.template.front() {
            let text = next.text.trim_matches(RUBY_WHITESPACE);
            if next.end_of_document || (!text.is_empty() && !is_haml_multiline(text)) {
                break;
            }
            line.text.push_str(text.strip_suffix('|').unwrap_or(text));
            let (index, owner) = (next.index, line.index);
            if !text.is_empty() {
                self.set_role(index, Role::Inner { owner, counted: false });
            }
            self.template.pop_front();
        }
    }

    fn process_indent(&mut self) -> Result<(), Halt> {
        let tabs = self.tabs(Which::Current)?;
        if tabs > self.template_tabs || self.template_tabs == 0 {
            return Ok(());
        }
        let to_close = self.template_tabs - tabs;
        let continues_block = self.is_continuation_script();
        for remaining in (0..to_close).rev() {
            if remaining > 0 || !continues_block {
                self.close();
            }
        }
        Ok(())
    }

    fn is_continuation_script(&self) -> bool {
        let Some(code) = self.line.text.strip_prefix('-') else { return false };
        block_keyword(code).is_some_and(|keyword| MID_BLOCK_KEYWORDS.contains(&keyword))
    }

    fn close(&mut self) {
        self.template_tabs -= 1;
        let Some(open) = self.open_blocks.pop() else { return };
        match open.node {
            Node::FlatSection => {
                self.flat = false;
                self.flat_spaces = None;
                self.finish_filter();
            }
            Node::Branch => {
                self.script_levels.pop();
            }
            Node::Other => {}
        }
        if open.children > 0 && open.ruby.is_some_and(|block| block.ends && !block.adopted) {
            self.emit(Piece::End(open.line));
        }
    }

    /// Haml's `close until @parent.type == :root` at the end of the template.
    fn close_all(&mut self) {
        while !self.open_blocks.is_empty() {
            self.close();
        }
    }

    fn block_opened(&mut self) -> Result<bool, Halt> {
        Ok(self.tabs(Which::Next)? > self.tabs(Which::Current)?)
    }

    fn filter_opened(&self) -> bool {
        match &self.indentation {
            Some(indentation) => self.next_line.full.starts_with(&indentation.repeat(self.template_tabs + 1)),
            None => self.next_line.full.starts_with(REGEXP_SPACE),
        }
    }

    fn start_flat_section(&mut self, comment: bool, body: FilterBody) -> Node {
        if !self.filter_opened() {
            return Node::Other;
        }
        self.filter = if comment { FilterBody::None } else { body };
        self.filter_start = self.compiled.len();
        self.flat = true;
        self.flat_owner = self.line.index;
        self.flat_comment = comment;
        self.flat_spaces = self.indentation.as_ref().map(|indentation| indentation.repeat(self.template_tabs + 1));
        Node::FlatSection
    }

    /// What `process_line` does that later lines depend on: the lines a node consumes, the flat sections and the
    /// `if` levels it opens.
    fn process_line(&mut self) -> Result<Node, Halt> {
        let text = std::mem::take(&mut self.line.text);
        let result = self.process_text(&text);
        self.line.text = text;
        result
    }

    fn process_text(&mut self, text: &str) -> Result<Node, Halt> {
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some('%' | '.'), _) => self.tag(text)?,
            (Some('#'), second) if !matches!(second, Some('{' | '@' | '$')) => self.tag(text)?,
            (Some('-'), Some('#')) => return Ok(self.start_flat_section(true, FilterBody::None)),
            (Some(':'), _) => {
                let body = self.filter(&text[1..])?;
                return Ok(self.start_flat_section(false, body));
            }
            (Some('/'), _) => self.comment(text, &text[1..])?,
            (Some('!'), _) if text.starts_with("!!!") => self.doctype()?,
            (Some('-' | '=' | '~' | '&' | '!'), _) => match text.strip_prefix('-').or_else(|| strip_output_prefix(text)) {
                Some(code) => return self.script(text, code),
                None => self.plain(text)?,
            },
            _ => self.plain(text)?,
        }
        Ok(Node::Other)
    }

    /// `filter`: the name must be a word; whether a compiler exists for it is only found out when Haml compiles.
    fn filter(&mut self, name: &str) -> Result<FilterBody, Halt> {
        let name = name.to_lowercase();
        if name.is_empty() || !name.chars().all(is_word) {
            return Err(halt(format!("Invalid filter name \":{name}\"."), self.line.index));
        }
        if self.unknown_filter.is_none() && !HAML_FILTERS.contains(&name.as_str()) {
            self.unknown_filter = Some(Error { message: format!("FilterCompiler for '{name}' was not found"), line: self.line.index });
        }
        Ok(match name.as_str() {
            "ruby" => FilterBody::Ruby,
            // The filters that compile their body into a string, `:erb` and the unknown ones don't.
            "plain" | "escaped" | "preserve" | "cdata" | "javascript" | "css" | "markdown" | "sass" | "scss" | "less" | "coffee"
            | "coffeescript" => FilterBody::Text,
            _ => FilterBody::None,
        })
    }

    fn nested_line_error(&mut self, message: &str) -> Halt {
        halt(message, self.next_line.index)
    }

    fn plain(&mut self, text: &str) -> Result<(), Halt> {
        if self.block_opened()? {
            return Err(self.nested_line_error(NESTING_PLAIN));
        }
        let body = &text[plain_text_start(text)..];
        self.push_text(body, self.line.index, self.line_column(body, text));
        Ok(())
    }

    fn doctype(&mut self) -> Result<(), Halt> {
        if self.block_opened()? {
            return Err(self.nested_line_error(NESTING_HEADER));
        }
        Ok(())
    }

    /// `comment`: `/ text` and `/[if IE] text` can't have nested lines.
    fn comment(&mut self, line_text: &str, text: &str) -> Result<(), Halt> {
        let text = text.trim_start_matches(RUBY_WHITESPACE);
        let text = if text.starts_with("![") { &text[1..] } else { text };
        let text = if text.starts_with('[') { balance_brackets(text).ok_or_else(|| halt(UNBALANCED, self.line.index))? } else { text };
        let text = text.trim_matches(RUBY_WHITESPACE);
        if self.block_opened()? && !text.is_empty() {
            return Err(self.nested_line_error(NESTING_CONTENT));
        }
        self.push_text(text, self.line.index, self.line_column(text, line_text));
        Ok(())
    }

    /// `script`, `flat_script` and `silent_script`: the `if` levels, and where `else`, `elsif` and `when` may stand.
    fn script(&mut self, text: &str, code: &str) -> Result<Node, Halt> {
        let silent = text.starts_with('-');
        if silent && code.trim_matches(RUBY_WHITESPACE) == "end" {
            return Err(halt(NO_END, self.line.index));
        }
        if !silent && code.trim_start_matches(REGEXP_SPACE).is_empty() {
            let action = if text[..text.len() - code.len()].ends_with('~') { '~' } else { '=' };
            return Err(halt(no_ruby_code(action), self.line.index));
        }
        let continued = self.join_ruby_lines(text)?;
        let keyword = block_keyword(code);
        let tabs = self.tabs(Which::Current)?;
        let opens_branch = matches!(keyword, Some("if" | "case" | "unless"));
        if opens_branch {
            self.script_levels.push(ScriptLevel { tabs, seen_when: false });
            self.tab_up = true;
        }
        if let Some(keyword @ ("else" | "elsif" | "when")) = keyword.filter(|_| silent) {
            self.check_branch_level(keyword, tabs)?;
        }
        let code_text = code.trim_start_matches(REGEXP_SPACE);
        let mut chunks = vec![self.chunk(self.line.index, code_text, self.line_column(code_text, text))];
        chunks.extend(continued);
        self.line_block = Some(RubyBlock {
            output: (!silent).then_some(self.pieces.len()),
            ends: true,
            case: keyword == Some("case"),
            when: keyword == Some("when"),
            adopted: false,
        });
        self.emit(if silent { Piece::Statement(chunks) } else { Piece::Output { code: chunks, block: false } });
        Ok(if opens_branch { Node::Branch } else { Node::Other })
    }

    fn check_branch_level(&mut self, keyword: &str, tabs: usize) -> Result<(), Halt> {
        let Some(level) = self.script_levels.last_mut() else {
            return Err(halt(format!("Got \"{keyword}\" with no preceding \"if\""), self.line.index));
        };
        // The first `when` may stand one level deeper than its `case`.
        if keyword == "when" && !level.seen_when {
            if level.tabs + 1 == tabs {
                level.tabs += 1;
            }
            level.seen_when = true;
        }
        if level.tabs != tabs {
            let message = format!("\"{keyword}\" is indented at wrong level: expected {}, but was at {tabs}.", level.tabs);
            return Err(halt(message, self.line.index));
        }
        Ok(())
    }

    /// `parse_tag` and `tag`: attribute groups that run over several lines, the nesting rules, then Ruby after the tag
    /// that continues after a comma.
    fn tag(&mut self, text: &str) -> Result<(), Halt> {
        let Some((name, attributes_start, head_len)) = tag_head(text) else {
            return Err(halt(format!("Invalid tag: \"{text}\"."), self.line.index));
        };
        if has_empty_class_or_id(&text[attributes_start..head_len]) {
            return Err(halt(ILLEGAL_ELEMENT, self.line.index));
        }
        let mut rest =
            Rest { text: text[head_len..].to_string(), line: self.line.index, column: self.line_column(&text[head_len..], text) };
        let (mut hash, mut list, mut reference) = (false, false, false);
        // The line the last attribute group ends on, which Haml reports some errors at.
        let mut last_line = self.line.index;
        let (mut object, mut list_hash, mut hash_code) = (Vec::new(), Vec::new(), Vec::new());
        loop {
            match rest.text.chars().next() {
                Some('{') if !hash => {
                    hash = true;
                    (rest, last_line, hash_code) = self.skip_hash(&rest)?;
                }
                Some('(') if !list => {
                    list = true;
                    (rest, last_line, list_hash) = self.skip_attribute_list(&rest)?;
                }
                Some('[') if !reference => {
                    reference = true;
                    let after = balance_brackets(&rest.text).ok_or_else(|| halt(UNBALANCED, self.line.index))?;
                    let consumed = rest.text.len() - after.len();
                    object = vec![self.chunk(rest.line, &rest.text[..consumed], rest.column)];
                    rest = Rest { text: after.to_string(), column: rest.column + consumed, ..rest };
                }
                _ => break,
            }
        }
        let content = tag_content(&rest.text);
        // This reads the next line's indentation before any Ruby lines are joined.
        let block_opened = self.block_opened()?;
        if block_opened && content.self_closing {
            return Err(self.nested_line_error(NESTING_SELF_CLOSING));
        }
        if let Some(action) = content.action.filter(|_| content.parse && content.value.is_empty()) {
            return Err(halt(no_ruby_code(action), last_line));
        }
        if content.self_closing && !content.value.is_empty() {
            return Err(halt(SELF_CLOSING_CONTENT, last_line));
        }
        if block_opened && !content.value.is_empty() && !is_ruby_multiline(&content.value) {
            let message = format!("Illegal nesting: content can't be both given on the same line as %{name} and nested within it.");
            return Err(self.nested_line_error(&message));
        }
        if VOID_ELEMENTS.contains(&name.to_ascii_lowercase().as_str()) && !content.self_closing {
            if !content.value.is_empty() {
                self.void_content.push(VoidContent { line: self.line.index, name: name.to_string(), nested: false });
            } else if block_opened {
                self.void_content.push(VoidContent { line: self.next_line.index, name: name.to_string(), nested: true });
            }
        }
        let continued = if content.parse { self.join_ruby_lines(&content.value)? } else { Vec::new() };
        if !list_hash.is_empty() && !hash_code.is_empty() {
            list_hash.push(Chunk::synthetic(", ", self.line.index));
        }
        list_hash.extend(hash_code);
        if !object.is_empty() || !list_hash.is_empty() {
            self.emit(Piece::Attributes { object, arguments: list_hash });
        }
        if let Some((source, range)) = content.source {
            let value = &rest.text[range.clone()];
            let column = rest.column + range.start;
            match source {
                Source::Text => self.push_text(value, rest.line, column),
                Source::Code => {
                    let mut code = vec![self.chunk(rest.line, value, column)];
                    code.extend(continued);
                    self.line_block =
                        Some(RubyBlock { output: Some(self.pieces.len()), ends: false, case: false, when: false, adopted: false });
                    self.emit(Piece::Output { code, block: false });
                }
            }
        }
        Ok(())
    }

    /// `parse_old_attributes`: what follows the `{...}` that starts `rest`, which may close on a later line, and the
    /// Ruby between the braces.
    fn skip_hash(&mut self, rest: &Rest) -> Result<(Rest, usize, Vec<Chunk>), Halt> {
        let mut scanner = BraceScanner::new();
        let mut segment = rest.text[1..].to_string();
        let (mut line, mut column) = (rest.line, rest.column + 1);
        let mut last_line = self.line.index;
        let mut code = Vec::new();
        loop {
            if let Some(end) = scanner.feed(&segment) {
                code.push(self.chunk(line, &segment[..end - 1], column));
                let after = Rest { text: segment[end..].to_string(), line, column: column + end };
                return Ok((after, last_line, code));
            }
            if self.next_line.end_of_document {
                return Err(halt(UNBALANCED, self.line.index));
            }
            code.push(self.chunk(line, &segment, column));
            code.push(Chunk::synthetic("\n", line));
            segment = self.next_line.text.clone();
            (line, column) = (self.next_line.index, self.next_line.indent());
            self.join_line(self.next_line.index, self.line.index, &segment);
            last_line += 1;
            self.next_line()?;
        }
    }

    /// `parse_new_attributes`: what follows the `(...)` that starts `rest`, whose attributes may continue on later lines,
    /// and the hash of its Ruby attributes (`{"title" => "x#{y}",}`), empty when there are none.
    fn skip_attribute_list(&mut self, rest: &Rest) -> Result<(Rest, usize, Vec<Chunk>), Halt> {
        let mut joined = rest.text.clone();
        let mut segment = rest.text[1..].trim_start_matches(REGEXP_SPACE).to_string();
        let (mut line, mut column) = (rest.line, rest.column + rest.text.len() - segment.len());
        let mut continuations = 0;
        let mut hash = Vec::new();
        loop {
            let mut dynamic = Vec::new();
            let scanned = scan_attribute_list(&segment, &mut dynamic);
            if self.ruby {
                for attribute in &dynamic {
                    hash.extend(self.dynamic_attribute(&segment, line, column, attribute));
                }
            }
            match scanned {
                AttributeList::Closed(end) => {
                    let after = Rest { text: segment[end..].to_string(), line, column: column + end };
                    if !hash.is_empty() {
                        hash.insert(0, Chunk::synthetic("{", rest.line));
                        hash.push(Chunk::synthetic("}", rest.line));
                    }
                    return Ok((after, self.line.index + continuations, hash));
                }
                AttributeList::Unbalanced => return Err(halt(UNBALANCED, self.line.index)),
                AttributeList::Invalid => {
                    let fragment = balance_parentheses(&joined).unwrap_or(&joined);
                    return Err(halt(format!("Invalid attribute list: {}.", ruby_inspect(fragment)), self.line.index + continuations));
                }
                AttributeList::Continues => {
                    joined.push(' ');
                    joined.push_str(&self.next_line.text);
                    segment = self.next_line.text.clone();
                    (line, column) = (self.next_line.index, self.next_line.indent());
                    self.join_line(self.next_line.index, self.line.index, &segment);
                    continuations += 1;
                    self.counted_at_end = true;
                    self.next_line()?;
                }
            }
        }
    }

    /// `"name" => value,` for an attribute of a `(...)` list, as Haml puts it in the hash it passes at run time.
    fn dynamic_attribute(&self, segment: &str, line: usize, column: usize, attribute: &DynamicAttribute) -> Vec<Chunk> {
        let name = format!("\"{}\" => ", &segment[attribute.name.clone()]);
        let value = &segment[attribute.value.clone()];
        let at = column + attribute.value.start;
        let ruby = match value.strip_prefix(['"', '\'']) {
            Some(quoted) => {
                let inner = &quoted[..quoted.len() - 1];
                self.interpolated(inner, &[(0, line, at + 1)], false)
            }
            None => Some(vec![self.chunk(line, value, at)]),
        };
        let Some(ruby) = ruby else { return Vec::new() };
        let mut chunks = vec![Chunk::synthetic(&name, line)];
        chunks.extend(ruby);
        chunks.push(Chunk::synthetic(",", line));
        chunks
    }

    /// `handle_ruby_multiline`: lines after one ending in a comma belong to it. Returns them as Ruby, each after a space.
    fn join_ruby_lines(&mut self, code: &str) -> Result<Vec<Chunk>, Halt> {
        let mut chunks = Vec::new();
        if !is_ruby_multiline(code.trim_end_matches(RUBY_WHITESPACE)) {
            return Ok(chunks);
        }
        let continues = |line: &Line| !line.end_of_document && is_ruby_multiline(&line.text);
        let owner = self.line.index;
        if !self.next_line.end_of_document {
            let next = self.next_line.clone();
            self.join_line(next.index, owner, &next.text);
            chunks.extend(self.continuation(&next));
        }
        let mut joined_continues = continues(&self.next_line);
        while joined_continues {
            let Some(next) = self.template.pop_front() else { break };
            if !next.end_of_document {
                self.join_line(next.index, owner, &next.text);
                chunks.extend(self.continuation(&next));
            }
            joined_continues = continues(&next);
        }
        self.next_line()?;
        Ok(chunks)
    }

    /// The Ruby a joined line adds: a space and its text.
    fn continuation(&self, line: &Line) -> Vec<Chunk> {
        if line.text.is_empty() {
            return Vec::new();
        }
        vec![Chunk::synthetic(" ", line.index), self.chunk(line.index, &line.text, line.indent())]
    }
}

/// The Ruby Haml compiles, collected as the lines are read.
impl Parser {
    fn emit(&mut self, piece: Piece) {
        if self.ruby {
            self.pieces.push(piece);
        }
    }

    /// `text` on `line`, `column` bytes from the line's start: a slice of the template, unless `|` multiline rewrote the line.
    fn chunk(&self, line: usize, text: &str, column: usize) -> Chunk {
        if !self.ruby {
            return Chunk::synthetic("", line);
        }
        let rewritten = self.pipe_joined.get(line).copied().unwrap_or(false);
        let offset = self.line_starts.get(line).filter(|_| !rewritten).map(|start| start + column);
        Chunk { text: text.to_string(), line, offset }
    }

    /// The column of `part`, a slice of `text`, the current line without its indentation.
    fn line_column(&self, part: &str, text: &str) -> usize {
        self.line.indent() + span_in(text, part).start
    }

    /// The string literal Haml makes of plain text, a comment or a tag's content when it has `#{}`.
    fn push_text(&mut self, text: &str, line: usize, column: usize) {
        if let Some(literal) = self.interpolated(text, &[(0, line, column)], true) {
            self.emit(Piece::Text(literal));
        }
    }

    /// The body of the filter that just ended, as the Ruby Haml compiles it.
    fn finish_filter(&mut self) {
        let body = std::mem::replace(&mut self.filter, FilterBody::None);
        if !self.ruby || body == FilterBody::None {
            return;
        }
        // (line, text, length of the indentation that is the body's prefix) of each line of the body.
        let lines: Vec<(usize, &str, usize)> = self.compiled[self.filter_start..]
            .iter()
            .filter_map(|node| match node {
                Compiled::Flat { index, text } => {
                    let prefix_len = match self.roles.get(*index) {
                        Some(Role::FlatBody { prefix_len, .. }) => *prefix_len,
                        _ => 0,
                    };
                    Some((*index, text.as_str(), prefix_len))
                }
                Compiled::Node { .. } => None,
            })
            .collect();
        let piece = match body {
            FilterBody::None => None,
            FilterBody::Ruby => {
                let mut code = Vec::new();
                for &(line, text, prefix_len) in &lines {
                    code.push(self.chunk(line, text, prefix_len));
                    code.push(Chunk::synthetic("\n", line));
                }
                Some(Piece::Statement(code))
            }
            FilterBody::Text => {
                let mut body = String::new();
                let mut segments = Vec::new();
                for &(line, text, prefix_len) in &lines {
                    segments.push((body.len(), line, prefix_len));
                    body.push_str(text);
                    body.push('\n');
                }
                body.pop();
                self.interpolated(&body, &segments, true).map(Piece::Text)
            }
        };
        if let Some(piece) = piece.filter(|_| !lines.is_empty()) {
            self.pieces.push(piece);
        }
    }

    /// Haml's `unescape_interpolation`: the Ruby string literal for `text` when it has `#{}`, and `#@x` or `#$x` for
    /// `variables`. `text` is lines joined by `\n`; `segments` say where each starts in it and in the template (as
    /// start, line and column). Nothing when there is no interpolation, or when one never closes, which Haml fails on
    /// by itself.
    fn interpolated(&self, text: &str, segments: &[(usize, usize, usize)], variables: bool) -> Option<Vec<Chunk>> {
        if !self.ruby {
            return None;
        }
        let first_line = segments.first()?.1;
        let mut chunks = vec![Chunk::synthetic("\"", first_line)];
        let (mut copied, mut at, mut found_any) = (0, 0, false);
        while let Some(found) = text[at..].find('#') {
            let hash = at + found;
            let sigil = text[hash + 1..].chars().next().filter(|ch| *ch == '{' || variables && matches!(ch, '@' | '$'));
            let backslashes = text[..hash].len() - text[..hash].trim_end_matches('\\').len();
            let Some(sigil) = sigil.filter(|_| backslashes % 2 == 0) else {
                at = hash + 1 + usize::from(sigil.is_some());
                continue;
            };
            let (from, to, next) = if sigil == '{' {
                let close = closing_brace(&text[hash + 2..])? + hash + 2;
                (hash + 2, close, close + 1)
            } else {
                let word = text[hash + 2..].find(|ch: char| !is_word(ch)).unwrap_or(text.len() - hash - 2);
                (hash + 1, hash + 2 + word, hash + 2 + word)
            };
            let line = segments[segments.partition_point(|segment| segment.0 <= hash).saturating_sub(1)].1;
            chunks.push(Chunk::synthetic(&ruby_string_body(&text[copied..hash]), line));
            chunks.push(Chunk::synthetic("#{", line));
            chunks.extend(self.slice_chunks(text, segments, from, to));
            chunks.push(Chunk::synthetic("}", line));
            (copied, at) = (next, next);
            found_any = true;
        }
        if !found_any {
            return None;
        }
        chunks.push(Chunk::synthetic(&ruby_string_body(&text[copied..]), first_line));
        chunks.push(Chunk::synthetic("\"", first_line));
        Some(chunks)
    }

    /// `text[from..to]` as the chunks of the template lines it spans.
    fn slice_chunks(&self, text: &str, segments: &[(usize, usize, usize)], from: usize, to: usize) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        let first = segments.partition_point(|segment| segment.0 <= from).saturating_sub(1);
        for (i, &(start, line, column)) in segments.iter().enumerate().skip(first) {
            if start > to {
                break;
            }
            let end = segments.get(i + 1).map_or(text.len(), |next| next.0 - 1);
            if from > end {
                continue;
            }
            if !chunks.is_empty() {
                chunks.push(Chunk::synthetic("\n", line));
            }
            let (a, b) = (from.max(start), to.min(end));
            chunks.push(self.chunk(line, &text[a..b], column + a - start));
        }
        chunks
    }
}

/// Where the `}` that closes an interpolation is in `text`, which follows its `#{`: Haml's `balance`, which counts only
/// the braces.
fn closing_brace(text: &str) -> Option<usize> {
    let mut depth = 1;
    text.char_indices().find_map(|(at, ch)| {
        depth += match ch {
            '{' => 1,
            '}' => -1,
            _ => 0,
        };
        (depth == 0).then_some(at)
    })
}

/// Where the text of a plain line starts: Haml's `strip!` drops the `\`, `&`, `!` or `==` that marks it.
fn plain_text_start(text: &str) -> usize {
    let after = |marker_len: usize| text.len() - text[marker_len..].trim_start_matches(' ').len();
    if text.starts_with("!==") || text.starts_with("&==") {
        after(3)
    } else if text.starts_with("==") {
        after(2)
    } else if text.starts_with(['!', '&']) && (text[1..].starts_with(' ') || text[1..].starts_with("#{")) {
        after(1)
    } else if text.starts_with('\\') {
        1
    } else {
        0
    }
}

fn is_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// The element name, where its `.class#id` part starts and where the head ends (Haml's `%([-:\w]+)([-:\w.#\@]*)`); a line
/// that starts with `.` or `#` is a `div` and only has the `.class#id` part.
fn tag_head(text: &str) -> Option<(&str, usize, usize)> {
    let (name, name_end) = if text.starts_with('%') {
        // Haml's regexp is unanchored: `%%p` is the element `p`.
        let start =
            text.char_indices().find_map(|(at, ch)| (ch == '%' && text[at + 1..].starts_with(is_tag_name_char)).then_some(at + 1))?;
        let name_len = text[start..].find(|ch: char| !is_tag_name_char(ch)).unwrap_or(text.len() - start);
        (&text[start..start + name_len], start + name_len)
    } else {
        ("div", 0)
    };
    let attributes = &text[name_end..];
    let attributes_len = attributes.find(|ch: char| !(is_tag_name_char(ch) || matches!(ch, '.' | '#' | '@'))).unwrap_or(attributes.len());
    Some((name, name_end, name_end + attributes_len))
}

fn is_tag_name_char(ch: char) -> bool {
    is_word(ch) || matches!(ch, '-' | ':')
}

/// Haml's `/[.#](\.|#|\z)/` over `.class#id`: a `.` or `#` followed by another or by nothing.
fn has_empty_class_or_id(attributes: &str) -> bool {
    attributes
        .char_indices()
        .any(|(at, ch)| matches!(ch, '.' | '#') && attributes[at + 1..].chars().next().is_none_or(|next| matches!(next, '.' | '#')))
}

/// What follows the `[...]` that starts `text` (Haml's `balance`, which counts only the brackets).
fn balance_brackets(text: &str) -> Option<&str> {
    let mut depth = 0;
    for (at, ch) in text.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => depth -= 1,
            _ => continue,
        }
        if depth == 0 {
            return Some(&text[at + 1..]);
        }
    }
    None
}

fn no_ruby_code(action: char) -> String {
    format!("There's no Ruby code for {action} to evaluate.")
}

/// What follows a tag's attributes, as `tag` reads it.
struct TagContent {
    action: Option<char>,
    self_closing: bool,
    /// Haml evaluates `value` (`%p= x`, `%p~ x`, `%p& = x`).
    parse: bool,
    /// The text or code; text with `#{}` or `==` is a string literal in Haml, which stands as `""` here.
    value: String,
    /// The Ruby Haml compiles, and where it is in the text the tag content was read from.
    source: Option<(Source, std::ops::Range<usize>)>,
}

/// Text that follows a tag.
enum Source {
    /// Code whose value is written.
    Code,
    /// Text with `#{}`.
    Text,
}

/// The rest of a tag's line, as the attribute groups leave it: where in the template it is, in `line` at byte `column`.
#[derive(Clone)]
struct Rest {
    text: String,
    line: usize,
    column: usize,
}

fn tag_content(rest: &str) -> TagContent {
    let stripped = rest.strip_prefix("<>").or_else(|| rest.strip_prefix("><")).or_else(|| rest.strip_prefix(['<', '>'])).unwrap_or(rest);
    let action = stripped.chars().next().filter(|ch| "=/~&!".contains(*ch));
    let value = stripped[action.map_or(0, char::len_utf8)..].trim_matches(RUBY_WHITESPACE);
    let interpolated = || "\"\"".to_string();
    let has_interpolation = ["#{", "#$", "#@"].iter().any(|opener| value.contains(opener));
    let trimmed = |text: &'_ str| text.trim_matches(RUBY_WHITESPACE).to_string();
    let source_of = |kind, part: &str| Some((kind, span_in(rest, part.trim_matches(RUBY_WHITESPACE))));
    let (parse, text, source) = match action {
        Some('/') => {
            return TagContent { action, self_closing: true, parse: false, value: value.to_string(), source: None };
        }
        Some('~') => (true, value.to_string(), source_of(Source::Code, value)),
        Some('=') if value.starts_with('=') => (true, interpolated(), source_of(Source::Text, &value[1..])),
        Some('=') => (true, value.to_string(), source_of(Source::Code, value)),
        Some('&' | '!') => match value.strip_prefix(['=', '~']) {
            Some(code) if code.starts_with('=') => (true, interpolated(), source_of(Source::Text, &code[1..])),
            Some(code) => (true, trimmed(code), source_of(Source::Code, code)),
            None if has_interpolation => (true, interpolated(), source_of(Source::Text, value)),
            None => (false, value.to_string(), None),
        },
        _ if has_interpolation => (true, interpolated(), source_of(Source::Text, value)),
        _ => (false, value.to_string(), None),
    };
    TagContent { action, self_closing: false, parse, value: text, source }
}

/// The byte range of `part` in `whole`, which it is a slice of.
fn span_in(whole: &str, part: &str) -> std::ops::Range<usize> {
    let start = part.as_ptr() as usize - whole.as_ptr() as usize;
    start..start + part.len()
}

enum AttributeList {
    /// Offset just past the `)`.
    Closed(usize),
    /// The line ended between attributes.
    Continues,
    /// Haml's "Invalid attribute list".
    Invalid,
    /// An interpolation in a value never closes.
    Unbalanced,
}

/// An attribute of a `(...)` list whose value is Ruby: a variable, or a quoted string with `#{}`.
struct DynamicAttribute {
    name: std::ops::Range<usize>,
    value: std::ops::Range<usize>,
}

/// `parse_new_attribute` in a loop over one line of `(a=1 b="x#{y}" c)`, starting at an attribute. The attributes whose
/// values are Ruby go to `dynamic`.
fn scan_attribute_list(segment: &str, dynamic: &mut Vec<DynamicAttribute>) -> AttributeList {
    let mut at = 0;
    loop {
        let rest = &segment[at..];
        let name_len = rest.find(|ch: char| !(is_word(ch) || matches!(ch, '-' | ':' | '@' | '#' | '.'))).unwrap_or(rest.len());
        if name_len == 0 {
            return if rest.starts_with(')') { AttributeList::Closed(at + 1) } else { AttributeList::Invalid };
        }
        let name = at..at + name_len;
        let after_name = rest[name_len..].trim_start_matches(REGEXP_SPACE);
        at = segment.len() - after_name.len();
        if let Some(after_equals) = after_name.strip_prefix('=') {
            let value = after_equals.trim_start_matches(REGEXP_SPACE);
            match attribute_value_len(value) {
                Ok((value_len, ruby)) => {
                    let start = segment.len() - value.len();
                    if ruby {
                        dynamic.push(DynamicAttribute { name, value: start..start + value_len });
                    }
                    at = start + value_len;
                }
                Err(failure) => return failure,
            }
        }
        at = segment.len() - segment[at..].trim_start_matches(REGEXP_SPACE).len();
        if at == segment.len() {
            return AttributeList::Continues;
        }
    }
}

/// Length of a quoted string (with `#{}` interpolation) or a variable at the start of `value`, and whether it is Ruby (a
/// variable, or a string with `#{}`); the failure otherwise.
fn attribute_value_len(value: &str) -> Result<(usize, bool), AttributeList> {
    let Some(quote @ ('"' | '\'')) = value.chars().next() else {
        let variable = ["@@", "@", "$"].iter().find_map(|prefix| value.strip_prefix(prefix)).unwrap_or(value);
        let name_len = variable.find(|ch: char| !is_word(ch)).unwrap_or(variable.len());
        return if name_len > 0 { Ok((value.len() - variable.len() + name_len, true)) } else { Err(AttributeList::Invalid) };
    };
    let mut at = 1;
    let mut interpolated = false;
    while let Some(ch) = value[at..].chars().next() {
        let after = &value[at + ch.len_utf8()..];
        match ch {
            '\\' => match after.chars().next() {
                Some(escaped) => at += 1 + escaped.len_utf8(),
                None => break,
            },
            '#' if after.starts_with('{') => {
                let close = closing_brace(&after[1..]);
                let Some(close) = close else { return Err(AttributeList::Unbalanced) };
                interpolated = true;
                at += 2 + close + 1;
            }
            ch if ch == quote => return Ok((at + 1, interpolated)),
            _ => at += ch.len_utf8(),
        }
    }
    Err(AttributeList::Invalid)
}

/// What Haml prints for a string in an error: Ruby's `String#inspect`.
fn ruby_inspect(text: &str) -> String {
    let mut out = String::from("\"");
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '#' if matches!(chars.peek(), Some('{' | '$' | '@')) => out.push_str("\\#"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\x1b' => out.push_str("\\e"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04X}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Haml's `balance` over `(` and `)`: the text up to the parenthesis that closes the first one, stripped.
fn balance_parentheses(text: &str) -> Option<&str> {
    let mut depth = 0;
    for (at, ch) in text.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => continue,
        }
        if depth == 0 {
            return Some(text[..at + 1].trim_matches(RUBY_WHITESPACE));
        }
    }
    None
}

/// `<<~ID`, `<<-ID`, `<<ID` and the quoted forms at `chars[at]`: the terminator, whether the body interpolates and the
/// index after the opener.
fn heredoc_start(chars: &[(usize, char)], at: usize) -> Option<(String, bool, usize)> {
    let mut i = at + 2;
    if matches!(chars.get(i), Some((_, '~' | '-'))) {
        i += 1;
    }
    let &(_, first) = chars.get(i)?;
    if matches!(first, '\'' | '"' | '`') {
        let length = chars[i + 1..].iter().position(|&(_, ch)| ch == first)?;
        let id: String = chars[i + 1..i + 1 + length].iter().map(|&(_, ch)| ch).collect();
        return Some((id, first != '\'', i + length + 2));
    }
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let length = chars[i..].iter().take_while(|&&(_, ch)| is_word(ch)).count();
    Some((chars[i..i + length].iter().map(|&(_, ch)| ch).collect(), true, i + length))
}

#[derive(Clone, Copy)]
enum Mode {
    Code {
        braces: usize,
    },
    Text {
        closer: char,
        opener: Option<char>,
        nested: usize,
        interpolates: bool,
    },
    /// The body of a heredoc: the lines up to one that is just its terminator (`heredoc_ids[id]`).
    Heredoc {
        id: usize,
        interpolates: bool,
    },
}

/// Finds the `}` that closes a Ruby `{` over several lines, as Haml does by lexing the text with Prism: a `}` inside a
/// string, comment, `%w()` literal, regexp or `?}` doesn't count. Lines are fed once each.
struct BraceScanner {
    modes: Vec<Mode>,
    /// The last character of code that isn't whitespace.
    previous: Option<char>,
    /// The last word of code, and whether the character just read continues it.
    word: String,
    in_word: bool,
    heredoc_ids: Vec<String>,
    /// Heredocs started on this line, whose bodies begin on the next.
    pending_heredocs: Vec<Mode>,
}

/// After these a value starts, so `/` and `%` begin a literal.
const KEYWORDS_BEFORE_VALUE: [&str; 15] =
    ["if", "unless", "while", "until", "and", "or", "not", "return", "when", "then", "do", "else", "elsif", "in", "begin"];

impl BraceScanner {
    /// Starts just after the opening `{`.
    fn new() -> BraceScanner {
        BraceScanner {
            modes: vec![Mode::Code { braces: 0 }],
            previous: Some('{'),
            word: String::new(),
            in_word: false,
            heredoc_ids: Vec::new(),
            pending_heredocs: Vec::new(),
        }
    }

    /// A value (string, regexp, literal) can start here, rather than an operator.
    fn value_expected(&self) -> bool {
        self.previous.is_none_or(|ch| "([{,;=:|&!<>?+-*/%^~".contains(ch))
            || self.previous.is_some_and(is_word) && KEYWORDS_BEFORE_VALUE.contains(&self.word.as_str())
    }

    /// The offset just past the closing `}` when this line holds it.
    fn feed(&mut self, line: &str) -> Option<usize> {
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        // A line break ends a statement, so what follows starts a value.
        if matches!(self.modes.last(), Some(Mode::Code { .. })) && self.previous.is_some_and(|ch| is_word(ch) || ")]}\"".contains(ch)) {
            self.previous = Some(';');
        }
        let mut i = 0;
        if let Some(&Mode::Heredoc { id, .. }) = self.modes.last() {
            if line == self.heredoc_ids[id] {
                self.modes.pop();
                self.previous = Some('"');
                return None;
            }
        }
        if matches!(self.modes.last(), Some(Mode::Text { closer: '\n', .. })) {
            self.modes.pop();
            self.previous = Some('"');
            return None;
        }
        let mut spaced = false;
        while i < chars.len() {
            let (at, ch) = chars[i];
            let next = chars.get(i + 1).map(|&(_, next)| next);
            match *self.modes.last()? {
                Mode::Text { closer, opener, nested, interpolates } => {
                    i += 1;
                    if ch == '\\' {
                        i += 1;
                    } else if interpolates && ch == '#' && next == Some('{') {
                        self.modes.push(Mode::Code { braces: 0 });
                        self.previous = Some('{');
                        i += 1;
                    } else if ch == closer && nested == 0 {
                        self.modes.pop();
                        self.previous = Some('"');
                    } else if ch == closer {
                        self.set_nested(nested - 1);
                    } else if Some(ch) == opener {
                        self.set_nested(nested + 1);
                    }
                }
                Mode::Heredoc { interpolates, .. } => {
                    i += 1;
                    if ch == '\\' {
                        i += 1;
                    } else if interpolates && ch == '#' && next == Some('{') {
                        self.modes.push(Mode::Code { braces: 0 });
                        self.previous = Some('{');
                        i += 1;
                    }
                }
                Mode::Code { braces } => {
                    i += 1;
                    // `puts /x/` and `link_to %w(a)`: after a word and a space, a `/` or `%` that isn't followed by a space starts a literal.
                    let after_word_and_space = spaced
                        && self.previous.is_some_and(is_word)
                        && !self.word.starts_with(|digit: char| digit.is_ascii_digit())
                        && next.is_some_and(|following| !following.is_whitespace() && following != '=');
                    let expected = self.value_expected();
                    let literal_expected = expected || after_word_and_space;
                    match ch {
                        ' ' | '\t' => {}
                        '#' => break,
                        '"' | '`' | '\'' => {
                            self.modes.push(Mode::Text { closer: ch, opener: None, nested: 0, interpolates: ch != '\'' });
                        }
                        '/' if literal_expected => {
                            self.modes.push(Mode::Text { closer: '/', opener: None, nested: 0, interpolates: true });
                        }
                        '%' if literal_expected => i = self.percent_literal(&chars, i - 1),
                        '<' if next == Some('<') => match heredoc_start(&chars, i - 1) {
                            Some((id, interpolates, end))
                                if literal_expected || chars[i + 1..end].iter().any(|&(_, c)| c == '~' || c == '-') =>
                            {
                                self.heredoc_ids.push(id);
                                self.pending_heredocs.push(Mode::Heredoc { id: self.heredoc_ids.len() - 1, interpolates });
                                self.previous = Some('"');
                                i = end;
                            }
                            _ => self.previous = Some(ch),
                        },
                        '?' if expected && next.is_some_and(|c| !c.is_whitespace()) => {
                            i += if next == Some('\\') { 2 } else { 1 };
                            self.previous = Some('"');
                        }
                        '{' => {
                            self.set_braces(braces + 1);
                            self.previous = Some(ch);
                        }
                        '}' if braces > 0 => {
                            self.set_braces(braces - 1);
                            self.previous = Some(ch);
                        }
                        '}' => {
                            self.modes.pop();
                            if self.modes.is_empty() {
                                return Some(at + 1);
                            }
                        }
                        _ => {
                            self.previous = Some(ch);
                            if is_word(ch) && !self.in_word {
                                self.word.clear();
                            }
                            if is_word(ch) {
                                self.word.push(ch);
                            }
                        }
                    }
                    self.in_word = is_word(ch);
                    spaced = ch == ' ' || ch == '\t';
                }
            }
        }
        self.modes.extend(self.pending_heredocs.drain(..).rev());
        None
    }

    /// `%w(`, `%(`, `%q|`: pushes the literal and returns the index after its delimiter; a lone `%` is an operator.
    fn percent_literal(&mut self, chars: &[(usize, char)], percent: usize) -> usize {
        let kind = chars.get(percent + 1).map(|&(_, ch)| ch).filter(|ch| "wWiIqQrsx".contains(*ch));
        let delimiter_at = percent + 1 + usize::from(kind.is_some());
        if delimiter_at == chars.len() {
            // `%` at the end of a line: the next line is the string, but a word list with a newline delimiter never ends.
            let closer = if matches!(kind, Some('w' | 'W' | 'i' | 'I')) { '\0' } else { '\n' };
            self.modes.push(Mode::Text { closer, opener: None, nested: 0, interpolates: false });
            return delimiter_at;
        }
        let Some(&(_, delimiter)) = chars.get(delimiter_at).filter(|(_, ch)| !ch.is_alphanumeric()) else {
            self.previous = Some('%');
            return percent + 1;
        };
        let (closer, opener) = match delimiter {
            '(' => (')', Some('(')),
            '[' => (']', Some('[')),
            '{' => ('}', Some('{')),
            '<' => ('>', Some('<')),
            other => (other, None),
        };
        let interpolates = !matches!(kind, Some('w' | 'i' | 'q' | 's'));
        self.modes.push(Mode::Text { closer, opener, nested: 0, interpolates });
        delimiter_at + 1
    }

    fn set_nested(&mut self, value: usize) {
        if let Some(Mode::Text { nested, .. }) = self.modes.last_mut() {
            *nested = value;
        }
    }

    fn set_braces(&mut self, value: usize) {
        if let Some(Mode::Code { braces }) = self.modes.last_mut() {
            *braces = value;
        }
    }
}

fn block_keyword(code: &str) -> Option<&str> {
    let caps = BLOCK_KEYWORD.captures(code)?;
    caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str())
}

/// Haml's `is_ruby_multiline?`: ends in `,`, except the character literals `?,` and `?\,`.
pub(crate) fn is_ruby_multiline(text: &str) -> bool {
    let b = text.as_bytes();
    let n = b.len();
    if n < 2 || b[n - 1] != b',' {
        return false;
    }
    let char_literal = n >= 3
        && ((b[n - 2] == b'?' && !(b[n - 3].is_ascii_alphanumeric() || b[n - 3] == b'_')) || (b[n - 3] == b'?' && b[n - 2] == b'\\'));
    !char_literal
}

pub(crate) fn strip_output_prefix(s: &str) -> Option<&str> {
    // `==`, `&==`, `!==` are interpolated plain text, not Ruby.
    if ["==", "&==", "!=="].iter().any(|p| s.starts_with(p)) {
        return None;
    }
    ["!=", "&=", "!~", "&~", "=", "~"].iter().find_map(|p| s.strip_prefix(p))
}

/// Haml's `is_multiline?`: ends in ` |`, unless it's a block argument list (`do | x |`).
pub(crate) fn is_haml_multiline(line: &str) -> bool {
    let t = line.trim_end_matches(RUBY_WHITESPACE);
    t.ends_with(" |") && !BLOCK_WITH_SPACES.is_match(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error_of(text: &str) -> Option<(usize, String)> {
        parse(text).error.map(|e| (e.line, e.message))
    }

    fn inconsistent(line: usize, used: &str, rest: &str) -> Option<(usize, String)> {
        let message =
            format!("Inconsistent indentation: {used} used for indentation, but the rest of the document was indented using {rest}.");
        Some((line, message))
    }

    fn deeper(line: usize, levels: usize) -> Option<(usize, String)> {
        Some((line, format!("The line was indented {levels} levels deeper than the previous line.")))
    }

    fn mixed(line: usize) -> Option<(usize, String)> {
        Some((line, MIXED_INDENTATION.to_string()))
    }

    /// Each case was checked against Haml 7.5.1 (`test/oracle/haml.rb`).
    #[test]
    fn indentation_errors_at_haml_lines() {
        let cases: Vec<(&str, Option<(usize, String)>)> = vec![
            ("%div\n  %p\n\t%b", inconsistent(2, "1 tab", "2 spaces")),
            ("%div\n\t%p\n  %b", inconsistent(2, "2 spaces", "1 tab")),
            ("%div\n  %p\n    %b\n   %i", inconsistent(3, "3 spaces", "2 spaces")),
            ("%div\n  %p\n      %b", deeper(2, 2)),
            ("%p\n  a\n   b", inconsistent(2, "3 spaces", "2 spaces")),
            ("%div\n    %p\n  %b", inconsistent(2, "2 spaces", "4 spaces")),
            ("%div\n  %p\n\n\n    %b\n   x", inconsistent(5, "3 spaces", "2 spaces")),
            ("%div\n  :javascript\n   a\n  %p", inconsistent(2, "3 spaces", "2 spaces")),
            ("%p\n  :plain\n\ta\n  %b", inconsistent(2, "1 tab", "2 spaces")),
            ("%p a |\n    b |\n%div\n  %p\n   %b", inconsistent(4, "3 spaces", "2 spaces")),
            ("%p= foo(1,\n 2)\n%div\n  %p", deeper(3, 2)),
            ("\u{feff}%p\n  %b\n x", inconsistent(2, "1 space", "2 spaces")),
            ("\u{feff} %p", Some((0, AT_START.to_string()))),
            ("%div\n \t%p", mixed(1)),
            ("%div\n\t %p", mixed(1)),
            ("%p\n  %a{b: 1,\n   c: 2}\n   %x", inconsistent(3, "3 spaces", "2 spaces")),
            ("%p\n  %a(b=1\n  c=2)\n   %x", inconsistent(3, "3 spaces", "2 spaces")),
            ("- if a\n  %p\n - else\n  %p", inconsistent(2, "1 space", "2 spaces")),
            ("-# c\n  x\n y\n%p", inconsistent(2, "1 space", "2 spaces")),
            ("%p\r\n  a\r\n   b\r\n", inconsistent(2, "3 spaces", "2 spaces")),
            ("/\n  a\n   b\n    c", inconsistent(2, "3 spaces", "2 spaces")),
            ("%div\n  %p\n    %b\n  x\n y", inconsistent(4, "1 space", "2 spaces")),
            (":plain\n  a\n  \tb\n c", inconsistent(3, "1 space", "2 spaces")),
            ("%a\n    %b\n  %c", inconsistent(2, "2 spaces", "4 spaces")),
            ("%a\n  %b\n    %c\n %d", inconsistent(3, "1 space", "2 spaces")),
            ("%a\n  - x = 1\n   y", inconsistent(2, "3 spaces", "2 spaces")),
        ];
        for (text, expected) in cases {
            assert_eq!(error_of(text), expected, "{text:?}");
        }
    }

    #[test]
    fn flat_sections_and_joined_lines_are_not_indentation() {
        for text in [
            "%div\n\t%p\n\t\t%b",
            "%div\n  :javascript\n    a\n        b\n      c\n  %p",
            "%div\n  :javascript\n    if (a) {\n    \tb();\n    }\n  :plain\n    \t x",
            "%p= \"a\" + |\n  \"b\" |\n%hr",
            "%p= link_to \"x\",\n    \"/y\"\n%b",
            "- if a\n  %p\n- else\n  %p",
            "",
            "\n\n",
        ] {
            assert_eq!(error_of(text), None, "{text:?}");
        }
    }

    #[test]
    fn pipe_lines_are_joined_into_their_line() {
        let cases: Vec<(&str, Option<(usize, String)>)> = vec![
            ("%div\n  %p= link_to |\n    \"a\", |\n      root_path", None),
            ("%div\n  %a{href: \"/\", |\n     class: \"x\"} |\n  %p\n      %i", deeper(4, 2)),
            ("- a, |\n  b |\n  %b\n   %i", inconsistent(3, "3 spaces", "2 spaces")),
        ];
        for (text, expected) in cases {
            assert_eq!(error_of(text), expected, "{text:?}");
        }
    }

    #[test]
    fn tag_heads_follow_parse_tag() {
        for text in [
            "%section\n  %div.@container.flex= render \"card\",\n      item: x",
            "%div\n  .@container{class: \"a\",\n               data: {x: 1}}",
            "%p& = format(x,\n  y,\n   z)",
            "%p(a=\"x\"\n   b=\"#{y}\" c=@d e=$f g\n h)\n%i",
        ] {
            assert_eq!(error_of(text), None, "{text:?}");
        }
        assert_eq!(error_of("%p<>= foo(1,\n 2)\n%i\n     %b"), deeper(3, 5));
        assert_eq!(error_of("%p{a: 1}= foo(x,\n y,\n   z\n%i\n  %b"), deeper(4, 2));
        let joined = |rest: &str| Some(tag_content(rest)).filter(|c| c.parse && is_ruby_multiline(&c.value));
        assert!(joined("{b: 2}= foo(x,").is_none());
        assert!(joined("& = format(x,").is_some());
        assert!(joined("== x,").is_none());
    }

    #[test]
    fn braces_inside_ruby_literals_do_not_close_the_hash() {
        for text in [
            "%div\n  %a{href: x, # {}\n      title: y} hi\n  %a{href: x, # }\n        title: y} hi",
            "%p{a: %w(}),\n b: 1}\n    %b",
            "%p{a: /}/,\n b: 1}\n    %b",
            "%p{a: ?},\n b: 1}\n    %b",
            "%p{a: \"#{x.map { |y| '}' }}\",\n b: 1}\n    %b",
            "%p{a: %q<}>,\n b: 1}\n    %b",
        ] {
            assert_eq!(error_of(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_percent_that_is_no_literal_is_text_in_the_hash() {
        let text = "%i\n\t%a{b: 1,\n\t\t%b= z\n\t\t\t%a[@x]\n\t\t\tc: 2}\n\t\t\t/ c";
        assert_eq!(error_of(text), deeper(5, 2));
    }

    #[test]
    fn brace_scanner_keeps_its_state_between_lines() {
        let mut scanner = BraceScanner::new();
        assert_eq!(scanner.feed("a: \"x"), None);
        assert_eq!(scanner.feed("}\", b: %w(} {),"), None);
        assert_eq!(scanner.feed("c: {d: 1}} tail"), Some(10));
    }

    #[test]
    fn a_hash_left_open_over_many_lines_ends_at_the_end_of_the_document() {
        let text = format!("%a{{b: 1,\n{}%p", "  c: 1,\n".repeat(20_000));
        assert_eq!(error_of(&text), Some((0, UNBALANCED.to_string())));
    }

    /// Each case was checked against Haml 7.5.1 (`test/oracle/haml.rb`): the error is at the nested line.
    #[test]
    fn illegal_nesting_at_the_nested_line() {
        let line = |name: &str| format!("Illegal nesting: content can't be both given on the same line as %{name} and nested within it.");
        let cases: Vec<(&str, usize, String)> = vec![
            ("a\n  b", 1, NESTING_PLAIN.into()),
            ("foo\n\n\n  bar", 3, NESTING_PLAIN.into()),
            ("#{x} t\n  %b", 1, NESTING_PLAIN.into()),
            ("== hi #{x}\n  %b", 1, NESTING_PLAIN.into()),
            ("& x\n  %b", 1, NESTING_PLAIN.into()),
            ("!== x\n  %b", 1, NESTING_PLAIN.into()),
            ("%p a\n  b", 1, line("p")),
            ("%p foo\n\n  bar", 2, line("p")),
            ("%p= x\n  b", 1, line("p")),
            ("%p== hi #{x}\n  %b", 1, line("p")),
            ("%p= x ?,\n  %b", 1, line("p")),
            (".user= x\n  %p", 1, line("div")),
            ("%p& text\n  %b", 1, line("p")),
            ("%p[@x] text\n  %b", 1, line("p")),
            ("%p(a=1) text\n  %b", 1, line("p")),
            ("%p/\n  a", 1, NESTING_SELF_CLOSING.into()),
            ("%p/\n\n  bar", 2, NESTING_SELF_CLOSING.into()),
            ("%a{b: 1}/\n  %c", 1, NESTING_SELF_CLOSING.into()),
            ("/ a\n  b", 1, NESTING_CONTENT.into()),
            ("/ foo\n\n  bar", 2, NESTING_CONTENT.into()),
            ("/[if IE] text\n  %p", 1, NESTING_CONTENT.into()),
            ("!!!\n  a", 1, NESTING_HEADER.into()),
            ("!!!\n\n  bar", 2, NESTING_HEADER.into()),
            ("!!! 5\n  %p", 1, NESTING_HEADER.into()),
        ];
        for (text, at, message) in cases {
            assert_eq!(error_of(text), Some((at, message)), "{text:?}");
        }
        for text in [
            "= x\n  %p",
            "- x = 1\n  %p",
            "~ x\n  %p",
            "&~ x\n  %p",
            "!= x\n  %b",
            "%p Dear Sir,\n  %b x",
            "%p= link_to |\n    \"a\", |\n      root_path",
            "/[[if IE]]\n  %span hello\n  world",
            "/[if IE]\n  %p",
            "/![if !IE]\n  %p x",
            "/\n  %p",
            "%br\n  %p x",
            "!!!\n",
        ] {
            assert_eq!(error_of(text), None, "{text:?}");
        }
    }

    #[test]
    fn empty_classes_and_ids_stop_the_parse() {
        let cases = [
            (". foo\n  %b", 0),
            ("# h\n  b", 0),
            ("%p..a\n  b", 0),
            ("%p.a.\n  b", 0),
            ("%p#\n  b", 0),
            (".#a", 0),
            ("%div\n  .\n    %b x", 1),
        ];
        for (text, at) in cases {
            assert_eq!(error_of(text), Some((at, ILLEGAL_ELEMENT.to_string())), "{text:?}");
        }
        assert_eq!(error_of("#$x\n  b"), Some((1, NESTING_PLAIN.to_string())));
        for text in ["#@x t", ".a-b.c_d#e:f", ".@container.x", "%p.a#b"] {
            assert_eq!(error_of(text), None, "{text:?}");
        }
    }

    /// Each case was checked against Haml 7.5.1 (`test/oracle/haml.rb`).
    #[test]
    fn attribute_errors() {
        let unbalanced = |at: usize| Some((at, UNBALANCED.to_string()));
        let invalid = |at: usize, fragment: &str| Some((at, format!("Invalid attribute list: {fragment}.")));
        let cases: Vec<(&str, Option<(usize, String)>)> = vec![
            ("%p{'foo => 'bar'}", unbalanced(0)),
            ("%p{:foo => 'bar}", unbalanced(0)),
            ("%p{:foo => 'bar\"}", unbalanced(0)),
            ("%p{a: 1", unbalanced(0)),
            ("%p{", unbalanced(0)),
            ("%p{a: 1,\n  b: 2", unbalanced(0)),
            ("%p{a: 1\n%b", unbalanced(0)),
            ("x\n%p{a: 1,\n", unbalanced(1)),
            ("%p[@a", unbalanced(0)),
            ("%p[@a\n%b", unbalanced(0)),
            ("%p(a='x#{y' b=1)", unbalanced(0)),
            ("%p(a=\"#{x\")", unbalanced(0)),
            ("/[if IE", unbalanced(0)),
            ("%p\n/![if IE", unbalanced(1)),
            ("%p{a: 1,\nunless z\n/x}\n}", unbalanced(0)),
            ("%p{a: (b /2),\n c: 3}\n    %b", unbalanced(0)),
            ("%p{a: 1,\nbegin /\n}\n", unbalanced(0)),
            ("%a{a: <<~H.strip\nbody }\nH}", unbalanced(0)),
            ("%p{% }", unbalanced(0)),
            ("%p{%\n}", unbalanced(0)),
            ("%%p[", unbalanced(0)),
            ("%p{a: 1,\n%i\nx\n}\n%b\n", unbalanced(0)),
            ("%p{a: 1,\n%w\n}\n}\n", unbalanced(0)),
            ("%p(", invalid(0, "\"(\"")),
            ("%p(foo 'bar')", invalid(0, "\"(foo 'bar')\"")),
            ("%p(foo 'bar'\nbaz='bang')", invalid(0, "\"(foo 'bar'\"")),
            ("%p(foo='bar'\nbaz 'bang'\nbip='bop')", invalid(1, "\"(foo='bar' baz 'bang'\"")),
            ("%p(foo=)", invalid(0, "\"(foo=)\"")),
            ("%p(foo=\nbar)", invalid(0, "\"(foo=\"")),
            ("%p(a=1\n%b", invalid(1, "\"(a=1 %b\"")),
            ("%p{a: 1}(b=", invalid(0, "\"(b=\"")),
            ("%p(a=\"x)", invalid(0, "\"(a=\\\"x)\"")),
            ("%p(a=1,b=2)", invalid(0, "\"(a=1,b=2)\"")),
        ];
        for (text, expected) in cases {
            assert_eq!(error_of(text), expected, "{text:?}");
        }
        for text in [
            "%p{a: 1,\n  b: z\n/}/}\n%i",
            "%p{a: 1 if x,\n  b: 2}\n  %b",
            "%p{a: (x if y /2/),\n b: 1}\n%b",
            "%p{a: x\n%w(}) }",
            "%a{a: <<~H,\n  body }\nH\n  b: 1}\n%p",
            "%a{a: <<-H,\n  }\n  H\n  b: 1}\n%p",
            "%a{a: <<H,\n}\nH\n  b: 1}\n%p",
            "%a{a: <<~'H',\n  #{ }\nH\n  b: 1}\n%p",
            "%a{a: <<~\"H\",\n  #{ {a: 1}[:a] } }\nH\n  b: 1}\n%p",
            "%a{a: <<~H, b: <<~J,\n x }\nH\n y }\nJ\n  c: 1}\n%p",
            "%a{a: 1 << 2,\n b: 3}\n%b",
            "%p{a: %\nx\n}",
            "%p{a: 1,\n%q\nx\n}\n%b",
            "%p{a: (1 /2),\n b: 1 / 2}",
            "%p(a=1",
            "%p{a: 1}(b=2",
            "%p{a: (1}",
            "%p{a: 1))}",
            "%p(a b)",
            "%p()",
            "%p( )",
            "%p{}",
            "%p(a=@@b c=$d e.f=g)",
            "%p{a: ->{ 1 }}",
            "%p(a=\"#{x}\")",
            "%p{a: \"}\"}x",
            "%p{a: 1}}",
            ":plain\n  %p{",
            "-# %p{\n",
        ] {
            assert_eq!(error_of(text), None, "{text:?}");
        }
    }

    /// Each case was checked against Haml 7.5.1 (`test/oracle/haml.rb`): the error is at the line the attributes end on.
    #[test]
    fn self_closing_tags_with_content() {
        let content = |at: usize| Some((at, SELF_CLOSING_CONTENT.to_string()));
        let cases = [
            ("%br/ text", content(0)),
            ("%p{:a => 'b',\n:c => 'd'}/ e", content(1)),
            ("%p(a=1\n b=2)/ x", content(1)),
            ("%p<>/ x", content(0)),
            ("%p[@x]/ x", content(0)),
            ("%br/ x,", content(0)),
            ("%br/ x\n  y", Some((1, NESTING_SELF_CLOSING.to_string()))),
            ("%a{b: /\\}/}\n%br/", None),
            ("%p/ \n%b", None),
        ];
        for (text, expected) in cases {
            assert_eq!(error_of(text), expected, "{text:?}");
        }
    }

    /// Each case was checked against Haml 7.5.1 (`test/oracle/haml.rb`).
    #[test]
    fn invalid_tags_missing_code_and_explicit_end() {
        let code = |action: char| no_ruby_code(action);
        let cases: Vec<(&str, usize, String)> = vec![
            ("% a", 0, "Invalid tag: \"% a\".".into()),
            ("%", 0, "Invalid tag: \"%\".".into()),
            ("%%", 0, "Invalid tag: \"%%\".".into()),
            ("=", 0, code('=')),
            ("~", 0, code('~')),
            ("&=", 0, code('=')),
            ("!=", 0, code('=')),
            ("&~", 0, code('~')),
            ("!~", 0, code('~')),
            ("%p=", 0, code('=')),
            ("%p= ", 0, code('=')),
            ("%p~", 0, code('~')),
            ("%p<>=", 0, code('=')),
            ("%p&=", 0, code('&')),
            ("%p!~", 0, code('!')),
            ("%p!=", 0, code('!')),
            ("%p{:a => 'b',\n:c => 'd'}=", 1, code('=')),
            ("%a(b=1)=", 0, code('=')),
            ("%a[@x]~", 0, code('~')),
            ("= x\n=\n", 1, code('=')),
            ("%p\n  =\n", 1, code('=')),
            ("- end", 0, NO_END.into()),
            ("-end", 0, NO_END.into()),
            ("-  end  ", 0, NO_END.into()),
            ("- if x\n  - end", 1, NO_END.into()),
        ];
        for (text, at, message) in cases {
            assert_eq!(error_of(text), Some((at, message)), "{text:?}");
        }
        for text in [
            "% foo %bar",
            "- end # x",
            "- endx",
            "- obj = Object.new; def obj.foo(a, b); a + b; end",
            "%span\n  %div\n    - num = obj.foo(1,\n2)\n    = num",
            "= obj.foo(1,\n2)",
            "-",
            "- ",
            "%p==",
            "%p&",
            "%p&==",
            "!!! 5",
            "==",
        ] {
            assert_eq!(error_of(text), None, "{text:?}");
        }
    }

    /// Each case was checked against Haml 7.5.1 (`test/oracle/haml.rb`).
    #[test]
    fn filters() {
        let not_found = |name: &str| format!("FilterCompiler for '{name}' was not found");
        let invalid = |name: &str| format!("Invalid filter name \":{name}\".");
        let cases: Vec<(&str, usize, String)> = vec![
            (":nope", 0, not_found("nope")),
            ("\n\n:k0kubun", 2, not_found("k0kubun")),
            (":a1", 0, not_found("a1")),
            (":maruku\n  x", 0, not_found("maruku")),
            (":textile\n  x", 0, not_found("textile")),
            ("%p\n  :nope\n    x", 1, not_found("nope")),
            (":NOPE\n:nope2", 0, not_found("nope")),
            (":nope2\n:nope", 0, not_found("nope2")),
            (":nope\n  = \n", 0, not_found("nope")),
            (":nope\n- end", 1, NO_END.into()),
            (":nope\n% a", 1, "Invalid tag: \"% a\".".into()),
            (":java-script", 0, invalid("java-script")),
            (":", 0, invalid("")),
            (": x", 0, invalid(" x")),
            (":plain foo", 0, invalid("plain foo")),
            (":a= b", 0, invalid("a= b")),
            ("%div\n  :ÄB", 1, invalid("äb")),
            ("%p{a: 1,\n:nope", 0, UNBALANCED.into()),
        ];
        for (text, at, message) in cases {
            assert_eq!(error_of(text), Some((at, message)), "{text:?}");
        }
        for text in [
            ":Javascript\n  x",
            ":PLAIN\n  x",
            ":plain\n  :nope",
            ":ruby\n  x = 1",
            ":cdata\n  x",
            ":escaped\n  x",
            ":css\n  a{}",
            ":erb\n  <%= 1 %>",
        ] {
            assert_eq!(error_of(text), None, "{text:?}");
        }
    }

    /// Each case was checked against Haml 7.5.1 (`test/oracle/haml.rb`).
    #[test]
    fn branches_follow_their_opener() {
        let missing = |keyword: &str| format!("Got \"{keyword}\" with no preceding \"if\"");
        let level = |keyword: &str, expected: usize, was: usize| {
            format!("\"{keyword}\" is indented at wrong level: expected {expected}, but was at {was}.")
        };
        let cases: Vec<(&str, usize, String)> = vec![
            ("- else", 0, missing("else")),
            ("%p\n- else", 1, missing("else")),
            ("- elsif a", 0, missing("elsif")),
            ("- when 1", 0, missing("when")),
            ("- foo if a\n- else", 1, missing("else")),
            // Ruby's `\w` is ASCII, so `café =` isn't the assignment the opener may start with.
            ("- café = if x\n  %p a\n- else", 2, missing("else")),
            ("- if a\n  - else", 1, level("else", 0, 1)),
            ("- if a\n  %p\n  - else", 2, level("else", 0, 1)),
            ("- case x\n  - when 1\n  - when 2\n    - else", 3, level("else", 1, 2)),
            ("- case x\n- when 1\n- when 2\n  - when 3", 3, level("when", 0, 1)),
            ("- if a\n  %b\n- elsif b\n  - else", 3, level("else", 0, 1)),
            ("- if a\n  -# c\n  - else", 2, level("else", 0, 1)),
        ];
        for (text, at, message) in cases {
            assert_eq!(error_of(text), Some((at, message)), "{text:?}");
        }
        for text in [
            "- if a\n  %p\n- else\n  %p",
            "- case x\n- when 1\n  %p\n- else\n  %p",
            "- case x\n  - when 1\n    %p\n  - when 2\n    %p\n  - else\n    %p",
            "- case x\n    - when 1",
            "- begin\n  %p\n- rescue => e\n  %p\n- ensure\n  %p",
            "- rescue",
            "- ensure",
            "- if a\n  %p\n\n- else\n  %p",
            "%p= x,\n\n- else",
            "- x = if a\n  - 1\n- else\n  - 2",
            "= if a\n  %p\n- else\n  %p",
            "~ if a\n  %p\n- else\n  %p",
            "- unless a\n  %p\n- else\n  %p",
            "- if a\n  - if b\n    %p\n  - else\n    %p\n- elsif c\n  %p",
            "- if a\n  - if b\n    %p\n- else\n",
            "- if a\n  %p\n-else\n  %p",
            "- if a\n  :plain\n    x\n- else\n  y",
        ] {
            assert_eq!(error_of(text), None, "{text:?}");
        }
    }

    /// Haml compiles content in a void element to an end tag (`%br\n  %p x` is `<br><p>x</p></br>`).
    #[test]
    fn void_elements_with_content() {
        let void = |text: &str| {
            let parsed = parse(text);
            assert!(parsed.error.is_none(), "{text:?}");
            parsed.void_content.into_iter().map(|v| (v.line, v.name, v.nested)).collect::<Vec<_>>()
        };
        assert_eq!(void("%br\n  %p x"), [(1, "br".into(), true)]);
        assert_eq!(void("%img\n  x"), [(1, "img".into(), true)]);
        assert_eq!(void(".a\n  %hr\n    %b"), [(2, "hr".into(), true)]);
        assert_eq!(void("%br hello"), [(0, "br".into(), false)]);
        assert_eq!(void("%img{src: 1} x"), [(0, "img".into(), false)]);
        assert_eq!(void("%br= 1"), [(0, "br".into(), false)]);
        assert_eq!(void("%BR\n  x"), [(1, "BR".into(), true)]);
        assert!(void("%br").is_empty());
        assert!(void("%br/").is_empty());
        assert!(void("%p\n  %br\n  %img{src: 1}\n  %b x").is_empty());
        assert!(parse("%br\n  %p x\n% a").void_content.is_empty());
    }

    /// Pairs of templates and whether `Haml::Engine` (`test/oracle/haml.rb --rails`) compiles them to the same source.
    #[test]
    fn same_compilation_agrees_with_haml() {
        let pairs: &[(&str, &str, &str, bool)] = &[
            ("indent x2", "%div\n  %p x\n  %b\n    text\n", "%div\n    %p x\n    %b\n        text\n", true),
            ("tabs", "%div\n  %p x\n  %b\n    text\n", "%div\n\t%p x\n\t%b\n\t\ttext\n", true),
            ("one space", "%div\n  %p x\n  %b\n    text\n", "%div\n %p x\n %b\n  text\n", true),
            ("crlf", "%div\n  %p x\n  %b\n    text\n", "%div\r\n  %p x\r\n  %b\r\n    text\r\n", true),
            ("lone cr", "%div\n  %p x\n  %b\n    text\n", "%div\r  %p x\r  %b\r    text\r", true),
            ("bom", "%div\n  %p x\n  %b\n    text\n", "\u{feff}%div\n  %p x\n  %b\n    text\n", true),
            ("no final newline", "%div\n  %p x\n  %b\n    text\n", "%div\n  %p x\n  %b\n    text", true),
            ("trailing blank lines", "%div\n  %p x\n  %b\n    text\n", "%div\n  %p x\n  %b\n    text\n\n\n", true),
            ("trailing ws-only lines", "%div\n  %p x\n  %b\n    text\n", "%div\n  %p x\n  %b\n    text\n   \n \n", true),
            ("trailing ws", "%div\n  %p x\n  %b\n    text\n", "%div  \n  %p x  \n  %b \n    text  \n", true),
            ("blank line added", "%div\n  %p x\n  %b\n    text\n", "%div\n\n  %p x\n  %b\n    text\n", false),
            ("ws-only line added", "%div\n  %p x\n  %b\n    text\n", "%div\n   \n  %p x\n  %b\n    text\n", false),
            ("leading blank line", "%div\n  %p x\n  %b\n    text\n", "\n%div\n  %p x\n  %b\n    text\n", false),
            ("blank removed", "%div\n\n  %p\n", "%div\n  %p\n", false),
            ("ws-only vs empty line", "%div\n\n  %p\n", "%div\n   \n  %p\n", true),
            ("text changed", "%div\n  %p x\n  %b\n    text\n", "%div\n  %p y\n  %b\n    text\n", false),
            ("depth changed", "%div\n  %p x\n  %b\n    text\n", "%div\n  %p x\n    %b\n    text\n", false),
            ("filter 2 vs 4", "%div\n  :javascript\n    a\n      b\n", "%div\n    :javascript\n        a\n          b\n", true),
            ("filter body relative changed", "%div\n  :javascript\n    a\n      b\n", "%div\n  :javascript\n    a\n        b\n", false),
            ("filter tab vs spaces body", "%div\n\t:plain\n\t\ta\n\t\t\tb\n", "%div\n  :plain\n    a\n    \tb\n", true),
            ("filter tab all spaces (content changes)", "%div\n\t:plain\n\t\ta\n\t\t\tb\n", "%div\n  :plain\n    a\n      b\n", false),
            ("filter ws-only line short", "%div\n  :plain\n    a\n  \n    b\n", "%div\n  :plain\n    a\n\n    b\n", true),
            (
                "filter ws-only line longer",
                "%div\n    :plain\n        a\n          \n        b\n",
                "%div\n  :plain\n    a\n      \n    b\n",
                true,
            ),
            (
                "filter ws-only line longer lost",
                "%div\n    :plain\n        a\n          \n        b\n",
                "%div\n  :plain\n    a\n\n    b\n",
                false,
            ),
            ("filter body trailing ws", "%div\n  :javascript\n    a\n", "%div\n  :javascript\n    a  \n", true),
            ("filter blank removed", "%div\n  :javascript\n    a\n\n    b\n", "%div\n  :javascript\n    a\n    b\n", false),
            ("filter trailing blank then dedent", "%div\n  :javascript\n    a\n%p\n", "%div\n  :javascript\n    a\n\n%p\n", false),
            ("filter trailing ws-only vs empty", "%div\n  :plain\n    a\n\n%p\n", "%div\n  :plain\n    a\n  \n%p\n", true),
            ("filter head trailing ws", ":plain\n  a\n", ":plain  \n  a\n", true),
            ("haml comment body", "%div\n  -# a\n    b\n      c\n", "%div\n    -# a\n        b\n          c\n", true),
            ("pipe 2 vs 4 cont", "%p a |\n  b |\n%i\n", "%p a |\n      b |\n%i\n", true),
            ("pipe cont at 0", "%p a |\n  b |\n%i\n", "%p a |\nb |\n%i\n", true),
            ("pipe joined text differs", "%p a |\n  b |\n%i\n", "%p a |\n  c |\n%i\n", false),
            ("pipe blank inside", "%p a |\n  b |\n%i\n", "%p a |\n\n  b |\n%i\n", false),
            ("ruby comma cont", "= foo(1,\n  2)\n%p\n", "= foo(1,\n        2)\n%p\n", true),
            ("ruby comma cont text differs", "= foo(1,\n  2)\n%p\n", "= foo(1,\n  3)\n%p\n", false),
            ("attr hash cont", "%a{b: 1,\n  c: 2} t\n", "%a{b: 1,\n        c: 2} t\n", true),
            ("attr hash cont at 0", "%a{b: 1,\n  c: 2} t\n", "%a{b: 1,\nc: 2} t\n", true),
            ("attr list cont", "%a(b=1\n  c=2) t\n", "%a(b=1\n         c=2) t\n", true),
            ("tag script comma cont", "%p= foo(1,\n  2)\n%i\n", "%p= foo(1,\n 2)\n%i\n", true),
            ("pre children", "%div\n  %pre\n    %b x\n    %i y\n", "%div\n    %pre\n        %b x\n        %i y\n", true),
            ("tilde", "%div\n  ~ \"a\"\n", "%div\n    ~ \"a\"\n", true),
            ("else dedent", "- if a\n  %p\n- else\n  %b\n", "- if a\n    %p\n- else\n    %b\n", true),
            ("case when", "- case x\n- when 1\n  %p\n", "- case x\n  - when 1\n    %p\n", true),
            ("plain text trailing ws", "%p\n  text\n", "%p\n  text   \n", true),
            ("plain text tab inside", "%p\n  a\tb\n", "%p\n  a b\n", false),
            ("error before", "%p a\n  b\n", "%p a\n    b\n", false),
            ("error vs fine", "%div\n  %p\n   %b\n", "%div\n  %p\n    %b\n", false),
            ("empty", "", "\n\n", true),
            ("only filter", ":plain\n  a\n", ":plain\n    a\n", true),
            ("filter first line blank body", ":plain\n    \n  a\n", ":plain\n\n  a\n", false),
            ("nested tab filter", "%a\n\t%b\n\t\t:plain\n\t\t\tx\n\t\t\t\ty\n", "%a\n  %b\n    :plain\n      x\n      \ty\n", true),
            ("unknown filter", ":nope\n  a\n", ":nope\n    a\n", false),
        ];
        let disagreements: Vec<_> =
            pairs.iter().filter(|(_, before, after, equal)| same_compilation(before, after) != *equal).map(|p| p.0).collect();
        // Haml reads a `when` one level under its `case` like a sibling; the trace keeps depths, so it only refuses.
        assert_eq!(disagreements, ["case when"]);
    }

    fn roles(text: &str) -> Vec<Role> {
        let parsed = parse(text);
        assert_eq!(parsed.error, None, "{text:?}");
        parsed.roles
    }

    #[test]
    fn roles_of_each_kind_of_line() {
        use Role::*;
        let text = "%div\n    %p a |\n         b |\n\n    %a{href: x,\n  title: y}\n    = link_to 1,\n 2\n    :plain\n        text\n\n          more\n      \n    %i\n";
        assert_eq!(
            roles(text),
            [
                Structural { depth: 0 },
                Structural { depth: 1 },
                Inner { owner: 1, counted: false },
                Blank,
                Structural { depth: 1 },
                Inner { owner: 4, counted: false },
                Structural { depth: 1 },
                Inner { owner: 6, counted: false },
                Structural { depth: 1 },
                FlatBody { owner: 8, prefix_len: 8 },
                FlatBody { owner: 8, prefix_len: 0 },
                FlatBody { owner: 8, prefix_len: 8 },
                FlatBody { owner: 8, prefix_len: 0 },
                Structural { depth: 1 },
            ]
        );
        assert_eq!(
            roles("-# note\n    body\n\n%p"),
            [Structural { depth: 0 }, FlatBody { owner: 0, prefix_len: 4 }, FlatBody { owner: 0, prefix_len: 0 }, Structural { depth: 0 }]
        );
        assert_eq!(roles("%p\n\n\n"), [Structural { depth: 0 }]);
        assert_eq!(roles("\n\n%p"), [Blank, Blank, Structural { depth: 0 }]);
    }

    /// `HAML_PAIRS=dir cargo test --release pairs -- --ignored`: `dir` holds `N.a`, `N.b` and `N.eq` (`1` when Haml
    /// compiles the two to the same source); prints how many `same_compilation` agrees with.
    #[test]
    #[ignore]
    fn pairs() {
        let dir = std::env::var("HAML_PAIRS").expect("HAML_PAIRS");
        let (mut agree, mut conservative, mut unsafe_) = (0, 0, Vec::new());
        let mut by_change = std::collections::BTreeMap::<String, (usize, usize)>::new();
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "a") {
                continue;
            }
            let read = |ext: &str| String::from_utf8_lossy(&std::fs::read(path.with_extension(ext)).unwrap()).into_owned();
            let (before, after, equal) = (read("a"), read("b"), read("eq").trim() == "1");
            let tally = by_change.entry(read("name")).or_default();
            match (same_compilation(&before, &after), equal) {
                (ours, haml) if ours == haml => agree += 1,
                (false, true) => {
                    conservative += 1;
                    tally.0 += 1;
                    if tally.0 <= 2 {
                        println!("conservative {}: {path:?}", read("name"));
                    }
                }
                (true, false) => {
                    tally.1 += 1;
                    unsafe_.push(path);
                }
                _ => unreachable!(),
            }
        }
        println!("conservative and unsafe by change: {by_change:?}");
        println!(
            "agree {agree}, conservative (Haml equal, trace different) {conservative}, unsafe (trace equal, Haml different) {}",
            unsafe_.len()
        );
        assert!(unsafe_.is_empty(), "{unsafe_:?}");
    }
}
