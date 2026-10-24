//! Rough Ruby snippet analysis: which snippets open blocks, which close them.
//! We never need a full parse; template engines only need `do`/`if`/`end` balance.

use once_cell::sync::Lazy;
use regex::Regex;

static STRINGS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#""(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|%[wiqQ]?\[[^\]]*\]|%[wiqQ]?\([^)]*\)"#).unwrap());
static TOKEN: Lazy<Regex> = Lazy::new(|| Regex::new(r"::|==|!=|&&|\|\||<=|>=|=>|->|[A-Za-z_]\w*[?!]?|\d+|[^\w\s]").unwrap());

/// Strip strings and comments so keywords inside them don't count.
pub fn strip(code: &str) -> String {
    let no_strings = STRINGS.replace_all(code, "\"\"");
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
const PREFIX_OK: &[&str] = &[
    "=", "(", ",", ";", "&&", "||", "and", "or", "not", "!", "return", "then", "else", "[", "{", "?", ":", "=>", "|", "<<", "+=", "-=",
    "||=",
];

/// Net block effect of a Ruby snippet. `end` count and opener count are also returned.
pub fn analyze(code: &str) -> (BlockEffect, usize, usize) {
    let stripped = strip(code);
    let tokens: Vec<&str> = TOKEN.find_iter(&stripped).map(|m| m.as_str()).collect();
    let mut opens = 0usize;
    let mut closes = 0usize;
    let mut has_continue = false;
    let mut loop_opened = false;

    for (i, &tok) in tokens.iter().enumerate() {
        let prev = if i > 0 { tokens[i - 1] } else { "" };
        let next = tokens.get(i + 1).copied().unwrap_or("");
        // `obj.class`, `foo.if`, `class: "x"`, `:if` are not keywords.
        if prev == "." || prev == "&." || (next == ":") || prev == ":" || prev == "def" {
            continue;
        }
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
                let statement_start = i == 0 || PREFIX_OK.contains(&prev);
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
}
