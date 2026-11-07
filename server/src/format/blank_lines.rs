//! The blank-line rules, run on the re-indented text: `Layout/EmptyLines`, `Layout/TrailingEmptyLines` and the ones that
//! want none or one blank line around the body of a block or element and around `else`/`when`. The formatter says per
//! line what it is and whether a blank line may go there; the rest (a filter body, a text block, a `<pre>`) keeps
//! every line and gets none.

use super::{Settings, Style};
use crate::haml::parser::block_keyword;
use crate::ruby::CONTINUERS;
use crate::util::BOM;

const SPACE: [char; 4] = [' ', '\t', '\r', '\n'];

/// What a line opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Body {
    /// A Ruby block: `<% if %>`, `- x.each do`.
    Block,
    /// An HTML element.
    Element,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LineRole {
    #[default]
    Other,
    /// Its body runs until the next line that is no deeper.
    Opens(Body),
    /// `else`, `elsif`, `when`, `in`, `rescue`, `ensure`.
    Branch,
}

/// What the formatter knows about one line of its output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LineInfo {
    /// A blank line may be removed. False where whitespace counts: filter and text blocks, `<pre>`, continuations of a
    /// tag or of Ruby.
    pub removable: bool,
    /// The nesting of a line that starts a node where a blank line may be added or removed before it.
    pub depth: Option<usize>,
    pub role: LineRole,
    /// The line leads with a closing tag or `<% end %>`, which ends the body it follows. Haml and Slim have no such line.
    pub closes: bool,
    /// How many of the lines after this one continue it (a tag or Ruby that goes on), so a body starts after them.
    pub tail: usize,
}

/// The role of the Ruby after `-` in Haml and Slim.
pub(crate) fn code_role(code: &str) -> LineRole {
    if block_keyword(code).is_some_and(|keyword| CONTINUERS.contains(&keyword)) {
        LineRole::Branch
    } else {
        LineRole::Opens(Body::Block)
    }
}

/// Haml and Slim lines open a body only when the next node is nested deeper. A Ruby line followed by a branch at its own
/// depth (`- if a` over `- else`) stays an opener, with an empty body.
pub(crate) fn drop_childless_openers(lines: &mut [LineInfo]) {
    let mut next: Option<(usize, LineRole)> = None;
    for line in lines.iter_mut().rev() {
        let Some(depth) = line.depth else { continue };
        let empty_block = line.role == LineRole::Opens(Body::Block) && next == Some((depth, LineRole::Branch));
        if matches!(line.role, LineRole::Opens(_)) && !empty_block && next.is_none_or(|(next, _)| next <= depth) {
            line.role = LineRole::Other;
        }
        next = Some((depth, line.role));
    }
}

/// `text` with the blank lines the rules want around bodies and branches, then runs of blank lines cut to
/// `settings.max_empty_lines` (so `max` wins over a rule that wants one), and the end of the file cut to one line ending.
/// A blank line is only removed or added where `lines` says so. `original` sets the line ending to add.
pub(crate) fn apply(text: &str, lines: &[LineInfo], settings: &Settings, original: &str) -> String {
    let body = text.strip_prefix(BOM).unwrap_or(text);
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..text.len() - body.len()]);
    let body_start = out.len();
    let rows: Vec<&str> = body.split_inclusive('\n').collect();
    let wanted = wanted_blank_lines(&rows, lines, settings);
    let ending = dominant_ending(original);
    let info = |row: usize| lines.get(row).copied().unwrap_or_default();
    let mut run = 0;
    let mut keep = |out: &mut String, line: &str, removable: bool| {
        run = if removable { run + 1 } else { 0 };
        if settings.max_empty_lines.is_none_or(|max| run <= max) {
            out.push_str(line);
        }
    };
    let mut at = 0;
    while at < rows.len() {
        let next = (at..rows.len()).find(|&row| !is_blank(rows[row])).unwrap_or(rows.len());
        let blanks = (at..next).map(|row| (rows[row], info(row).removable));
        match wanted.get(next).copied().flatten() {
            Some(count) => {
                let existing = blanks.map(|(line, _)| line).chain(std::iter::repeat(ending));
                existing.take(count).for_each(|line| keep(&mut out, line, true));
            }
            None => blanks.for_each(|(line, removable)| keep(&mut out, line, removable)),
        }
        if let Some(row) = rows.get(next) {
            keep(&mut out, row, false);
        }
        at = next + 1;
    }
    if settings.trailing_empty_lines {
        end_with_one_line_ending(&mut out, body_start, original);
    }
    out
}

/// For each row: how many blank lines the rules want right before it, `None` when no rule speaks. The rows in front of it
/// are all blank lines that may go.
fn wanted_blank_lines(rows: &[&str], lines: &[LineInfo], settings: &Settings) -> Vec<Option<usize>> {
    let info = |row: usize| lines.get(row).copied().unwrap_or_default();
    let count = |style: Option<Style>| style.map(|style| usize::from(style == Style::EmptyLines));
    let body_style = |body| match body {
        Body::Block => settings.around_block_body,
        Body::Element => settings.around_element_body,
    };
    let mut wanted = vec![None; rows.len()];
    // The openers whose body is still going, outermost first.
    let mut open: Vec<(usize, Body)> = Vec::new();
    // The last row with content, and the last row with a depth together with where its own lines end.
    let mut last: Option<usize> = None;
    let mut head: Option<(usize, usize)> = None;
    for (row, text) in rows.iter().enumerate() {
        if is_blank(text) {
            continue;
        }
        let Some(depth) = info(row).depth else {
            last = Some(row);
            continue;
        };
        let (role, closes) = (info(row).role, info(row).closes);
        let separated = last.is_some_and(|last| (last + 1..row).all(|blank| info(blank).removable));
        // What stands right above the blank lines, if it is a line of its own and not the middle of something.
        let above = head.filter(|&(_, end)| last == Some(end)).map(|(above, _)| info(above)).unwrap_or_default();
        wanted[row] = separated.then_some(above).and_then(|above| {
            let style = if role == LineRole::Branch || above.role == LineRole::Branch {
                count(settings.around_branches)
            } else if let LineRole::Opens(body) = above.role {
                count(body_style(body))
            } else if closes {
                open.iter().rev().take_while(|&&(opener, _)| opener >= depth).find_map(|&(_, body)| count(body_style(body)))
            } else {
                None
            };
            // Nothing in the body to separate from the tags: none stays none, and one is not added.
            let empty = matches!(above.role, LineRole::Opens(_) | LineRole::Branch) && above.depth.is_some_and(|opener| depth <= opener);
            style.filter(|&count| count == 0 || !empty)
        });
        let ended = open.iter().rev().take_while(|&&(opener, _)| opener >= depth).map(|&(_, body)| body).next();
        open.retain(|&(opener, _)| opener < depth);
        match (role, ended) {
            (LineRole::Opens(body), _) => open.push((depth, body)),
            // A branch's body ends the way the block's does.
            (LineRole::Branch, Some(body)) => open.push((depth, body)),
            _ => {}
        }
        head = Some((row, row + info(row).tail));
        last = Some(row);
    }
    wanted
}

fn is_blank(line: &str) -> bool {
    line.trim_matches(SPACE).is_empty()
}

/// Drops the blank lines after the last line with content, which then ends with a line ending; `from` is where the text starts.
fn end_with_one_line_ending(text: &mut String, from: usize, original: &str) {
    let mut at = from;
    let mut end = from;
    for line in text[from..].split_inclusive('\n') {
        at += line.len();
        if !is_blank(line) {
            end = at;
        }
    }
    text.truncate(end);
    if end > from && !text.ends_with(['\n', '\r']) {
        text.push_str(dominant_ending(original));
    }
}

fn dominant_ending(text: &str) -> &'static str {
    let lf = text.matches('\n').count();
    let crlf = text.matches("\r\n").count();
    if crlf * 2 > lf {
        "\r\n"
    } else {
        "\n"
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ruby::strict_locals::RubyContext;

    pub(crate) fn removable(count: usize) -> Vec<LineInfo> {
        vec![LineInfo { removable: true, ..LineInfo::default() }; count]
    }

    fn run(text: &str, max: Option<usize>, trailing: bool) -> String {
        let settings = Settings { max_empty_lines: max, trailing_empty_lines: trailing, ..Settings::default() };
        apply(text, &removable(text.split('\n').count()), &settings, text)
    }

    const NO: Option<Style> = Some(Style::NoEmptyLines);
    const ONE: Option<Style> = Some(Style::EmptyLines);

    /// `text` through its language's re-indenter, which must leave it as it is, then the blank-line pass.
    fn tidy(lang: &str, text: &str, settings: &Settings) -> String {
        let indented = match lang {
            "erb" => crate::format::erb::indent(text, &Settings::default(), false, &RubyContext::outside_project()).ok(),
            "haml" => crate::format::haml::format(text, &Settings::default()),
            _ => crate::format::slim::format(text, &Settings::default(), &[]),
        };
        let indented = indented.unwrap_or_else(|| panic!("{lang} refused {text:?}"));
        assert_eq!(indented.text, text, "{lang}: the input should be indented already");
        apply(&indented.text, &indented.lines, settings, text)
    }

    fn only(rule: fn(&mut Settings) -> &mut Option<Style>, style: Option<Style>) -> Settings {
        let mut settings = Settings { trailing_empty_lines: false, ..Settings::default() };
        *rule(&mut settings) = style;
        settings
    }

    fn block(style: Option<Style>) -> Settings {
        only(|s| &mut s.around_block_body, style)
    }

    fn element(style: Option<Style>) -> Settings {
        only(|s| &mut s.around_element_body, style)
    }

    fn branches(style: Option<Style>) -> Settings {
        only(|s| &mut s.around_branches, style)
    }

    /// `input` becomes `no` under `no_empty_lines`, `one` under `empty_lines`, and neither changes when run again.
    fn both_styles(lang: &str, rule: fn(Option<Style>) -> Settings, input: &str, no: &str, one: &str) {
        assert_eq!(tidy(lang, input, &rule(None)), input, "{lang}: off");
        assert_eq!(tidy(lang, input, &rule(NO)), no, "{lang}: no_empty_lines");
        assert_eq!(tidy(lang, input, &rule(ONE)), one, "{lang}: empty_lines");
        assert_eq!(tidy(lang, no, &rule(NO)), no, "{lang}: no_empty_lines twice");
        assert_eq!(tidy(lang, one, &rule(ONE)), one, "{lang}: empty_lines twice");
    }

    #[test]
    fn block_bodies_in_both_styles_nested() {
        both_styles(
            "erb",
            block,
            "<% if a %>\n\n\n  <% b.each do |x| %>\n    <p><%= x %></p>\n\n  <% end %>\n\n\n<% end %>\n\n\n<p>x</p>\n",
            "<% if a %>\n  <% b.each do |x| %>\n    <p><%= x %></p>\n  <% end %>\n<% end %>\n\n\n<p>x</p>\n",
            "<% if a %>\n\n  <% b.each do |x| %>\n\n    <p><%= x %></p>\n\n  <% end %>\n\n<% end %>\n\n\n<p>x</p>\n",
        );
        both_styles(
            "haml",
            block,
            "- if a\n\n\n  - b.each do |x|\n    %p= x\n\n  %i\n\n\n%footer\n",
            "- if a\n  - b.each do |x|\n    %p= x\n\n  %i\n\n\n%footer\n",
            "- if a\n\n  - b.each do |x|\n\n    %p= x\n\n  %i\n\n\n%footer\n",
        );
        both_styles(
            "slim",
            block,
            "- if a\n\n\n  - b.each do |x|\n    p = x\n\n  i\n\n\nfooter\n",
            "- if a\n  - b.each do |x|\n    p = x\n\n  i\n\n\nfooter\n",
            "- if a\n\n  - b.each do |x|\n\n    p = x\n\n  i\n\n\nfooter\n",
        );
    }

    #[test]
    fn element_bodies_in_both_styles_nested() {
        both_styles(
            "erb",
            element,
            "<section>\n\n  <h2>x</h2>\n\n  <ul>\n    <li>a</li>\n\n  </ul>\n\n\n</section>\n\n<p>y</p>\n",
            "<section>\n  <h2>x</h2>\n\n  <ul>\n    <li>a</li>\n  </ul>\n</section>\n\n<p>y</p>\n",
            "<section>\n\n  <h2>x</h2>\n\n  <ul>\n\n    <li>a</li>\n\n  </ul>\n\n</section>\n\n<p>y</p>\n",
        );
        both_styles(
            "haml",
            element,
            "%section\n\n  %h2 x\n\n  %ul\n    %li a\n\n\n%p y\n",
            "%section\n  %h2 x\n\n  %ul\n    %li a\n\n\n%p y\n",
            "%section\n\n  %h2 x\n\n  %ul\n\n    %li a\n\n\n%p y\n",
        );
        both_styles(
            "slim",
            element,
            "section\n\n  h2 x\n\n  ul\n    li a\n\n\np y\n",
            "section\n  h2 x\n\n  ul\n    li a\n\n\np y\n",
            "section\n\n  h2 x\n\n  ul\n\n    li a\n\n\np y\n",
        );
    }

    #[test]
    fn branches_in_both_styles() {
        both_styles(
            "erb",
            branches,
            "<% if a %>\n  <p>1</p>\n\n\n<% elsif b %>\n\n  <p>2</p>\n<% else %>\n  <p>3</p>\n<% end %>\n",
            "<% if a %>\n  <p>1</p>\n<% elsif b %>\n  <p>2</p>\n<% else %>\n  <p>3</p>\n<% end %>\n",
            "<% if a %>\n  <p>1</p>\n\n<% elsif b %>\n\n  <p>2</p>\n\n<% else %>\n\n  <p>3</p>\n<% end %>\n",
        );
        both_styles(
            "erb",
            branches,
            "<% case a %>\n<% when 1 %>\n\n  <p>1</p>\n\n<% when 2 %>\n  <p>2</p>\n<% end %>\n",
            "<% case a %>\n<% when 1 %>\n  <p>1</p>\n<% when 2 %>\n  <p>2</p>\n<% end %>\n",
            "<% case a %>\n<% when 1 %>\n\n  <p>1</p>\n\n<% when 2 %>\n\n  <p>2</p>\n<% end %>\n",
        );
        both_styles(
            "haml",
            branches,
            "- if a\n  %p 1\n\n\n- elsif b\n\n  %p 2\n- else\n  %p 3\n",
            "- if a\n  %p 1\n- elsif b\n  %p 2\n- else\n  %p 3\n",
            "- if a\n  %p 1\n\n- elsif b\n\n  %p 2\n\n- else\n\n  %p 3\n",
        );
        both_styles(
            "slim",
            branches,
            "- if a\n  p 1\n\n\n- elsif b\n\n  p 2\n- else\n  p 3\n",
            "- if a\n  p 1\n- elsif b\n  p 2\n- else\n  p 3\n",
            "- if a\n  p 1\n\n- elsif b\n\n  p 2\n\n- else\n\n  p 3\n",
        );
    }

    #[test]
    fn a_branch_ends_its_body_like_the_block_it_continues() {
        let settings = Settings { around_block_body: ONE, around_branches: NO, trailing_empty_lines: false, ..Settings::default() };
        let erb = "<% if a %>\n  <p>1</p>\n\n<% else %>\n\n  <p>2</p>\n<% end %>\n";
        assert_eq!(tidy("erb", erb, &settings), "<% if a %>\n\n  <p>1</p>\n<% else %>\n  <p>2</p>\n\n<% end %>\n");
        let haml = "- if a\n  %p 1\n\n- else\n\n  %p 2\n%p z\n";
        assert_eq!(tidy("haml", haml, &settings), "- if a\n\n  %p 1\n- else\n  %p 2\n%p z\n");
    }

    #[test]
    fn an_empty_body_loses_its_blank_lines_and_never_gains_one() {
        both_styles("erb", element, "<div>\n\n\n</div>\n", "<div>\n</div>\n", "<div>\n\n\n</div>\n");
        both_styles("erb", element, "<div>\n</div>\n", "<div>\n</div>\n", "<div>\n</div>\n");
        both_styles("erb", block, "<% if a %>\n\n<% end %>\n", "<% if a %>\n<% end %>\n", "<% if a %>\n\n<% end %>\n");
        // Haml and Slim have no empty element: the next line at the same depth is a sibling.
        both_styles("haml", element, "%div\n\n%p\n", "%div\n\n%p\n", "%div\n\n%p\n");
        both_styles("slim", block, "- a\n\np\n", "- a\n\np\n", "- a\n\np\n");
    }

    #[test]
    fn a_body_at_the_end_of_the_file_has_no_closing_line_to_separate_from() {
        both_styles("haml", element, "%div\n  %p\n", "%div\n  %p\n", "%div\n\n  %p\n");
        both_styles("slim", element, "div\n  p\n\n\n", "div\n  p\n\n\n", "div\n\n  p\n\n\n");
        both_styles("erb", element, "<div>\n  <p>x</p>\n</div>", "<div>\n  <p>x</p>\n</div>", "<div>\n\n  <p>x</p>\n\n</div>");
    }

    #[test]
    fn added_lines_use_the_files_line_ending() {
        assert_eq!(tidy("erb", "<div>\r\n  <p>x</p>\r\n</div>\r\n", &element(ONE)), "<div>\r\n\r\n  <p>x</p>\r\n\r\n</div>\r\n");
        assert_eq!(tidy("haml", "%div\r\n  %p x\r\n%i\r\n", &element(ONE)), "%div\r\n\r\n  %p x\r\n%i\r\n");
        assert_eq!(tidy("slim", "div\r\n\r\n\r\n  p x\r\ni\r\n", &element(NO)), "div\r\n  p x\r\ni\r\n");
        assert_eq!(tidy("slim", "div\n  p x\ni\n", &element(ONE)), "div\n\n  p x\ni\n");
    }

    #[test]
    fn whitespace_stays_where_it_counts() {
        let erb = "<div>\n  <pre>\n\n  a\n\n</pre>\n  <script>\n\n  x\n\n  </script>\n  <p title=\"a\n\nb\">c</p>\n</div>\n";
        assert_eq!(tidy("erb", erb, &element(NO)), erb);
        assert_eq!(
            tidy("erb", erb, &element(ONE)),
            "<div>\n\n  <pre>\n\n  a\n\n</pre>\n  <script>\n\n  x\n\n  </script>\n  <p title=\"a\n\nb\">c</p>\n\n</div>\n"
        );
        let haml = "%div\n  :plain\n    a\n\n    b\n  %p x\n";
        assert_eq!(tidy("haml", haml, &element(NO)), haml);
        assert_eq!(tidy("haml", haml, &element(ONE)), "%div\n\n  :plain\n    a\n\n    b\n  %p x\n");
        let slim = "div\n  | a\n\n    b\n  p x\n  javascript:\n    x\n\n    y\n";
        assert_eq!(tidy("slim", slim, &element(NO)), slim);
        assert_eq!(tidy("slim", slim, &element(ONE)), "div\n\n  | a\n\n    b\n  p x\n  javascript:\n    x\n\n    y\n");
        // Nothing closes a Slim element: the blank lines before the next line are the file's.
        assert_eq!(tidy("slim", "div\n  | a\n\n\np\n", &element(NO)), "div\n  | a\n\n\np\n");
        assert_eq!(tidy("slim", "div\n  | a\np\n", &element(ONE)), "div\n\n  | a\np\n");
    }

    #[test]
    fn haml_and_slim_never_touch_the_blank_lines_before_a_dedent() {
        for (lang, text) in [("haml", ".card\n  %p a\n\n.next\n"), ("slim", ".card\n  p a\n\n.next\n"), ("slim", "- if a\n  p a\n\np b\n")]
        {
            for settings in [block(NO), element(NO), branches(NO)] {
                assert_eq!(tidy(lang, text, &settings), text, "{lang}: {text:?}");
            }
        }
        let text = ".card\n  %p a\n.next\n";
        assert_eq!(tidy("haml", text, &element(ONE)), ".card\n\n  %p a\n.next\n", "nothing is added before the sibling");
    }

    #[test]
    fn an_empty_branch_body_is_not_given_a_blank_line() {
        for (lang, text) in [
            ("erb", "<% if a %>\n<% else %>\n<% end %>\n"),
            ("erb", "<% case a %>\n<% when 1 %>\n<% when 2 %>\n<% end %>\n"),
            ("haml", "- if a\n- else\n%p z\n"),
            ("haml", "- case a\n- when 1\n- when 2\n%p z\n"),
            ("slim", "- if a\n- else\np z\n"),
        ] {
            assert_eq!(tidy(lang, text, &branches(ONE)), text, "{lang}: {text:?}");
        }
        // Blank lines there go with no_empty_lines, and one that is there stays with empty_lines.
        for (lang, spaced, none) in [
            ("erb", "<% if a %>\n\n<% else %>\n\n\n<% end %>\n", "<% if a %>\n<% else %>\n<% end %>\n"),
            ("haml", "- if a\n\n- else\n%p z\n", "- if a\n- else\n%p z\n"),
            ("slim", "- if a\n\n- else\n\np z\n", "- if a\n- else\np z\n"),
        ] {
            assert_eq!(tidy(lang, spaced, &branches(NO)), none, "{lang}");
        }
        assert_eq!(tidy("haml", "- if a\n\n- else\n%p z\n", &branches(ONE)), "- if a\n\n- else\n%p z\n");
    }

    #[test]
    fn a_body_starts_after_a_tag_or_code_that_spans_lines() {
        let erb = "<div\n  class=\"a\">\n\n\n  <p>x</p>\n</div>\n";
        assert_eq!(tidy("erb", erb, &element(NO)), "<div\n  class=\"a\">\n  <p>x</p>\n</div>\n");
        assert_eq!(
            tidy("erb", "<div\n  class=\"a\">\n  <p>x</p>\n</div>\n", &element(ONE)),
            "<div\n  class=\"a\">\n\n  <p>x</p>\n\n</div>\n"
        );
        let ruby = "<% if a &&\nb %>\n\n  <p>x</p>\n<% end %>\n";
        assert_eq!(tidy("erb", ruby, &block(NO)), "<% if a &&\nb %>\n  <p>x</p>\n<% end %>\n");
        assert_eq!(tidy("erb", "<% if a &&\nb %>\n  <p>x</p>\n<% end %>\n", &block(ONE)), "<% if a &&\nb %>\n\n  <p>x</p>\n\n<% end %>\n");
        assert_eq!(tidy("haml", "%div{a: 1,\n  b: 2}\n\n  %p x\n", &element(NO)), "%div{a: 1,\n  b: 2}\n  %p x\n");
        assert_eq!(tidy("haml", "%div{a: 1,\n  b: 2}\n  %p x\n", &element(ONE)), "%div{a: 1,\n  b: 2}\n\n  %p x\n");
        assert_eq!(tidy("slim", "div class=a,\n  b\n\n  p x\n", &element(NO)), "div class=a,\n  b\n  p x\n");
        // A tag that opens and closes on its own lines is no opener.
        let closed = "<div>\n  <p title=\"a\n\nb\">c</p>\n</div>\n";
        assert_eq!(tidy("erb", closed, &element(ONE)), "<div>\n\n  <p title=\"a\n\nb\">c</p>\n\n</div>\n");
    }

    #[test]
    fn max_applies_after_the_rules() {
        let with_max = |mut settings: Settings, max| {
            settings.max_empty_lines = Some(max);
            settings
        };
        let text = "<div>\n  <p>x</p>\n</div>\n";
        assert_eq!(tidy("erb", text, &with_max(element(ONE), 0)), text, "max = 0 removes what empty_lines adds");
        assert_eq!(tidy("erb", text, &with_max(element(ONE), 1)), "<div>\n\n  <p>x</p>\n\n</div>\n");
        let many = "<div>\n\n\n\n  <p>x</p>\n\n\n</div>\n";
        assert_eq!(tidy("erb", many, &with_max(element(ONE), 3)), "<div>\n\n  <p>x</p>\n\n</div>\n", "exactly one, whatever max allows");
        assert_eq!(tidy("erb", many, &with_max(element(NO), 1)), "<div>\n  <p>x</p>\n</div>\n");
        let elsewhere = "<div>\n  <p>x</p>\n\n\n\n  <p>y</p>\n</div>\n";
        assert_eq!(tidy("erb", elsewhere, &with_max(element(ONE), 1)), "<div>\n\n  <p>x</p>\n\n  <p>y</p>\n\n</div>\n");
        assert_eq!(tidy("haml", "%div\n\n\n  %p x\n", &with_max(element(ONE), 0)), "%div\n  %p x\n");
    }

    #[test]
    fn runs_are_cut_to_max_at_the_start_in_the_middle_and_at_the_end() {
        let text = "\n\n\na\n\n\n\nb\n\n\n";
        assert_eq!(run(text, Some(0), false), "a\nb\n");
        assert_eq!(run(text, Some(1), false), "\na\n\nb\n\n");
        assert_eq!(run(text, Some(2), false), "\n\na\n\n\nb\n\n\n");
        assert_eq!(run(text, None, false), text);
        assert_eq!(run(text, Some(1), true), "\na\n\nb\n");
    }

    fn mask(removable: &[bool]) -> Vec<LineInfo> {
        removable.iter().map(|&removable| LineInfo { removable, ..LineInfo::default() }).collect()
    }

    #[test]
    fn only_removable_lines_are_cut() {
        let settings = Settings { max_empty_lines: Some(0), trailing_empty_lines: false, ..Settings::default() };
        assert_eq!(apply("a\n\n\nb\n\n", &mask(&[true, false, true, true, true, true]), &settings, ""), "a\n\nb\n");
        assert_eq!(apply("a\n\n", &[], &settings, ""), "a\n\n");
    }

    #[test]
    fn whitespace_only_lines_count_as_blank_and_endings_stay() {
        assert_eq!(run("a\r\n\r\n \t\r\n\r\nb\r\n", Some(1), false), "a\r\n\r\nb\r\n");
        assert_eq!(run("\u{feff}\n\n\na\n", Some(1), false), "\u{feff}\na\n");
    }

    #[test]
    fn the_end_of_the_file_is_one_line_ending() {
        assert_eq!(run("a\n\n\n", None, true), "a\n");
        assert_eq!(run("a", None, true), "a\n");
        assert_eq!(run("a\r\nb", None, true), "a\r\nb\r\n");
        assert_eq!(run("a  \n \n  ", None, true), "a  \n");
        assert_eq!(run("a\r", None, true), "a\r");
        assert_eq!(run("\n\n", None, true), "");
        assert_eq!(run("\u{feff}\n", None, true), "\u{feff}");
        assert_eq!(run("a\n\n\n", None, false), "a\n\n\n");
    }
}
