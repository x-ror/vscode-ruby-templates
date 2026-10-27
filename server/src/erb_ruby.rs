//! Ruby syntax errors in an ERB template: the method source Rails 8.1.4 evals for it (Erubi 1.13.1 with the
//! ActionView handler's options and method wrapper), parsed by Prism.

use crate::prism;
use crate::util::{error, leading_ws, line_range, offset_to_position};
use regex::Regex;
use std::ops::Range as ByteRange;
use std::sync::LazyLock;
use tower_lsp::lsp_types::{Diagnostic, Range};

/// Ruby's `\s`, which the regexps below are written in.
const RUBY_SPACE: [char; 6] = [' ', '\t', '\r', '\n', '\x0C', '\x0B'];
const OUTPUT_BUFFER: &str = " @output_buffer";
/// Prism's message for the wrapper's own `def`, which the template's code never gets to close.
const UNCLOSED_DEF: &str = "expected an `end` to close the `def` statement";

// `Erubi::Engine::DEFAULT_REGEXP`.
static ERUBI_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<%(={1,2}|-|#|%)?(.*?)([-=])?%>([ \t]*\r?\n)?").unwrap());
// `ActionView::Template::Handlers::ERB::ENCODING_TAG`.
static ENCODING_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A<%#.*coding[:=][ \t\r\n\x0C\x0B]*\S+[ \t]*-?%>[ \t]*").unwrap());
// `ActionView::Template::Handlers::ERB::Erubi::BLOCK_EXPR`.
static BLOCK_EXPR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:(?:[ \t\r\n\x0C\x0B]|\))do|\{)(?:[ \t\r\n\x0C\x0B]*\|[^|]*\|)?[ \t\r\n\x0C\x0B]*\z").unwrap());
// The start of `ActionView::Template::STRICT_LOCALS_REGEX`; Rust's regexps have no lookahead for the rest.
static STRICT_LOCALS_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#[ \t\r\n\x0C\x0B]+locals:[ \t\r\n\x0C\x0B]+\(").unwrap());

/// The `locals: (…)` magic comment Rails cuts out of the source, and the parameters it holds.
struct StrictLocals {
    comment: ByteRange<usize>,
    parameters: String,
}

/// `STRICT_LOCALS_REGEX`'s first match: `# locals: (` up to the first `)` that a tag end or a line end follows.
fn strict_locals(text: &str) -> Option<StrictLocals> {
    let mut from = 0;
    while let Some(head) = STRICT_LOCALS_HEAD.find_at(text, from) {
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
            return Some(StrictLocals { comment: head.start()..close + 1, parameters: parameters.to_string() });
        }
        from = head.start() + 1;
    }
    None
}

/// `Erubi::Engine` as `ActionView::Template::Handlers::ERB::Erubi` configures it (`trim: true`, `@output_buffer`).
#[derive(Default)]
struct Erubi {
    src: String,
    /// Lone "\n" texts wait here, so they land inside the next text literal (the handler's `@newline_pending`).
    newline_pending: usize,
    /// Code copied as it is: its byte range in `src` and the offset of its first byte in the template Erubi read.
    verbatim: Vec<(usize, usize, usize)>,
}

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

    /// Erubi's scan of `template`, `Erubi::Engine#initialize`.
    fn compile(&mut self, template: &str) {
        let mut pos = 0;
        let mut is_bol = true;
        for tag in ERUBI_TAG.captures_iter(template) {
            let Some(whole) = tag.get(0) else { continue };
            let indicator = tag.get(1).map(|m| m.as_str());
            let (code_text, code_at) = tag.get(2).map_or(("", 0), |m| (m.as_str(), m.start()));
            let tail = tag.get(3).map(|m| m.as_str());
            let mut rspace = tag.get(4).map(|m| m.as_str());
            let mut text = &template[pos..whole.start()];
            pos = whole.end();
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
            self.add_text(text);
            match kind {
                Some('=') => {
                    if tail.is_some() {
                        rspace = None;
                    }
                    self.add_expression(indicator.unwrap_or(""), code_text, code_at);
                    self.add_text(rspace.unwrap_or(""));
                }
                None | Some('-') => {
                    if let (Some(before), Some(after)) = (lspace, rspace) {
                        let start = self.add_code(&format!("{before}{code_text}{after}"));
                        self.record(start + before.len(), code_text, code_at);
                    } else {
                        self.add_text(lspace.unwrap_or(""));
                        let start = self.add_code(code_text);
                        self.record(start, code_text, code_at);
                        self.add_text(rspace.unwrap_or(""));
                    }
                }
                Some('#') => {
                    let newlines = "\n".repeat(code_text.matches('\n').count() + usize::from(rspace.is_some()));
                    if lspace.is_some() && rspace.is_some() {
                        self.add_code(&newlines);
                    } else {
                        self.add_text(lspace.unwrap_or(""));
                        self.add_code(&newlines);
                        self.add_text(rspace.unwrap_or(""));
                    }
                }
                Some(_) => self.add_text(&format!("{}<%{code_text}{}%>{}", lspace.unwrap_or(""), tail.unwrap_or(""), rspace.unwrap_or(""))),
            }
        }
        self.add_text(&template[pos..]);
        if !self.src.ends_with('\n') {
            self.src.push('\n');
        }
        self.flush_newline_if_pending();
        self.src.push_str(OUTPUT_BUFFER.trim_start());
    }
}

/// The method source Rails evals for a template, and where the template's text went in it.
struct Compiled {
    source: String,
    /// The line the template's first line is on.
    code_line: usize,
    /// Code copied as it is: its byte range in `source` and the offset of its first byte in the template.
    verbatim: Vec<(usize, usize, usize)>,
    /// The line of the `locals:` comment, which the wrapper's `def` line holds the parameters of.
    signature_line: Option<usize>,
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

/// `Template#compiled_source` for `text`.
fn compile(text: &str) -> Compiled {
    let strict = strict_locals(text);
    let mut template = text.to_string();
    if let Some(strict) = &strict {
        template.replace_range(strict.comment.clone(), "");
    }
    let encoding_tag = ENCODING_TAG.find(&template).map_or(0, |m| m.end());
    let mut erubi = Erubi::default();
    erubi.compile(&template[encoding_tag..]);

    let parameters = match &strict {
        Some(strict) if strict.parameters.contains('&') => format!("local_assigns, output_buffer, {}", strict.parameters),
        Some(strict) => format!("local_assigns, output_buffer, {}, &_", strict.parameters),
        None => "local_assigns, output_buffer, &_".to_string(),
    };
    let head = format!("          def _template({parameters})\n            @virtual_path = \"template\";;");
    let signature_line = strict.as_ref().map(|s| text[..s.comment.start].matches('\n').count());
    Compiled {
        code_line: head.matches('\n').count(),
        verbatim: erubi.verbatim.into_iter().map(|(from, to, at)| (from + head.len(), to + head.len(), at)).collect(),
        source: format!("{head}{}\n          end\n", erubi.src),
        signature_line,
        cut: Cut { encoding_tag, strict_locals: strict.map(|s| s.comment) },
    }
}

/// Prism's errors for the method source of `text`, at the template lines they come from.
pub fn diagnostics(text: &str) -> Vec<Diagnostic> {
    let compiled = compile(text);
    let lines: Vec<&str> = text.lines().collect();
    prism::syntax_errors(&compiled.source)
        .into_iter()
        .filter_map(|e| Some(error(template_range(text, &lines, &compiled, &e)?, e.message)))
        .collect()
}

/// Exact inside code the template wrote, else the whole of the line the error is on; none for the wrapper's own error.
fn template_range(text: &str, lines: &[&str], compiled: &Compiled, e: &prism::RubyError) -> Option<Range> {
    if let Some(&(from, to, at)) = compiled.verbatim.iter().find(|(from, to, _)| (*from..*to).contains(&e.start)) {
        let position = |offset: usize| offset_to_position(text, compiled.cut.original(at + offset - from));
        return Some(Range::new(position(e.start), position(e.end.min(to))));
    }
    let generated_line = e.range.start.line as usize;
    let line = if generated_line < compiled.code_line {
        if e.message == UNCLOSED_DEF {
            return None;
        }
        compiled.signature_line?
    } else {
        // Past the template are the lines Erubi and the wrapper add after it.
        (generated_line - compiled.code_line).min(lines.len().saturating_sub(1))
    };
    let raw = lines.get(line).copied().unwrap_or("");
    let start = leading_ws(raw).len();
    Some(line_range(text, line, start, raw.trim_end().len().max(start)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Position;

    /// (0-based line, message) of each error.
    fn errors(text: &str) -> Vec<(u32, String)> {
        diagnostics(text).into_iter().map(|d| (d.range.start.line, d.message)).collect()
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
            assert_eq!(compile(template).source, rails, "{template:?}");
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
            assert!(compile(&template).source.contains("output_buffer, **nil, &_)"), "{template:?}");
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
    fn signature_error_range_is_the_comment_line_and_later_errors_keep_their_line() {
        let found = diagnostics("<p>\n  <%# locals: (a: ] ) %>");
        assert_eq!(found[0].range, Range::new(Position::new(1, 2), Position::new(1, 24)));
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
}
