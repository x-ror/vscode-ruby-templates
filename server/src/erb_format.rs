//! ERB re-indentation. A line that starts in markup gets one indent unit per open HTML element and ERB block, read
//! with `tags.rs`; a line that starts inside a `<script>`/`<style>` element, a multi-line tag, comment or ERB tag
//! moves with the line its owner starts on, keeping its relative indentation; a `<pre>`, `<textarea>` or `<title>`
//! body, a quoted attribute value and Ruby holding a heredoc or a multi-line string keep every byte.

use crate::format::{skip, Indented, SkipKind, Skipped};
use crate::ruby::{code_end, BlockEffect, CodeEnd};
use crate::tags::{self, Block, Tag, Token, OPTIONAL_END, VOID};
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;
use tower_lsp::lsp_types::DiagnosticSeverity;

// `<<~SQL`, `<<-EOS`, `<<"END"`: the body's indentation is the string's content.
static HEREDOC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<<[~-]?["'`]?[A-Za-z_]"#).unwrap());

/// Where a line starts.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    Markup,
    /// Inside content whose whitespace is significant.
    Fixed,
    /// Inside the unit with this index, which moves with its first line.
    Unit(usize),
}

struct Unit {
    owner_line: usize,
    /// Ruby whose line breaks may fall inside a string: it can't move at all.
    rigid: bool,
}

struct Line<'a> {
    start: usize,
    ws: &'a str,
    rest: &'a str,
    /// `"\r"` when the line ends with CRLF.
    cr: &'a str,
}

/// Inclusive ranges of line starts, or of line-end offsets.
type Ranges = Vec<(usize, usize)>;

/// `text` with every line re-indented by `unit`, and trailing whitespace dropped when `trim` is set.
pub(crate) fn indent(text: &str, unit: &str, trim: bool, partial: bool) -> Result<Indented, Skipped> {
    if let Some(d) = crate::erb::diagnostics(text).into_iter().find(|d| d.severity == Some(DiagnosticSeverity::ERROR)) {
        return skip(SkipKind::Doubt, format!("line {}: {}", d.range.start.line + 1, d.message));
    }
    if let Some(d) = tags::diagnostics(text, partial).first() {
        return skip(SkipKind::Doubt, format!("line {}: {}", d.range.start.line + 1, d.message));
    }
    let Some(tokens) = tags::tokens(text) else {
        return skip(SkipKind::Doubt, "a `javascript_tag`/`content_tag :script` block or unreadable ERB");
    };
    let lines = split(text);
    let starts: Vec<usize> = lines.iter().map(|l| l.start).collect();
    let line_of = |offset: usize| starts.partition_point(|&s| s <= offset) - 1;

    let mut fixed: Ranges = Vec::new();
    let mut keep_end: Ranges = Vec::new();
    // (first, last line start, owner offset, rigid), outer before inner.
    let mut unit_spans: Vec<(usize, usize, usize, bool)> = Vec::new();
    let paired = paired_optional(&tokens);
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Erb(e) => {
                let code = &text[e.at..e.end];
                unit_spans.push((e.at + 1, e.end - 1, e.at, rigid_ruby(code)));
                keep_end.push((e.at + 1, e.end - 1));
            }
            Token::Open { tag, quoted, .. } | Token::Close { tag, quoted } => {
                unit_spans.push((tag.start + 1, tag.end - 1, tag.start, false));
                for &(q, q_end) in quoted {
                    fixed.push((q + 1, q_end - 1));
                    keep_end.push((q + 1, q_end - 1));
                }
                if let Token::Open { tag, .. } = token {
                    if tag.name == "pre" {
                        let Some(close) = closing(&tokens[i + 1..], "pre") else {
                            return skip(SkipKind::Doubt, format!("line {}: `<pre>` is never closed", line_of(tag.start) + 1));
                        };
                        fixed.push((tag.end, close.start));
                        keep_end.push((tag.end, close.start.saturating_sub(1)));
                    }
                }
            }
            Token::Raw { tag, close } => {
                let Some((close_start, close_end)) = *close else {
                    return skip(SkipKind::Doubt, format!("line {}: `<{}>` is never closed", line_of(tag.start) + 1, tag.name));
                };
                keep_end.push((tag.end, close_start.saturating_sub(1)));
                if tag.name == "script" || tag.name == "style" {
                    unit_spans.push((tag.start + 1, close_end - 1, tag.start, false));
                } else {
                    unit_spans.push((tag.start + 1, tag.end - 1, tag.start, false));
                    fixed.push((tag.end, close_start));
                }
            }
            Token::Comment { start, end } => unit_spans.push((start + 1, end - 1, *start, false)),
            Token::Unterminated => return skip(SkipKind::Doubt, "a tag is never closed with `>`"),
        }
    }

    let mut roles = vec![Role::Markup; lines.len()];
    let mut units = Vec::new();
    // Inner units start later; sorting by start lets them win over the unit around them.
    unit_spans.sort_by_key(|&(_, _, owner, _)| owner);
    for (first, last, owner, rigid) in unit_spans {
        let lines_inside = lines_starting_in(&starts, first, last);
        if lines_inside.is_empty() {
            continue;
        }
        let index = units.len();
        units.push(Unit { owner_line: line_of(owner), rigid });
        for line in lines_inside {
            roles[line] = Role::Unit(index);
        }
    }
    for &(first, last) in &fixed {
        for line in lines_starting_in(&starts, first, last) {
            roles[line] = Role::Fixed;
        }
    }
    let mut keeps_end = vec![false; lines.len()];
    for (i, line) in lines.iter().enumerate() {
        let end = line.start + line.ws.len() + line.rest.len();
        keeps_end[i] = keep_end.iter().any(|&(a, b)| a <= end && end <= b);
    }

    let (levels, uncertain) = levels(text, &tokens, &paired, &lines, &roles, partial);
    let notes = uncertain
        .map(|line| format!("line {}: indentation kept from here on: ERB branches leave different tags open, or tags overlap", line + 1))
        .into_iter()
        .collect();
    let decided = decide(&lines, &roles, &units, &levels, unit);

    let mut out = String::with_capacity(text.len());
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let blank = line.rest.trim_matches([' ', '\t']).is_empty();
        let content = if trim && !keeps_end[i] { line.rest.trim_end_matches([' ', '\t']) } else { line.rest };
        match &decided[i] {
            _ if blank && roles[i] == Role::Markup && trim => {}
            Some(ws) if !content.is_empty() => out.push_str(ws),
            _ => out.push_str(line.ws),
        }
        out.push_str(content);
        out.push_str(line.cr);
    }
    let fixed_lines = roles.iter().zip(&keeps_end).map(|(role, keeps)| *role != Role::Markup || *keeps).collect();
    Ok(Indented { text: out, fixed: fixed_lines, fixed_end: false, notes })
}

fn split(text: &str) -> Vec<Line<'_>> {
    let mut start = 0;
    text.split('\n')
        .map(|raw| {
            let (body, cr) = raw.strip_suffix('\r').map_or((raw, ""), |b| (b, "\r"));
            let rest = body.trim_start_matches([' ', '\t']);
            let line = Line { start, ws: &body[..body.len() - rest.len()], rest, cr };
            start += raw.len() + 1;
            line
        })
        .collect()
}

fn lines_starting_in(starts: &[usize], first: usize, last: usize) -> std::ops::Range<usize> {
    if first > last {
        return 0..0;
    }
    starts.partition_point(|&s| s < first)..starts.partition_point(|&s| s <= last)
}

/// Ruby code that a move could change: a line break inside a string, or a heredoc.
fn rigid_ruby(code: &str) -> bool {
    if !code.contains('\n') {
        return false;
    }
    HEREDOC.is_match(code) || code.match_indices('\n').any(|(at, _)| code_end(&code[..at]) == CodeEnd::String)
}

fn closing<'t>(tokens: &'t [Token], name: &str) -> Option<&'t Tag> {
    tokens.iter().find_map(|t| match t {
        Token::Close { tag, .. } if tag.name == name => Some(tag),
        _ => None,
    })
}

/// Indices of optional-end start and end tags (`<li>`…`</li>`) that pair up, paired as `tags.rs` pairs them: only
/// those indent, since an implied end (`<li>a<li>b`) has no line to dedent.
fn paired_optional(tokens: &[Token]) -> HashSet<usize> {
    let mut open: Vec<(usize, &str)> = Vec::new();
    let mut paired = HashSet::new();
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Open { tag, .. } if OPTIONAL_END.contains(&tag.name.as_str()) => open.push((i, &tag.name)),
            Token::Close { tag, .. } if OPTIONAL_END.contains(&tag.name.as_str()) => {
                if let Some(at) = open.iter().rposition(|(_, name)| *name == tag.name) {
                    paired.extend([open[at].0, i]);
                    open.truncate(at);
                }
            }
            _ => {}
        }
    }
    paired
}

fn span(token: &Token) -> (usize, usize) {
    match token {
        Token::Erb(e) => (e.at, e.end),
        Token::Open { tag, .. } | Token::Close { tag, .. } => (tag.start, tag.end),
        Token::Raw { tag, close } => (tag.start, close.map_or(tag.end, |(_, end)| end)),
        Token::Comment { start, end } => (*start, *end),
        Token::Unterminated => (usize::MAX, usize::MAX),
    }
}

/// The tag stack and open ERB blocks while reading the tokens in order.
struct Nesting<'t> {
    tokens: &'t [Token<'t>],
    paired: &'t HashSet<usize>,
    partial: bool,
    stack: Vec<Tag>,
    blocks: Vec<Block>,
}

enum Kind {
    /// Lowers the nesting: an end tag that matches, `<% end %>`, `<% else %>`.
    Closer,
    /// Changes nothing.
    Transparent,
    Other,
}

impl Nesting<'_> {
    fn depth(&self) -> usize {
        self.stack.len() + self.blocks.len()
    }

    fn kind(&self, i: usize) -> Kind {
        match &self.tokens[i] {
            Token::Erb(e) if matches!(e.effect, BlockEffect::Close(_) | BlockEffect::Continue) => Kind::Closer,
            Token::Close { tag, .. } if self.ignored_close(i, tag) => Kind::Transparent,
            Token::Close { .. } => Kind::Closer,
            _ => Kind::Other,
        }
    }

    fn ignored_close(&self, i: usize, tag: &Tag) -> bool {
        let name = tag.name.as_str();
        VOID.contains(&name) || (OPTIONAL_END.contains(&name) && !self.paired.contains(&i)) || (self.partial && self.stack.is_empty())
    }

    /// Applies token `i`; false once the nesting depends on which branch ran.
    fn step(&mut self, i: usize) -> bool {
        match &self.tokens[i] {
            Token::Erb(e) => tags::apply(e, &mut self.stack, &mut self.blocks),
            Token::Open { tag, pairs, .. } => {
                if *pairs || self.paired.contains(&i) {
                    self.stack.push(tag.clone());
                }
                true
            }
            Token::Close { tag, .. } if self.ignored_close(i, tag) => true,
            Token::Close { tag, .. } => {
                // A mismatch `tags.rs` doesn't report: optional-end tags crossing others (`<p><div></p></div>`).
                if self.stack.last().is_none_or(|top| top.name != tag.name) {
                    return false;
                }
                self.stack.pop();
                true
            }
            _ => true,
        }
    }
}

/// The nesting level of each markup line, and the line from which the nesting is uncertain, if any.
fn levels(
    text: &str,
    tokens: &[Token],
    paired: &HashSet<usize>,
    lines: &[Line],
    roles: &[Role],
    partial: bool,
) -> (Vec<Option<usize>>, Option<usize>) {
    let mut nesting = Nesting { tokens, paired, partial, stack: Vec::new(), blocks: Vec::new() };
    let mut out = vec![None; lines.len()];
    let mut next = 0;
    for (l, line) in lines.iter().enumerate() {
        while next < tokens.len() && span(&tokens[next]).0 < line.start {
            if !nesting.step(next) {
                return (out, Some(l));
            }
            next += 1;
        }
        if roles[l] != Role::Markup || line.rest.is_empty() {
            continue;
        }
        let line_end = line.start + line.ws.len() + line.rest.len();
        let mut level = nesting.depth();
        let mut at = line.start + line.ws.len();
        while next < tokens.len() {
            let (start, end) = span(&tokens[next]);
            if start >= line_end || !text[at..start].trim_matches([' ', '\t']).is_empty() {
                break;
            }
            let kind = nesting.kind(next);
            if matches!(kind, Kind::Other) {
                break;
            }
            // `<% end %>` and `<% else %>` line up with their block's opening, under tags a branch left open.
            let block_level = match &tokens[next] {
                Token::Erb(e) => match e.effect {
                    BlockEffect::Close(n) => nesting.blocks.len().checked_sub(n).map(|rest| nesting.blocks[rest].entry.len() + rest),
                    _ => nesting.blocks.len().checked_sub(1).map(|rest| nesting.blocks[rest].entry.len() + rest),
                },
                _ => None,
            };
            if !nesting.step(next) {
                return (out, Some(l));
            }
            if matches!(kind, Kind::Closer) {
                level = level.min(block_level.unwrap_or(nesting.depth()));
            }
            at = end;
            next += 1;
        }
        out[l] = Some(level);
    }
    (out, None)
}

/// The new leading whitespace of each line, `None` to keep it. A unit moves only as a whole: if one of its lines
/// can't follow the first (it is indented less, or holds Ruby that can't move), the first line stays too.
fn decide(lines: &[Line], roles: &[Role], units: &[Unit], levels: &[Option<usize>], unit: &str) -> Vec<Option<String>> {
    let mut pinned = vec![false; lines.len()];
    let mut moves = vec![true; units.len()];
    loop {
        let mut decided: Vec<Option<String>> = Vec::with_capacity(lines.len());
        for (i, line) in lines.iter().enumerate() {
            let ws = match roles[i] {
                Role::Markup if !pinned[i] => levels[i].map(|level| unit.repeat(level)),
                Role::Unit(u) if moves[u] && !line.rest.is_empty() => {
                    let owner = &lines[units[u].owner_line];
                    decided[units[u].owner_line].as_ref().and_then(|new| Some(format!("{new}{}", line.ws.strip_prefix(owner.ws)?)))
                }
                _ => None,
            };
            decided.push(ws);
        }
        let stuck = units.iter().enumerate().find(|&(u, unit)| {
            let owner = &lines[unit.owner_line];
            let moved = decided[unit.owner_line].as_deref().is_some_and(|new| new != owner.ws);
            let inside = || roles.iter().zip(lines).filter(|(role, line)| **role == Role::Unit(u) && !line.rest.is_empty());
            moves[u] && moved && (unit.rigid || inside().any(|(_, line)| !line.ws.starts_with(owner.ws)))
        });
        let Some((u, unit)) = stuck else { return decided };
        moves[u] = false;
        match roles[unit.owner_line] {
            Role::Unit(outer) => moves[outer] = false,
            _ => pinned[unit.owner_line] = true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(text: &str) -> String {
        let once = indent(text, "  ", true, false).unwrap_or_else(|e| panic!("{e}: {text}")).text;
        assert_eq!(indent(&once, "  ", true, false).map(|i| i.text).as_deref(), Ok(once.as_str()), "idempotent");
        once
    }

    fn lines(list: &[&str]) -> String {
        list.join("\n")
    }

    #[test]
    fn nests_elements_and_blocks() {
        let text = lines(&["<div>", "<% if a %>", "<p>", "x", "</p>", "<% elsif b %>", "y", "<% else %>", "z", "<% end %>", "</div>"]);
        let want = lines(&[
            "<div>",
            "  <% if a %>",
            "    <p>",
            "      x",
            "    </p>",
            "  <% elsif b %>",
            "    y",
            "  <% else %>",
            "    z",
            "  <% end %>",
            "</div>",
        ]);
        assert_eq!(fmt(&text), want);
        let each = lines(&["<ul>", "<% items.each do |i| %>", "<li><%= i %></li>", "<% end %>", "</ul>"]);
        assert_eq!(fmt(&each), lines(&["<ul>", "  <% items.each do |i| %>", "    <li><%= i %></li>", "  <% end %>", "</ul>"]));
        let case = lines(&["<% case x %>", "<% when 1 %>", "<b>1</b>", "<% else %>", "<i>2</i>", "<% end %>"]);
        assert_eq!(fmt(&case), lines(&["<% case x %>", "<% when 1 %>", "  <b>1</b>", "<% else %>", "  <i>2</i>", "<% end %>"]));
    }

    #[test]
    fn closers_at_the_start_of_a_line_dedent_it() {
        assert_eq!(fmt("<div><section>\nx\n</section></div>\n"), "<div><section>\n    x\n</section></div>\n");
        assert_eq!(fmt("<li><% if a %>\nx\n<% end %></li>"), "<li><% if a %>\n    x\n<% end %></li>");
        assert_eq!(fmt("<div>\n</div><div>\nx\n</div>"), "<div>\n</div><div>\n  x\n</div>");
        assert_eq!(fmt("<div>\nx</div>"), "<div>\n  x</div>");
    }

    #[test]
    fn void_self_closing_and_optional_end_elements() {
        assert_eq!(
            fmt("<div>\n<br>\n<img src=\"a\">\n<x-icon />\n<input/>\n</div>"),
            "<div>\n  <br>\n  <img src=\"a\">\n  <x-icon />\n  <input/>\n</div>"
        );
        assert_eq!(fmt("<ul>\n<li>\na\n</li>\n<li>b\n</ul>"), "<ul>\n  <li>\n    a\n  </li>\n  <li>b\n</ul>");
        assert_eq!(fmt("<table>\n<tr><td>1</td></tr>\n</table>"), "<table>\n  <tr><td>1</td></tr>\n</table>");
    }

    #[test]
    fn comments_and_multi_line_tags_move_with_their_first_line() {
        let text = lines(&["<div>", "      <input type=\"text\"", "             name=\"q\">", "<!-- a", "       b -->", "</div>"]);
        let want = lines(&["<div>", "  <input type=\"text\"", "         name=\"q\">", "  <!-- a", "         b -->", "</div>"]);
        assert_eq!(fmt(&text), want);
        // A value that spans lines is content.
        let title = lines(&["<div>", "    <a title=\"one", "      two\"", "      href=\"/\">x</a>", "</div>"]);
        assert_eq!(fmt(&title), lines(&["<div>", "  <a title=\"one", "      two\"", "    href=\"/\">x</a>", "</div>"]));
    }

    #[test]
    fn multi_line_erb_moves_as_a_unit_unless_it_holds_a_string() {
        let text = lines(&["<div>", "<%= render partial: \"x\",", "           locals: { a: 1 } %>", "</div>"]);
        assert_eq!(fmt(&text), lines(&["<div>", "  <%= render partial: \"x\",", "             locals: { a: 1 } %>", "</div>"]));
        for rigid in [
            lines(&["<div>", "<% sql = <<~SQL", "  select 1", "SQL", "%>", "</div>"]),
            lines(&["<div>", "<%= t(\"a", "  b\") %>", "</div>"]),
        ] {
            assert_eq!(fmt(&rigid), rigid);
        }
        let block = lines(&["<%= link_to x,", "      class: \"a\" do %>", "<span>y</span>", "<% end %>"]);
        assert_eq!(fmt(&block), lines(&["<%= link_to x,", "      class: \"a\" do %>", "  <span>y</span>", "<% end %>"]));
    }

    #[test]
    fn raw_text_and_pre() {
        let script = lines(&["<div>", "    <script>", "      if (a) {", "        b();", "      }", "    </script>", "</div>"]);
        assert_eq!(fmt(&script), lines(&["<div>", "  <script>", "    if (a) {", "      b();", "    }", "  </script>", "</div>"]));
        let shallow = lines(&["<div>", "    <style>", "a {}", "    </style>", "</div>"]);
        assert_eq!(fmt(&shallow), shallow, "a body left of its tag can't keep its offsets");
        let pre = lines(&["<div>", "    <pre>", "  a  ", "    b", "    </pre>", "  <textarea>", " x ", "</textarea>", "</div>"]);
        assert_eq!(fmt(&pre), lines(&["<div>", "  <pre>", "  a  ", "    b", "    </pre>", "  <textarea>", " x ", "</textarea>", "</div>"]));
    }

    #[test]
    fn trailing_whitespace_only_outside_content() {
        assert_eq!(fmt("<p>  \n x \t\n</p>"), "<p>\n  x\n</p>");
        assert_eq!(fmt("<% x = 1  \n  y = 2 %>  \n"), "<% x = 1  \n  y = 2 %>\n");
        assert_eq!(fmt("   \n\t\n"), "\n\n");
        assert_eq!(fmt("<p>\r\n  x  \r\n</p>\r\n"), "<p>\r\n  x\r\n</p>\r\n");
        let kept = indent("<p>  \n x\n</p>", "  ", false, false).unwrap().text;
        assert_eq!(kept, "<p>  \n  x\n</p>");
    }

    #[test]
    fn tabs() {
        assert_eq!(indent("<div>\n<p>\nx\n</p>\n</div>", "\t", true, false).unwrap().text, "<div>\n\t<p>\n\t\tx\n\t</p>\n</div>");
    }

    #[test]
    fn branches_that_disagree_freeze_the_rest() {
        let text = lines(&["<div>", "<% if a %>", "<section>", "<% end %>", "  x", "<% if a %>", "</section>", "<% end %>", "</div>"]);
        let want =
            lines(&["<div>", "  <% if a %>", "    <section>", "<% end %>", "  x", "<% if a %>", "</section>", "<% end %>", "</div>"]);
        assert_eq!(fmt(&text), want);
        let agree = lines(&["<% if a %>", "<div class=\"x\">", "<% else %>", "<div class=\"y\">", "<% end %>", "body", "</div>"]);
        let want = lines(&["<% if a %>", "  <div class=\"x\">", "<% else %>", "  <div class=\"y\">", "<% end %>", "  body", "</div>"]);
        assert_eq!(fmt(&agree), want);
    }

    #[test]
    fn partials_may_close_what_they_did_not_open() {
        assert_eq!(indent("    </main>\n  </div>\n</body>", "  ", true, true).unwrap().text, "</main>\n</div>\n</body>");
        assert_eq!(
            indent("<div class=\"wrapper\">\n<header>x</header>", "  ", true, true).unwrap().text,
            "<div class=\"wrapper\">\n  <header>x</header>"
        );
    }

    #[test]
    fn doubt_leaves_the_file_alone() {
        for broken in ["<div><% if a %>", "<div>\n<span>\n</div>", "<% end %>", "<div><% foo", "<pre>\nx", "<script>x", "<div class=\"a"] {
            assert!(indent(broken, "  ", true, false).is_err(), "{broken}");
        }
        assert!(indent("<%= javascript_tag do %>\n  x()\n<% end %>", "  ", true, false).is_err());
        assert!(indent("<p><div></p></div>", "  ", true, false).is_ok_and(|i| i.text == "<p><div></p></div>"));
    }
}
