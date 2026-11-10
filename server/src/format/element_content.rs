//! `Layout/ElementContentOnOwnLine`: the content of a listed element moves from the line of its tags to a line of its own
//! between them. It runs before the other rules, which indent the new line. It adds whitespace to the page, so the guard
//! accepts exactly that: ERB keeps every non-whitespace character and every element; Haml and Slim must read the same code
//! and the moved content must stand one level below its tag.

use super::blank_lines::dominant_ending;
use super::element_style::split_bom;
use super::inline::replaced;
use super::{same_non_whitespace, Settings};
use crate::erb::{self, Edit};
use crate::haml::parser::{self as haml, Chunk, Parsed, Piece};
use crate::lang::TemplateLang;
use crate::slim::parser as slim;
use crate::util::leading_ws;
use std::ops::Range;

const REFUSED: &str = "element content kept on its line: moving it would change more than whitespace";
/// Elements that show their whitespace as it is written.
pub(super) const SENSITIVE: &[&str] = &["pre", "textarea", "script", "style", "listing", "xmp", "plaintext"];
/// Elements in which a new line break shows as a space between words.
const INLINE: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "button", "cite", "code", "data", "dfn", "em", "i", "kbd", "label", "mark", "q", "s", "samp", "small",
    "span", "strong", "sub", "sup", "time", "u", "var",
];
const SPACE: [char; 2] = [' ', '\t'];

/// What a line is to the Haml or Slim parser, as far as moving content under a tag goes.
#[derive(Clone, Copy, PartialEq)]
enum Line {
    Blank,
    Structural(usize),
    /// Continues another line.
    Joined,
}

/// `text` with the content of the listed elements on lines of their own; `Ok(None)` when nothing changes, and a note when the
/// rewrite is refused.
pub(super) fn apply(lang: TemplateLang, text: &str, settings: &Settings, extra_engines: &[String]) -> Result<Option<String>, String> {
    let Some(tags) = &settings.content_on_own_line else { return Ok(None) };
    let moved = match lang {
        TemplateLang::Erb => erb_moved(text, tags),
        TemplateLang::Haml => haml_moved(text, tags, settings),
        TemplateLang::Slim | TemplateLang::Skim => slim_moved(text, tags, settings, extra_engines),
        _ => return Ok(None),
    };
    moved.ok_or_else(|| REFUSED.to_string())
}

fn is_listed(tags: &[String], name: &str) -> bool {
    tags.iter().any(|tag| tag.eq_ignore_ascii_case(name)) && !is_in(SENSITIVE, name)
}

fn is_in(names: &[&str], name: &str) -> bool {
    names.iter().any(|candidate| candidate.eq_ignore_ascii_case(name))
}

/// Whether text in the element around `name` shows a line break.
fn shows_line_breaks(name: &str) -> bool {
    is_in(SENSITIVE, name) || is_in(INLINE, name)
}

/// `Some(None)` when there is nothing to move, `None` when the move is refused.
fn erb_moved(text: &str, tags: &[String]) -> Option<Option<String>> {
    let elements = erb::tags::elements(text);
    let erb_tags = erb::ruby::tags(text);
    let eol = dominant_ending(text);
    let mut edits: Vec<Edit> = Vec::new();
    for element in elements.iter().filter(|element| is_listed(tags, &element.name)) {
        let (open, close) = (element.open_end, element.close_start);
        let content = &text[open..close];
        let blank_before = content.len() - content.trim_start_matches(SPACE).len();
        let blank_after = content.len() - content.trim_end_matches(SPACE).len();
        let line_start = text[..element.start].rfind('\n').map_or(0, |at| at + 1);
        let first_on_line = text[line_start..element.start].trim_matches(SPACE).is_empty();
        let on_one_line = !text[element.start..element.end].contains(['\n', '\r']);
        let inside_inline =
            elements.iter().any(|outer| outer.start < element.start && element.end <= outer.end && shows_line_breaks(&outer.name));
        let plain_tags = erb_tags
            .iter()
            .filter(|tag| tag.start >= element.start && tag.end <= element.end)
            .all(|tag| (tag.indicator == Some("=") || tag.is_comment()) && !text[tag.code.end..tag.end].starts_with('-'));
        if !first_on_line || !on_one_line || inside_inline || !plain_tags || content.trim_matches(SPACE).is_empty() {
            continue;
        }
        edits.push((open..open + blank_before, eol.to_string()));
        edits.push((close - blank_after..close, eol.to_string()));
    }
    if edits.is_empty() {
        return Some(None);
    }
    edits.sort_by_key(|(range, _)| range.start);
    let rewritten = replaced(text, &edits);
    let names = |text: &str| erb::tags::elements(text).into_iter().map(|element| element.name).collect::<Vec<_>>();
    if !(same_non_whitespace(text, &rewritten) && names(text) == names(&rewritten)) {
        return None;
    }
    // An element that follows its parent's start tag on a line is first on its own line only after the parent moved.
    Some(Some(match erb_moved(&rewritten, tags) {
        Some(Some(deeper)) => deeper,
        _ => rewritten,
    }))
}

/// A tag whose content moves: the line it is on, the line its content is on and the tag's depth.
struct Move {
    tag_line: usize,
    content_line: usize,
    depth: usize,
}

fn haml_moved(text: &str, tags: &[String], settings: &Settings) -> Option<Option<String>> {
    let (bom, body) = split_bom(text);
    let parsed = haml::parse_with_ruby(body);
    if parsed.error.is_some() {
        return Some(None);
    }
    let lines: Vec<Line> = parsed
        .roles
        .iter()
        .map(|role| match role {
            haml::Role::Blank => Line::Blank,
            haml::Role::Structural { depth } => Line::Structural(*depth),
            _ => Line::Joined,
        })
        .collect();
    let layout = Layout::new(body, &lines, settings);
    let mut open = Vec::new();
    let mut moves = Vec::new();
    let mut edits = Vec::new();
    for tag in parsed.tags.iter().filter(|tag| tag.head != (0..0)) {
        let tag_line = layout.line_of(tag.head.start);
        let Line::Structural(depth) = lines[tag_line] else { continue };
        let inside = in_line_break_context(&mut open, depth, &tag.element.name, false);
        let Some(content) = tag.element.content.clone().filter(|_| !inside && is_listed(tags, &tag.element.name)) else { continue };
        if !haml_content_moves(body, &content) {
            continue;
        }
        let content_line = layout.line_of(content.start);
        if layout.stands_alone(content_line, depth) {
            edits.push(layout.edit(body, tag_line, &content, ""));
            moves.push(Move { tag_line, content_line, depth });
        }
    }
    if edits.is_empty() {
        return Some(None);
    }
    let rewritten = replaced(body, &edits);
    haml_kept(body, &rewritten, &moves).then(|| Some(format!("{bom}{rewritten}")))
}

/// Whether Haml reads `content` on a line of its own as it does after the tag: script after its marker, text unless it starts
/// with a character that makes a line something else.
fn haml_content_moves(text: &str, content: &Range<usize>) -> bool {
    let attached = !text[..content.start].ends_with(SPACE);
    let written = &text[content.clone()];
    match written.chars().next() {
        Some('=' | '~') if attached => true,
        Some('&' | '!') if attached => matches!(written[1..].chars().next(), Some('=' | ' ')) || written[1..].starts_with("#{"),
        Some('<' | '>' | '/') if attached => false,
        Some('#') => written.starts_with("#{"),
        Some('%' | '.' | '-' | '=' | '~' | '&' | '!' | '/' | ':' | '\\' | '|' | ',') | None => false,
        Some(_) => true,
    }
}

/// The Ruby Haml generates for the tags and scripts, line by line, apart from the tags' own markup.
fn haml_pieces(parsed: &Parsed) -> Vec<String> {
    let code = |chunks: &[Chunk]| chunks.iter().map(|chunk| &*chunk.text).collect::<String>();
    parsed
        .pieces
        .iter()
        .filter_map(|piece| match piece {
            Piece::Statement(chunks) => Some(format!("-{}", code(chunks))),
            Piece::Output { code: chunks, block } => Some(format!("={block}{}", code(chunks))),
            Piece::Text(chunks) => Some(format!("#{}", code(chunks))),
            Piece::Attributes { object, arguments } => Some(format!("[{}]{}", code(object), code(arguments))),
            Piece::End(_) => Some("end".to_string()),
            Piece::Dropped => None,
        })
        .collect()
}

/// Haml reads the same tags and code, and each moved content is one level below its tag; every earlier move added a line.
fn haml_kept(before: &str, after: &str, moves: &[Move]) -> bool {
    let (before, after) = (haml::parse_with_ruby(before), haml::parse_with_ruby(after));
    if after.error.is_some() || before.tags.len() != after.tags.len() || haml_pieces(&before) != haml_pieces(&after) {
        return false;
    }
    moves.iter().enumerate().all(|(earlier, moved)| {
        let role = |line: usize| after.roles.get(line).copied();
        role(moved.tag_line + earlier) == Some(haml::Role::Structural { depth: moved.depth })
            && role(moved.content_line + earlier + 1) == Some(haml::Role::Structural { depth: moved.depth + 1 })
    })
}

fn slim_moved(text: &str, tags: &[String], settings: &Settings, extra_engines: &[String]) -> Option<Option<String>> {
    let parsed = slim::parse(text, extra_engines);
    if !parsed.valid {
        return Some(None);
    }
    let lines: Vec<Line> = parsed
        .lines
        .iter()
        .map(|info| match info.role {
            slim::Role::Blank => Line::Blank,
            slim::Role::Structural { depth } => Line::Structural(depth),
            slim::Role::Inner { .. } => Line::Joined,
        })
        .collect();
    let layout = Layout::new(text, &lines, settings);
    let mut open = Vec::new();
    let mut last_line = None;
    let mut edits = Vec::new();
    for element in &parsed.elements {
        let tag_line = layout.line_of(element.at.start);
        let Line::Structural(depth) = lines[tag_line] else { continue };
        let inside = in_line_break_context(&mut open, depth, &element.name, last_line == Some(tag_line));
        last_line = Some(tag_line);
        let Some(content) = element.content.clone().filter(|_| !inside && is_listed(tags, &element.name)) else { continue };
        let written = &text[content.clone()];
        let output = written.starts_with('=');
        if output && written.trim_start_matches('=').starts_with(['\'', '<', '>']) {
            continue;
        }
        if layout.stands_alone(layout.line_of(content.start), depth) {
            edits.push(layout.edit(text, tag_line, &content, if output { "" } else { "| " }));
        }
    }
    if edits.is_empty() {
        return Some(None);
    }
    let rewritten = replaced(text, &edits);
    crate::slim::ruby::same_compilation_ignoring_layout(text, &rewritten, extra_engines).then_some(Some(rewritten))
}

/// Whether the element at `depth` is inside one that shows line breaks; `open` holds the elements above it.
fn in_line_break_context(open: &mut Vec<(usize, String)>, depth: usize, name: &str, chained: bool) -> bool {
    // Elements chained on one line (`pre: div`) share a depth, and the earlier ones contain the later.
    if !chained {
        open.retain(|(above, _)| *above < depth);
    }
    let inside = open.iter().any(|(_, outer)| shows_line_breaks(outer));
    open.push((depth, name.to_string()));
    inside
}

/// The lines of a Haml or Slim text, and what the new line is made of: its ending and its indentation unit.
struct Layout<'a> {
    starts: Vec<usize>,
    lines: &'a [Line],
    eol: &'static str,
    unit: String,
}

impl<'a> Layout<'a> {
    fn new(text: &str, lines: &'a [Line], settings: &Settings) -> Self {
        let starts: Vec<usize> = haml::lines_with_endings(text)
            .iter()
            .scan(0, |start, (line, ending)| {
                let at = *start;
                *start += line.len() + ending.len();
                Some(at)
            })
            .collect();
        // The file's own unit: Haml and Slim take the first indentation as the one, and read another as a mistake.
        let first_level = lines.iter().position(|line| *line == Line::Structural(1));
        let unit = first_level.map_or_else(|| settings.indent_unit(), |at| leading_ws(&text[starts[at]..]).to_string());
        Layout { starts, lines, eol: dominant_ending(text), unit }
    }

    fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|&start| start <= offset) - 1
    }

    /// Whether no line below `line` belongs to it or to the tag: the next one is no deeper than the tag at `depth`.
    fn stands_alone(&self, line: usize, depth: usize) -> bool {
        match self.lines[line + 1..].iter().find(|next| **next != Line::Blank) {
            None => true,
            Some(Line::Structural(next)) => *next <= depth,
            Some(_) => false,
        }
    }

    /// The edit that puts `content` on a line below the tag's, behind `prefix`, without the blanks before it.
    fn edit(&self, text: &str, tag_line: usize, content: &Range<usize>, prefix: &str) -> Edit {
        let line = &text[self.starts[tag_line]..];
        let indent = leading_ws(line);
        let from = text[..content.start].trim_end_matches(SPACE).len();
        (from..content.end, format!("{}{indent}{}{prefix}{}", self.eol, self.unit, &text[content.clone()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{format, Style};
    use tower_lsp_server::ls_types::Uri;

    fn settings(tags: &[&str]) -> Settings {
        Settings { content_on_own_line: Some(tags.iter().map(|tag| tag.to_string()).collect()), ..Settings::default() }
    }

    fn all() -> Settings {
        settings(&["div", "section", "article", "header", "footer", "ul", "ol"])
    }

    /// The text through the whole formatter, or the text itself when the formatter leaves it.
    fn formatted(name: &str, text: &str, settings: &Settings) -> String {
        let uri = format!("file:///app/views/users/{name}").parse::<Uri>().unwrap();
        let lang = TemplateLang::from_uri_or_id(&uri, None).unwrap();
        format(&uri, lang, false, text, settings, &[]).map_or_else(|skipped| format!("skipped: {skipped}"), |done| done.text)
    }

    fn stage(lang: TemplateLang, text: &str) -> Option<String> {
        apply(lang, text, &all(), &[]).unwrap()
    }

    #[test]
    fn erb_content_moves_between_the_tags() {
        let text = "<div class=\"card\" id=\"<%= dom_id(user) %>\"><%= user.name %></div>\n";
        assert_eq!(formatted("a.html.erb", text, &all()), "<div class=\"card\" id=\"<%= dom_id(user) %>\">\n  <%= user.name %>\n</div>\n");
        assert_eq!(formatted("a.html.erb", "<div> x </div>\n", &all()), "<div>\n  x\n</div>\n");
        assert_eq!(formatted("a.html.erb", "<DIV>x</DIV>\n", &all()), "<DIV>\n  x\n</DIV>\n");
        assert_eq!(formatted("a.html.erb", "<ul><li>a</li><li>b</li></ul>\n", &all()), "<ul>\n  <li>a</li><li>b</li>\n</ul>\n");
    }

    #[test]
    fn erb_listed_elements_inside_each_other_all_move() {
        let nested = "<section><div><%= a %></div></section>\n";
        assert_eq!(formatted("a.html.erb", nested, &all()), "<section>\n  <div>\n    <%= a %>\n  </div>\n</section>\n");
        assert_eq!(formatted("a.html.erb", "<div>\n  <div>x</div>\n</div>\n", &all()), "<div>\n  <div>\n    x\n  </div>\n</div>\n");
    }

    #[test]
    fn erb_content_stays_where_a_line_break_would_show_or_the_element_is_unclear() {
        for text in [
            "<span>x</span>\n",
            "<a class=\"btn\"><%= name %></a>\n",
            "<div></div>\n",
            "<div> </div>\n",
            "<div>\n  x</div>\n",
            "<div>x\n</div>\n",
            "<pre><div>x</div></pre>\n",
            "<pre>x</pre>\n",
            "<textarea>x</textarea>\n",
            "<script>x</script>\n",
            "<style>x</style>\n",
            "<span><div>x</div></span>\n",
            "<a href=\"/\"><div>x</div></a>\n",
            "<div><% if a %>x<% end %></div>\n",
            "<div><%- a -%></div>\n",
            "<div><%= a -%></div>\n",
            "<div <% if a %>hidden<% end %>>x</div>\n",
            "<div><%% x %></div>\n",
            "<!-- <div>x</div> -->\n",
            "<div>x\n",
            "<% if a %><div>x</div><% end %>\n",
        ] {
            assert_eq!(stage(TemplateLang::Erb, text), None, "{text:?}");
        }
    }

    #[test]
    fn erb_comments_and_output_in_the_content_move() {
        assert_eq!(formatted("a.html.erb", "<div><%# note %> <%= a %></div>\n", &all()), "<div>\n  <%# note %> <%= a %>\n</div>\n");
    }

    #[test]
    fn erb_only_the_listed_names_move() {
        let text = "<div>a</div><p>b</p><section>c</section>\n";
        assert_eq!(stage(TemplateLang::Erb, text).as_deref(), Some("<div>\na\n</div><p>b</p><section>c</section>\n"));
        assert_eq!(apply(TemplateLang::Erb, text, &settings(&["p"]), &[]).unwrap().as_deref(), None);
        assert_eq!(apply(TemplateLang::Erb, text, &settings(&[]), &[]).unwrap(), None);
        assert_eq!(apply(TemplateLang::Erb, text, &settings(&["pre"]), &[]).unwrap(), None);
        assert_eq!(apply(TemplateLang::Erb, text, &Settings::default(), &[]).unwrap(), None);
    }

    #[test]
    fn erb_results_are_stable_and_keep_the_files_line_endings() {
        let text = "<div>\r\n  <section>x</section>\r\n</div>\r\n";
        let once = formatted("a.html.erb", text, &all());
        assert_eq!(once, "<div>\r\n  <section>\r\n    x\r\n  </section>\r\n</div>\r\n");
        assert_eq!(formatted("a.html.erb", &once, &all()), once);
        let tabs = Settings { indent_style: crate::format::IndentStyle::Tabs, ..all() };
        assert_eq!(formatted("a.html.erb", "<div><p>x</p></div>\n", &tabs), "<div>\n\t<p>x</p>\n</div>\n");
    }

    #[test]
    fn erb_new_lines_follow_the_blank_line_rules() {
        let around = Settings { around_element_body: Some(Style::EmptyLines), ..all() };
        assert_eq!(formatted("a.html.erb", "<div><%= a %></div>\n", &around), "<div>\n\n  <%= a %>\n\n</div>\n");
        let none = Settings { around_element_body: Some(Style::NoEmptyLines), ..all() };
        assert_eq!(formatted("a.html.erb", "<div><%= a %></div>\n", &none), "<div>\n  <%= a %>\n</div>\n");
    }

    #[test]
    fn haml_content_moves_below_its_tag() {
        for (text, want) in [
            ("%section.card#user= user.name\n", "%section.card#user\n  = user.name\n"),
            ("%div text\n", "%div\n  text\n"),
            (".card Hello\n", ".card\n  Hello\n"),
            ("%div{a: 1} text\n", "%div{a: 1}\n  text\n"),
            ("%div(a=\"1\")= x\n", "%div(a=\"1\")\n  = x\n"),
            ("%div&= x\n", "%div\n  &= x\n"),
            ("%div!= x\n", "%div\n  != x\n"),
            ("%div~ x\n", "%div\n  ~ x\n"),
            ("%div== a #{b}\n", "%div\n  == a #{b}\n"),
            ("%div #{count} items\n", "%div\n  #{count} items\n"),
            ("%div   spaced   \n", "%div\n  spaced\n"),
            ("%div\n  %header Hi\n  %p x\n", "%div\n  %header\n    Hi\n  %p x\n"),
            ("%div{a: 1,\n      b: 2} text\n", "%div{a: 1,\n  b: 2}\n  text\n"),
            ("%ul\n  %li a\n%ol 1\n", "%ul\n  %li a\n%ol\n  1\n"),
            ("%div a\n%div b\n", "%div\n  a\n%div\n  b\n"),
        ] {
            assert_eq!(formatted("a.html.haml", text, &all()), want, "{text:?}");
        }
    }

    #[test]
    fn haml_keeps_the_files_own_indentation() {
        assert_eq!(
            stage(TemplateLang::Haml, "%div\n    %p\n        %section x\n").as_deref(),
            Some("%div\n    %p\n        %section\n            x\n")
        );
        assert_eq!(stage(TemplateLang::Haml, "%div\n\t%section x\n").as_deref(), Some("%div\n\t%section\n\t\tx\n"));
        assert_eq!(stage(TemplateLang::Haml, "%div x\r\n").as_deref(), Some("%div\r\n  x\r\n"));
        let wide = Settings { indent_size: 4, ..all() };
        assert_eq!(formatted("a.html.haml", "%div\n  %section x\n", &wide), "%div\n    %section\n        x\n");
    }

    #[test]
    fn haml_content_stays_where_it_would_read_differently() {
        for text in [
            "%div> x\n",
            "%div< x\n",
            "%div/\n",
            "%div\n",
            "%div.card\n  %p x\n",
            "%p text\n",
            "%pre text\n",
            "%textarea text\n",
            "%span\n  %div x\n",
            "%pre\n  %div x\n",
            "%a{href: \"/\"}\n  %div x\n",
            "%div = x\n",
            "%div - x\n",
            "%div % x\n",
            "%div / x\n",
            "%div .x\n",
            "%div # x\n",
            "%div \\- x\n",
            "%div: x\n",
            "%div= foo do |a|\n  = a\n",
            "%div= link_to a,\n  b\n",
            "%div text |\n  more |\n",
            "%div x\n  %p y\n",
            "-# %div x\n",
            ":plain\n  %div x\n",
            "/ %div x\n",
            "%div\n  %p\n    %div x\n    %span y\n  %br\n",
        ] {
            let moved = stage(TemplateLang::Haml, text);
            let inside_p = text == "%div\n  %p\n    %div x\n    %span y\n  %br\n";
            assert!(moved.is_none() || inside_p, "{text:?} -> {moved:?}");
        }
    }

    #[test]
    fn haml_attached_bang_and_ampersand_must_be_markers() {
        for text in ["%div!x\n", "%div&x\n", "%div&!x\n", "%div&\n", "%div!\n", "%div!!!\n", "%div!!x\n"] {
            assert_eq!(stage(TemplateLang::Haml, text), None, "{text:?}");
        }
        assert_eq!(stage(TemplateLang::Haml, "%div& x\n").as_deref(), Some("%div\n  & x\n"));
        assert_eq!(stage(TemplateLang::Haml, "%div!= x\n").as_deref(), Some("%div\n  != x\n"));
    }

    #[test]
    fn slim_output_markers_with_whitespace_modifiers_stay() {
        for text in ["div=> x\n", "div=< x\n", "div=<> x\n", "div==> x\n", "div==< x\n"] {
            assert_eq!(stage(TemplateLang::Slim, text), None, "{text:?}");
        }
    }

    #[test]
    fn slim_chained_elements_keep_their_outer_ones_as_context() {
        for text in ["pre: div\n  div x\n", "span: div\n  div x\n"] {
            assert_eq!(stage(TemplateLang::Slim, text), None, "{text:?}");
        }
        assert_eq!(stage(TemplateLang::Slim, "ul: li\n  div x\n").as_deref(), Some("ul: li\n  div\n    | x\n"));
    }

    #[test]
    fn erb_elements_move_only_when_they_start_their_line() {
        for text in ["<li><div>x</div></li>\n", "<td><span></span><div>x</div></td>\n", "<%= link_to \"/\" do %><div>x</div><% end %>\n"] {
            assert_eq!(stage(TemplateLang::Erb, text), None, "{text:?}");
        }
        assert_eq!(formatted("a.html.erb", "<ul>\n  <li><div>x</div></li>\n</ul>\n", &all()), "<ul>\n  <li><div>x</div></li>\n</ul>\n");
        let nested = formatted("a.html.erb", "<div><div>x</div></div>\n", &all());
        assert_eq!(nested, "<div>\n  <div>\n    x\n  </div>\n</div>\n");
        assert_eq!(formatted("a.html.erb", &nested, &all()), nested);
    }

    #[test]
    fn haml_a_template_with_an_error_is_left_alone() {
        assert_eq!(stage(TemplateLang::Haml, "%div x\n   %p\n"), None);
        assert_eq!(stage(TemplateLang::Haml, "%div{a: 1 x\n"), None);
    }

    #[test]
    fn slim_content_moves_below_its_tag() {
        for (text, want) in [
            ("section.card#user = user.name\n", "section.card#user\n  = user.name\n"),
            ("div text\n", "div\n  | text\n"),
            (".card Hello #{name}\n", ".card\n  | Hello #{name}\n"),
            ("div class=\"a\" text\n", "div class=\"a\"\n  | text\n"),
            ("div == raw_x\n", "div\n  == raw_x\n"),
            ("div=x\n", "div\n  =x\n"),
            ("div text   \n", "div\n  | text   \n"),
            ("div\n  header Hi\n  p x\n", "div\n  header\n    | Hi\n  p x\n"),
        ] {
            assert_eq!(formatted("a.html.slim", text, &all()), want, "{text:?}");
        }
        assert_eq!(stage(TemplateLang::Slim, "div\n    ul 1\n").as_deref(), Some("div\n    ul\n        | 1\n"));
        assert_eq!(stage(TemplateLang::Slim, "div\n\tul 1\n").as_deref(), Some("div\n\tul\n\t\t| 1\n"));
    }

    #[test]
    fn slim_content_stays_where_it_would_compile_differently() {
        for text in [
            "div\n",
            "p text\n",
            "pre text\n",
            "div: span x\n",
            "li: div x\n",
            "span\n  div x\n",
            "pre\n  div x\n",
            "div=' x\n",
            "div==' x\n",
            "div text\n  more\n",
            "div = foo do\n  | x\n",
            "div = foo a,\n  b\n",
            "div class=\"a\"\n  p x\n",
            "div/\n",
            "javascript:\n  div x\n",
            "/ div x\n",
            "| div x\n",
        ] {
            assert_eq!(stage(TemplateLang::Slim, text), None, "{text:?}");
        }
    }

    #[test]
    fn the_guards_refuse_a_move_that_changes_more_than_the_layout() {
        let moves = [Move { tag_line: 0, content_line: 0, depth: 0 }];
        assert!(haml_kept("%div x\n", "%div\n  x\n", &moves));
        assert!(haml_kept("%div= x\n", "%div\n  = x\n", &moves));
        for after in ["%div\n  - x\n", "%div\n  = x\n", "%div\nx\n", "%div\n  %p x\n", "%div\n  x\n  = y\n"] {
            assert!(!haml_kept("%div x\n", after, &moves), "{after:?}");
        }
        assert!(!haml_kept("%div= x\n", "%div\n  = y\n", &moves));
        let same = |before, after| crate::slim::ruby::same_compilation_ignoring_layout(before, after, &[]);
        assert!(same("div text\n", "div\n  | text\n") && same("div = x\n", "div\n  = x\n"));
        for (before, after) in [
            ("div text\n", "div\n  | other\n"),
            ("div = x\n", "div\n  = y\n"),
            ("div text\n", "div\n  p text\n"),
            ("div = x\n", "div\n  == x\n"),
        ] {
            assert!(!same(before, after), "{after:?}");
        }
    }

    #[test]
    fn slim_results_are_stable() {
        let once = formatted("a.html.slim", "div\n  section x\n  article = y\n", &all());
        assert_eq!(once, "div\n  section\n    | x\n  article\n    = y\n");
        assert_eq!(formatted("a.html.slim", &once, &all()), once);
        let once = formatted("a.html.haml", "%div\n  %section x\n  %article= y\n", &all());
        assert_eq!(once, "%div\n  %section\n    x\n  %article\n    = y\n");
        assert_eq!(formatted("a.html.haml", &once, &all()), once);
    }
}
