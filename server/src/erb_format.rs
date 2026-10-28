//! ERB re-indentation. A line that starts in markup gets one indent unit per open HTML element and ERB block, read
//! with `tags.rs`; a line that starts inside a multi-line tag moves with the line its owner starts on, keeping its
//! relative indentation; a `<script>`, `<style>`, `<pre>`, `<textarea>` or `<title>` body (its end tag's line too), a
//! quoted attribute value, every line after the first of an HTML comment and every line that starts inside an ERB tag
//! (its Ruby) keep every byte.

use crate::checks::RubyContext;
use crate::erb_ruby;
use crate::format::{skip, Indented, SkipKind, Skipped};
use crate::ruby::BlockEffect;
use crate::tags::{self, Block, PState, Tag, Token, OPTIONAL_END, VOID};
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;
use tower_lsp::lsp_types::DiagnosticSeverity;

// A `<pre>`/`<textarea>` built by a helper: its block body keeps every space, but `tags.rs` sees no element.
static PRE_BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(tag\.(pre|textarea)|content_tag[ (]\s*:(pre|textarea))\b").unwrap());

/// Where a line starts.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    Markup,
    /// Inside content whose whitespace is significant.
    Fixed,
    /// Inside the unit with this index, which moves with its first line.
    Unit(usize),
}

/// Lines starting in `first..=last` move with the line holding `owner`.
struct UnitSpan {
    first: usize,
    last: usize,
    owner: usize,
}

struct Unit {
    owner_line: usize,
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

/// `text` with every line re-indented by `unit`, and trailing whitespace dropped.
pub(crate) fn indent(text: &str, unit: &str, partial: bool, ruby: &RubyContext) -> Result<Indented, Skipped> {
    let doubt = crate::erb::diagnostics(text, ruby)
        .into_iter()
        .find(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .or_else(|| crate::erb::html_diagnostics(text).into_iter().next())
        .or_else(|| tags::diagnostics(text, partial).into_iter().next());
    if let Some(d) = doubt {
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
    let mut unit_spans: Vec<UnitSpan> = Vec::new();
    let span = |first, last, owner| UnitSpan { first, last, owner };
    let paired = paired_optional(&tokens);
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Erb(e) => {
                let code = erb_code(&text[e.at..e.end]);
                if matches!(e.effect, BlockEffect::Open(_)) && PRE_BLOCK.is_match(code) {
                    return skip(SkipKind::Doubt, format!("line {}: a `<pre>`/`<textarea>` helper block", line_of(e.at) + 1));
                }
                fixed.push((e.at + 1, e.end - 1));
                keep_end.push((e.at + 1, e.end - 1));
            }
            Token::Open { tag, quoted, .. } | Token::Close { tag, quoted } => {
                unit_spans.push(span(tag.start + 1, tag.end - 1, tag.start));
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
                let Some((close_start, _)) = *close else {
                    return skip(SkipKind::Doubt, format!("line {}: `<{}>` is never closed", line_of(tag.start) + 1, tag.name));
                };
                keep_end.push((tag.end, close_start.saturating_sub(1)));
                unit_spans.push(span(tag.start + 1, tag.end - 1, tag.start));
                fixed.push((tag.end, close_start));
            }
            Token::Comment { start, end } => {
                // The line holding `<!--` is re-indented; the lines after it, the one with `-->` too, keep every byte.
                let Some(newline) = text[*start..*end].find('\n') else { continue };
                fixed.push((start + newline + 1, end - 1));
                keep_end.push((start + newline + 1, end - 1));
            }
            Token::Unterminated => return skip(SkipKind::Doubt, "a tag is never closed with `>`"),
        }
    }

    // A `<%%` literal is text to `tags.rs`, but the oracle keeps its bytes like any other ERB tag's.
    for tag in erb_ruby::tags(text).iter().filter(|t| t.is_literal()) {
        fixed.push((tag.start + 1, tag.end - 1));
        keep_end.push((tag.start + 1, tag.end - 1));
    }

    let mut roles = vec![Role::Markup; lines.len()];
    let mut units = Vec::new();
    // Inner units start later; sorting by start lets them win over the unit around them.
    unit_spans.sort_by_key(|u| u.owner);
    for UnitSpan { first, last, owner } in unit_spans {
        let lines_inside = lines_starting_in(&starts, first, last);
        if lines_inside.is_empty() {
            continue;
        }
        let index = units.len();
        units.push(Unit { owner_line: line_of(owner) });
        for line in lines_inside {
            roles[line] = Role::Unit(index);
        }
    }
    for &(first, last) in &fixed {
        for line in lines_starting_in(&starts, first, last) {
            roles[line] = Role::Fixed;
        }
    }
    let ends: Vec<usize> = lines.iter().map(|l| l.start + l.ws.len() + l.rest.len()).collect();
    let mut keeps_end = vec![false; lines.len()];
    for &(first, last) in &keep_end {
        lines_starting_in(&ends, first, last).for_each(|line| keeps_end[line] = true);
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
        let content = if !keeps_end[i] { line.rest.trim_end_matches([' ', '\t']) } else { line.rest };
        match &decided[i] {
            _ if blank && roles[i] == Role::Markup => {}
            Some(ws) if !content.is_empty() => out.push_str(ws),
            _ => out.push_str(line.ws),
        }
        out.push_str(content);
        out.push_str(line.cr);
    }
    Ok(Indented { text: out, notes })
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

/// The Ruby inside an ERB tag; empty for an ERB comment, whose text never reaches the output.
fn erb_code(tag: &str) -> &str {
    if tag.starts_with("<%#") {
        return "";
    }
    let inner = tag.strip_prefix("<%").unwrap_or(tag).trim_start_matches(['=', '-']);
    inner.strip_suffix("%>").unwrap_or(inner).trim_end_matches('-')
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
        Token::Unterminated => unreachable!("`indent` refuses unterminated tags"),
    }
}

/// The tag stack and open ERB blocks while reading the tokens in order.
struct Nesting<'t> {
    tokens: &'t [Token<'t>],
    paired: &'t HashSet<usize>,
    partial: bool,
    stack: Vec<Tag>,
    p: PState,
    blocks: Vec<Block>,
    /// False after an ERB block whose branches left different tags open.
    agreed: bool,
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

    /// Applies token `i`; false once an end tag doesn't match the stack.
    fn step(&mut self, i: usize) -> bool {
        match &self.tokens[i] {
            // Branches that disagree (`<% if a %><div><% end %>`) go on with the tags the last one left open: the usual
            // conditional wrapper closes them under the same condition, and a guess that's wrong ends in a mismatch.
            Token::Erb(e) => {
                self.agreed = tags::apply(e, &mut self.stack, &mut self.p, &mut self.blocks);
                true
            }
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
    let mut nesting = Nesting { tokens, paired, partial, stack: Vec::new(), p: PState::default(), blocks: Vec::new(), agreed: true };
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
            // ERB inside a tag that leads the line (`</div <%= x %>>`) belongs to it.
            let Some(gap) = text.get(at..start) else {
                if !nesting.step(next) {
                    return (out, Some(l));
                }
                next += 1;
                continue;
            };
            if start >= line_end || !gap.trim_matches([' ', '\t']).is_empty() {
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
                // An `<% end %>` whose branches disagree has no single opening to line up with.
                let aligned = block_level.filter(|_| nesting.agreed);
                level = level.min(aligned.unwrap_or(nesting.depth()));
            }
            at = end;
            next += 1;
        }
        out[l] = Some(level);
    }
    (out, None)
}

/// The new leading whitespace of each line, `None` to keep it. A unit moves only as a whole: if one of its lines
/// can't follow the first (it is indented less), the first line stays too, and so does every unit that line
/// belongs to.
fn decide(lines: &[Line], roles: &[Role], units: &[Unit], levels: &[Option<usize>], unit: &str) -> Vec<Option<String>> {
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); units.len()];
    for (i, role) in roles.iter().enumerate() {
        if let Role::Unit(u) = role {
            members[*u].push(i);
        }
    }
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
        let stuck: Vec<usize> = (0..units.len())
            .filter(|&u| {
                let owner = &lines[units[u].owner_line];
                let moved = decided[units[u].owner_line].as_deref().is_some_and(|new| new != owner.ws);
                let behind = || members[u].iter().any(|&i| !lines[i].rest.is_empty() && !lines[i].ws.starts_with(owner.ws));
                moves[u] && moved && behind()
            })
            .collect();
        if stuck.is_empty() {
            return decided;
        }
        for mut u in stuck {
            loop {
                moves[u] = false;
                match roles[units[u].owner_line] {
                    Role::Unit(outer) => u = outer,
                    _ => {
                        pinned[units[u].owner_line] = true;
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indent(text: &str, unit: &str, partial: bool) -> Result<Indented, Skipped> {
        super::indent(text, unit, partial, &RubyContext::outside_project())
    }

    fn fmt(text: &str) -> String {
        let once = indent(text, "  ", false).unwrap_or_else(|e| panic!("{e}: {text}")).text;
        assert_eq!(indent(&once, "  ", false).map(|i| i.text).as_deref(), Ok(once.as_str()), "idempotent");
        once
    }

    fn lines(list: &[&str]) -> String {
        list.join("\n")
    }

    #[test]
    fn unclosed_tag_is_the_reason_to_refuse() {
        let reason = indent("<div>\n<p>x</p>\n<% foo\n</div>", "  ", false).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(reason.contains("line 3") && reason.contains("invalid-first-character-of-tag-name"), "{reason}");
    }

    #[test]
    fn tag_ends_at_the_first_erubi_close() {
        let text = "<div>\n<% a = 1 %%>\n<% if b %>\n<p>x</p>\n<% end %>\n</div>\n";
        assert_eq!(fmt(text), "<div>\n  <% a = 1 %%>\n  <% if b %>\n    <p>x</p>\n  <% end %>\n</div>\n");
        let text = "<div>\n<% x = 1 %%>\n y %>\n</div>\n";
        assert_eq!(fmt(text), "<div>\n  <% x = 1 %%>\n  y %>\n</div>\n");
    }

    #[test]
    fn literal_tag_keeps_its_lines() {
        let text = "<div>\n<%% foo\n    bar %>\n<p>x</p>\n</div>\n";
        assert_eq!(fmt(text), "<div>\n  <%% foo\n    bar %>\n  <p>x</p>\n</div>\n");
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
        assert_eq!(fmt("<div>\n<p>x</p>\n</div <%= x %>>\n"), "<div>\n  <p>x</p>\n</div <%= x %>>\n");
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
    fn multi_line_tags_move_with_their_first_line() {
        let text = lines(&["<div>", "      <input type=\"text\"", "             name=\"q\">", "</div>"]);
        let want = lines(&["<div>", "  <input type=\"text\"", "         name=\"q\">", "</div>"]);
        assert_eq!(fmt(&text), want);
        // A value that spans lines is content.
        let title = lines(&["<div>", "    <a title=\"one", "      two\"", "      href=\"/\">x</a>", "</div>"]);
        assert_eq!(fmt(&title), lines(&["<div>", "  <a title=\"one", "      two\"", "    href=\"/\">x</a>", "</div>"]));
    }

    #[test]
    fn comment_interiors_are_content() {
        assert_eq!(fmt("<div>\n<!-- a\n  b -->\n</div>\n"), "<div>\n  <!-- a\n  b -->\n</div>\n");
        assert_eq!(fmt("<div>\n<!-- a  \n  b  \n-->  \n</div>\n"), "<div>\n  <!-- a\n  b  \n-->\n</div>\n");
        let mso = lines(&["<div>", "<!--[if mso]>", "      <table><tr><td>", "  <![endif]-->", "<p>x</p>", "</div>"]);
        assert_eq!(fmt(&mso), lines(&["<div>", "  <!--[if mso]>", "      <table><tr><td>", "  <![endif]-->", "  <p>x</p>", "</div>"]));
    }

    #[test]
    fn multi_line_erb_keeps_its_ruby() {
        let text = lines(&["<div>", "<%= render partial: \"x\",", "           locals: { a: 1 } %>", "</div>"]);
        assert_eq!(fmt(&text), lines(&["<div>", "  <%= render partial: \"x\",", "           locals: { a: 1 } %>", "</div>"]));
        let hash = lines(&["<div>", "<%= render \"x\", {", "    id: 1,", "  } %>", "</div>"]);
        assert_eq!(fmt(&hash), lines(&["<div>", "  <%= render \"x\", {", "    id: 1,", "  } %>", "</div>"]));
        let cond = lines(&["<div>", "<% if a &&", "      b %>", "x", "<% end %>", "</div>"]);
        assert_eq!(fmt(&cond), lines(&["<div>", "  <% if a &&", "      b %>", "    x", "  <% end %>", "</div>"]));
        for rigid in [
            lines(&["<div>", "<% sql = <<~SQL", "  select 1", "SQL", "%>", "</div>"]),
            lines(&["<div>", "<%= t(\"a", "  b\") %>", "</div>"]),
        ] {
            assert_eq!(fmt(&rigid), rigid.replacen("\n<%", "\n  <%", 1));
        }
        for literal in ["t %(a\n  b)", "x = `a\n  b`", "x = /a\n  b/", "x 1 # note\n  \"a\n  b\""] {
            let text = format!("<div>\n<%= {literal} %>\n</div>");
            assert_eq!(fmt(&text), text.replacen("\n<%", "\n  <%", 1), "{literal}");
        }
        for literal in ["\"a\n b\".scan /a\n b/", "x\n=begin\nnote\n=end\n", "case y\nwhen /a\n b/ then 1 end"] {
            let text = format!("<div>\n<%= {literal} %>\n</div>");
            assert_eq!(fmt(&text), text.replacen("\n<%", "\n  <%", 1), "{literal}");
        }
        assert_eq!(fmt("<div>\n<%= a / b +\n  c %>\n</div>"), "<div>\n  <%= a / b +\n  c %>\n</div>");
        assert_eq!(fmt("<div>\n<%# a\n  b %>\n</div>"), "<div>\n  <%# a\n  b %>\n</div>", "comment text is kept");
        let block = lines(&["<%= link_to x,", "      class: \"a\" do %>", "<span>y</span>", "<% end %>"]);
        assert_eq!(fmt(&block), lines(&["<%= link_to x,", "      class: \"a\" do %>", "  <span>y</span>", "<% end %>"]));
    }

    #[test]
    fn raw_text_and_pre() {
        let script = lines(&["<div>", "    <script>", "      if (a) {", "        b();", "      }", "    </script>", "</div>"]);
        assert_eq!(fmt(&script), lines(&["<div>", "  <script>", "      if (a) {", "        b();", "      }", "    </script>", "</div>"]));
        let style = lines(&["<div>", "<style>", "    a {}", "</style>", "</div>"]);
        assert_eq!(fmt(&style), lines(&["<div>", "  <style>", "    a {}", "</style>", "</div>"]));
        let nested = lines(&["<div>", "<script>", "  var a = <%= raw \"x", "  y\" %>;", "</script>", "</div>"]);
        assert_eq!(fmt(&nested), lines(&["<div>", "  <script>", "  var a = <%= raw \"x", "  y\" %>;", "</script>", "</div>"]));
        let pre = lines(&["<div>", "    <pre>", "  a  ", "    b", "    </pre>", "  <textarea>", " x ", "</textarea>", "</div>"]);
        assert_eq!(fmt(&pre), lines(&["<div>", "  <pre>", "  a  ", "    b", "    </pre>", "  <textarea>", " x ", "</textarea>", "</div>"]));
    }

    #[test]
    fn trailing_whitespace_only_outside_content() {
        assert_eq!(fmt("<p>  \n x \t\n</p>"), "<p>\n  x\n</p>");
        assert_eq!(fmt("<% x = 1  \n  y = 2 %>  \n"), "<% x = 1  \n  y = 2 %>\n");
        assert_eq!(fmt("   \n\t\n"), "\n\n");
        assert_eq!(fmt("<p>\r\n  x  \r\n</p>\r\n"), "<p>\r\n  x\r\n</p>\r\n");
    }

    #[test]
    fn tabs() {
        assert_eq!(indent("<div>\n<p>\nx\n</p>\n</div>", "\t", false).unwrap().text, "<div>\n\t<p>\n\t\tx\n\t</p>\n</div>");
    }

    #[test]
    fn branches_that_disagree_keep_the_tags_the_last_one_left_open() {
        let text = lines(&["<div>", "<% if a %>", "<section>", "<% end %>", "x", "<% if a %>", "</section>", "<% end %>", "</div>"]);
        let want = lines(&[
            "<div>",
            "  <% if a %>",
            "    <section>",
            "    <% end %>",
            "    x",
            "    <% if a %>",
            "    </section>",
            "  <% end %>",
            "</div>",
        ]);
        assert_eq!(fmt(&text), want);
        // A guess the markup contradicts keeps the indentation from there on.
        let text = lines(&["<div>", "<% if a %>", "<section>", "<% else %>", "<article>", "<% end %>", "x", "</section>", "</div>"]);
        let want = lines(&[
            "<div>",
            "  <% if a %>",
            "    <section>",
            "  <% else %>",
            "    <article>",
            "    <% end %>",
            "    x",
            "</section>",
            "</div>",
        ]);
        assert_eq!(fmt(&text), want);
        let agree = lines(&["<% if a %>", "<div class=\"x\">", "<% else %>", "<div class=\"y\">", "<% end %>", "body", "</div>"]);
        let want = lines(&["<% if a %>", "  <div class=\"x\">", "<% else %>", "  <div class=\"y\">", "<% end %>", "  body", "</div>"]);
        assert_eq!(fmt(&agree), want);
    }

    #[test]
    fn partials_may_close_what_they_did_not_open() {
        assert_eq!(indent("    </main>\n  </div>\n</body>", "  ", true).unwrap().text, "</main>\n</div>\n</body>");
        assert_eq!(
            indent("<div class=\"wrapper\">\n<header>x</header>", "  ", true).unwrap().text,
            "<div class=\"wrapper\">\n  <header>x</header>"
        );
    }

    #[test]
    fn doubt_leaves_the_file_alone() {
        for broken in [
            "<div><% if a %>",
            "<div>\n<span>\n</div>",
            "<% end %>",
            "<div><% foo",
            "<pre>\nx",
            "<script>x",
            "<div class=\"a",
            "<p><div></p></div>",
        ] {
            assert!(indent(broken, "  ", false).is_err(), "{broken}");
        }
        for helper in ["<%= javascript_tag do %>", "<%= content_tag :pre do %>", "<%= tag.textarea(name: \"a\") do %>"] {
            assert!(indent(&format!("<div>\n{helper}\na\n  b\n<% end %>\n</div>"), "  ", false).is_err(), "{helper}");
        }
    }
}
