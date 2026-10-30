//! Ruby syntax errors in a Slim template: the Ruby `Slim::Engine` generates from the parser's output, parsed by Prism.

use crate::generated_ruby::{ruby_string_body, Compiled};
use crate::slim_parser::{Origin, Parsed, Piece};
use regex::Regex;
use std::sync::LazyLock;
use tower_lsp::lsp_types::Diagnostic;

// Slim::EndInserter, Slim::DoInserter and Slim::Controls decide where blocks open and close.
static OPENS_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)\A(if|begin|unless|else|elsif|when|in|rescue|ensure)\b|\bdo\s*(\|[^|]*\|)?\s*$").unwrap());
static CONTINUES_BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A(else|elsif|when|in|rescue|ensure)\b").unwrap());
static NEEDS_NO_DO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A(if|unless|else|elsif|when|in|begin|rescue|ensure|case)\b|\bdo\s*(\|[^|]*\|\s*)?\n?\z").unwrap());
static CAPTURES_BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)\A(if|unless)\b|\bdo\s*(\|[^|]*\|)?\s*$").unwrap());

/// What the last line of a depth still owes after its nested lines.
enum Closer {
    Markup(String),
    /// The end of a captured `=` block.
    Code(String),
}

#[derive(Default)]
struct Level {
    /// `Slim::EndInserter`'s `prev_indent`: the last sibling opened a block that still needs its `end`.
    open: bool,
    closers: Vec<Closer>,
}

struct Generator<'a> {
    pieces: &'a [Piece],
    out: Compiled,
    levels: Vec<Level>,
    depth: usize,
    current_line: usize,
    has_child_lines: bool,
    has_content: bool,
    /// Markup not yet written: Temple merges neighbouring static strings into one.
    markup: String,
}

/// The Ruby `Slim::Engine` would generate, as far as this module reproduces it: markup, code and line breaks.
fn generated(text: &str, extra_engines: &[String]) -> String {
    compile(&crate::slim_parser::parse(text, extra_engines).pieces).source
}

/// Whether `Slim::Engine` would compile both texts to the same Ruby.
pub fn same_compilation(before: &str, after: &str, extra_engines: &[String]) -> bool {
    before == after || generated(before, extra_engines) == generated(after, extra_engines)
}

pub fn diagnostics(text: &str, parsed: &Parsed, ruby: ruby_prism::Version) -> Vec<Diagnostic> {
    compile(&parsed.pieces).diagnostics(text, ruby)
}

fn compile(pieces: &[Piece]) -> Compiled {
    let mut generator = Generator {
        pieces,
        out: Compiled::new(),
        levels: vec![Level::default()],
        depth: 0,
        current_line: 0,
        has_child_lines: false,
        has_content: false,
        markup: String::new(),
    };
    generator.run();
    generator.out
}

impl Generator<'_> {
    fn run(&mut self) {
        self.write("_buf = ''.dup; ", 0);
        for (i, piece) in self.pieces.iter().enumerate() {
            match piece {
                Piece::Line { depth, line } => self.start_line(i, *depth, *line),
                Piece::Newline => {
                    self.flush_markup(self.current_line);
                    self.out.new_line();
                    self.out.push_floating("; ");
                }
                Piece::Static(text) => self.markup.push_str(text),
                Piece::Close(text) => {
                    if self.has_child_lines {
                        self.levels[self.depth].closers.push(Closer::Markup(text.clone()));
                    } else {
                        self.markup.push_str(text);
                    }
                }
                Piece::Control { code, at } => self.control(code, at),
                Piece::Output { escape, code, block, at } => self.output(*escape, code, *block, at),
                Piece::Attribute { name, code, splat, at } => self.attribute(name, code, *splat, at),
                Piece::Splat { code, at } => {
                    self.write("_slim_splat.splat_attrs((", at.line);
                    self.code(code, at);
                    self.write(")); ", at.line);
                }
                Piece::Code { text, at } => {
                    self.code(text, at);
                    self.write("; ", at.line);
                }
            }
        }
        // What closes at the end of the file lands past the last line; errors there are shown on it.
        self.current_line = usize::MAX;
        self.finish_to(0, true);
        self.write("_buf", self.current_line);
    }

    /// Writes the pending markup as the static string Temple generates for it.
    fn flush_markup(&mut self, line: usize) {
        if self.markup.is_empty() {
            return;
        }
        let literal = format!("_buf << (\"{}\".freeze); ", ruby_string_body(&std::mem::take(&mut self.markup)));
        self.raw(&literal, line);
    }

    /// Appends text that starts on template line `line`, after any pending markup.
    fn write(&mut self, text: &str, line: usize) {
        self.flush_markup(line);
        self.raw(text, line);
    }

    fn raw(&mut self, text: &str, line: usize) {
        self.out.push(text, line);
    }

    /// Appends the code of `at`, remembering where it came from when it is one verbatim run.
    fn code(&mut self, code: &str, at: &Origin) {
        // The run starts after the markup that `write` would flush first.
        self.flush_markup(at.line);
        let from = self.out.source.len();
        self.raw(code, at.line);
        if let Some(offset) = at.offset {
            self.out.mark_verbatim(from, code.len(), offset);
        }
    }

    /// Whether lines nest below the one at `index`, and whether they hold more than newlines (Slim's `empty_exp?`).
    fn below(&self, index: usize, depth: usize) -> (bool, bool) {
        let (mut lines, mut content) = (false, false);
        for piece in &self.pieces[index + 1..] {
            match piece {
                Piece::Line { depth: d, .. } if *d <= depth => break,
                Piece::Line { .. } => lines = true,
                Piece::Newline => {}
                _ => content |= lines,
            }
        }
        (lines, content)
    }

    /// Closes what the lines deeper than `target` left open; at the end of the template, `target`'s own level too.
    fn finish_to(&mut self, target: usize, at_end: bool) {
        while self.levels.len() > target + 1 {
            let level = self.levels.pop().unwrap_or_default();
            if level.open {
                self.write("end; ", self.current_line);
            }
            if let Some(parent) = self.levels.last_mut() {
                let closers = std::mem::take(&mut parent.closers);
                self.close(closers);
            }
        }
        if let Some(level) = self.levels.get_mut(target) {
            let closers = std::mem::take(&mut level.closers);
            self.close(closers);
        }
        if at_end && self.levels.first().is_some_and(|level| level.open) {
            self.write("end; ", self.current_line);
        }
    }

    fn close(&mut self, closers: Vec<Closer>) {
        for closer in closers {
            match closer {
                Closer::Markup(text) => self.markup.push_str(&text),
                Closer::Code(code) => self.write(&code, self.current_line),
            }
        }
    }

    fn start_line(&mut self, index: usize, depth: usize, line: usize) {
        self.current_line = line;
        self.finish_to(depth, false);
        while self.levels.len() <= depth {
            self.levels.push(Level::default());
        }
        self.depth = depth;
        (self.has_child_lines, self.has_content) = self.below(index, depth);
        let first = self.pieces[index + 1..].iter().take_while(|p| !matches!(p, Piece::Line { .. })).find(|p| !matches!(p, Piece::Newline));
        // Anything but a control line or a Slim comment ends the block the previous sibling opened.
        if first.is_some_and(|p| !matches!(p, Piece::Control { .. })) && self.levels[depth].open {
            self.levels[depth].open = false;
            self.write("end; ", line);
        }
    }

    /// `Slim::DoInserter`: a line with content nested below it that doesn't open a block itself gets one.
    fn needs_do(&self, code: &str, owns_block: bool) -> bool {
        owns_block && self.has_content && !NEEDS_NO_DO.is_match(code)
    }

    fn control(&mut self, code: &str, at: &Origin) {
        let needs_do = self.needs_do(code, true);
        let full = if needs_do { format!("{code} do") } else { code.to_string() };
        let depth = self.depth;
        if self.levels[depth].open && !CONTINUES_BLOCK.is_match(&full) {
            self.write("end; ", at.line);
        }
        self.code(code, at);
        self.write(if needs_do { " do; " } else { "; " }, at.line);
        self.levels[depth].open = OPENS_BLOCK.is_match(&full);
    }

    fn output(&mut self, escape: bool, code: &str, owns_block: bool, at: &Origin) {
        let needs_do = self.needs_do(code, owns_block);
        let full = if needs_do { format!("{code} do") } else { code.to_string() };
        if !CAPTURES_BLOCK.is_match(&full) {
            self.write(if escape { "_buf << ((::Temple::Utils.escape_html((" } else { "_buf << ((" }, at.line);
            self.code(code, at);
            self.write(if escape { "))).to_s); " } else { ").to_s); " }, at.line);
            return;
        }
        self.write("_slim_controls1 = ", at.line);
        self.code(code, at);
        self.write(if needs_do { " do; _slim_controls2 = ''.dup; " } else { "; _slim_controls2 = ''.dup; " }, at.line);
        let value = if escape { "((::Temple::Utils.escape_html((_slim_controls1))).to_s)" } else { "((_slim_controls1).to_s)" };
        let closer = format!("_slim_controls2; end; _buf << {value}; ");
        if owns_block && self.has_child_lines {
            self.levels[self.depth].closers.push(Closer::Code(closer));
        } else {
            self.write(&closer, at.line);
        }
    }

    fn attribute(&mut self, name: &str, code: &str, splat: bool, at: &Origin) {
        if matches!(code, "true" | "false" | "nil") {
            return;
        }
        if splat {
            self.write(&format!("_slim_splat.code_attr(\"{name}\", true, ("), at.line);
            self.code(code, at);
            self.write(")); ", at.line);
        } else {
            self.write("_slim_codeattributes = ", at.line);
            self.code(code, at);
            self.write("; ", at.line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slim_parser::parse;

    fn diagnostics(text: &str) -> Vec<Diagnostic> {
        super::diagnostics(text, &parse(text, &[]), ruby_prism::Version::Latest)
    }

    /// (0-based line, Prism's message) of each error.
    fn errors(text: &str) -> Vec<(u32, String)> {
        diagnostics(text).into_iter().map(|d| (d.range.start.line, d.message)).collect()
    }

    fn at(line: u32, message: &str) -> (u32, String) {
        (line, message.to_string())
    }

    #[test]
    fn compilation_changes_with_text_offsets_but_not_with_indentation() {
        let same = |a, b| same_compilation(a, b, &[]);
        // A `|` continuation line that lost its offset, or text that lost its trailing space, changes the output.
        assert!(!same("div\n  | a\n      b\n", "div\n  | a\n   b\n"));
        assert!(!same("div\n  | a\n      b\n", "div\n  | a\n  b\n"));
        assert!(!same("p text  \n", "p text\n"));
        assert!(!same("p\n  | a\n\n  b\n", "p\n  | a\n  b\n"));
        assert!(!same("javascript:\n  if (x) {\n    y();\n  }\n", "javascript:\n  if (x) {\n  y();\n  }\n"));
        assert!(!same("ruby:\n  a = <<~T\n    x\n  T\n", "ruby:\n  a = <<~T\n  x\n  T\n"));
        // Moving a block as a whole, trailing empty lines and a tab for four spaces change nothing.
        assert!(same("div\n  | a\n      b\n", "div\n      | a\n          b\n"));
        assert!(same("p\n  - if x\n    a\n", "p\n    - if x\n        a\n"));
        assert!(same("p\n\n\n", "p\n"));
        assert!(same("p\n\tq\n", "p\n    q\n"));
        assert!(same("p\r\n  q\r\n", "p\n  q\n"));
    }

    #[test]
    fn blocks_close_where_slim_closes_them() {
        for valid in [
            "- if a\n  p x\n- elsif b\n  p y\n- else\n  p z\n",
            "- case x\n- when 1\n  p a\n- else\n  p b\n",
            "- case x\n  - when 1\n    p a\n  - else\n    p b\n",
            "- begin\n  p\n- rescue => e\n  q\n- else\n  r\n- ensure\n  s\n",
            "ul\n  - items.each do |i|\n    li\n      - if i\n        b\n      - else\n        i\n  li z\n",
            "- if a\n  / note\n- else\n  p\n",
            "- if a\n\n- else\n  p\n",
            "- x = [1,\n  2]\np = x.sum\n",
        ] {
            assert_eq!(errors(valid), [], "{valid:?}");
        }
    }

    #[test]
    fn nesting_under_a_line_that_cannot_hold_a_block() {
        // Slim's test_invalid_nested_code and test_invalid_nested_output: `do` is appended and `end` follows.
        let expected = [
            at(2, "unexpected 'do', expecting end-of-input"),
            at(2, "unexpected 'do', ignoring it"),
            at(3, "unexpected 'end', ignoring it"),
        ];
        assert_eq!(errors("\np\n  - test = 123\n    = \"Hello from within a block! \"\n"), expected);
        assert_eq!(errors("\np\n  = \"Hello Ruby!\"\n    = \"Hello from within a block! \"\n"), expected);
    }

    #[test]
    fn code_after_closed_markup_keeps_its_exact_range() {
        let range = |text| diagnostics(text)[0].range;
        let alone = range("- x = foo(1,,2)\n");
        let after_markup = range("div\n  p\n- x = foo(1,,2)\n");
        assert_eq!((after_markup.start.line, after_markup.start.character), (2, alone.start.character));
        assert_eq!(after_markup.end.character, alone.end.character);
    }

    #[test]
    fn captured_blocks_and_block_arguments_are_valid() {
        assert_eq!(errors("= form_with model: @u do |f|\n  = f.text_field :x\np after\n"), []);
        assert_eq!(errors("div\n  = capture do\n    p\n  = yield\n"), []);
        assert_eq!(errors("- foo do |a|\n  = a\np\n"), []);
    }

    #[test]
    fn unfinished_code_is_reported_where_prism_finds_it() {
        assert_eq!(
            errors("= foo(\np\n"),
            [at(1, "unexpected end-of-input, assuming it is closing the parent top level context"), at(1, "expected a matching `)`")]
        );
        assert_eq!(
            errors("p\n  = bar baz,\n    qux)\n"),
            [at(2, "unexpected ')', expecting end-of-input"), at(2, "unexpected ')', ignoring it")]
        );
        assert_eq!(errors("- else\n  p\n"), [at(0, "unexpected 'else', ignoring it"), at(1, "unexpected 'end', ignoring it")]);
    }

    #[test]
    fn errors_inside_one_verbatim_line_are_exact() {
        // Prism puts this one on the space before the `2`.
        let text = "é = 1\np\n  - @a = [1 2]\n";
        let found = diagnostics(text);
        assert_eq!(found[0].message, "unexpected integer; expected a `,` separator for the array elements");
        assert_eq!((found[0].range.start.line, found[0].range.start.character), (2, 11));
        // Continued code isn't one run of the template: the whole line stands in.
        let text = "p= foo(1,\n  2))\n";
        let found = diagnostics(text);
        assert_eq!((found[0].range.start.line, found[0].range.start.character), (1, 2));
    }

    #[test]
    fn attributes_splats_and_interpolation() {
        assert_eq!(errors("a href=bar(1, 2) title=\"x#{y}z\" *attrs t\n"), []);
        assert_eq!(errors("p class=foo(\n").len(), 2);
        assert_eq!(errors("p text #{foo(} more\n").len(), 2);
        assert_eq!(errors("/! #{foo(}\np\n").len(), 2);
        assert_eq!(errors("a href=\"x#{y +}\" t\n")[0].0, 0);
        assert_eq!(errors("a(href=\"x\"\n  title=foo(\n  id=\"z\") t\n")[0].0, 1);
    }

    #[test]
    fn interpolation_ends_at_the_first_closing_brace_even_in_a_string() {
        // `Slim::Interpolation` matches braces only, so the quote opens a string that swallows the generated markup.
        let messages = |text| errors(text).into_iter().map(|(_, m)| m).collect::<Vec<_>>();
        assert_eq!(
            messages("p Use #{'}'} here\n"),
            [
                "unexpected '}', expecting end-of-input",
                "unexpected '}', ignoring it",
                "expected a matching `)`",
                "unexpected '}'; expected a `)` to close the arguments",
                "unexpected '}', expecting end-of-input",
                "unexpected '}', ignoring it",
                "expected a matching `)`",
                "unexpected '}', expecting end-of-input",
                "unexpected '}', ignoring it",
                "expected a matching `)`",
                "unexpected '}', expecting end-of-input",
                "unexpected '}', ignoring it",
                "unterminated regexp meets end of file; expected a closing delimiter",
            ]
        );
        assert_eq!(messages("p a #{x(\"}\")} b\n")[0], "unexpected backslash, ignoring it");
        assert_eq!(errors("p Use #{'}'} here\\#{x\n").len(), 13);
        assert_eq!(errors("p a #{x('}')} b\n").len(), 13);
        assert_eq!(errors("div\n  p a #{'x'}\n  p \\#{b} #{c}\n"), []);
    }

    #[test]
    fn embedded_ruby_keeps_its_own_lines() {
        assert_eq!(errors("ruby:\n  x = 1\n  y = 2\np #{x} #{y}\n"), []);
        // Slim's `collect_text` drops blank lines before the first one, and keeps later ones.
        assert_eq!(
            errors("ruby:\n  def f\n    1\np t\n"),
            [
                at(3, "unexpected end-of-input, assuming it is closing the parent top level context"),
                at(1, "expected an `end` to close the `def` statement")
            ]
        );
        let found = errors("ruby:\n\n  x = 1\n\n  y = (\np t\n");
        // Prism finds the unclosed `(` at the end of the code, which is the line after it.
        assert!(found.contains(&at(5, "expected a matching `)`")), "{found:?}");
    }
}
