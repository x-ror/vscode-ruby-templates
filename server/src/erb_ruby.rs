//! Ruby syntax errors in an ERB template: the method source Rails 8.1.4 evals for it (Erubi 1.13.1 with the
//! ActionView handler's options and method wrapper), parsed by Prism.

use crate::generated_ruby::{generated_offset, template_span};
use crate::prism;
use crate::strict_locals::{self, Close, Comment, Rails};
use crate::util::{error, LineIndex};
use regex::Regex;
use std::ops::Range as ByteRange;
use std::sync::LazyLock;
use tower_lsp::lsp_types::{Diagnostic, Range};

const OUTPUT_BUFFER: &str = " @output_buffer";
/// The start of the method Rails wraps a template in, up to its parameters.
const WRAPPER_DEF: &str = "          def _template(";
/// Prism's message for the wrapper's own `def`, which the template's code never gets to close.
const UNCLOSED_DEF: &str = "expected an `end` to close the `def` statement";

/// One match of `Erubi::Engine::DEFAULT_REGEXP`, `<%(={1,2}|-|#|%)?(.*?)([-=])?%>([ \t]*\r?\n)?`, by its groups.
struct Match<'a> {
    whole: ByteRange<usize>,
    indicator: Option<&'a str>,
    code: ByteRange<usize>,
    tail: Option<&'a str>,
    rspace: Option<&'a str>,
}

/// The regex's matches, scanned by hand: the lazy `(.*?)` with captures made the `regex` crate's slow engines run per tag.
fn erubi_tags(text: &str) -> impl Iterator<Item = Match<'_>> {
    let bytes = text.as_bytes();
    let mut pos = 0;
    std::iter::from_fn(move || {
        let open = pos + text.get(pos..)?.find("<%")?;
        let longest = match &bytes[open + 2..] {
            [b'=', b'=', ..] => 2,
            [b'=' | b'-' | b'#' | b'%', ..] => 1,
            _ => 0,
        };
        // The regex tries the longest indicator first; only `<%%>` matches without the one it has.
        let (indicator_len, code_start, close) = (0..=longest).rev().find_map(|len| {
            let code_start = open + 2 + len;
            text[code_start..].find("%>").map(|at| (len, code_start, code_start + at))
        })?;
        let tail = close > code_start && matches!(bytes[close - 1], b'-' | b'=');
        let code_end = close - usize::from(tail);
        let end = close + 2;
        let spaces = bytes[end..].iter().take_while(|&&b| b == b' ' || b == b'\t').count();
        let newline = match &bytes[end + spaces..] {
            [b'\r', b'\n', ..] => 2,
            [b'\n', ..] => 1,
            _ => 0,
        };
        let rspace = (newline > 0).then(|| &text[end..end + spaces + newline]);
        pos = end + rspace.map_or(0, str::len);
        Some(Match {
            whole: open..pos,
            indicator: (indicator_len > 0).then(|| &text[open + 2..code_start]),
            code: code_start..code_end,
            tail: tail.then(|| &text[code_end..close]),
            rspace,
        })
    })
}
// `ActionView::Template::Handlers::ERB::ENCODING_TAG`.
static ENCODING_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A<%#.*coding[:=][ \t\r\n\x0C\x0B]*\S+[ \t]*-?%>[ \t]*").unwrap());
// `ActionView::Template::Handlers::ERB::Erubi::BLOCK_EXPR`.
static BLOCK_EXPR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:(?:[ \t\r\n\x0C\x0B]|\))do|\{)(?:[ \t\r\n\x0C\x0B]*\|[^|]*\|)?[ \t\r\n\x0C\x0B]*\z").unwrap());
/// `Erubi::Engine` as `ActionView::Template::Handlers::ERB::Erubi` configures it (`trim: true`, `@output_buffer`).
#[derive(Default)]
struct Erubi {
    src: String,
    /// Lone "\n" texts wait here, so they land inside the next text literal (the handler's `@newline_pending`).
    newline_pending: usize,
    /// Code copied as it is: its byte range in `src` and the offset of its first byte in the template Erubi read.
    verbatim: Vec<(usize, usize, usize)>,
    /// The texts before the text after the last tag, as the HTML they render (an output tag as one character).
    texts: Vec<String>,
    /// Where the text after the last tag starts in the template Erubi read.
    tail_at: usize,
}

/// The value is unknown; one plain character keeps a tag or an unquoted value open as the output would.
const OUTPUT_PLACEHOLDER: &str = "x";

impl Erubi {
    fn add_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if text == "\n" {
            self.newline_pending += 1;
            return;
        }
        self.append_literal(text);
    }

    fn flush_newline_if_pending(&mut self) {
        if self.newline_pending > 0 {
            self.append_literal("");
        }
    }

    /// The pending newlines and `text` as one `safe_append` literal.
    fn append_literal(&mut self, text: &str) {
        self.src.push_str(OUTPUT_BUFFER);
        self.src.push_str(".safe_append='");
        self.src.push_str(&"\n".repeat(self.newline_pending));
        for c in text.chars() {
            if matches!(c, '\'' | '\\') {
                self.src.push('\\');
            }
            self.src.push(c);
        }
        self.src.push_str("'.freeze;");
        self.newline_pending = 0;
    }

    /// Appends `code` (a tag's Ruby, around which Erubi may have put whitespace) and returns where it starts in `src`.
    fn add_code(&mut self, code: &str) -> usize {
        self.flush_newline_if_pending();
        let start = self.src.len();
        self.src.push_str(code);
        if !code.ends_with('\n') {
            self.src.push(';');
        }
        start
    }

    fn add_expression(&mut self, indicator: &str, code: &str, at: usize) {
        self.flush_newline_if_pending();
        self.src.push_str(OUTPUT_BUFFER);
        self.src.push_str(if indicator == "==" { ".safe_expr_append=" } else { ".append=" });
        let block = BLOCK_EXPR.is_match(code);
        self.src.push_str(if block { " " } else { "(" });
        self.record(self.src.len(), code, at);
        self.src.push_str(code);
        if !block {
            self.src.push(')');
        }
        self.src.push(';');
    }

    fn record(&mut self, start: usize, code: &str, at: usize) {
        if !code.is_empty() {
            self.verbatim.push((start, start + code.len(), at));
        }
    }

    /// `add_text`, keeping `text` for `rendered_texts`; every text but the one after the last tag goes through here.
    fn add_rendered_text(&mut self, text: &str) {
        if !text.is_empty() {
            self.texts.push(text.to_string());
        }
        self.add_text(text);
    }

    /// Erubi's scan of `template`, `Erubi::Engine#initialize`.
    fn compile(&mut self, template: &str) {
        let mut pos = 0;
        let mut is_bol = true;
        for tag in erubi_tags(template) {
            let indicator = tag.indicator;
            let (code_text, code_at) = (&template[tag.code.clone()], tag.code.start);
            let tail = tag.tail;
            let mut rspace = tag.rspace;
            let mut text = &template[pos..tag.whole.start];
            pos = tag.whole.end;
            let kind = indicator.and_then(|i| i.chars().next());

            let blank = |s: &str| s.chars().all(|c| c == ' ' || c == '\t');
            let mut lspace = None;
            if kind != Some('=') {
                if text.is_empty() {
                    lspace = is_bol.then_some("");
                } else if text.ends_with('\n') {
                    lspace = Some("");
                } else if let Some(newline) = text.rfind('\n') {
                    let line = &text[newline + 1..];
                    if blank(line) {
                        lspace = Some(line);
                        text = &text[..=newline];
                    }
                } else if is_bol && blank(text) {
                    lspace = Some(text);
                    text = "";
                }
            }

            is_bol = rspace.is_some();
            self.add_rendered_text(text);
            match kind {
                Some('=') => {
                    if tail.is_some() {
                        rspace = None;
                    }
                    self.texts.push(OUTPUT_PLACEHOLDER.to_string());
                    self.add_expression(indicator.unwrap_or(""), code_text, code_at);
                    self.add_rendered_text(rspace.unwrap_or(""));
                }
                None | Some('-') => {
                    if let (Some(before), Some(after)) = (lspace, rspace) {
                        let start = self.add_code(&format!("{before}{code_text}{after}"));
                        self.record(start + before.len(), code_text, code_at);
                    } else {
                        self.add_rendered_text(lspace.unwrap_or(""));
                        let start = self.add_code(code_text);
                        self.record(start, code_text, code_at);
                        self.add_rendered_text(rspace.unwrap_or(""));
                    }
                }
                Some('#') => {
                    let newlines = "\n".repeat(code_text.matches('\n').count() + usize::from(rspace.is_some()));
                    if lspace.is_some() && rspace.is_some() {
                        self.add_code(&newlines);
                    } else {
                        self.add_rendered_text(lspace.unwrap_or(""));
                        self.add_code(&newlines);
                        self.add_rendered_text(rspace.unwrap_or(""));
                    }
                }
                Some(_) => self.add_rendered_text(&format!(
                    "{}<%{code_text}{}%>{}",
                    lspace.unwrap_or(""),
                    tail.unwrap_or(""),
                    rspace.unwrap_or("")
                )),
            }
        }
        self.tail_at = pos;
        self.add_text(&template[pos..]);
        if !self.src.ends_with('\n') {
            self.src.push('\n');
        }
        self.flush_newline_if_pending();
        self.src.push_str(OUTPUT_BUFFER.trim_start());
    }
}

/// A tag as Erubi's scan finds it.
pub struct ErbTag<'a> {
    /// From `<%` to just past `%>`, without the newline Erubi may trim after it.
    pub start: usize,
    pub end: usize,
    pub indicator: Option<&'a str>,
    /// Between the indicator and the `-`/`=` before `%>`.
    pub code: ByteRange<usize>,
}

impl ErbTag<'_> {
    pub fn is_comment(&self) -> bool {
        self.indicator == Some("#")
    }

    /// `<%% … %>`, which renders as the text `<% … %>`.
    pub fn is_literal(&self) -> bool {
        self.indicator == Some("%")
    }
}

/// Every tag Erubi finds in `text`; what follows the last one is text, even if it holds a `<%`.
pub fn tags(text: &str) -> Vec<ErbTag<'_>> {
    erubi_tags(text)
        .map(|tag| {
            let trimmed = tag.rspace.map_or(0, str::len);
            ErbTag { start: tag.whole.start, end: tag.whole.end - trimmed, indicator: tag.indicator, code: tag.code }
        })
        .collect()
}

/// Erubi's read of `template` after the encoding tag Rails cuts, and that tag's length.
fn read_after_encoding_tag(template: &str) -> (Erubi, usize) {
    let encoding_tag = ENCODING_TAG.find(template).map_or(0, |m| m.end());
    let mut erubi = Erubi::default();
    erubi.compile(&template[encoding_tag..]);
    (erubi, encoding_tag)
}

/// The texts Erubi renders for `text` before the text after its last tag, and where in `text` that text starts.
pub fn rendered_texts(text: &str) -> (Vec<String>, usize) {
    let (erubi, encoding_tag) = read_after_encoding_tag(text);
    (erubi.texts, encoding_tag + erubi.tail_at)
}

/// The method source Rails evals for a template, and where the template's text went in it.
struct Compiled {
    source: String,
    /// The line the template's first line is on.
    code_line: usize,
    /// Code copied as it is: its byte range in `source` and the offset of its first byte in the template.
    verbatim: Vec<(usize, usize, usize)>,
    /// The `locals:` comment, which the wrapper's `def` line holds the parameters of.
    strict_locals: Option<Comment>,
    /// What Rails cut out of the template before Erubi saw it.
    cut: Cut,
}

#[derive(Default)]
struct Cut {
    encoding_tag: usize,
    strict_locals: Option<ByteRange<usize>>,
}

impl Cut {
    /// The offset in the original template of `offset` in the text Erubi read.
    fn original(&self, offset: usize) -> usize {
        let offset = offset + self.encoding_tag;
        match &self.strict_locals {
            Some(comment) if offset >= comment.start => offset + comment.len(),
            _ => offset,
        }
    }
}

/// `Template#compiled_source` for `text`; without strict locals (Rails before 7.1) the `locals:` line is a plain comment.
fn compile(text: &str, close: Option<Close>) -> Compiled {
    let strict = close.and_then(|close| strict_locals::find(text, close));
    let mut template = text.to_string();
    if let Some(strict) = &strict {
        template.replace_range(strict.range.clone(), "");
    }
    let (erubi, encoding_tag) = read_after_encoding_tag(&template);

    let parameters = strict_locals::method_parameters(strict.as_ref());
    let head = format!("{WRAPPER_DEF}{parameters})\n            @virtual_path = \"template\";;");
    Compiled {
        code_line: head.matches('\n').count(),
        verbatim: erubi.verbatim.into_iter().map(|(from, to, at)| (from + head.len(), to + head.len(), at)).collect(),
        source: format!("{head}{}\n          end\n", erubi.src),
        cut: Cut { encoding_tag, strict_locals: strict.as_ref().map(|s| s.range.clone()) },
        strict_locals: strict,
    }
}

/// Prism's errors for the method source of `text`, at the template lines they come from; with none, Rails' check that the
/// `locals:` parameters are keywords. `rails` is given only where strict locals exist.
pub fn diagnostics(text: &str, ruby: ruby_prism::Version, rails: Option<&Rails>) -> Vec<Diagnostic> {
    let compiled = compile(text, rails.map(|r| r.close));
    let index = LineIndex::new(text);
    let (errors, (positional, block)) = prism::errors_and_def_parameters(&compiled.source, ruby);
    let found: Vec<Diagnostic> =
        errors.into_iter().filter_map(|e| Some(error(template_range(text, &index, &compiled, &e)?, e.message))).collect();
    if !found.is_empty() {
        return found;
    }
    let (Some(comment), Some(rails)) = (&compiled.strict_locals, rails) else { return Vec::new() };
    let range = index.text_range(comment.line(text));
    strict_locals::non_keyword_message(positional, block, &rails.identifier).map(|message| error(range, message)).into_iter().collect()
}

/// The block variables in scope at `offset` of the template.
pub fn block_locals(text: &str, offset: usize) -> Vec<String> {
    let compiled = compile(text, None);
    let Some(read_at) = offset.checked_sub(compiled.cut.encoding_tag) else { return Vec::new() };
    generated_offset(&compiled.verbatim, read_at).map_or_else(Vec::new, |at| prism::block_locals(&compiled.source, at))
}

/// The template byte ranges where the strict local `name` is read; the `locals:` comment, which Rails cuts out, makes the
/// method's parameters.
pub fn local_reads(text: &str, close: Close, name: &str) -> Vec<(usize, usize)> {
    let compiled = compile(text, Some(close));
    let original = |(start, end): (usize, usize)| (compiled.cut.original(start), compiled.cut.original(end));
    prism::local_reads(&compiled.source, name)
        .into_iter()
        .filter_map(|(start, end)| template_span(&compiled.verbatim, start, end).map(original))
        .collect()
}

/// Exact inside code the template wrote, else the whole of the line the error is on; none for the wrapper's own error.
fn template_range(text: &str, index: &LineIndex, compiled: &Compiled, e: &prism::RubyError) -> Option<Range> {
    // Erubi records the runs in source order: the first that ends after `start` is the only one that may hold it.
    let run = compiled.verbatim.get(compiled.verbatim.partition_point(|&(_, to, _)| to <= e.start));
    if let Some(&(from, to, at)) = run.filter(|&&(from, _, _)| from <= e.start) {
        let position = |offset: usize| index.position(compiled.cut.original(at + offset - from));
        return Some(Range::new(position(e.start), position(e.end.min(to))));
    }
    let generated_line = e.range.start.line as usize;
    let line = if generated_line < compiled.code_line {
        if e.message == UNCLOSED_DEF {
            return None;
        }
        let comment = compiled.strict_locals.as_ref()?;
        if let Some(range) = comment.error_range(text, WRAPPER_DEF.len(), e.start, e.end) {
            return Some(range);
        }
        comment.line(text)
    } else {
        // Past the template are the lines Erubi and the wrapper add after it.
        (generated_line - compiled.code_line).min(index.line_count().saturating_sub(1))
    };
    Some(index.text_range(line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Position;

    const IDENTIFIER: &str = "template.html.erb";

    #[test]
    fn tag_scan_matches_erubis_regexp() {
        let erubi = Regex::new(r"(?s)<%(={1,2}|-|#|%)?(.*?)([-=])?%>([ \t]*\r?\n)?").unwrap();
        let texts = [
            "a <%= x %> b <% y -%>\n<%# c %>  \r\n<%- z =%>",
            "<%%> a %> <%% b %> <%== c %> <%=%> <%==%> <%=-%> <%-%> <%#%>",
            "<%= a\nb %>\t \n<% open <% inner %> <% unclosed",
            "<% é = 1 %>\u{00a0}\n<%= \"%\" %>%>",
        ];
        for text in texts {
            let ours: Vec<_> = erubi_tags(text).map(|m| (m.whole, m.indicator, &text[m.code], m.tail, m.rspace)).collect();
            let regex: Vec<_> = erubi
                .captures_iter(text)
                .map(|c| {
                    let group = |i| c.get(i).map(|m| m.as_str());
                    (c.get(0).unwrap().range(), group(1), group(2).unwrap(), group(3), group(4))
                })
                .collect();
            assert_eq!(ours, regex, "{text:?}");
        }
    }

    fn rails() -> Rails {
        Rails { close: Close::default(), identifier: IDENTIFIER.into() }
    }

    fn diagnostics(text: &str) -> Vec<Diagnostic> {
        super::diagnostics(text, ruby_prism::Version::Latest, Some(&rails()))
    }

    /// (0-based line, message) of each error.
    fn errors(text: &str) -> Vec<(u32, String)> {
        errors_in(text, ruby_prism::Version::Latest)
    }

    fn errors_in(text: &str, ruby: ruby_prism::Version) -> Vec<(u32, String)> {
        super::diagnostics(text, ruby, Some(&rails())).into_iter().map(|d| (d.range.start.line, d.message)).collect()
    }

    fn messages(text: &str) -> Vec<String> {
        errors(text).into_iter().map(|(_, message)| message).collect()
    }

    // Each source is what `ruby test/oracle/erb.rb --file <name>` prints for the template (Rails 8.1.4, Erubi 1.13.1).
    // The `&block` case comes from `Template#compiled_source`, which the script doesn't reach for it (Rails refuses the
    // parameter later).
    #[test]
    fn source_matches_rails() {
        let cases = [
        ("<p>hi</p>\n", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; @output_buffer.safe_append='<p>hi</p>\n'.freeze;\n@output_buffer\n          end\n"),
        ("<% x = 1 %>\n<p><%= x %></p>\n", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; x = 1 \n @output_buffer.safe_append='<p>'.freeze; @output_buffer.append=( x ); @output_buffer.safe_append='</p>\n'.freeze;\n@output_buffer\n          end\n"),
        ("  <% if x -%>\n  <b>y</b>\n  <%- end -%>\n<%= z -%>\nw\n", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";;   if x \n @output_buffer.safe_append='  <b>y</b>\n'.freeze;   end \n @output_buffer.append=( z ); @output_buffer.safe_append='w\n'.freeze;\n@output_buffer\n          end\n"),
        ("a <% x %> b\n<%# one\n two %>\n<%# c %>\nz <%# d %>\n", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; @output_buffer.safe_append='a '.freeze; x ; @output_buffer.safe_append=' b\n'.freeze;\n\n\n @output_buffer.safe_append='z '.freeze;\n @output_buffer.safe_append='\n'.freeze;@output_buffer\n          end\n"),
        ("<%% literal %> and <%%= x %>\n  <%% y %>\n", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; @output_buffer.safe_append='<% literal %>'.freeze; @output_buffer.safe_append=' and '.freeze; @output_buffer.safe_append='<%= x %>\n'.freeze; @output_buffer.safe_append='  <% y %>\n'.freeze;\n@output_buffer\n          end\n"),
        ("<%== raw_html %> <%= a %>\n", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; @output_buffer.safe_expr_append=( raw_html ); @output_buffer.safe_append=' '.freeze; @output_buffer.append=( a );\n @output_buffer.safe_append='\n'.freeze;@output_buffer\n          end\n"),
        ("<%= form_with model: u do |f| %>\n<%= f.text_field :a %>\n<% end %>\n<%= items.map { |i| i } %>\n<%= x.each {|i|\n} %>\n", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; @output_buffer.append=  form_with model: u do |f| ; @output_buffer.safe_append='\n'.freeze; @output_buffer.append=( f.text_field :a ); @output_buffer.safe_append='\n'.freeze; end \n @output_buffer.append=( items.map { |i| i } ); @output_buffer.safe_append='\n'.freeze; @output_buffer.append=( x.each {|i|\n} );\n @output_buffer.safe_append='\n'.freeze;@output_buffer\n          end\n"),
        ("\n\n<p>a</p>\n\n<% x %>\n\n", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; @output_buffer.safe_append='\n\n<p>a</p>\n\n'.freeze; x \n @output_buffer.safe_append='\n'.freeze;@output_buffer\n          end\n"),
        ("a\r\n<% x %>\r\n<%= y %>\r\nb", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; @output_buffer.safe_append='a\r\n'.freeze; x \r\n @output_buffer.append=( y ); @output_buffer.safe_append='\r\n'.freeze; @output_buffer.safe_append='b'.freeze;\n@output_buffer\n          end\n"),
        ("it's a \\ \"b\" <%= \"it's\" %> é 😀\n", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; @output_buffer.safe_append='it\\'s a \\\\ \"b\" '.freeze; @output_buffer.append=( \"it's\" ); @output_buffer.safe_append=' é 😀\n'.freeze;\n@output_buffer\n          end\n"),
        ("<%# locals: (a:, b: 1) -%>\n<p><%= a %></p>\n", "          def _template(local_assigns, output_buffer, a:, b: 1, &_)\n            @virtual_path = \"template\";; \n @output_buffer.safe_append='<p>'.freeze; @output_buffer.append=( a ); @output_buffer.safe_append='</p>\n'.freeze;\n@output_buffer\n          end\n"),
        ("<p>x</p>\n<%# locals: () %>\n", "          def _template(local_assigns, output_buffer, **nil, &_)\n            @virtual_path = \"template\";; @output_buffer.safe_append='<p>x</p>\n'.freeze; \n@output_buffer\n          end\n"),
        ("<%# locals: (a:, **rest) -%>\n<%= a %>", "          def _template(local_assigns, output_buffer, a:, **rest, &_)\n            @virtual_path = \"template\";; \n @output_buffer.append=( a );\n@output_buffer\n          end\n"),
        ("<%# encoding: utf-8 %>  \n<p>é</p>\n", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; @output_buffer.safe_append='\n<p>é</p>\n'.freeze;\n@output_buffer\n          end\n"),
        ("<%= a =%>\nb <%- c -%>\n<%=a%>", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; @output_buffer.append=( a ); @output_buffer.safe_append='b '.freeze; c ; @output_buffer.safe_append='\n'.freeze; @output_buffer.append=(a);\n@output_buffer\n          end\n"),
        ("", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";;\n@output_buffer\n          end\n"),
        ("x", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; @output_buffer.safe_append='x'.freeze;\n@output_buffer\n          end\n"),
        ("<%= foo\n  .bar %>\n<% if x\n %>y<% end %>", "          def _template(local_assigns, output_buffer, &_)\n            @virtual_path = \"template\";; @output_buffer.append=( foo\n  .bar ); @output_buffer.safe_append='\n'.freeze; if x\n ; @output_buffer.safe_append='y'.freeze; end ;\n@output_buffer\n          end\n"),
        ("<%# locals: (a:, &block) -%>\n<%= a %>", "          def _template(local_assigns, output_buffer, a:, &block)\n            @virtual_path = \"template\";; \n @output_buffer.append=( a );\n@output_buffer\n          end\n"),
        ];
        for (template, rails) in cases {
            assert_eq!(compile(template, Some(Close::default())).source, rails, "{template:?}");
        }
    }

    // The messages and lines are those of `ruby test/oracle/erb.rb` (Prism 2.0.0), 0-based here.
    #[test]
    fn output_tag_around_control_flow() {
        let at_if: Vec<(u32, String)> = [
            "expected `then` or `;` or '\\n'",
            "unexpected ')', assuming it is closing the parent parentheses",
            "expected an `end` to close the conditional clause",
        ]
        .map(|m| (1, m.to_string()))
        .into();
        assert_eq!(errors("<p>\n<%= if x %>y<% end %>\n"), at_if);
        assert!(errors("<%= if a then b else c end %>\n<%= if a\n  b\nelse\n  c\nend %>").is_empty());
    }

    #[test]
    fn block_never_closed_is_shown_at_the_end() {
        let found = diagnostics("<% if x %>\n<p>\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].message, "unexpected end-of-input, assuming it is closing the parent top level context");
        assert_eq!(found[0].range, Range::new(Position::new(1, 0), Position::new(1, 3)));
        assert_eq!(
            errors("<p>\n<%= link_to x do %>\n"),
            [(1, "unexpected end-of-input, assuming it is closing the parent top level context".to_string())]
        );
    }

    #[test]
    fn stray_end_and_branch() {
        assert_eq!(errors("<% end %>\n"), [(0, "unexpected 'end', ignoring it".to_string())]);
        assert_eq!(errors("a\n<% elsif y %>"), [(1, "unexpected 'elsif', ignoring it".to_string())]);
    }

    #[test]
    fn code_position_is_utf16_and_exact() {
        // The emoji is 2 UTF-16 units: the `)` is at column 10.
        let found = diagnostics("😀 <% 1 + ) %>");
        assert_eq!(found[0].range, Range::new(Position::new(0, 10), Position::new(0, 11)));
        let multi = diagnostics("a\n<%\n  é + )\n%>");
        assert_eq!(multi[0].range, Range::new(Position::new(2, 6), Position::new(2, 7)));
    }

    #[test]
    fn percent_gt_inside_a_string_ends_the_tag() {
        assert_eq!(
            messages(r#"<%= "50%>" %>"#),
            [
                "unexpected '>'; expected an expression after the operator",
                "unterminated string meets end of file",
                "unexpected end-of-input, assuming it is closing the parent top level context",
                "expected a matching `)`",
            ]
        );
    }

    #[test]
    fn tag_left_open_before_the_next_one() {
        let lines: Vec<u32> = errors("<%= render \"sh\n<%= link_to x %>\n").into_iter().map(|(line, _)| line).collect();
        assert_eq!(lines, [1, 1, 1]);
        let split = errors("x <% foo(1,\n 2 %> y\n<% bar) %>");
        assert_eq!(split.iter().map(|(line, _)| *line).collect::<Vec<_>>(), [1, 2, 2]);
        assert_eq!(split[0].1, "unexpected ';'; expected a `)` to close the arguments");
    }

    #[test]
    fn strict_locals_signature_errors_are_on_the_comment_line() {
        let found = errors("<p>\n<%# locals: (a: ] ) %>\nx");
        assert!(found.len() > 1 && found.iter().all(|(line, _)| *line == 1), "{found:?}");
        assert_eq!(found[0].1, "unexpected ']'; expected a `)` to close the parameters");
        assert!(errors("<%# locals: (a:, b: 1) -%>\n<%= a %>").is_empty());
        // Without `)` before a tag end or a line end it is a plain comment.
        assert!(errors("<%# locals: (a:, b: 1 %>\n<p>").is_empty());
    }

    // Ruby 3.3 and 3.4 reject a line starting with `&&` after an expression; the latest grammar takes it.
    #[test]
    fn errors_depend_on_the_ruby_version() {
        let text = "<% x = a\n  && b %>\n";
        let rejected = [(1, "unexpected '&&', ignoring it".to_string())];
        assert_eq!(errors_in(text, ruby_prism::Version::CRuby3_3), rejected);
        assert_eq!(errors_in(text, ruby_prism::Version::CRuby3_4), rejected);
        assert_eq!(errors(text), []);
    }

    fn locals_messages(signature: &str) -> Vec<(u32, String)> {
        let text = format!("<p>\n<%# locals: {signature} -%>\nx\n");
        let rails = Rails { close: Close::default(), identifier: "app/views/a.html.erb".into() };
        let found = super::diagnostics(&text, ruby_prism::Version::Latest, Some(&rails));
        found.into_iter().map(|d| (d.range.start.line, d.message)).collect()
    }

    // Each message is what `ruby test/oracle/erb.rb --file app/views/a.html.erb` gives (Rails 8.1.4 `Template#compile`).
    #[test]
    fn strict_locals_must_be_keywords() {
        let tail = "for app/views/a.html.erb. Locals can only be set as keyword arguments.";
        for (signature, names, noun) in [
            ("(a, b:)", "`a`", "argument"),
            ("(a, b, c:)", "`a` and `b`", "arguments"),
            ("(a, b, c, d:)", "`a`, `b`, and `c`", "arguments"),
            ("(*, **)", "`*`", "argument"),
            ("(*)", "`*`", "argument"),
            ("(&)", "`&`", "argument"),
            ("(&blk)", "`blk`", "argument"),
            ("(a, &blk)", "`a` and `blk`", "arguments"),
            ("(a, *rest, b:)", "`a` and `rest`", "arguments"),
            ("(*, a)", "`*` and `a`", "arguments"),
            ("(a = 1, b:)", "`a`", "argument"),
            ("((a, b), c:)", "``", "argument"),
            ("(a, a2 = 1, *r, z, k: 1, **o, &b)", "`a`, `a2`, `r`, `z`, and `b`", "arguments"),
        ] {
            assert_eq!(locals_messages(signature), [(1, format!("{names} set as non-keyword {noun} {tail}"))], "{signature}");
        }
    }

    #[test]
    fn keyword_only_strict_locals_are_accepted() {
        for signature in ["(a:, b: 1)", "()", "(a:, **rest)", "(**nil)", "(a:, &_)", "(**)"] {
            assert_eq!(locals_messages(signature), [], "{signature}");
        }
    }

    #[test]
    fn strict_locals_check_needs_a_project_and_no_syntax_error() {
        let text = "<%# locals: (a) -%>\nx";
        assert!(super::diagnostics(text, ruby_prism::Version::Latest, None).is_empty());
        let syntax = messages("<%# locals: (a, b: ] ) -%>\nx");
        assert!(!syntax.is_empty());
        assert!(syntax.iter().all(|message| !message.contains("non-keyword")), "{syntax:?}");
    }

    #[test]
    fn locals_line_is_a_plain_comment_without_strict_locals() {
        let text = "<%# locals: (a: ] ) %>\n<p>\n";
        assert!(super::diagnostics(text, ruby_prism::Version::Latest, None).is_empty());
        assert!(compile(text, None).source.contains("(local_assigns, output_buffer, &_)"));
        assert_eq!(errors(text)[0].1, "unexpected ']'; expected a `)` to close the parameters");
    }

    #[test]
    fn valid_templates_are_clean() {
        for template in [
            "",
            "<% x = 1 %><%# c %><%% lit %>\n<%= yield %>",
            "<%= form_with model: u do |f| %>\n<%= f.text_field :a %>\n<% end %>\n<%= items.map { |i| i } %>",
            "<% if x %>é<% else %>😀<% end %>\n<%= link_to 'a', b %>",
            "<% return unless current_user %>\n<p>hi</p>",
            "<%# encoding: utf-8 %>\n<p>é</p>",
        ] {
            assert!(errors(template).is_empty(), "{template:?}");
        }
    }

    #[test]
    fn whitespace_only_line_gets_an_ordered_range() {
        let found = diagnostics("<% if x %>\n   ");
        assert_eq!(found[0].range, Range::new(Position::new(1, 3), Position::new(1, 3)));
    }

    #[test]
    fn blank_signature_is_nil_keywords() {
        for blank in ["", " ", "\u{a0}"] {
            let template = format!("<%# locals: ({blank}) -%>\n<p>x</p>");
            assert!(compile(&template, Some(Close::default())).source.contains("output_buffer, **nil, &_)"), "{template:?}");
            assert!(errors(&template).is_empty());
        }
    }

    #[test]
    fn fallback_range_is_the_trimmed_line_in_utf16() {
        let found = diagnostics("é😀 <% if x %>\n");
        assert_eq!(found[0].range, Range::new(Position::new(0, 0), Position::new(0, 14)));
        let crlf = diagnostics("<% if x %>\r\n  <p>  \r\n");
        assert_eq!(crlf[0].range, Range::new(Position::new(1, 2), Position::new(1, 5)));
    }

    #[test]
    fn signature_error_range_is_in_the_comment_and_later_errors_keep_their_line() {
        let found = diagnostics("<p>\n  <%# locals: (a: ] ) %>");
        assert_eq!(found[0].range, Range::new(Position::new(1, 18), Position::new(1, 19)));
        let later = errors("<%# locals: (a: ] ) %>\n<p>\n<% if y %>\n");
        assert_eq!(later.last(), Some(&(2, "unexpected end-of-input, assuming it is closing the parent top level context".to_string())));
    }

    #[test]
    fn code_position_skips_what_rails_cut_before_the_tag() {
        let expected = Range::new(Position::new(1, 7), Position::new(1, 8));
        assert_eq!(diagnostics("<%# locals: (a:) -%>\n<% 1 + ) %>")[0].range, expected);
        assert_eq!(diagnostics("<%# encoding: utf-8 %>\n<% 1 + ) %>")[0].range, expected);
        assert_eq!(diagnostics("é <%# locals: (a:) %> <% 1 + ) %>")[0].range, Range::new(Position::new(0, 29), Position::new(0, 30)));
    }

    #[test]
    fn block_locals_after_an_encoding_tag_and_before_the_end() {
        let text = "<%# encoding: utf-8 %>\n<% @users.each do |user| %>\n<%= us %>\n<% end %>\n<%= us %>\n";
        let first = text.find("us %>").unwrap() + 2;
        assert_eq!(block_locals(text, first), ["user"]);
        assert!(block_locals(text, text.rfind("us").unwrap() + 2).is_empty());
        assert!(block_locals(text, 5).is_empty());
    }
}
