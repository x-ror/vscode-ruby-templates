//! Port of `Slim::Parser` (slim 5.2) that records byte offsets instead of building Temple: which
//! physical lines start a Slim line, which belong to another line's block or continuation, and
//! where the Ruby code, tag and attribute name slots and line markers are.

use crate::ruby::comment_before;
use std::ops::Range;

pub const BOM: &str = "\u{feff}";
/// Slim::Parser default `tabsize`.
const TABSIZE: usize = 4;
const DEFAULT_TAG: &str = "div";
// Slim 5 `Slim::Embedded` registrations; apps can register more (see `extra_engines`).
pub(crate) const SLIM_ENGINES: &[&str] =
    &["ruby", "javascript", "css", "sass", "scss", "coffee", "markdown", "textile", "rdoc", "asciidoc"];
pub const VOID_ELEMENTS: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];
/// Engines that run `#{}` through Slim's interpolation: TagEngine without a Tilt engine, and InterpolateTiltEngine.
const INTERPOLATING_ENGINES: &[&str] = &["javascript", "css", "markdown", "textile", "rdoc", "asciidoc"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Blank,
    /// Starts a Slim line nested `depth` levels deep.
    Structural {
        depth: usize,
    },
    /// Text, comment or engine body, or a Ruby/attribute continuation of line `owner`.
    Inner {
        owner: usize,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct LineInfo {
    pub role: Role,
    /// Trailing whitespace on this line ends up in the output.
    pub keeps_trailing: bool,
    /// A text block's base indentation is this line's raw character column, where a tab counts as one.
    pub raw_column: bool,
    /// A Ruby or attribute continuation: Slim strips its indentation.
    pub free_indent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Control,
    Output,
    Comment,
    HtmlComment,
    ConditionalComment,
    Text,
    Engine,
}

/// Where a piece of Ruby came from in the template.
#[derive(Debug, Clone, Copy)]
pub struct Origin {
    pub line: usize,
    /// Byte offset of the code in the text, when it is one verbatim run on that line.
    pub offset: Option<usize>,
}

/// What `Slim::Parser` hands to the later filters that matters for the Ruby the engine generates, in source order.
#[derive(Debug)]
pub enum Piece {
    /// A structural line starts, `depth` levels deep.
    Line {
        depth: usize,
        line: usize,
    },
    /// Slim's `[:newline]`.
    Newline,
    /// Output with no Ruby in it: markup and text, as the template writes it (`Slim::Interpolation`'s unescaped `\#{`).
    Static(String),
    /// Closing markup that follows the content of the line, nested lines included.
    Close(String),
    /// A `-` line.
    Control {
        code: String,
        at: Origin,
    },
    /// `=`, `==` and `#{}`; `block` when the deeper lines that follow are its content.
    Output {
        escape: bool,
        code: String,
        block: bool,
        at: Origin,
    },
    /// An attribute whose value is Ruby; `splat` when its tag also has a `*` attribute.
    Attribute {
        name: String,
        code: String,
        splat: bool,
        at: Origin,
    },
    Splat {
        code: String,
        at: Origin,
    },
    /// The body of a `ruby:` engine.
    Code {
        text: String,
        at: Origin,
    },
}

/// A syntax error as Slim reports it.
pub struct SyntaxError {
    pub line: usize,
    /// Characters, as Slim counts them.
    pub column: usize,
    pub message: String,
}

pub struct Parsed {
    pub lines: Vec<LineInfo>,
    /// Byte ranges of Ruby code.
    pub ruby: Vec<Range<usize>>,
    /// Byte offsets where a tag name starts.
    pub tag_slots: Vec<usize>,
    /// Byte offsets where an attribute name may start.
    pub attr_slots: Vec<usize>,
    pub markers: Vec<(Range<usize>, Marker)>,
    /// Byte ranges of text that interpolates `#{}`, one per line.
    pub text: Vec<Range<usize>>,
    /// No Slim syntax error was found.
    pub valid: bool,
    /// Line where Slim would have stopped for a reason other than indentation.
    pub stop_line: Option<usize>,
    /// The first syntax error Slim words; Slim stops there.
    pub syntax_error: Option<SyntaxError>,
    /// Line and name of the first attribute repeated in one tag; Temple rejects it after parsing (only `class` merges).
    pub repeated_attribute: Option<(usize, String)>,
    /// Line and (lower-cased) name of the first doctype Temple's xhtml format doesn't know.
    pub invalid_doctype: Option<(usize, String)>,
    pub pieces: Vec<Piece>,
}

#[derive(Clone, Copy)]
enum Body {
    Ruby,
    Interpolated,
    Opaque,
}

struct Engine<'a> {
    name: &'a str,
    attrs: Range<usize>,
    body: usize,
}

/// Whether the UTF-16 position `pos` is in Ruby code, outside a `#` comment.
/// Whether byte `offset` is in Ruby code, outside a `#` comment.
pub fn in_ruby(text: &str, parsed: &Parsed, offset: usize) -> bool {
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    parsed.ruby.iter().any(|r| r.start <= offset && offset <= r.end && !comment_before(&text[r.start.max(line_start)..offset]))
}

/// Whether byte `offset` follows a `#{` that is still open in interpolated text. Slim treats it as
/// static text, but while typing it is where Ruby is about to go.
pub fn in_open_interpolation(text: &str, parsed: &Parsed, offset: usize) -> bool {
    let Some(segment) = parsed.text.iter().find(|r| r.start <= offset && offset <= r.end) else {
        return false;
    };
    let before = &text[segment.start..offset];
    let Some(open) = before.rfind("#{") else {
        return false;
    };
    let escaped = before[..open].ends_with('\\');
    let code = &before[open + 2..];
    let depth = code.chars().fold(1i32, |d, c| match c {
        '{' => d + 1,
        '}' => d - 1,
        _ => d,
    });
    !escaped && depth > 0
}

pub fn parse(text: &str, extra_engines: &[String]) -> Parsed {
    let mut starts = Vec::new();
    let mut lines = Vec::new();
    // Slim drops a byte order mark before parsing.
    let mut start = if text.starts_with(BOM) { BOM.len() } else { 0 };
    for line in text[start..].split('\n') {
        starts.push(start);
        start += line.len() + 1;
        lines.push(line.strip_suffix('\r').unwrap_or(line));
    }
    let engines = SLIM_ENGINES.iter().copied().chain(extra_engines.iter().map(String::as_str)).collect();
    let blank = LineInfo { role: Role::Blank, keeps_trailing: false, raw_column: false, free_indent: false };
    let mut p = Parser {
        out: Parsed {
            lines: vec![blank; lines.len()],
            ruby: Vec::new(),
            tag_slots: Vec::new(),
            attr_slots: Vec::new(),
            markers: Vec::new(),
            text: Vec::new(),
            valid: true,
            stop_line: None,
            syntax_error: None,
            repeated_attribute: None,
            invalid_doctype: None,
            pieces: Vec::new(),
        },
        lines,
        starts,
        engines,
        i: 0,
        pos: 0,
        end: 0,
        owner: 0,
        indents: Vec::new(),
        expecting: false,
        count: 0,
    };
    p.count = p.lines.iter().rposition(|l| !l.is_empty()).map_or(0, |i| i + 1);
    let mut next = 0;
    while next < p.count {
        p.i = next;
        p.parse_line();
        next = p.i + 1;
    }
    p.out
}

/// Ruby's `\s`.
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_blank(line: &str) -> bool {
    line.chars().all(is_space)
}

fn trim_start(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

/// `get_indent`: a tab advances to the next multiple of `TABSIZE`.
pub fn indent_width(line: &str) -> usize {
    line.chars().take_while(|c| matches!(c, ' ' | '\t')).fold(0, |col, c| if c == '\t' { (col / TABSIZE + 1) * TABSIZE } else { col + 1 })
}

fn closer(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

/// Length of an attribute name: anything but whitespace, NUL, quotes, `<>/=` and brackets.
fn attr_name_len(s: &str) -> usize {
    s.find(|c: char| is_space(c) || "\0\"'></=()[]{}".contains(c)).unwrap_or(s.len())
}

/// `@tag_re`: `Some((len, named))`; `.class`, `#id` and `*splat` tags consume nothing.
fn match_tag(r: &str) -> Option<(usize, bool)> {
    let first = r.chars().next()?;
    if first == '#' || first == '.' {
        return Some((0, false));
    }
    if first == '*' {
        return r[1..].chars().next().filter(|c| !is_space(*c)).map(|_| (0, false));
    }
    if !is_word(first) {
        return None;
    }
    let run = r.find(|c: char| !(is_word(c) || c == ':' || c == '-')).unwrap_or(r.len());
    Some((r[..run].trim_end_matches([':', '-']).len(), true))
}

/// Temple::HTML::Fast's `DOCTYPES[:xhtml]` and its `xml` directive.
fn is_known_doctype(kind: &str) -> bool {
    const KNOWN: &[&str] = &["1.1", "5", "html", "strict", "frameset", "mobile", "basic", "transitional", "svg"];
    KNOWN.contains(&kind)
        || kind == "xml"
        || kind.strip_prefix("xml").is_some_and(|rest| rest.starts_with(is_space) && !trim_start(rest).is_empty())
}

fn is_doctype(r: &str) -> bool {
    r.strip_prefix("doctype").is_some_and(|rest| !rest.starts_with(is_word))
}

/// `$2` of `\A([\|'])([<>]{1,2}(?: |\z)| ?)`, given the text after `|`/`'`.
fn text_modifiers(after: &str) -> &str {
    let arrows = after.bytes().take(2).take_while(|b| matches!(b, b'<' | b'>')).count();
    for n in (1..=arrows).rev() {
        let rest = &after[n..];
        if rest.is_empty() {
            return &after[..n];
        }
        if rest.starts_with(' ') {
            return &after[..n + 1];
        }
    }
    if after.starts_with(' ') {
        " "
    } else {
        ""
    }
}

/// For each byte offset of a `{` in `s`, the offset just past its matching `}`. Counting depth from a
/// given `{` (Slim's interpolation regex, braces only) finds the same `}` as one stack pass, without
/// rescanning the line for every unclosed `#{`.
fn matching_braces(s: &str) -> Vec<Option<usize>> {
    let mut closers = vec![None; s.len()];
    let mut open = Vec::new();
    for (i, c) in s.char_indices() {
        match c {
            '{' => open.push(i),
            '}' => {
                if let Some(o) = open.pop() {
                    closers[o] = Some(i + 1);
                }
            }
            _ => {}
        }
    }
    closers
}

/// How `parse_text_block` lays a text block out: the column the lines are measured from (the first line's, lowered by any
/// line indented less) and the empty lines waiting for the next one.
struct Layout {
    text_indent: Option<usize>,
    blank: usize,
}

impl Layout {
    fn push_blank(&mut self) {
        if self.text_indent.is_some() {
            self.blank += 1;
        }
    }

    /// What goes before the text of a line indented `indent`: its empty lines, a line break, and its offset.
    fn lead(&mut self, indent: usize) -> String {
        let mut offset = 0;
        if let Some(base) = self.text_indent {
            if indent < base {
                self.text_indent = Some(indent);
            } else {
                offset = indent - base;
            }
        }
        let mut lead = "\n".repeat(self.blank);
        self.blank = 0;
        if self.text_indent.is_some() {
            lead.push('\n');
        }
        lead.push_str(&" ".repeat(offset));
        self.text_indent.get_or_insert(indent);
        lead
    }
}

/// A `ruby:` body, as `collect_text` joins it.
struct RubyBody {
    /// The line the code starts on: the inline text's, else the first line of the block.
    line: Option<usize>,
    text: String,
}

impl RubyBody {
    fn push_line(&mut self, number: usize, lead: &str, line: &str) {
        self.line.get_or_insert(number);
        self.text.push_str(lead);
        self.text.push_str(line);
    }

    fn finish(&self) -> String {
        format!("{}\n", self.text)
    }
}

struct Parser<'a> {
    lines: Vec<&'a str>,
    starts: Vec<usize>,
    engines: Vec<&'a str>,
    /// Current line, and the unconsumed part of it (`@line`) as a byte range.
    i: usize,
    pos: usize,
    end: usize,
    /// The structural line being parsed.
    owner: usize,
    indents: Vec<usize>,
    /// The previous structural line opened a block (`@stacks.size > @indents.size`).
    expecting: bool,
    /// The lines up to the last non-empty one: Slim's `split(/\r?\n/)` drops the empty lines at the end.
    count: usize,
    out: Parsed,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.lines[self.i][self.pos..self.end]
    }

    fn at(&self) -> usize {
        self.starts[self.i] + self.pos
    }

    fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    fn base_indent(&self) -> usize {
        self.indents.last().copied().unwrap_or(0)
    }

    fn fail(&mut self) {
        self.out.valid = false;
        self.out.stop_line.get_or_insert(self.i);
    }

    fn fail_at(&mut self, message: String, column: usize) {
        if self.out.stop_line.is_none() {
            self.out.syntax_error = Some(SyntaxError { line: self.i, column, message });
        }
        self.fail();
    }

    fn has_next_line(&self) -> bool {
        self.i + 1 < self.count
    }

    fn marker(&mut self, len: usize, kind: Marker) {
        let at = self.at();
        self.out.markers.push((at..at + len, kind));
    }

    fn ruby(&mut self, from: usize, to: usize) {
        let base = self.starts[self.i];
        self.out.ruby.push(base + from..base + to);
    }

    fn enter(&mut self, i: usize) {
        self.i = i;
        self.pos = 0;
        self.end = self.lines[i].len();
        self.out.lines[i].role = Role::Inner { owner: self.owner };
    }

    /// `next_line`: the following line, unstripped.
    fn next_line(&mut self) -> bool {
        if !self.has_next_line() {
            return false;
        }
        self.enter(self.i + 1);
        self.out.lines[self.i].free_indent = true;
        true
    }

    /// `expect_next_line`: the following line, stripped; a syntax error at the end of the file.
    fn expect_next_line(&mut self) -> bool {
        if !self.next_line() {
            // Slim's `@line` is nil by then, so the column is 0.
            self.fail_at("Unexpected end of file".to_string(), 0);
            return false;
        }
        self.strip_line();
        true
    }

    fn strip_line(&mut self) {
        let line = self.lines[self.i];
        self.end = line.trim_end_matches(is_space).len();
        self.pos = (line.len() - trim_start(line).len()).min(self.end);
    }

    /// Characters of the current line before `tail`, a suffix of it.
    fn chars_before(&self, tail: &str) -> usize {
        self.lines[self.i].chars().count() - tail.chars().count()
    }

    /// Slim's `@orig_line.size - @line.size`.
    fn column(&self) -> usize {
        self.lines[self.i].chars().count() - self.rest().chars().count()
    }

    fn parse_line(&mut self) {
        let line = self.lines[self.i];
        if is_blank(line) {
            self.out.pieces.push(Piece::Newline);
            return;
        }
        let indent = indent_width(line);
        if self.indents.is_empty() {
            self.indents.push(indent);
        }
        if indent > self.base_indent() {
            if !self.expecting {
                self.out.valid = false;
            }
            self.indents.push(indent);
        } else {
            while self.indents.len() > 1 && indent < self.base_indent() {
                self.indents.pop();
            }
            if indent != self.base_indent() {
                self.out.valid = false;
                self.indents.push(indent);
            }
        }
        self.expecting = false;
        self.owner = self.i;
        let depth = self.indents.len() - 1;
        self.out.lines[self.i].role = Role::Structural { depth };
        self.out.pieces.push(Piece::Line { depth, line: self.i });
        self.pos = line.len() - trim_start(line).len();
        self.end = line.len();
        self.parse_line_indicators();
        self.out.pieces.push(Piece::Newline);
    }

    fn parse_line_indicators(&mut self) {
        let r = self.rest();
        if let Some(after) = r.strip_prefix("/!") {
            self.marker(2, Marker::HtmlComment);
            self.skip(2 + usize::from(after.starts_with(' ')));
            self.out.pieces.push(Piece::Static("<!--".to_string()));
            self.text_block(Body::Interpolated, false, self.base_indent() + usize::from(after.starts_with(' ')) + 2);
            self.out.pieces.push(Piece::Static("-->".to_string()));
        } else if r.len() > 2 && r.starts_with("/[") && r.trim_end_matches(is_space).ends_with(']') {
            self.marker(r.trim_end_matches(is_space).len(), Marker::ConditionalComment);
            let condition = r[2..].trim_end_matches(is_space).trim_end_matches(']').trim_matches(is_space);
            self.out.pieces.push(Piece::Static(format!("<!--[{condition}]>")));
            self.out.pieces.push(Piece::Close("<![endif]-->".to_string()));
            self.expecting = true;
        } else if r.starts_with('/') {
            self.marker(1, Marker::Comment);
            self.comment_block();
        } else if r.starts_with(['|', '\'']) {
            let modifiers = text_modifiers(&r[1..]);
            self.marker(1 + modifiers.trim_end().len(), Marker::Text);
            self.skip(1 + modifiers.len());
            let text_indent = self.base_indent() + modifiers.matches(' ').count() + 1;
            self.text_block(Body::Interpolated, false, text_indent);
        } else if r.starts_with('<') {
            self.interpolate(self.pos, self.end);
            self.out.lines[self.i].keeps_trailing = true;
            self.expecting = true;
        } else if let Some(code) = r.strip_prefix('-') {
            self.marker(1, Marker::Control);
            self.skip(r.len() - code.len());
            let (code, at) = self.broken_line();
            self.out.pieces.push(Piece::Control { code, at });
            self.expecting = true;
        } else if r.starts_with('=') {
            self.output(false);
        } else if let Some(engine) = self.match_engine(r) {
            self.embedded(engine);
        } else if is_doctype(r) {
            let kind = r["doctype".len()..].trim_matches(is_space).to_lowercase();
            if !is_known_doctype(&kind) {
                self.out.invalid_doctype.get_or_insert((self.i, kind));
            }
            self.out.pieces.push(Piece::Static("<!DOCTYPE html>".to_string()));
        } else if let Some((len, named)) = match_tag(r) {
            self.out.tag_slots.push(self.at());
            let name = if named { &r[..len] } else { DEFAULT_TAG };
            if named {
                self.skip(len);
            }
            self.parse_tag(name);
        } else {
            self.fail_at("Unknown line indicator".to_string(), self.column());
        }
    }

    /// `=(=?)` plus whitespace modifiers, then the Ruby; only a tag's `=` also takes a (no-op) `'`.
    fn output(&mut self, after_tag: bool) {
        let r = self.rest();
        let eq = 1 + usize::from(r[1..].starts_with('='));
        let modifier = |b: &u8| matches!(b, b'<' | b'>') || (after_tag && *b == b'\'');
        let modifiers = r[eq..].bytes().take_while(modifier).count();
        self.marker(eq + modifiers, Marker::Output);
        self.skip(eq + modifiers);
        let (code, at) = self.broken_line();
        self.out.pieces.push(Piece::Output { escape: eq == 1, code, block: true, at });
        self.expecting = true;
    }

    /// `@embedded_re`: `engine(attrs):body`, where the last `:` on the line ends the attributes.
    fn match_engine(&self, r: &'a str) -> Option<Engine<'a>> {
        let name = *self.engines.iter().find(|e| r.starts_with(**e))?;
        let after = &r[name.len()..];
        let colon = after.rfind(':')?;
        let attrs_end = self.pos + name.len() + colon;
        let attrs_start = attrs_end - trim_start(&after[..colon]).len();
        let body = self.pos + r.len() - trim_start(&after[colon + 1..]).len();
        Some(Engine { name, attrs: attrs_start..attrs_end, body })
    }

    fn embedded(&mut self, engine: Engine) {
        self.marker(engine.name.len(), Marker::Engine);
        let line = self.i;
        self.pos = engine.attrs.start;
        self.end = engine.attrs.end;
        let first_piece = self.out.pieces.len();
        self.parse_attributes(&mut Vec::new());
        if engine.name == "ruby" {
            // `RubyEngine` drops the attributes.
            self.out.pieces.truncate(first_piece);
        } else {
            self.out.pieces.push(Piece::Static(String::new()));
        }
        if self.i != line {
            // Wrapped attributes across lines put the body somewhere this port doesn't follow.
            self.fail();
            return;
        }
        self.pos = engine.body;
        self.end = self.lines[line].len();
        let body = match engine.name {
            "ruby" => Body::Ruby,
            name if INTERPOLATING_ENGINES.contains(&name) => Body::Interpolated,
            _ => Body::Opaque,
        };
        let attributes = self.lines[line][engine.attrs.clone()].chars().count();
        let text_indent = self.lines[line][..engine.body].chars().count() + attributes;
        self.text_block(body, true, text_indent);
    }

    fn parse_tag(&mut self, name: &str) {
        self.out.pieces.push(Piece::Static(format!("<{name}")));
        if self.parse_tag_rest(name) {
            self.out.pieces.push(Piece::Close(format!("</{name}>")));
        }
    }

    /// Everything after the tag name; whether the tag needs its closing tag.
    fn parse_tag_rest(&mut self, name: &str) -> bool {
        let mut names = Vec::new();
        // `.class` / `#id` shortcuts, `((?:\p{Word}|-|\/\d+|:(\w|-)+)*)` after each.
        loop {
            let r = self.rest();
            let prefix = r.bytes().take_while(|b| matches!(b, b'#' | b'.')).count();
            if prefix == 0 {
                break;
            }
            if prefix > 1 {
                self.fail_at("Illegal shortcut".to_string(), self.column());
            }
            let mut n = prefix;
            loop {
                let s = &r[n..];
                let Some(c) = s.chars().next() else { break };
                let word_run = |t: &str| t.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')).unwrap_or(t.len());
                let digits = |t: &str| t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
                if is_word(c) || c == '-' {
                    n += c.len_utf8();
                } else if c == '/' && digits(&s[1..]) > 0 {
                    n += 1 + digits(&s[1..]);
                } else if c == ':' && word_run(&s[1..]) > 0 {
                    n += 1 + word_run(&s[1..]);
                } else {
                    break;
                }
            }
            self.note_attribute(&mut names, if r.starts_with('#') { "id" } else { "class" });
            self.skip(n);
        }
        let r = self.rest();
        self.skip(r.bytes().take_while(|b| matches!(b, b'<' | b'>' | b'\'')).count());
        let first_attribute = self.out.pieces.len();
        self.parse_attributes(&mut names);
        let attributes = &mut self.out.pieces[first_attribute..];
        if attributes.iter().any(|p| matches!(p, Piece::Splat { .. })) {
            for piece in attributes {
                if let Piece::Attribute { splat, .. } = piece {
                    *splat = true;
                }
            }
        }

        let r = self.rest();
        let t = trim_start(r);
        let ws = r.len() - t.len();
        let closed = t.starts_with('/');
        let void = closed || VOID_ELEMENTS.contains(&name);
        self.out.pieces.push(Piece::Static(if void { " />" } else { ">" }.to_string()));
        if let Some(after) = t.strip_prefix(':') {
            // Block expansion: `li: a`.
            self.skip(ws + 1 + after.len() - trim_start(after).len());
            let r = self.rest();
            if let Some(engine) = self.match_engine(r) {
                self.embedded(engine);
                return true;
            }
            self.out.tag_slots.push(self.at());
            let Some((len, named)) = match_tag(r) else {
                self.fail_at("Expected tag".to_string(), self.column());
                return true;
            };
            let name = if named { &r[..len] } else { DEFAULT_TAG };
            if named {
                self.skip(len);
            }
            self.parse_tag(name);
        } else if t.starts_with('=') {
            self.skip(ws);
            self.output(true);
        } else if let Some(after) = t.strip_prefix('/') {
            let text = trim_start(after);
            if !text.is_empty() {
                self.fail_at("Unexpected text after closed tag".to_string(), self.chars_before(text));
            }
            self.pos = self.end;
        } else if t.is_empty() {
            self.expecting = true;
        } else {
            self.skip(usize::from(r.starts_with(' ')));
            let text_indent = self.lines[self.i][..self.pos].chars().count();
            self.text_block(Body::Interpolated, true, text_indent);
        }
        !void
    }

    fn note_attribute(&mut self, names: &mut Vec<&'a str>, name: &'a str) {
        if name != "class" && names.contains(&name) {
            self.out.repeated_attribute.get_or_insert((self.owner, name.to_string()));
        }
        names.push(name);
    }

    fn parse_attributes(&mut self, names: &mut Vec<&'a str>) {
        let mut close = None;
        let t = trim_start(self.rest());
        if let Some(c) = t.chars().next().and_then(closer) {
            self.skip(self.rest().len() - t.len() + 1);
            close = Some(c);
        }
        loop {
            let r = self.rest();
            let t = trim_start(r);
            let ws = r.len() - t.len();
            self.out.attr_slots.push(self.at() + ws);
            if t.starts_with('*') && t[1..].starts_with(|c: char| !is_space(c)) {
                self.skip(ws + 1);
                let (_, code, at) = self.ruby_code(close);
                self.out.pieces.push(Piece::Splat { code, at });
                continue;
            }
            let name = attr_name_len(t);
            if name > 0 {
                if let Some(value) = trim_start(&t[name..]).strip_prefix('=') {
                    self.note_attribute(names, &t[..name]);
                    let value = trim_start(value.strip_prefix('=').unwrap_or(value));
                    self.skip(r.len() - value.len());
                    if let Some(quote) = value.chars().next().filter(|c| matches!(c, '"' | '\'')) {
                        self.skip(1);
                        self.out.pieces.push(Piece::Static(format!(" {}=\"", &t[..name])));
                        self.quoted_attribute(quote);
                        self.out.pieces.push(Piece::Static("\"".to_string()));
                    } else {
                        let (taken, code, at) = self.ruby_code(close);
                        if taken == 0 {
                            self.fail_at("Invalid empty attribute".to_string(), self.column());
                        }
                        self.out.pieces.push(Piece::Attribute { name: t[..name].to_string(), code, splat: false, at });
                    }
                    continue;
                }
            }
            let Some(close) = close else { break };
            if name > 0 && t[name..].chars().next().is_none_or(|c| is_space(c) || c == close) {
                self.note_attribute(names, &t[..name]);
                self.skip(ws + name);
            } else if t.starts_with(close) {
                self.skip(ws + 1);
                break;
            } else if !t.is_empty() {
                self.fail_at("Expected attribute".to_string(), self.chars_before(t));
                break;
            } else {
                self.out.pieces.push(Piece::Newline);
                if !self.next_line() {
                    let column = self.lines[self.i].chars().count();
                    self.fail_at(format!("Expected closing delimiter {close}"), column);
                    break;
                }
            }
        }
    }

    /// `parse_ruby_code`: up to whitespace or `outer` outside brackets; a lone `,` or `\` continues it.
    /// Returns the number of characters taken and the code, with a newline where a line continued.
    fn ruby_code(&mut self, outer: Option<char>) -> (usize, String, Origin) {
        let mut start = self.pos;
        let mut taken = 0;
        let mut code = String::new();
        let mut at = Origin { line: self.i, offset: Some(self.starts[self.i] + self.pos) };
        let mut depth = 0;
        let (mut open, mut close) = (' ', ' ');
        loop {
            let r = self.rest();
            let Some(c) = r.chars().next() else { break };
            if depth == 0 && (is_space(c) || Some(c) == outer) {
                break;
            }
            if r == "," || r == "\\" {
                self.ruby(start, self.end);
                taken += 1;
                code.push_str(r);
                code.push('\n');
                at.offset = None;
                if !self.expect_next_line() {
                    return (taken, code, at);
                }
                start = self.pos;
                continue;
            }
            if depth > 0 {
                if c == open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                }
            } else if let Some(cl) = closer(c) {
                depth = 1;
                (open, close) = (c, cl);
            }
            code.push(c);
            self.skip(c.len_utf8());
            taken += 1;
        }
        self.ruby(start, self.pos);
        if depth != 0 {
            self.fail_at(format!("Expected closing delimiter {close}"), self.column());
        }
        (taken, code, at)
    }

    /// `parse_quoted_attribute`: the value is text with `#{}`, and may run across lines.
    fn quoted_attribute(&mut self, quote: char) {
        let mut segment = self.pos;
        let mut depth = 0;
        loop {
            let r = self.rest();
            if depth == 0 && r.starts_with(quote) {
                break;
            }
            if r.is_empty() || r == "\\" {
                self.interpolate(segment, self.pos);
                self.out.lines[self.i].keeps_trailing = true;
                if !self.expect_next_line() {
                    return;
                }
                segment = self.pos;
                continue;
            }
            let c = r.chars().next().unwrap_or(' ');
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            self.skip(c.len_utf8());
        }
        self.interpolate(segment, self.pos);
        self.skip(1);
    }

    /// `parse_broken_line`: the stripped rest, plus following lines while it ends in `,` or `\`.
    fn broken_line(&mut self) -> (String, Origin) {
        let r = self.rest();
        let first = r.trim_matches(is_space);
        let start = self.pos + r.len() - trim_start(r).len();
        self.ruby(start, start + first.len());
        let mut at = Origin { line: self.i, offset: Some(self.starts[self.i] + start) };
        let mut code = first.to_string();
        let mut continues = first.ends_with([',', '\\']);
        let mut previous_empty = first.is_empty();
        while continues {
            at.offset = None;
            if !self.expect_next_line() {
                return (code, at);
            }
            let line = self.rest();
            self.ruby(self.pos, self.end);
            code.push('\n');
            code.push_str(line);
            // An empty line still continues: Slim tests `/[,\\]\Z/` and `\Z` also matches before a final newline.
            continues = if line.is_empty() { !previous_empty } else { line.ends_with([',', '\\']) };
            previous_empty = line.is_empty();
        }
        self.pos = self.end;
        (code, at)
    }

    /// `parse_text_block`: the rest of this line and the following lines indented deeper than this one.
    fn text_block(&mut self, body: Body, raw_column: bool, text_indent: usize) {
        let first = self.i;
        let has_first = self.pos < self.end;
        let mut layout = Layout { text_indent: has_first.then_some(text_indent), blank: 0 };
        // The `ruby:` engine takes the block as one piece of code.
        let mut ruby = matches!(body, Body::Ruby).then(|| RubyBody { line: has_first.then_some(first), text: self.rest().to_string() });
        if ruby.is_some() {
            self.out.pieces.push(Piece::Newline);
        }
        if has_first {
            self.out.lines[first].keeps_trailing = true;
            self.body_segment(body);
        }
        let base = self.base_indent();
        let mut continued = false;
        while let Some(&next) = self.lines.get(self.i + 1) {
            if is_blank(next) {
                self.i += 1;
                layout.push_blank();
                if ruby.is_none() {
                    self.out.pieces.push(Piece::Newline);
                }
                continue;
            }
            if indent_width(next) <= base {
                break;
            }
            self.enter(self.i + 1);
            self.pos = next.len() - trim_start(next).len();
            self.out.lines[self.i].keeps_trailing = true;
            let lead = layout.lead(indent_width(next));
            if let Some(code) = &mut ruby {
                code.push_line(self.i, &lead, self.rest());
            } else {
                self.out.pieces.push(Piece::Newline);
                self.out.pieces.push(Piece::Static(lead));
            }
            self.body_segment(body);
            continued = true;
        }
        if let Some(code) = ruby {
            self.out.pieces.push(Piece::Code { text: code.finish(), at: Origin { line: code.line.unwrap_or(first), offset: None } });
        }
        if raw_column && has_first && continued {
            self.out.lines[first].raw_column = true;
        }
    }

    fn body_segment(&mut self, body: Body) {
        match body {
            Body::Ruby => self.ruby(self.pos, self.end),
            Body::Interpolated => self.interpolate(self.pos, self.end),
            Body::Opaque => {}
        }
        self.pos = self.end;
    }

    fn comment_block(&mut self) {
        let base = self.base_indent();
        while let Some(&next) = self.lines.get(self.i + 1) {
            if is_blank(next) {
                self.i += 1;
                self.out.pieces.push(Piece::Newline);
            } else if indent_width(next) > base {
                self.enter(self.i + 1);
                self.out.pieces.push(Piece::Newline);
            } else {
                break;
            }
        }
    }

    /// Ruby spans of the `#{…}` in columns `from..to` of the current line; `\#{` is literal.
    fn interpolate(&mut self, from: usize, to: usize) {
        let s = &self.lines[self.i][from..to];
        let base = self.starts[self.i];
        self.out.text.push(base + from..base + to);
        let closers = matching_braces(s);
        let mut text = String::new();
        let mut k = 0;
        while k < s.len() {
            let r = &s[k..];
            if r.starts_with("\\#{") {
                text.push_str("#{");
                k += 3;
                continue;
            }
            if r.starts_with("#{") {
                if let Some(close) = closers[k + 1] {
                    let len = close - k - 1;
                    let inner = &r[2..len];
                    // `#{{x}}` is unescaped output of `x`.
                    let unescaped = inner.len() >= 2 && inner.starts_with('{') && inner.ends_with('}');
                    let (a, b) = if unescaped { (k + 3, k + len - 1) } else { (k + 2, k + len) };
                    self.ruby(from + a, from + b);
                    let at = Origin { line: self.i, offset: Some(base + from + a) };
                    self.out.pieces.push(Piece::Static(std::mem::take(&mut text)));
                    self.out.pieces.push(Piece::Output { escape: !unescaped, code: s[a..b].to_string(), block: false, at });
                    k += 1 + len;
                    continue;
                }
            }
            let c = r.chars().next().unwrap_or(' ');
            text.push(c);
            k += c.len_utf8();
        }
        self.out.pieces.push(Piece::Static(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::position_to_offset;
    use tower_lsp::lsp_types::Position;

    /// The Ruby source of every span, in order.
    fn ruby(text: &str) -> Vec<&str> {
        parse(text, &[]).ruby.iter().map(|r| &text[r.clone()]).filter(|s| !s.is_empty()).collect()
    }

    #[test]
    fn ruby_spans() {
        let cases: &[(&str, &[&str])] = &[
            ("- if x\n  = y\n== z", &["if x", "y", "z"]),
            ("=< a\n=> b\n==<> c\n=' s'", &["a", "b", "c", "' s'"]),
            ("p= x\np==y\na>= z\na(href=\"/\")= w\np =' v", &["x", "y", "z", "w", "v"]),
            ("a href=user_path(u, x: 1) class=\"c #{k}\" title='#{t}' Text", &["user_path(u, x: 1)", "k", "t"]),
            ("a href=\"/\" = link", &["link"]),
            ("a(href=foo[1] disabled) x #{y}", &["foo[1]", "y"]),
            ("a[x=a(1) y=\"#{b}\"]", &["a(1)", "b"]),
            ("div{data=h} t", &["h"]),
            ("div *{role: \"row\"} *attrs text", &["{role: \"row\"}", "attrs"]),
            ("*{tag: :a} x", &["{tag: :a}"]),
            ("| Hi #{name} \\#{not} #{{raw}}", &["name", "raw"]),
            ("p Text\n  more #{x}\n'  y #{z}", &["x", "z"]),
            ("/! c #{x}\n/ c #{no}\n  #{no}", &["x"]),
            ("<div id=\"#{d}\">", &["d"]),
            ("ruby:\n  x = 1\n  y = 2\np", &["x = 1", "y = 2"]),
            ("javascript:\n  var a = #{j};\nsass:\n  .a\n    b: #{c}\ncoffee:\n  x = \"#{y}\"", &["j"]),
            ("li: a href=h Home", &["h"]),
            ("= link_to a,\n    b\np c", &["link_to a,", "b"]),
            ("- x = [1,\n\n  2]", &["x = [1,", "2]"]),
            ("a href=x,\n  y c", &["x,", "y"]),
            ("a(href=\"/\"\n  class=k) t", &["k"]),
            ("p title=\"a\n  #{b}\" c", &["b"]),
            ("javascript nonce=n:\n  #{x}", &["n", "x"]),
            ("doctype html\n/[if IE]\n  p #{ie}", &["ie"]),
        ];
        for (text, want) in cases {
            assert_eq!(ruby(text), *want, "{text:?}");
        }
    }

    #[test]
    fn in_ruby_positions() {
        let text = "p= link_\n- # note\n| a #{b} c\na href=x Text\né= y";
        let parsed = parse(text, &[]);
        let at = |line, character| position_to_offset(text, Position::new(line, character)).is_some_and(|o| in_ruby(text, &parsed, o));
        assert!(at(0, 8), "end of `p= link_`");
        assert!(!at(0, 1));
        assert!(!at(1, 7), "inside a Ruby comment");
        assert!(at(2, 6) && !at(2, 9));
        assert!(at(3, 8) && !at(3, 12));
        assert!(at(4, 4), "UTF-16 column after `é`");
        assert!(!at(9, 0), "past the end");
    }

    #[test]
    fn open_interpolation_while_typing() {
        let open = |text: &str| {
            let offset = text.find('^').unwrap();
            let text = text.replacen('^', "", 1);
            let parsed = parse(&text, &[]);
            !in_ruby(&text, &parsed, offset) && in_open_interpolation(&text, &parsed, offset)
        };
        assert!(open("p Hi #{user.na^"));
        assert!(open("a title=\"#{t(^\" x"));
        assert!(open("| a #{x} #{h({^"));
        assert!(!open("p Hi \\#{user^"));
        assert!(!open("p Hi #{a} b^"));
        assert!(!open("- x = \"#{a^"), "inside Ruby already");
    }

    #[test]
    fn byte_order_mark_is_skipped() {
        let text = "\u{feff}doctype html\np= x";
        let p = parse(text, &[]);
        assert!(p.valid);
        assert_eq!(ruby(text), ["x"]);
    }

    #[test]
    fn many_unclosed_interpolations_stay_linear() {
        let text = format!("p {}", "#{{".repeat(200_000));
        let started = std::time::Instant::now();
        assert!(parse(&text, &[]).ruby.is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn extra_engines_hide_their_body() {
        let text = "mjml:\n  = not_ruby";
        assert_eq!(ruby(text), ["not_ruby"]);
        let extra = ["mjml".to_string()];
        assert!(parse(text, &extra).ruby.is_empty());
    }

    #[test]
    fn roles_and_validity() {
        let text = "html\n  body\n    p Text\n      more\n\n    = x,\n        y\n  / c\n    gone\n";
        let p = parse(text, &[]);
        assert!(p.valid);
        let roles: Vec<Role> = p.lines.iter().map(|l| l.role).collect();
        use Role::*;
        assert_eq!(
            roles,
            [
                Structural { depth: 0 },
                Structural { depth: 1 },
                Structural { depth: 2 },
                Inner { owner: 2 },
                Blank,
                Structural { depth: 2 },
                Inner { owner: 5 },
                Structural { depth: 1 },
                Inner { owner: 7 },
                Blank,
            ]
        );
        assert!(p.lines[2].keeps_trailing && p.lines[2].raw_column && !p.lines[5].keeps_trailing);
        for invalid in ["p\n    a\n  b", "doctype html\n  p", "a(href=x", "p title=\"x", "> smart", "img/ x", "- a,"] {
            assert!(!parse(invalid, &[]).valid, "{invalid:?}");
        }
    }

    #[test]
    fn slots_and_markers() {
        let text = "li: a href=\"/\" cl\ninput(type=\"x\" che)\n==< x\n/! c\n|> t";
        let p = parse(text, &[]);
        assert_eq!(p.tag_slots, [0, 4, 18]);
        for word in ["href", "cl", "type", "che"] {
            assert!(p.attr_slots.contains(&text.find(word).unwrap()), "{word}");
        }
        let markers: Vec<(&str, Marker)> = p.markers.iter().map(|(r, m)| (&text[r.clone()], *m)).collect();
        assert_eq!(markers, [("==<", Marker::Output), ("/!", Marker::HtmlComment), ("|>", Marker::Text)]);
    }
}
