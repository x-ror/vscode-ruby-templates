//! A port of the line and indentation handling of `Haml::Parser` (Haml 7.5.1, `lib/haml/parser.rb`):
//! the first error Haml raises about indentation, at the line it names.

use crate::haml_indent::{block_keyword, is_haml_multiline, is_ruby_multiline, strip_output_prefix, MID_BLOCK_KEYWORDS};
use std::collections::VecDeque;
use std::convert::Infallible;

pub const BOM: char = '\u{feff}';
const AT_START: &str = "Indenting at the beginning of the document is illegal.";
const UNBALANCED: &str = "Unbalanced brackets.";
const ILLEGAL_ELEMENT: &str = "Illegal element: classes and ids must have values.";
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
    /// One entry per open block; `true` for a filter or `-#` comment, whose end closes the flat section.
    open_blocks: Vec<bool>,
    tab_up: bool,
}

/// The first indentation error Haml raises for `text`.
pub fn first_error(text: &str) -> Option<Error> {
    let mut parser = Parser::new(text.strip_prefix(BOM).unwrap_or(text));
    match parser.run() {
        Err(Halt::Error(error)) => Some(error),
        Err(Halt::Done) => None,
    }
}

/// The lines of `text` as Haml (and the LSP) split them: at `\r\n`, `\r` and `\n`.
pub fn physical_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut rest = text;
    loop {
        let end = rest.find(['\r', '\n']).unwrap_or(rest.len());
        lines.push(&rest[..end]);
        let tail = &rest[end..];
        let skip = if tail.starts_with("\r\n") { 2 } else { tail.len().min(1) };
        if skip == 0 {
            return lines;
        }
        rest = &tail[skip..];
    }
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
    fn new(text: &str) -> Parser {
        let mut template: VecDeque<Line> = split_lines(text).into();
        let index = template.len();
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
                self.advance();
                continue;
            }
            self.tab_up = false;
            let opens_flat_section = !self.line.text.is_empty() && self.process_line()?;
            if self.block_opened()? || self.tab_up {
                self.template_tabs += 1;
                self.open_blocks.push(opens_flat_section);
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
        while let Some(next) = self.template.front() {
            let text = next.text.trim_matches(RUBY_WHITESPACE);
            if next.end_of_document || (!text.is_empty() && !is_haml_multiline(text)) {
                break;
            }
            line.text.push_str(text.strip_suffix('|').unwrap_or(text));
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
        if self.open_blocks.pop() == Some(true) {
            self.flat = false;
            self.flat_spaces = None;
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

    fn start_flat_section(&mut self) -> bool {
        if !self.filter_opened() {
            return false;
        }
        self.flat = true;
        self.flat_spaces = self.indentation.as_ref().map(|indentation| indentation.repeat(self.template_tabs + 1));
        true
    }

    /// What `process_line` does that later lines depend on: the lines a node consumes and the flat sections it opens.
    /// Returns whether the line is a filter or `-#` comment whose block is flat.
    fn process_line(&mut self) -> Result<bool, Halt> {
        let text = std::mem::take(&mut self.line.text);
        let result = self.process_text(&text);
        self.line.text = text;
        result
    }

    fn process_text(&mut self, text: &str) -> Result<bool, Halt> {
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some('%' | '.'), _) => self.tag(text)?,
            (Some('#'), second) if !matches!(second, Some('{' | '@' | '$')) => self.tag(text)?,
            (Some('-'), Some('#')) | (Some(':'), _) => return Ok(self.start_flat_section()),
            (Some('/'), _) => self.comment(&text[1..])?,
            (Some('!'), _) if text.starts_with("!!!") => self.doctype()?,
            (Some('-' | '=' | '~' | '&' | '!'), _) => match text.strip_prefix('-').or_else(|| strip_output_prefix(text)) {
                Some(code) => self.script(text, code)?,
                None => self.plain()?,
            },
            _ => self.plain()?,
        }
        Ok(false)
    }

    fn nested_line_error(&mut self, message: &str) -> Halt {
        halt(message, self.next_line.index)
    }

    fn plain(&mut self) -> Result<(), Halt> {
        if self.block_opened()? {
            return Err(self.nested_line_error(NESTING_PLAIN));
        }
        Ok(())
    }

    fn doctype(&mut self) -> Result<(), Halt> {
        if self.block_opened()? {
            return Err(self.nested_line_error(NESTING_HEADER));
        }
        Ok(())
    }

    /// `comment`: `/ text` and `/[if IE] text` can't have nested lines.
    fn comment(&mut self, text: &str) -> Result<(), Halt> {
        let text = text.trim_start_matches(RUBY_WHITESPACE);
        let text = if text.starts_with("![") { &text[1..] } else { text };
        let text = if text.starts_with('[') { balance_brackets(text).ok_or_else(|| halt(UNBALANCED, self.line.index))? } else { text };
        let has_text = !text.trim_matches(RUBY_WHITESPACE).is_empty();
        if self.block_opened()? && has_text {
            return Err(self.nested_line_error(NESTING_CONTENT));
        }
        Ok(())
    }

    /// `check_push_script_stack` and `handle_ruby_multiline` for a line that starts with Ruby.
    fn script(&mut self, text: &str, code: &str) -> Result<(), Halt> {
        if block_keyword(code).is_some_and(|keyword| matches!(keyword, "if" | "case" | "unless")) {
            self.tab_up = true;
        }
        self.join_ruby_lines(text)
    }

    /// `parse_tag` and `tag`: attribute groups that run over several lines, the nesting rules, then Ruby after the tag
    /// that continues after a comma.
    fn tag(&mut self, text: &str) -> Result<(), Halt> {
        let Some((name, attributes_start, head_len)) = tag_head(text) else { return Ok(()) };
        if has_empty_class_or_id(&text[attributes_start..head_len]) {
            return Err(halt(ILLEGAL_ELEMENT, self.line.index));
        }
        let mut rest = text[head_len..].to_string();
        let (mut hash, mut list, mut reference) = (false, false, false);
        // The line the last attribute group ends on, which Haml reports some errors at.
        let mut last_line = self.line.index;
        loop {
            match rest.chars().next() {
                Some('{') if !hash => {
                    hash = true;
                    (rest, last_line) = self.skip_hash(&rest)?;
                }
                Some('(') if !list => {
                    list = true;
                    (rest, last_line) = self.skip_attribute_list(&rest)?;
                }
                Some('[') if !reference => {
                    reference = true;
                    rest = balance_brackets(&rest).ok_or_else(|| halt(UNBALANCED, self.line.index))?.to_string();
                }
                _ => break,
            }
        }
        let content = tag_content(&rest);
        // This reads the next line's indentation before any Ruby lines are joined.
        let block_opened = self.block_opened()?;
        if block_opened && content.self_closing {
            return Err(self.nested_line_error(NESTING_SELF_CLOSING));
        }
        if content.self_closing && !content.value.is_empty() {
            return Err(halt(SELF_CLOSING_CONTENT, last_line));
        }
        if block_opened && !content.value.is_empty() && !is_ruby_multiline(&content.value) {
            let message = format!("Illegal nesting: content can't be both given on the same line as %{name} and nested within it.");
            return Err(self.nested_line_error(&message));
        }
        if content.parse {
            self.join_ruby_lines(&content.value)?;
        }
        Ok(())
    }

    /// `parse_old_attributes`: what follows the `{...}` that starts `text`, which may close on a later line.
    fn skip_hash(&mut self, text: &str) -> Result<(String, usize), Halt> {
        let mut scanner = BraceScanner::new();
        let mut segment = text[1..].to_string();
        let mut last_line = self.line.index;
        loop {
            if let Some(end) = scanner.feed(&segment) {
                return Ok((segment[end..].to_string(), last_line));
            }
            if self.next_line.end_of_document {
                return Err(halt(UNBALANCED, self.line.index));
            }
            segment = self.next_line.text.clone();
            last_line += 1;
            self.next_line()?;
        }
    }

    /// `parse_new_attributes`: what follows the `(...)` that starts `text`, whose attributes may continue on later lines.
    fn skip_attribute_list(&mut self, text: &str) -> Result<(String, usize), Halt> {
        let mut joined = text.to_string();
        let mut segment = text[1..].trim_start_matches(REGEXP_SPACE).to_string();
        let mut continuations = 0;
        loop {
            match scan_attribute_list(&segment) {
                AttributeList::Closed(end) => return Ok((segment[end..].to_string(), self.line.index + continuations)),
                AttributeList::Unbalanced => return Err(halt(UNBALANCED, self.line.index)),
                AttributeList::Invalid => {
                    let fragment = balance_parentheses(&joined).unwrap_or(&joined);
                    return Err(halt(format!("Invalid attribute list: {}.", ruby_inspect(fragment)), self.line.index + continuations));
                }
                AttributeList::Continues => {
                    joined.push(' ');
                    joined.push_str(&self.next_line.text);
                    segment = self.next_line.text.clone();
                    continuations += 1;
                    self.next_line()?;
                }
            }
        }
    }

    /// `handle_ruby_multiline`: lines after one ending in a comma belong to it.
    fn join_ruby_lines(&mut self, code: &str) -> Result<(), Halt> {
        if !is_ruby_multiline(code.trim_end_matches(RUBY_WHITESPACE)) {
            return Ok(());
        }
        let continues = |line: &Line| !line.end_of_document && is_ruby_multiline(&line.text);
        let mut joined_continues = continues(&self.next_line);
        while joined_continues {
            let Some(next) = self.template.pop_front() else { break };
            joined_continues = continues(&next);
        }
        self.next_line()
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

/// What follows a tag's attributes, as `tag` reads it.
struct TagContent {
    self_closing: bool,
    /// Haml evaluates `value` (`%p= x`, `%p~ x`, `%p& = x`).
    parse: bool,
    /// The text or code; text with `#{}` or `==` is a string literal in Haml, which stands as `""` here.
    value: String,
}

fn tag_content(rest: &str) -> TagContent {
    let rest = rest.strip_prefix("<>").or_else(|| rest.strip_prefix("><")).or_else(|| rest.strip_prefix(['<', '>'])).unwrap_or(rest);
    let action = rest.chars().next().filter(|ch| "=/~&!".contains(*ch));
    let value = rest[action.map_or(0, char::len_utf8)..].trim_matches(RUBY_WHITESPACE);
    let interpolated = || "\"\"".to_string();
    let has_interpolation = ["#{", "#$", "#@"].iter().any(|opener| value.contains(opener));
    let (parse, value) = match action {
        Some('/') => {
            return TagContent { self_closing: true, parse: false, value: value.to_string() };
        }
        Some('~') => (true, value.to_string()),
        Some('=') if value.starts_with('=') => (true, interpolated()),
        Some('=') => (true, value.to_string()),
        Some('&' | '!') => match value.strip_prefix(['=', '~']) {
            Some(code) if code.starts_with('=') => (true, interpolated()),
            Some(code) => (true, code.trim_matches(RUBY_WHITESPACE).to_string()),
            None if has_interpolation => (true, interpolated()),
            None => (false, value.to_string()),
        },
        _ if has_interpolation => (true, interpolated()),
        _ => (false, value.to_string()),
    };
    TagContent { self_closing: false, parse, value }
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

/// `parse_new_attribute` in a loop over one line of `(a=1 b="x#{y}" c)`, starting at an attribute.
fn scan_attribute_list(segment: &str) -> AttributeList {
    let mut at = 0;
    loop {
        let rest = &segment[at..];
        let name_len = rest.find(|ch: char| !(is_word(ch) || matches!(ch, '-' | ':' | '@' | '#' | '.'))).unwrap_or(rest.len());
        if name_len == 0 {
            return if rest.starts_with(')') { AttributeList::Closed(at + 1) } else { AttributeList::Invalid };
        }
        let after_name = rest[name_len..].trim_start_matches(REGEXP_SPACE);
        at = segment.len() - after_name.len();
        if let Some(after_equals) = after_name.strip_prefix('=') {
            let value = after_equals.trim_start_matches(REGEXP_SPACE);
            match attribute_value_len(value) {
                Ok(value_len) => at = segment.len() - value.len() + value_len,
                Err(failure) => return failure,
            }
        }
        at = segment.len() - segment[at..].trim_start_matches(REGEXP_SPACE).len();
        if at == segment.len() {
            return AttributeList::Continues;
        }
    }
}

/// Length of a quoted string (with `#{}` interpolation) or a variable at the start of `value`; the failure otherwise.
fn attribute_value_len(value: &str) -> Result<usize, AttributeList> {
    let Some(quote @ ('"' | '\'')) = value.chars().next() else {
        let variable = ["@@", "@", "$"].iter().find_map(|prefix| value.strip_prefix(prefix)).unwrap_or(value);
        let name_len = variable.find(|ch: char| !is_word(ch)).unwrap_or(variable.len());
        return if name_len > 0 { Ok(value.len() - variable.len() + name_len) } else { Err(AttributeList::Invalid) };
    };
    let mut at = 1;
    while let Some(ch) = value[at..].chars().next() {
        let after = &value[at + ch.len_utf8()..];
        match ch {
            '\\' => match after.chars().next() {
                Some(escaped) => at += 1 + escaped.len_utf8(),
                None => break,
            },
            '#' if after.starts_with('{') => {
                // Haml's `balance`, which counts only the braces.
                let mut depth = 1;
                let close = after[1..].char_indices().find_map(|(offset, brace)| {
                    depth += match brace {
                        '{' => 1,
                        '}' => -1,
                        _ => 0,
                    };
                    (depth == 0).then_some(offset)
                });
                let Some(close) = close else { return Err(AttributeList::Unbalanced) };
                at += 2 + close + 1;
            }
            ch if ch == quote => return Ok(at + 1),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn error_of(text: &str) -> Option<(usize, String)> {
        first_error(text).map(|e| (e.line, e.message))
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
}
