//! Rough Ruby snippet analysis: which snippets open blocks, which close them.
//! We never need a full parse; template engines only need `do`/`if`/`end` balance.

use regex::Regex;
use std::sync::LazyLock;

static STRINGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|%[wiqQ]?\[[^\]]*\]|%[wiqQ]?\([^)]*\)"#).unwrap());
static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"::|==|!=|&&|\|\||<=|>=|=>|->|[A-Za-z_]\w*[?!]?|\d+|[^\w\s]").unwrap());
// `=begin` … `=end` at line starts; kaminari's generated partials open with one.
static BLOCK_COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?ms)^=begin\b.*?(?:^=end\b[^\n]*|\z)").unwrap());

/// Strip strings and comments so keywords inside them don't count.
pub fn strip(code: &str) -> String {
    let code = BLOCK_COMMENT.replace_all(code, "");
    let no_strings = STRINGS.replace_all(&code, "\"\"");
    // Drop `# comment` to end of line, but keep `#{interpolation}`.
    let mut out = String::with_capacity(no_strings.len());
    for line in no_strings.split_inclusive('\n') {
        let mut cut = line.len();
        let b = line.as_bytes();
        for i in 0..b.len() {
            if b[i] == b'#' && b.get(i + 1) != Some(&b'{') {
                cut = i;
                break;
            }
        }
        out.push_str(&line[..cut]);
        if cut < line.len() && line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Where a Ruby snippet ends up: in code, inside a `"…"`/`'…'` string, or after a `#` (not `#{`) comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeEnd {
    Code,
    String,
    Comment,
}

pub fn code_end(code: &str) -> CodeEnd {
    let mut quote = None;
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(_), '\\') => {
                chars.next();
            }
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '#') if chars.peek() != Some(&'{') => return CodeEnd::Comment,
            _ => {}
        }
    }
    if quote.is_some() {
        CodeEnd::String
    } else {
        CodeEnd::Code
    }
}

/// The rest of the line after `code` is a Ruby comment.
pub fn comment_before(code: &str) -> bool {
    code_end(code) == CodeEnd::Comment
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockEffect {
    /// Opens N blocks (net), e.g. `if x` or `items.each do |i|`.
    Open(usize),
    /// Closes N blocks (net), e.g. `end` or `end end`.
    Close(usize),
    /// `else`, `elsif`, `when`, `rescue`, `ensure`: continues the current block.
    Continue,
    Neutral,
}

const OPENERS: &[&str] = &["if", "unless", "while", "until", "case", "begin", "def", "class", "module", "for", "do"];
const CONTINUERS: &[&str] = &["else", "elsif", "when", "rescue", "ensure", "in"];
const PREFIX_OK: &[&str] =
    &["=", "(", ",", ";", "&&", "||", "and", "or", "not", "!", "then", "else", "[", "{", "?", ":", "=>", "|", "<<", "+=", "-=", "||="];

/// Net block effect of a Ruby snippet. `end` count and opener count are also returned.
pub fn analyze(code: &str) -> (BlockEffect, usize, usize) {
    let stripped = strip(code);
    let matches: Vec<regex::Match> = TOKEN.find_iter(&stripped).collect();
    let tokens: Vec<&str> = matches.iter().map(|m| m.as_str()).collect();
    let mut opens = 0usize;
    let mut closes = 0usize;
    let mut has_continue = false;
    let mut loop_opened = false;

    for (i, &tok) in tokens.iter().enumerate() {
        let prev = if i > 0 { tokens[i - 1] } else { "" };
        let next = tokens.get(i + 1).copied().unwrap_or("");
        // `:if` and `if:` touch their colon; `actors: if x` is a hash value.
        let symbol = prev == ":" && matches[i - 1].end() == matches[i].start();
        let key = next == ":" && matches[i].end() == matches[i + 1].start();
        // `obj.class`, `foo.if`, `class: "x"`, `:if` are not keywords.
        if prev == "." || prev == "&." || key || symbol || prev == "def" {
            continue;
        }
        // A line break ends a statement unless the line ends in `\`.
        let new_line = i > 0 && prev != "\\" && stripped[matches[i - 1].end()..matches[i].start()].contains('\n');
        match tok {
            "end" => closes += 1,
            "do" => {
                if loop_opened {
                    loop_opened = false; // `while cond do` is the same block
                } else {
                    opens += 1;
                }
            }
            "if" | "unless" | "while" | "until" => {
                // Modifier form (`x if y`) does not open a block.
                let statement_start = i == 0 || new_line || PREFIX_OK.contains(&prev);
                if statement_start {
                    opens += 1;
                    if tok == "while" || tok == "until" {
                        loop_opened = true;
                    }
                }
            }
            "for" => {
                opens += 1;
                loop_opened = true;
            }
            t if OPENERS.contains(&t) => opens += 1,
            t if CONTINUERS.contains(&t) && i == 0 => has_continue = true,
            _ => {}
        }
    }

    let effect = if opens > closes {
        BlockEffect::Open(opens - closes)
    } else if closes > opens {
        BlockEffect::Close(closes - opens)
    } else if has_continue {
        BlockEffect::Continue
    } else {
        BlockEffect::Neutral
    };
    (effect, opens, closes)
}

/// Does the snippet end with a block that expects nested template content (`do`, `do |x|`)?
pub fn ends_with_block(code: &str) -> bool {
    let stripped = strip(code);
    let t = stripped.trim_end();
    t.ends_with(" do") || t == "do" || {
        // `do |a, b|`
        if let Some(idx) = t.rfind(" do |") {
            t[idx..].trim_end().ends_with('|')
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn line_breaks_start_statements() {
        assert_eq!(analyze("\nroles = []\nif admin\n  roles = [1]\nend\n").0, BlockEffect::Neutral);
        assert_eq!(analyze("tags = list\nif flare").0, BlockEffect::Open(1));
        assert_eq!(analyze("x = long_call \\\n  if y").0, BlockEffect::Neutral);
        assert_eq!(analyze("t(\"k\",\n  actors: if one\n    a\n  else\n    b\n  end)").0, BlockEffect::Neutral);
        assert_eq!(analyze("link_to x, if: y").0, BlockEffect::Neutral);
        assert_eq!(analyze("validate :x, :if => :y").0, BlockEffect::Neutral);
    }

    #[test]
    fn block_comments_hide_keywords() {
        assert_eq!(analyze("\n=begin\nfor each page\n- if any\n=end\n").0, BlockEffect::Neutral);
        assert_eq!(analyze("=begin\nunclosed for").0, BlockEffect::Neutral);
        assert_eq!(analyze("x = 1 # =begin\ny.each do").0, BlockEffect::Open(1));
    }

    #[test]
    fn basics() {
        assert_eq!(analyze("@users.each do |u|").0, BlockEffect::Open(1));
        assert_eq!(analyze("if user.admin?").0, BlockEffect::Open(1));
        assert_eq!(analyze("end").0, BlockEffect::Close(1));
        assert_eq!(analyze("else").0, BlockEffect::Continue);
        assert_eq!(analyze("link_to 'x', path, class: 'a' if ok").0, BlockEffect::Neutral);
        assert_eq!(analyze("x = if a then 1 else 2 end").0, BlockEffect::Neutral);
        assert_eq!(analyze("while i < 3 do").0, BlockEffect::Open(1));
        assert_eq!(analyze("render partial: 'x', locals: { class: foo.class }").0, BlockEffect::Neutral);
        assert_eq!(analyze("form_with model: @u do |f|").0, BlockEffect::Open(1));
        assert_eq!(analyze("puts \"if\" # if end").0, BlockEffect::Neutral);
        assert!(ends_with_block("form_with model: @u do |f|"));
        assert!(!ends_with_block("link_to 'x', y"));
    }

    #[test]
    fn strip_drops_comments_keeps_interpolation_and_newlines() {
        assert_eq!(strip("a # c\nb"), "a \nb");
        assert_eq!(strip("a\nb\n"), "a\nb\n");
        assert_eq!(strip("a # c"), "a ");
        assert_eq!(strip("x = #{y} # c\n"), "x = #{y} \n");
        assert_eq!(strip("puts 'a # b' # c"), "puts \"\" ");
    }

    #[test]
    fn code_end_tracks_quotes_escapes_and_comments() {
        assert_eq!(code_end("x = 1"), CodeEnd::Code);
        assert_eq!(code_end("x = \"a"), CodeEnd::String);
        assert_eq!(code_end("x = \"a\" + 'b"), CodeEnd::String);
        assert_eq!(code_end(r#"x = "a\"b"#), CodeEnd::String);
        assert_eq!(code_end(r#"x = "a\\" + 1"#), CodeEnd::Code);
        assert_eq!(code_end(r#"x = "it's""#), CodeEnd::Code);
        assert_eq!(code_end(r#"x = 'say "hi"'"#), CodeEnd::Code);
        assert_eq!(code_end("x = \"a # b\""), CodeEnd::Code);
        assert_eq!(code_end("x = 1 # c"), CodeEnd::Comment);
        assert_eq!(code_end("x = #{y}"), CodeEnd::Code);
        assert!(comment_before("link_to x # why"));
        assert!(!comment_before("link_to x"));
    }

    #[test]
    fn analyze_counts_nested_blocks_loops_and_modifiers() {
        assert_eq!(analyze("if a"), (BlockEffect::Open(1), 1, 0));
        assert_eq!(analyze("if a; if b"), (BlockEffect::Open(2), 2, 0));
        assert_eq!(analyze("end; if a; if b"), (BlockEffect::Open(1), 2, 1));
        assert_eq!(analyze("x = :if").0, BlockEffect::Neutral);
        assert_eq!(analyze("def end").0, BlockEffect::Open(1));
        assert_eq!(analyze("end; end"), (BlockEffect::Close(2), 0, 2));
        assert_eq!(analyze("if a; end; end"), (BlockEffect::Close(1), 1, 2));
        assert_eq!(analyze("for x in xs").0, BlockEffect::Open(1));
        assert_eq!(analyze("for x in xs do").0, BlockEffect::Open(1));
        assert_eq!(analyze("until done do").0, BlockEffect::Open(1));
        assert_eq!(analyze("elsif b").0, BlockEffect::Continue);
        assert_eq!(analyze("x = y rescue nil").0, BlockEffect::Neutral);
        assert_eq!(analyze("return if x").0, BlockEffect::Neutral);
        assert_eq!(analyze("return unless current_user").0, BlockEffect::Neutral);
        assert_eq!(analyze("x = 1 if (a)").0, BlockEffect::Neutral);
        assert_eq!(analyze("x = (if a then 1 else 2 end)").0, BlockEffect::Neutral);
    }
}
