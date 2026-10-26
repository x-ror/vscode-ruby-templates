//! A port of the line and indentation handling of `Haml::Parser` (Haml 7.5.1, `lib/haml/parser.rb`):
//! the first error Haml raises about indentation, at the line it names.

use crate::haml_indent::{block_keyword, is_haml_multiline, is_ruby_multiline, strip_output_prefix, MID_BLOCK_KEYWORDS};
use std::collections::VecDeque;
use std::convert::Infallible;

pub const BOM: char = '\u{feff}';
const AT_START: &str = "Indenting at the beginning of the document is illegal.";
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
            (Some('-' | '=' | '~' | '&' | '!'), _) if !text.starts_with("!!!") => {
                if let Some(code) = text.strip_prefix('-').or_else(|| strip_output_prefix(text)) {
                    self.script(text, code)?;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    /// `check_push_script_stack` and `handle_ruby_multiline` for a line that starts with Ruby.
    fn script(&mut self, text: &str, code: &str) -> Result<(), Halt> {
        if block_keyword(code).is_some_and(|keyword| matches!(keyword, "if" | "case" | "unless")) {
            self.tab_up = true;
        }
        self.join_ruby_lines(text)
    }

    /// `parse_tag`: attribute groups that run over several lines, then Ruby after the tag that continues after a comma.
    fn tag(&mut self, text: &str) -> Result<(), Halt> {
        let Some(head_len) = tag_head_len(text) else { return Ok(()) };
        let mut rest = text[head_len..].to_string();
        let (mut hash, mut list, mut reference) = (false, false, false);
        loop {
            match rest.chars().next() {
                Some('{') if !hash => {
                    hash = true;
                    rest = self.skip_hash(&rest)?;
                }
                Some('(') if !list => {
                    list = true;
                    rest = self.skip_attribute_list(&rest)?;
                }
                Some('[') if !reference => {
                    reference = true;
                    rest = skip_object_reference(&rest);
                }
                _ => break,
            }
        }
        // Haml checks for nesting here, which reads the next line's indentation before any Ruby lines are joined.
        self.block_opened()?;
        match tag_code(&rest) {
            Some(code) => self.join_ruby_lines(code),
            None => Ok(()),
        }
    }

    /// `parse_old_attributes`: what follows the `{...}` that starts `text`, which may close on a later line.
    fn skip_hash(&mut self, text: &str) -> Result<String, Halt> {
        let mut scanner = BraceScanner::new();
        let mut segment = text[1..].to_string();
        loop {
            if let Some(end) = scanner.feed(&segment) {
                return Ok(segment[end..].to_string());
            }
            if self.next_line.end_of_document {
                return Ok(String::new());
            }
            segment = self.next_line.text.clone();
            self.next_line()?;
        }
    }

    /// `parse_new_attributes`: what follows the `(...)` that starts `text`, whose attributes may continue on later lines.
    fn skip_attribute_list(&mut self, text: &str) -> Result<String, Halt> {
        let mut segment = text[1..].trim_start_matches(REGEXP_SPACE).to_string();
        loop {
            match scan_attribute_list(&segment) {
                AttributeList::Closed(end) => return Ok(segment[end..].to_string()),
                AttributeList::Invalid => return Ok(String::new()),
                AttributeList::Continues => {
                    segment = self.next_line.text.clone();
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

/// Length of `%name.class#id` (Haml's `%([-:\w]+)([-:\w.#\@]*)`), or of `.class#id` for the `%div` a leading `.` or `#` implies.
fn tag_head_len(text: &str) -> Option<usize> {
    let name_len = match text.strip_prefix('%') {
        Some(after) => 1 + after.find(|ch: char| !(is_word(ch) || matches!(ch, '-' | ':'))).unwrap_or(after.len()),
        None => 0,
    };
    if name_len == 1 {
        return None;
    }
    let attributes = &text[name_len..];
    Some(name_len + attributes.find(|ch: char| !(is_word(ch) || matches!(ch, '-' | ':' | '.' | '#' | '@'))).unwrap_or(attributes.len()))
}

/// What follows `[...]` (Haml's `balance`, which counts only the brackets); the rest of the line when it never closes.
fn skip_object_reference(text: &str) -> String {
    let mut depth = 0;
    for (at, ch) in text.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => depth -= 1,
            _ => continue,
        }
        if depth == 0 {
            return text[at + 1..].to_string();
        }
    }
    String::new()
}

/// The Ruby Haml evaluates after a tag's attributes (`%p= x`, `%p~ x`, `%p& = x`); `None` for text, `/` and interpolation.
fn tag_code(rest: &str) -> Option<&str> {
    let rest = rest.strip_prefix("<>").or_else(|| rest.strip_prefix("><")).or_else(|| rest.strip_prefix(['<', '>'])).unwrap_or(rest);
    let action = rest.chars().next().filter(|ch| "=/~&!".contains(*ch))?;
    let value = rest[1..].trim_matches(RUBY_WHITESPACE);
    match action {
        '=' => (!value.starts_with('=')).then_some(value),
        '~' => Some(value),
        '&' | '!' => {
            let code = value.strip_prefix(['=', '~'])?;
            (!code.starts_with('=')).then(|| code.trim_matches(RUBY_WHITESPACE))
        }
        _ => None,
    }
}

enum AttributeList {
    /// Offset just past the `)`.
    Closed(usize),
    /// The line ended between attributes.
    Continues,
    /// Haml's "Invalid attribute list".
    Invalid,
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
            let Some(value_len) = attribute_value_len(value) else { return AttributeList::Invalid };
            at = segment.len() - value.len() + value_len;
        }
        at = segment.len() - segment[at..].trim_start_matches(REGEXP_SPACE).len();
        if at == segment.len() {
            return AttributeList::Continues;
        }
    }
}

/// Length of a quoted string (with `#{}` interpolation) or a variable at the start of `value`.
fn attribute_value_len(value: &str) -> Option<usize> {
    let Some(quote @ ('"' | '\'')) = value.chars().next() else {
        let variable = value.trim_start_matches(['@', '$']);
        let name_len = variable.find(|ch: char| !is_word(ch)).unwrap_or(variable.len());
        return (name_len > 0).then_some(value.len() - variable.len() + name_len);
    };
    let mut chars = value.char_indices().skip(1).peekable();
    while let Some((at, ch)) = chars.next() {
        match ch {
            '\\' => {
                chars.next();
            }
            '#' if chars.peek().is_some_and(|&(_, next)| next == '{') => {
                let interpolation = &value[at + 2..];
                let mut scanner = BraceScanner::new();
                let end = scanner.feed(interpolation)?;
                let resume = at + 2 + end;
                while chars.peek().is_some_and(|&(next, _)| next < resume) {
                    chars.next();
                }
            }
            ch if ch == quote => return Some(at + 1),
            _ => {}
        }
    }
    None
}

#[derive(Clone, Copy)]
enum Mode {
    Code { braces: usize },
    Text { closer: char, opener: Option<char>, nested: usize, interpolates: bool },
}

/// Finds the `}` that closes a Ruby `{` over several lines, as Haml does by lexing the text with Prism: a `}` inside a
/// string, comment, `%w()` literal, regexp or `?}` doesn't count. Lines are fed once each.
struct BraceScanner {
    modes: Vec<Mode>,
    previous: Option<char>,
}

impl BraceScanner {
    /// Starts just after the opening `{`.
    fn new() -> BraceScanner {
        BraceScanner { modes: vec![Mode::Code { braces: 0 }], previous: Some('{') }
    }

    /// A value (string, regexp, literal) can start here, rather than an operator.
    fn value_expected(&self) -> bool {
        self.previous.is_none_or(|ch| "([{,;=:|&!<>?+-*/%^~".contains(ch))
    }

    /// The offset just past the closing `}` when this line holds it.
    fn feed(&mut self, line: &str) -> Option<usize> {
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let mut i = 0;
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
                Mode::Code { braces } => {
                    i += 1;
                    match ch {
                        ' ' | '\t' => {}
                        '#' => return None,
                        '"' | '`' | '\'' => {
                            self.modes.push(Mode::Text { closer: ch, opener: None, nested: 0, interpolates: ch != '\'' });
                        }
                        '/' if self.value_expected() => {
                            self.modes.push(Mode::Text { closer: '/', opener: None, nested: 0, interpolates: true });
                        }
                        '%' if self.value_expected() => i = self.percent_literal(&chars, i - 1),
                        '?' if self.value_expected() && next.is_some_and(|c| !c.is_whitespace()) => {
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
                        _ => self.previous = Some(ch),
                    }
                }
            }
        }
        None
    }

    /// `%w(`, `%(`, `%q|`: pushes the literal and returns the index after its delimiter; a lone `%` is an operator.
    fn percent_literal(&mut self, chars: &[(usize, char)], percent: usize) -> usize {
        let kind = chars.get(percent + 1).map(|&(_, ch)| ch).filter(|ch| "wWiIqQrsx".contains(*ch));
        let delimiter_at = percent + 1 + usize::from(kind.is_some());
        let Some(&(_, delimiter)) = chars.get(delimiter_at).filter(|(_, ch)| !ch.is_alphanumeric() && !ch.is_whitespace()) else {
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
        assert_eq!(tag_code("{b: 2}= foo(x,"), None);
        assert_eq!(tag_code("& = format(x,"), Some("format(x,"));
        assert_eq!(tag_code("== x,"), None);
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
        assert_eq!(error_of(&text), None);
    }
}
