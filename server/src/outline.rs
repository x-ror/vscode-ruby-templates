//! Folding ranges and document symbols for ERB templates.

use crate::erb;
use crate::partials::render_refs;
use crate::ruby::{analyze, BlockEffect};
use crate::tags;
use crate::util::offset_to_position;
use once_cell::sync::Lazy;
use regex::Regex;
use tower_lsp::lsp_types::{DocumentSymbol, FoldingRange, FoldingRangeKind, Range, SymbolKind};

// `folding.markers` from language-configuration/erb.json; VS Code stops applying them once a provider answers.
static REGION_START: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*<!--\s*#region\b.*-->|^\s*<%#\s*region\b").unwrap());
static REGION_END: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*<!--\s*#endregion\b.*-->|^\s*<%#\s*endregion\b").unwrap());

static CONTENT_FOR: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*content_for\s*\(?\s*:(\w+)").unwrap());
static FORM: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*(?:form_with|form_for)\b").unwrap());
static TURBO_FRAME: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^\s*turbo_frame_tag\s*\(?\s*("[^"]*"|'[^']*'|[^\s,]+)"#).unwrap());
static BLOCK_TAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+do\s*(\|[^|]*\|)?\s*$").unwrap());
static ID_ATTR: Lazy<Regex> = Lazy::new(|| Regex::new(r#"\sid\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap());

const LANDMARKS: &[&str] = &["header", "nav", "main", "section", "aside", "footer", "form"];

/// A `<% %>`, `<%= %>` or `<%# %>` tag as byte offsets; `code` is empty for comments.
struct ErbTag<'a> { start: usize, end: usize, comment: bool, code_start: usize, code: &'a str }

fn erb_tags(text: &str) -> Vec<ErbTag<'_>> {
    let Some(tree) = erb::parse(text) else { return vec![] };
    let root = tree.root_node();
    let mut cursor = root.walk();
    let tags = root.children(&mut cursor).filter(|n| matches!(n.kind(), "directive" | "output_directive" | "comment_directive"));
    tags.map(|node| {
        let mut c = node.walk();
        let code = node.children(&mut c).find(|n| n.kind() == "code").map_or(node.end_byte()..node.end_byte(), |n| n.byte_range());
        ErbTag { start: node.start_byte(), end: node.end_byte(), comment: node.kind() == "comment_directive", code_start: code.start, code: &text[code] }
    })
    .collect()
}

// VS Code's default `editor.tabSize`; only relative indentation matters, so the exact value rarely does.
const TAB_SIZE: i64 = 4;

/// ERB blocks (split at `else`/`elsif`/`when`), multi-line ERB tags and `<%# %>`, HTML elements, region markers,
/// plus indentation folds: any array we return disables VS Code's own indentation folding, so we reproduce it.
pub fn folding_ranges(text: &str) -> Vec<FoldingRange> {
    let line_starts: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let line = |offset: usize| (line_starts.partition_point(|&s| s <= offset) - 1) as u32;
    let mut out = Vec::new();
    let mut push = |start_line: u32, end_line: u32, kind: Option<FoldingRangeKind>| {
        if end_line > start_line {
            out.push(FoldingRange { start_line, end_line, kind, ..Default::default() });
        }
    };

    let mut open: Vec<u32> = Vec::new();
    for tag in erb_tags(text) {
        let at = line(tag.start);
        if tag.comment {
            push(at, line(tag.end), Some(FoldingRangeKind::Comment));
            continue;
        }
        match analyze(tag.code).0 {
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
    for e in tags::elements(text) {
        push(line(e.start), line(e.close_start).saturating_sub(1), None);
    }
    let mut regions: Vec<u32> = Vec::new();
    for (i, l) in text.lines().enumerate() {
        if REGION_START.is_match(l) {
            regions.push(i as u32);
        } else if let Some(start) = REGION_END.is_match(l).then(|| regions.pop()).flatten() {
            push(start, i as u32, Some(FoldingRangeKind::Region));
        }
    }
    let syntax_starts: std::collections::HashSet<u32> = out.iter().map(|r| r.start_line).collect();
    for (start_line, end_line) in indentation_folds(text) {
        if !syntax_starts.contains(&start_line) {
            out.push(FoldingRange { start_line, end_line, ..Default::default() });
        }
    }
    out
}

/// Port of VS Code's indentation folding (non-offside): a line folds down to just before the next non-blank
/// line indented no deeper, when the lines in between are indented deeper. Blank lines inside are included.
fn indentation_folds(text: &str) -> Vec<(u32, u32)> {
    let lines: Vec<&str> = text.split('\n').collect();
    // (indent, first line of the following same-or-shallower block), innermost last.
    let mut regions: Vec<(i64, usize)> = vec![(-1, lines.len())];
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate().rev() {
        let Some(indent) = indent_level(l) else { continue };
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
fn indent_level(line: &str) -> Option<i64> {
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
    let mut items: Vec<(usize, usize, DocumentSymbol)> = Vec::new();
    let mut add = |name: String, kind, range: (usize, usize), selection: (usize, usize)| {
        items.push((range.0, range.1, symbol(text, name, kind, range, selection)));
    };

    // One entry per open Ruby block: the symbol it will become once its `end` is seen.
    let mut open: Vec<Option<(String, SymbolKind, usize, usize)>> = Vec::new();
    for tag in erb_tags(text).into_iter().filter(|t| !t.comment) {
        for r in render_refs(tag.code) {
            let name_range = (tag.code_start + r.start, tag.code_start + r.end);
            add(format!("render \"{}\"", r.name), SymbolKind::FILE, name_range, name_range);
        }
        let named = erb_symbol(tag.code);
        match analyze(tag.code).0 {
            BlockEffect::Open(n) => {
                open.extend(std::iter::repeat_with(|| None).take(n - 1));
                open.push(named.map(|(name, kind)| (name, kind, tag.start, tag.end)));
            }
            BlockEffect::Close(n) => {
                for (name, kind, start, end) in (0..n).filter_map(|_| open.pop()).flatten() {
                    add(name, kind, (start, tag.end), (start, end));
                }
            }
            _ => {
                if let Some((name, kind)) = named {
                    add(name, kind, (tag.start, tag.end), (tag.start, tag.end));
                }
            }
        }
    }
    for (name, kind, start, end) in open.into_iter().flatten() {
        add(name, kind, (start, end), (start, end));
    }

    if html {
        for e in tags::elements(text) {
            let id = ID_ATTR.captures(&text[e.start..e.open_end]).and_then(|c| c.iter().skip(1).flatten().next()).map(|m| m.as_str());
            if id.is_none() && !LANDMARKS.contains(&e.name.as_str()) {
                continue;
            }
            let name = id.map_or_else(|| e.name.clone(), |id| format!("{}#{id}", e.name));
            add(name, SymbolKind::FIELD, (e.start, e.end), (e.start, e.open_end));
        }
    }
    nest(items)
}

fn erb_symbol(code: &str) -> Option<(String, SymbolKind)> {
    if let Some(c) = CONTENT_FOR.captures(code) {
        return Some((format!("content_for :{}", &c[1]), SymbolKind::NAMESPACE));
    }
    if let Some(c) = TURBO_FRAME.captures(code) {
        return Some((format!("turbo_frame_tag {}", &c[1]), SymbolKind::MODULE));
    }
    FORM.is_match(code).then(|| (BLOCK_TAIL.replace(code, "").split_whitespace().collect::<Vec<_>>().join(" "), SymbolKind::OBJECT))
}

fn symbol(text: &str, name: String, kind: SymbolKind, range: (usize, usize), selection: (usize, usize)) -> DocumentSymbol {
    let to_range = |(start, end)| Range::new(offset_to_position(text, start), offset_to_position(text, end));
    #[allow(deprecated)]
    DocumentSymbol { name, detail: None, kind, tags: None, deprecated: None, range: to_range(range), selection_range: to_range(selection), children: None }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Position;

    fn folds(text: &str) -> Vec<(u32, u32, Option<FoldingRangeKind>)> {
        let mut f: Vec<_> = folding_ranges(text).into_iter().map(|r| (r.start_line, r.end_line, r.kind)).collect();
        f.sort_by_key(|&(s, e, _)| (s, e));
        f
    }

    #[test]
    fn folds_erb_branches_elements_comments_regions() {
        let text = concat!(
            "<% if a %>\n",           // 0
            "  x\n",                  // 1
            "<% elsif b %>\n",        // 2
            "  y\n",                  // 3
            "<% else %>\n",           // 4
            "  z\n",                  // 5
            "<% end %>\n",            // 6
            "<div>\n",                // 7
            "  <ul><li>\n",           // 8
            "    a</li>\n",           // 9
            "  </ul>\n",              // 10
            "</div><%# one %>\n",     // 11
            "<%# two\n",              // 12
            "lines %>\n",             // 13
            "<%# region Head %>\n",   // 14
            "<script>\n",             // 15
            "</script>\n",            // 16
            "<%# endregion %>\n",     // 17
            "<% items.each do |i| %><% end %>\n", // 18
        );
        assert_eq!(folds(text), vec![
            (0, 1, None),
            (2, 3, None),
            (4, 5, None),
            (7, 10, None),
            (8, 9, None),
            (12, 13, Some(FoldingRangeKind::Comment)),
            (14, 17, Some(FoldingRangeKind::Region)),
        ]);
    }

    #[test]
    fn case_head_without_body_is_not_folded() {
        assert_eq!(folds("<% case x %>\n<% when 1 %>\n a\n<% end %>"), vec![(1, 2, None)]);
    }

    #[test]
    fn indentation_folds_match_vscode() {
        let mut f = indentation_folds("a\n  b\n\n  c\nd\n\tx\n    y\n");
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
            for (start, _) in indentation_folds(text) {
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

    fn outline(syms: &[DocumentSymbol]) -> Vec<String> {
        syms.iter()
            .map(|s| match &s.children {
                Some(c) => format!("{}[{}]", s.name, outline(c).join(",")),
                None => s.name.clone(),
            })
            .collect()
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
        assert_eq!(outline(&document_symbols(text, true)), [
            "content_for :sidebar[nav[render \"shared/menu\"]]",
            "section#main-<%= x %>[turbo_frame_tag \"cart\"[form_with model: @cart[div#f]]]",
            "turbo_frame_tag dom_id(@x)",
        ]);
        assert_eq!(outline(&document_symbols(text, false)), [
            "content_for :sidebar[render \"shared/menu\"]",
            "turbo_frame_tag \"cart\"[form_with model: @cart]",
            "turbo_frame_tag dom_id(@x)",
        ]);
    }

    #[test]
    fn symbol_ranges_are_utf16() {
        let syms = document_symbols("😀<% content_for :a do %>\né<% end %>", false);
        assert_eq!(syms[0].range, Range::new(Position::new(0, 2), Position::new(1, 10)));
        assert_eq!(syms[0].selection_range, Range::new(Position::new(0, 2), Position::new(0, 25)));
    }
}
