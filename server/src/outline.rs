//! Folding ranges and document symbols for ERB templates, and what the Haml and Slim outlines share.

pub mod haml;
pub mod slim;

use crate::erb::tags;
use crate::rails::render_refs::render_refs;
use crate::ruby;
use crate::ruby::BlockEffect;
use crate::util::LineIndex;
use regex::Regex;
use std::sync::LazyLock;
use tower_lsp_server::ls_types::{DocumentSymbol, FoldingRange, FoldingRangeKind, SymbolKind};

// `folding.markers` from language-configuration/erb.json; VS Code stops applying them once a provider answers.
static REGION_START: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*<!--\s*#region\b.*-->|^\s*<%#\s*region\b").unwrap());
static REGION_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*<!--\s*#endregion\b.*-->|^\s*<%#\s*endregion\b").unwrap());

static CONTENT_FOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*content_for\s*\(?\s*:(\w+)").unwrap());
static FORM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:form_with|form_for)\b").unwrap());
static TURBO_FRAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*turbo_frame_tag\s*\(?\s*("[^"]*"|'[^']*'|[^\s,]+)"#).unwrap());
static BLOCK_TAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+do\s*(\|[^|]*\|)?\s*$").unwrap());
static ID_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\sid\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap());

pub(crate) const LANDMARKS: &[&str] = &["header", "nav", "main", "section", "aside", "footer", "form"];

// VS Code's default `editor.tabSize`; only relative indentation matters, so the exact value rarely does.
const TAB_SIZE: i64 = 4;

/// ERB blocks (split at `else`/`elsif`/`when`), multi-line ERB tags and `<%# %>`, HTML elements, region markers,
/// plus indentation folds: any array we return disables VS Code's own indentation folding, so we reproduce it.
pub fn folding_ranges(text: &str) -> Vec<FoldingRange> {
    let index = LineIndex::new(text);
    let line = |offset: usize| index.line_of(offset) as u32;
    let mut out = Vec::new();
    let mut push = |start_line: u32, end_line: u32, kind: Option<FoldingRangeKind>| {
        if end_line > start_line {
            out.push(FoldingRange { start_line, end_line, kind, ..Default::default() });
        }
    };

    let mut open: Vec<u32> = Vec::new();
    let analyzed = tags::analyzed(text);
    for (tag, effect) in analyzed.iter().filter(|(t, _)| !t.is_literal()) {
        let at = line(tag.start);
        if tag.is_comment() {
            push(at, line(tag.end), Some(FoldingRangeKind::Comment));
            continue;
        }
        match *effect {
            BlockEffect::Open(n) => open.extend(std::iter::repeat_n(at, n)),
            BlockEffect::Continue => {
                if let Some(start) = open.pop() {
                    push(start, at.saturating_sub(1), None);
                    open.push(at);
                }
            }
            BlockEffect::Close(n) => {
                for start in (0..n).filter_map(|_| open.pop()) {
                    push(start, at.saturating_sub(1), None);
                }
            }
            BlockEffect::Neutral => push(at, line(tag.end).saturating_sub(1), None),
        }
    }
    for e in tags::elements_of(text, &analyzed) {
        push(line(e.start), line(e.close_start).saturating_sub(1), None);
    }
    out.extend(region_folds(text, &REGION_START, &REGION_END));
    let syntax_starts: std::collections::HashSet<u32> = out.iter().map(|r| r.start_line).collect();
    for (start_line, end_line) in indentation_folds(text, false) {
        if !syntax_starts.contains(&start_line) {
            out.push(FoldingRange { start_line, end_line, ..Default::default() });
        }
    }
    out
}

fn region_folds(text: &str, start: &Regex, end: &Regex) -> Vec<FoldingRange> {
    let mut open: Vec<u32> = Vec::new();
    let mut out = Vec::new();
    for (i, l) in text.lines().enumerate() {
        if start.is_match(l) {
            open.push(i as u32);
        } else if let Some(start_line) = end.is_match(l).then(|| open.pop()).flatten() {
            out.push(FoldingRange { start_line, end_line: i as u32, kind: Some(FoldingRangeKind::Region), ..Default::default() });
        }
    }
    out
}

/// Port of VS Code's indentation folding: a line folds down to just before the next non-blank line indented
/// no deeper, when the lines in between are indented deeper. Blank lines inside are included, except trailing
/// ones with `off_side` (the language configuration's `folding.offSide`).
fn indentation_folds(text: &str, off_side: bool) -> Vec<(u32, u32)> {
    let lines: Vec<&str> = text.split('\n').collect();
    // (indent, first line of the following same-or-shallower block), innermost last.
    let mut regions: Vec<(i64, usize)> = vec![(-1, lines.len())];
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate().rev() {
        let Some(indent) = indent_level(l) else {
            if let Some(top) = regions.last_mut().filter(|_| off_side) {
                top.1 = i;
            }
            continue;
        };
        let deeper_below = regions.last().is_some_and(|&(r, _)| r > indent);
        while regions.last().is_some_and(|&(r, _)| r > indent) {
            regions.pop();
        }
        let Some(top) = regions.last_mut() else { continue };
        if deeper_below && top.1 - 1 > i {
            out.push((i as u32, (top.1 - 1) as u32));
        }
        if top.0 == indent {
            top.1 = i;
        } else {
            regions.push((indent, i));
        }
    }
    out
}

/// Columns of leading whitespace with tabs to the next `TAB_SIZE` stop; `None` for blank lines.
pub(crate) fn indent_level(line: &str) -> Option<i64> {
    let mut indent = 0;
    for c in line.chars() {
        match c {
            ' ' => indent += 1,
            '\t' => indent += TAB_SIZE - indent % TAB_SIZE,
            c if c.is_whitespace() => {}
            _ => return Some(indent),
        }
    }
    None
}

/// Outline: `content_for`, forms, Turbo Frames, `render` calls and, with an HTML host, landmark/`id` elements.
pub fn document_symbols(text: &str, html: bool) -> Vec<DocumentSymbol> {
    let index = LineIndex::new(text);
    let mut symbols = Symbols::new(&index);

    // One entry per open Ruby block: the symbol it will become once its `end` is seen.
    let mut open: Vec<Option<(String, SymbolKind, usize, usize)>> = Vec::new();
    let analyzed = tags::analyzed(text);
    for (tag, effect) in analyzed.iter().filter(|(t, _)| !t.is_literal() && !t.is_comment()) {
        let code = &text[tag.code.clone()];
        symbols.add_render_refs(code, tag.code.start);
        let named = erb_symbol(code);
        match *effect {
            BlockEffect::Open(n) => {
                open.extend(std::iter::repeat_with(|| None).take(n - 1));
                open.push(named.map(|(name, kind)| (name, kind, tag.start, tag.end)));
            }
            BlockEffect::Close(n) => {
                for (name, kind, start, end) in (0..n).filter_map(|_| open.pop()).flatten() {
                    symbols.add(name, kind, (start, tag.end), (start, end));
                }
            }
            _ => {
                if let Some((name, kind)) = named {
                    symbols.add(name, kind, (tag.start, tag.end), (tag.start, tag.end));
                }
            }
        }
    }
    for (name, kind, start, end) in open.into_iter().flatten() {
        symbols.add(name, kind, (start, end), (start, end));
    }

    if html {
        for e in tags::elements_of(text, &analyzed) {
            let id = id_attr(&text[e.start..e.open_end]);
            if id.is_none() && !LANDMARKS.contains(&e.name.as_str()) {
                continue;
            }
            let name = id.map_or_else(|| e.name.clone(), |id| format!("{}#{id}", e.name));
            symbols.add(name, SymbolKind::FIELD, (e.start, e.end), (e.start, e.open_end));
        }
    }
    symbols.finish()
}

pub(crate) fn erb_symbol(code: &str) -> Option<(String, SymbolKind)> {
    if let Some(c) = CONTENT_FOR.captures(code) {
        return Some((format!("content_for :{}", &c[1]), SymbolKind::NAMESPACE));
    }
    if let Some(c) = TURBO_FRAME.captures(code) {
        return Some((format!("turbo_frame_tag {}", &c[1]), SymbolKind::MODULE));
    }
    FORM.is_match(code).then(|| (BLOCK_TAIL.replace(code, "").split_whitespace().collect::<Vec<_>>().join(" "), SymbolKind::OBJECT))
}

/// The literal `id` in a start tag's attributes.
pub(crate) fn id_attr(attributes: &str) -> Option<&str> {
    ID_ATTR.captures(attributes).and_then(|c| c.iter().skip(1).flatten().next()).map(|m| m.as_str())
}

/// A document's symbols by byte range, nested once all are in.
pub(crate) struct Symbols<'a> {
    index: &'a LineIndex<'a>,
    items: Vec<(usize, usize, DocumentSymbol)>,
}

impl<'a> Symbols<'a> {
    pub fn new(index: &'a LineIndex<'a>) -> Self {
        Symbols { index, items: Vec::new() }
    }

    pub fn add(&mut self, name: String, kind: SymbolKind, range: (usize, usize), selection: (usize, usize)) {
        let to_range = |(start, end)| self.index.range(start, end);
        #[allow(deprecated)]
        let symbol = DocumentSymbol {
            name,
            detail: None,
            kind,
            tags: None,
            deprecated: None,
            range: to_range(range),
            selection_range: to_range(selection),
            children: None,
        };
        self.items.push((range.0, range.1, symbol));
    }

    /// A symbol for each `render` call in `code`, which starts at byte `code_start`, but those behind a Ruby comment
    /// on their line.
    pub fn add_render_refs(&mut self, code: &str, code_start: usize) {
        let line_before = |at: usize| &code[code[..at].rfind('\n').map_or(0, |i| i + 1)..at];
        for r in render_refs(code).into_iter().filter(|r| !ruby::comment_before(line_before(r.start))) {
            let name_range = (code_start + r.start, code_start + r.end);
            self.add(format!("render \"{}\"", r.name), SymbolKind::FILE, name_range, name_range);
        }
    }

    pub fn finish(self) -> Vec<DocumentSymbol> {
        nest(self.items)
    }
}

/// Nest symbols by byte-range containment; partially overlapping ones become siblings.
fn nest(mut items: Vec<(usize, usize, DocumentSymbol)>) -> Vec<DocumentSymbol> {
    items.sort_by_key(|&(start, end, _)| (start, std::cmp::Reverse(end)));
    let mut roots = Vec::new();
    let mut stack: Vec<(usize, DocumentSymbol)> = Vec::new();
    for (_, end, sym) in items {
        while stack.last().is_some_and(|(parent_end, _)| *parent_end < end) {
            close_innermost(&mut stack, &mut roots);
        }
        stack.push((end, sym));
    }
    while !stack.is_empty() {
        close_innermost(&mut stack, &mut roots);
    }
    roots
}

fn close_innermost(stack: &mut Vec<(usize, DocumentSymbol)>, roots: &mut Vec<DocumentSymbol>) {
    let Some((_, sym)) = stack.pop() else { return };
    match stack.last_mut() {
        Some((_, parent)) => parent.children.get_or_insert_with(Vec::new).push(sym),
        None => roots.push(sym),
    }
}

/// Symbol names with their children in brackets: `section#main[render "card"]`.
#[cfg(test)]
pub(crate) fn names(symbols: &[DocumentSymbol]) -> Vec<String> {
    symbols
        .iter()
        .map(|s| match &s.children {
            Some(c) => format!("{}[{}]", s.name, names(c).join(",")),
            None => s.name.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp_server::ls_types::{Position, Range};

    fn folds(text: &str) -> Vec<(u32, u32, Option<FoldingRangeKind>)> {
        let mut f: Vec<_> = folding_ranges(text).into_iter().map(|r| (r.start_line, r.end_line, r.kind)).collect();
        f.sort_by_key(|&(s, e, _)| (s, e));
        f
    }

    #[test]
    fn folds_erb_branches_elements_comments_regions() {
        let text = concat!(
            "<% if a %>\n",                       // 0
            "  x\n",                              // 1
            "<% elsif b %>\n",                    // 2
            "  y\n",                              // 3
            "<% else %>\n",                       // 4
            "  z\n",                              // 5
            "<% end %>\n",                        // 6
            "<div>\n",                            // 7
            "  <ul><li>\n",                       // 8
            "    a</li>\n",                       // 9
            "  </ul>\n",                          // 10
            "</div><%# one %>\n",                 // 11
            "<%# two\n",                          // 12
            "lines %>\n",                         // 13
            "<%# region Head %>\n",               // 14
            "<script>\n",                         // 15
            "</script>\n",                        // 16
            "<%# endregion %>\n",                 // 17
            "<% items.each do |i| %><% end %>\n", // 18
        );
        assert_eq!(
            folds(text),
            vec![
                (0, 1, None),
                (2, 3, None),
                (4, 5, None),
                (7, 10, None),
                (8, 9, None),
                (12, 13, Some(FoldingRangeKind::Comment)),
                (14, 17, Some(FoldingRangeKind::Region)),
            ]
        );
    }

    #[test]
    fn case_head_without_body_is_not_folded() {
        assert_eq!(folds("<% case x %>\n<% when 1 %>\n a\n<% end %>"), vec![(1, 2, None)]);
    }

    #[test]
    fn indentation_folds_match_vscode() {
        let mut f = indentation_folds("a\n  b\n\n  c\nd\n\tx\n    y\n", false);
        f.sort();
        assert_eq!(f, [(0, 3), (4, 7)]);
    }

    #[test]
    fn indentation_folds_survive_bailed_tag_pairing() {
        assert_eq!(folds("<div>\n  <span>\n</div>\n<p>\n  x\n</p>"), [(0, 1, None), (3, 4, None)]);
        for text in [
            "<% if @boxed %><div><% end %>\n  <section>\n    x\n  </section>\n<% if @boxed %></div><% end %>",
            "<%= javascript_tag do %>\n  init();\n<% end %>\n<main>\n  <p>\n    x\n  </p>\n</main>",
            "<div class=\"a\n  <p>\n    x\n",
        ] {
            let starts: Vec<u32> = folds(text).into_iter().map(|f| f.0).collect();
            for (start, _) in indentation_folds(text, false) {
                assert!(starts.contains(&start), "{text:?}: lost indentation fold at line {start}");
            }
        }
    }

    #[test]
    fn multi_line_output_tag_keeps_closing_line_visible() {
        assert_eq!(folds("<%= render partial: \"x\",\n  locals: {a: 1},\n  cached: true %>"), [(0, 1, None)]);
    }

    #[test]
    fn multi_line_form_symbol_name_is_one_line() {
        let syms = document_symbols("<%= form_with(\n  model: @u\n) do |f| %>\n<% end %>", false);
        assert_eq!(syms[0].name, "form_with( model: @u )");
    }

    #[test]
    fn render_far_below_the_top_of_a_long_code_block_does_not_panic() {
        let text = format!("<%\n{}render partial: \"card\"\n%>\n", "x = 1\n".repeat(25));
        document_symbols(&text, true);
    }

    #[test]
    fn symbols_nest_by_containment() {
        let text = concat!(
            "<% content_for :sidebar do %>\n",
            "  <nav><%= render \"shared/menu\" %></nav>\n",
            "<% end %>\n",
            "<section id=\"main-<%= x %>\" class=a>\n",
            "  <%= turbo_frame_tag \"cart\" do %>\n",
            "    <%= form_with model: @cart do |f| %><div id='f'>x</div><% end %>\n",
            "  <% end %>\n",
            "  <div>plain</div>\n",
            "</section>\n",
            "<%= turbo_frame_tag dom_id(@x), src: y %>\n",
        );
        assert_eq!(
            names(&document_symbols(text, true)),
            [
                "content_for :sidebar[nav[render \"shared/menu\"]]",
                "section#main-<%= x %>[turbo_frame_tag \"cart\"[form_with model: @cart[div#f]]]",
                "turbo_frame_tag dom_id(@x)",
            ]
        );
        assert_eq!(
            names(&document_symbols(text, false)),
            [
                "content_for :sidebar[render \"shared/menu\"]",
                "turbo_frame_tag \"cart\"[form_with model: @cart]",
                "turbo_frame_tag dom_id(@x)",
            ]
        );
    }

    #[test]
    fn render_behind_a_ruby_comment_is_no_symbol() {
        let text = "<% # render \"old\" %>\n<%= render \"card\" %>\n<% x = 1 # render \"note\" %>\n";
        assert_eq!(names(&document_symbols(text, false)), ["render \"card\""]);
        let after_comment_line = "<%\n  # the card\n  render \"card\"\n%>\n";
        assert_eq!(names(&document_symbols(after_comment_line, false)), ["render \"card\""]);
    }

    #[test]
    fn symbol_ranges_are_utf16() {
        let syms = document_symbols("😀<% content_for :a do %>\né<% end %>", false);
        assert_eq!(syms[0].range, Range::new(Position::new(0, 2), Position::new(1, 10)));
        assert_eq!(syms[0].selection_range, Range::new(Position::new(0, 2), Position::new(0, 25)));
    }
}
