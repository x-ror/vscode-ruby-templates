//! Ruby syntax errors in a Haml template: the Ruby `Haml::Engine` generates for it as Rails compiles it, parsed by Prism.

use crate::generated_ruby::Compiled;
use crate::haml_parser::{Chunk, Parsed, Piece, BOM};
use crate::strict_locals::{self, Comment};
use tower_lsp::lsp_types::Diagnostic;

/// The method Rails compiles a template into, up to its parameters.
const WRAPPER_DEF: &str = "def _template(";
/// What `Haml::RailsTemplate` starts the method body with.
const PREAMBLE: &str = "@output_buffer = output_buffer || ActionView::OutputBuffer.new";
/// The call `Haml::AttributeCompiler#runtime_compile` makes, up to the object reference.
const ATTRIBUTES_CALL: &str = "::Haml::AttributeBuilder.build(true, \"\\\"\".freeze, :html, ";
const VARIABLE: &str = "_haml_compiler";

/// `comment` is the strict locals comment, whose signature `strict_locals::comment_diagnostics` checks.
pub fn diagnostics(text: &str, parsed: &Parsed, ruby: ruby_prism::Version, comment: Option<&Comment>) -> Vec<Diagnostic> {
    let bom_len = if text.starts_with(BOM) { BOM.len_utf8() } else { 0 };
    // Haml drops the blank lines at the end, so the source ends on the last line that holds something.
    let last_line = text.lines().enumerate().filter(|(_, line)| !line.trim().is_empty()).last().map_or(0, |(index, _)| index);
    let wrapper = format!("{WRAPPER_DEF}{})", strict_locals::method_parameters(comment));
    compile(&parsed.pieces, bom_len, last_line, &wrapper).diagnostics(text, ruby)
}

/// Temple joins the code parts it generates with `; `, and a newline is a part of its own.
struct Generator {
    out: Compiled,
    /// The next part has to be separated from the one before.
    separate: bool,
    /// Where the template starts in the text, past a BOM.
    bom_len: usize,
}

fn compile(pieces: &[Piece], bom_len: usize, last_line: usize, wrapper: &str) -> Compiled {
    let mut generator = Generator { out: Compiled::new(), separate: false, bom_len };
    generator.out.push(wrapper, 0);
    generator.out.new_line();
    generator.out.wrapper_len = generator.out.source.len();
    generator.part(0, |g| g.out.push(PREAMBLE, 0));
    for piece in pieces {
        generator.piece(piece);
    }
    // What the end of the source is blamed for lands on the template's last line.
    generator.newline();
    generator.part(last_line, |g| g.out.push("@output_buffer", last_line));
    generator.out.new_line();
    generator.out.push("end", last_line);
    generator.out
}

impl Generator {
    fn piece(&mut self, piece: &Piece) {
        match piece {
            Piece::Statement(code) => {
                self.part(line_of(code), |g| g.chunks(code));
                self.newline();
            }
            Piece::Output { code, block: true } => {
                self.part(line_of(code), |g| {
                    g.out.push(&format!("{VARIABLE} = "), line_of(code));
                    g.chunks(code);
                });
                self.newline();
            }
            Piece::Output { code, block: false } => {
                self.part(line_of(code), |g| {
                    g.out.push(&format!("{VARIABLE} = ("), line_of(code));
                    g.chunks(code);
                });
                self.newline();
                // Haml's newline puts the closing parenthesis on the line after the code.
                let line = line_of(code) + 1;
                self.part(line, |g| g.out.push(")", line));
            }
            Piece::Text(literal) => self.part(line_of(literal), |g| {
                g.out.push("(", line_of(literal));
                g.chunks(literal);
                g.out.push(")", line_of(literal));
            }),
            Piece::Attributes { object, arguments } => {
                let line = line_of(if object.is_empty() { arguments } else { object });
                self.part(line, |g| {
                    g.out.push(ATTRIBUTES_CALL, line);
                    if object.is_empty() {
                        g.out.push("nil", line);
                    }
                    g.chunks(object);
                    g.out.push(", ", line);
                    g.chunks(arguments);
                    g.out.push(")", line);
                });
            }
            Piece::Dropped => {}
            Piece::End(line) => self.part(*line, |g| g.out.push("end", *line)),
        }
    }

    /// Writes one part of the code, after the `; ` that separates it from the part before.
    fn part(&mut self, line: usize, write: impl FnOnce(&mut Self)) {
        if std::mem::take(&mut self.separate) {
            self.out.push("; ", line);
        }
        write(self);
        self.separate = true;
    }

    /// A line break; the `; ` before it is on the line it ends.
    fn newline(&mut self) {
        if std::mem::take(&mut self.separate) {
            self.out.push_floating("; ");
        }
        self.out.new_line();
        self.separate = true;
    }

    fn chunks(&mut self, chunks: &[Chunk]) {
        for chunk in chunks {
            if chunk.text == "\n" {
                self.out.new_line();
                continue;
            }
            let from = self.out.source.len();
            self.out.push(&chunk.text, chunk.line);
            if let Some(offset) = chunk.offset {
                self.out.mark_verbatim(from, chunk.text.len(), self.bom_len + offset);
            }
        }
    }
}

fn line_of(chunks: &[Chunk]) -> usize {
    chunks.first().map_or(0, |chunk| chunk.line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::haml_parser::parse_with_ruby as parse;

    fn diagnostics(text: &str) -> Vec<Diagnostic> {
        super::diagnostics(text, &parse(text), ruby_prism::Version::Latest, None)
    }

    /// (0-based line, first column, Prism's message) of each error.
    fn errors(text: &str) -> Vec<(u32, u32, String)> {
        diagnostics(text).into_iter().map(|d| (d.range.start.line, d.range.start.character, d.message)).collect()
    }

    fn messages(text: &str) -> Vec<String> {
        errors(text).into_iter().map(|(_, _, message)| message).collect()
    }

    #[test]
    fn blocks_close_where_haml_closes_them() {
        for valid in [
            "- if a\n  %p x\n- elsif b\n  %p y\n- else\n  %p z\n",
            "- case x\n- when 1\n  %p a\n- else\n  %p b\n",
            "- case x\n  - when 1\n    %p a\n  - when 2\n    %p b\n  - else\n    %p c\n",
            "- begin\n  %p\n- rescue => e\n  = e\n- ensure\n  %p\n",
            "%ul\n  - items.each do |i|\n    %li\n      - if i\n        %b= i\n      - else\n        %i z\n  %li z\n",
            "- if a\n  -# note\n- else\n  %p\n",
            "- if a\n\n- else\n  %p\n",
            "- x = if a\n  - 1\n- else\n  - 2\n",
            "- foo if bar\n%p\n",
            "= form_with model: @u do |f|\n  = f.text_field :x\n%p after\n",
            "%div\n  = capture do\n    %p\n  = yield\n",
            "- foo do |a|\n  = a\n%p\n",
        ] {
            assert_eq!(errors(valid), [], "{valid:?}");
        }
    }

    #[test]
    fn lines_nested_under_code_that_opens_no_block_leave_an_end_unmatched() {
        // Haml accepts the nesting and writes the `end`; Ruby refuses it, at the end of the template.
        assert_eq!(messages("- x = 1\n  %p hi\n%p after\n"), ["unexpected 'end', ignoring it"]);
        // The unmatched `end` is the method's own, at the end of the template.
        assert_eq!(errors("%p\n  = \"a\"\n    %b\n")[0].0, 2);
        // An `if` nobody nests under has no `end`: the method's takes it, and what is left open is reported at the end.
        assert_eq!(errors("- if x\n%p\n")[0].0, 1);
    }

    #[test]
    fn the_first_when_gives_its_lines_to_the_case() {
        // Haml moves the lines under the first `when` to the `case`, and only then; `in` has no such rule.
        assert_eq!(errors("- case x\n  - when 1\n    %p a\n  - when 2\n    %p b\n"), []);
        assert_eq!(messages("- case x\n  - in 1\n    %p a\n  - in 2\n    %p b\n"), ["unexpected 'end', ignoring it"]);
        assert_eq!(messages("- case x\n  - when 1\n  - when 2\n    %p b\n"), ["unexpected 'end', ignoring it"]);
    }

    #[test]
    fn a_clause_in_the_wrong_place_is_ruby_s_error() {
        assert_eq!(
            messages("- if a\n  %p\n  - rescue\n    %p\n")[0],
            "unexpected 'rescue', assuming it is closing the parent method definition"
        );
        assert_eq!(messages("%p\n  - if a\n    - ensure\n")[0], "unexpected 'ensure', assuming it is closing the parent method definition");
        // At the top of the template they belong to the method Rails wraps it in.
        assert_eq!(errors("- rescue\n  %p\n- ensure\n  %p\n").len(), 1);
        assert_eq!(messages("- if a\n  %p\n- else\n  %p\n- else\n  %p\n")[0], "unexpected 'else', ignoring it");
        assert_eq!(messages("- in 1\n"), ["unexpected 'in', ignoring it"]);
    }

    #[test]
    fn unfinished_code_is_reported_in_the_template_s_code() {
        let found = errors("%p\n  = foo(\n%p x\n");
        assert_eq!(found[0], (1, 8, "unexpected ';'; expected an argument".to_string()));
        assert_eq!(messages("- @a = [1 2]\n"), ["unexpected integer; expected a `,` separator for the array elements"]);
        // `!=`, `&=` and `~` after a tag are code as well.
        assert_eq!(errors("%p!= foo(\n%b&= bar(\n%i~ baz(\n").iter().map(|e| e.0).collect::<Vec<_>>(), [0, 0, 1, 1, 2, 2]);
    }

    #[test]
    fn code_on_several_lines_is_one_statement() {
        assert_eq!(errors("= link_to \"a\",\n  \"/b\",\n  class: \"c\"\n"), []);
        assert_eq!(errors("%p= foo(1,\n  2)\n"), []);
        assert_eq!(errors("- x = [1,\n  2]\n%p= x.sum\n"), []);
        // The error is on the line that holds it, but the range is the line: the code is no longer a slice.
        let found = errors("= foo(1,\n  2))\n");
        assert_eq!((found[0].0, found[0].1), (1, 2));
    }

    #[test]
    fn multiline_pipes_and_commas_in_text_are_text() {
        assert_eq!(errors("%p hello, |\n  world, |\n  again |\n%b= x\n"), []);
        assert_eq!(errors("= link_to(\"a\", |\n  \"b\") |\n%p\n"), []);
        assert_eq!(errors("Dear Sir,\n%p x,\n"), []);
    }

    #[test]
    fn attribute_hashes_are_checked_wherever_they_run() {
        assert_eq!(errors("%a{href: x, class: [\"a\", b]} t\n.c#d{id: 1, **opts}\n"), []);
        assert_eq!(errors("%p{ { a: 1 }, b: 2 } t\n"), []);
        let found = errors("%p{a: foo(, b: 2} t\n");
        assert_eq!(found[0].0, 0);
        assert!(found[0].2.starts_with("unexpected ','"), "{found:?}");
        // The hash goes on over lines, each exact in the template.
        let found = errors("%p{a: 1,\n  b: foo(,\n  c: 3}\n  t\n");
        assert_eq!(found[0].0, 1);
        assert_eq!(errors("%a{href: x,\n   title: y} t\n%p\n"), []);
        // The Ruby in the values of `()` attributes: variables and strings with `#{}`.
        assert_eq!(errors("%a(href=x title=\"t#{u}\" class='c' checked) text\n%p(a=@b c=$d)\n"), []);
        let found = errors("%a(href=x\n   title=\"t#{u(}\") text\n");
        assert_eq!(found[0].0, 1);
        // An object reference is Ruby too.
        assert_eq!(errors("%p[@a(] x\n")[0].0, 0);
        assert_eq!(errors("%p[@a, :b]\n"), []);
    }

    #[test]
    fn a_block_under_a_tag_or_a_script() {
        assert_eq!(errors("%ul\n  - items.each do |x|\n    %li= x\n%p\n  = foo do |x|\n    = x\n"), []);
        // Nobody nests under the block, so its `end` is missing.
        assert!(messages("%p\n  = foo do |x|\n%b\n").contains(&"expected a block beginning with `do` to end with `end`".to_string()));
    }

    #[test]
    fn the_ruby_filter_is_code() {
        assert_eq!(errors(":ruby\n  x = 1\n  y = [x,\n    2]\n%p= y\n"), []);
        let found = errors("%p\n  :ruby\n    x = (1\n    y = 2\n");
        assert!(found.iter().any(|(line, ..)| *line >= 2), "{found:?}");
        assert_eq!(errors(":ruby\n  x = <<~T\n    hi\n  T\n"), []);
        // Other filters are text.
        assert_eq!(errors(":javascript\n  if (x) { y(; }\n:css\n  a { color: red }\n:plain\n  x = (\n"), []);
    }

    #[test]
    fn interpolation_is_checked_where_haml_compiles_it() {
        assert_eq!(errors("%p hi #{x} and #{y.map { |z| z }}\n= \"a#{b}\"\nplain #@a #$b \\#{not}\n"), []);
        assert_eq!(errors("%p hi #{foo(} there\n")[0].0, 0);
        assert_eq!(errors("plain #{foo(}\n")[0].0, 0);
        assert_eq!(errors("/ c #{foo(}\n")[0].0, 0);
        assert_eq!(errors("%p== a #{foo(}\n")[0].0, 0);
        for line in ["& #{foo(}", "! #{foo(}", "!== #{foo(}", "\\= #{foo(}", "%p& #{foo(}", "%p! #{foo(}", "%p&== #{foo(}"] {
            assert_eq!(errors(&format!("{line}\n"))[0].0, 0, "{line}");
        }
        // A bare `#@` or `#$` makes Ruby's `#{@}`.
        assert_eq!(messages("%p a #@ b\n"), ["'@' without identifiers is not allowed as an instance variable name"]);
        assert_eq!(errors(":javascript\n  var a = 1;\n  var b = #{foo(};\n")[0].0, 2);
        // An interpolation that never closes fails in Haml itself.
        assert_eq!(errors("%p hi #{foo(\n"), []);
        // Haml ends the interpolation at the first `}`, even in a string.
        assert_eq!(errors("%p Use #{'}'} here\n"), []);
        assert!(!errors("%p a #{\"}\"} b\n").is_empty());
    }

    #[test]
    fn a_byte_order_mark_does_not_move_the_ranges() {
        let found = errors("\u{feff}%p\n  - @a = [1 2]\n");
        assert_eq!((found[0].0, found[0].1), (1, 11));
    }

    #[test]
    fn code_of_a_tag_that_has_lines_nested_is_dropped() {
        // Haml joins the next line onto the code, then compiles the tag's lines and not its value.
        assert_eq!(errors("%p= a,\n%b{c: 1} x\n  %i y\n"), []);
        assert_eq!(errors("%p= foo,\n  bar\n  %b x\n"), []);
        // Tags with code that goes on over lines, nested in one another: every piece keeps its place.
        assert_eq!(errors("%p= foo,\n  bar\n  %i= y,\n    z\n    %b x\n"), []);
        assert_eq!(errors("%p= foo,\n  bar\n  %b{a: 1}= y,\n    z\n    %b x\n"), []);
        // Script code with lines nested under it keeps its block, so the `end` it gets is unmatched.
        assert_eq!(messages("%p= foo,\n  bar\n  = y,\n    z\n    %b x\n"), ["unexpected 'end', ignoring it"]);
    }

    #[test]
    fn a_node_cut_short_by_the_end_of_the_document_still_counts() {
        // The attribute list takes the end of the document, and the tag is a child of the code above it.
        assert_eq!(messages("= foo\n  %a(href=x\n"), ["unexpected 'end', ignoring it"]);
        // Code that ends in a comma loses its line instead.
        assert_eq!(errors("= foo\n  = bar,\n"), []);
    }

    #[test]
    fn the_template_is_a_method_body() {
        assert!(messages("- FOO = 1\n").contains(&"dynamic constant assignment".to_string()));
        assert!(!messages("- class X; end\n").is_empty());
        assert_eq!(errors("- return if x\n= yield\n"), []);
        // An open `if` swallows the wrapper's `end`; the wrapper's own error isn't shown.
        assert_eq!(messages("- if x\n").len(), 1);
    }

    #[test]
    fn strict_locals_are_the_parameters() {
        let find = |text: &str| strict_locals::find(text, strict_locals::Close::default());
        let text = "-# locals: (**)\n= render \"x\", **\n";
        let found = super::diagnostics(text, &parse(text), ruby_prism::Version::Latest, find(text).as_ref());
        assert!(found.is_empty(), "{found:?}");
        // A bad signature is `strict_locals::diagnostics`'s to report.
        let text = "-# locals: (a b)\n%p= a\n";
        assert_eq!(super::diagnostics(text, &parse(text), ruby_prism::Version::Latest, find(text).as_ref()).len(), 0);
    }
}
