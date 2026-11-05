//! Haml folding ranges and document symbols.

use super::{erb_symbol, indent_level, indentation_folds, region_folds, Symbols, LANDMARKS};
use crate::haml::indent::{haml_code_start, haml_structure, is_haml_tag, tag_name_len};
use crate::util::{closing_bracket, find_balanced, leading_ws, LineIndex};
use regex::Regex;
use std::sync::LazyLock;
use tower_lsp_server::ls_types::{DocumentSymbol, FoldingRange, SymbolKind};

// `folding.markers` from language-configuration/haml.json.
static REGION_START: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*-#\s*region\b").unwrap());
static REGION_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*-#\s*endregion\b").unwrap());
// A literal id in `{id: "x"}`, `{:id => "x"}`, `{"id" => "x"}` or `(id="x")`; interpolated ids are skipped.
static ATTR_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:^|[\s,{(])(?:id:|:id\s*=>|["']id["']\s*=>|id\s*=)\s*["']([^"'#]+)["']"#).unwrap());

/// VS Code's off-side indentation folding plus what it misses: region markers, and `|` / `,` / multi-line `{}`
/// groups whose continuation lines aren't indented deeper.
pub fn folding_ranges(text: &str) -> Vec<FoldingRange> {
    let mut out = region_folds(text, &REGION_START, &REGION_END);
    out.extend(indentation_folds(text, true).into_iter().map(|(start_line, end_line)| FoldingRange {
        start_line,
        end_line,
        ..Default::default()
    }));
    let starts: std::collections::HashSet<u32> = out.iter().map(|r| r.start_line).collect();
    for (line, span) in haml_structure(text) {
        let (start_line, end_line) = (line as u32, (line + span - 1) as u32);
        if span > 1 && !starts.contains(&start_line) {
            out.push(FoldingRange { start_line, end_line, ..Default::default() });
        }
    }
    out
}

/// Haml outline: the same symbols as ERB (`content_for`, forms, Turbo Frames, `render`), landmark tags and
/// `#id` tags, each spanning its line and the lines nested under it.
pub fn document_symbols(text: &str) -> Vec<DocumentSymbol> {
    let lines: Vec<&str> = text.split('\n').collect();
    let index = LineIndex::new(text);
    let line_end = |i: usize| index.line_start(i) + lines[i].trim_end().len();
    let mut symbols = Symbols::new(&index);
    for (i, span) in haml_structure(text) {
        let line = lines[i];
        let line_start = index.line_start(i);
        let content_start = line_start + leading_ws(line).len();
        let selection = (content_start, line_end(i + span - 1));
        let range = (content_start, line_end(block_end(&lines, i, span)));
        if let Some(code_start) = haml_code_start(line) {
            let code = &line[code_start..];
            symbols.add_render_refs(code, line_start + code_start);
            if let Some((name, kind)) = erb_symbol(code) {
                symbols.add(name, kind, range, selection);
            }
        }
        if let Some(name) = element_name(&line[leading_ws(line).len()..]) {
            symbols.add(name, SymbolKind::FIELD, range, selection);
        }
    }
    symbols.finish()
}

/// Last line of the block a structural line opens: its own span, then every following line indented deeper
/// (trailing blank lines excluded).
fn block_end(lines: &[&str], i: usize, span: usize) -> usize {
    let Some(indent) = indent_level(lines[i]) else { return i };
    let mut end = i + span - 1;
    for (j, l) in lines.iter().enumerate().skip(i + span) {
        match indent_level(l) {
            None => {}
            Some(level) if level > indent => end = j,
            Some(_) => break,
        }
    }
    end
}

/// `section#main` for `%section#main.wide`, `div#cart` for `#cart`, `nav` for `%nav`; `None` for other tags.
fn element_name(content: &str) -> Option<String> {
    if !is_haml_tag(content) {
        return None;
    }
    let head = &content[..tag_name_len(content)];
    let tag = head.strip_prefix('%').map_or("div", |rest| &rest[..rest.find(['.', '#']).unwrap_or(rest.len())]);
    let shorthand = head.split_once('#').map(|(_, rest)| &rest[..rest.find(['.', '#']).unwrap_or(rest.len())]);
    let id = shorthand.filter(|id| !id.is_empty()).or_else(|| attribute_id(&content[head.len()..]));
    match id {
        Some(id) => Some(format!("{tag}#{id}")),
        None => LANDMARKS.contains(&tag).then(|| tag.to_string()),
    }
}

/// The literal top-level `id` in the `{…}` / `(…)` attribute groups right after a tag head (first line only).
fn attribute_id(mut attrs: &str) -> Option<&str> {
    while let Some(close) = attrs.chars().next().and_then(closing_bracket) {
        let open = attrs.chars().next()?;
        let group = &attrs[..find_balanced(attrs, open, close)?];
        // `{data: {id: 1}}` renders `data-id`, so only depth one counts.
        let top_level = |at: usize| group[..at].matches('{').count() <= group[..at].matches('}').count() + 1;
        if open != '[' {
            if let Some(m) = ATTR_ID.captures_iter(group).filter_map(|c| c.get(1)).find(|m| top_level(m.start())) {
                return Some(m.as_str());
            }
        }
        attrs = &attrs[group.len()..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outline::names;
    use tower_lsp_server::ls_types::{FoldingRangeKind, Position, Range};

    #[test]
    fn symbols_nest_by_indentation() {
        let text = concat!(
            "- content_for :sidebar do\n",
            "  %nav= render \"shared/menu\"\n",
            "\n",
            "%section#main.wide\n",
            "  = turbo_frame_tag \"cart\" do\n",
            "    = form_with model: @cart do |f|\n",
            "      #f x\n",
            "  %div plain\n",
            ":markdown\n",
            "  #not-a-tag\n",
            "- # render \"old\"\n",
            "%p{id: x}= render 'card'\n",
            "%div{id: \"cart\", class: \"x\"}\n",
            "%form(id='signup' method='post')\n",
            "%aside{data: {id: \"no\"}}[@user]{:id => 'sidebar'}\n",
            "%span{\"id\" => \"a#{b}\"}\n",
            "%p(title=\"}\" id=\"intro\")\n",
        );
        assert_eq!(
            names(&document_symbols(text)),
            [
                "content_for :sidebar[nav[render \"shared/menu\"]]",
                "section#main[turbo_frame_tag \"cart\"[form_with model: @cart[div#f]]]",
                "render \"card\"",
                "div#cart",
                "form#signup",
                "aside#sidebar",
                "p#intro",
            ]
        );
        let syms = document_symbols(text);
        assert_eq!(syms[0].range, Range::new(Position::new(0, 0), Position::new(1, 28)));
        assert_eq!(syms[1].selection_range, Range::new(Position::new(3, 0), Position::new(3, 18)));
    }

    fn folds(text: &str) -> Vec<(u32, u32, Option<FoldingRangeKind>)> {
        let mut f: Vec<_> = folding_ranges(text).into_iter().map(|r| (r.start_line, r.end_line, r.kind)).collect();
        f.sort_by_key(|&(s, e, _)| (s, e));
        f
    }

    #[test]
    fn folds_indentation_regions_and_continuations() {
        let text = concat!(
            "%div\n",            // 0
            "  %p x\n",          // 1
            "\n",                // 2
            "-# region Form\n",  // 3
            "%a{href: \"x\",\n", // 4
            "title: \"y\"} z\n", // 5
            "= \"a\" + |\n",     // 6
            "\"b\" |\n",         // 7
            "-# endregion\n",    // 8
            "%b{c: 1,\n",        // 9
            "   d: 2}\n",        // 10
            "  %i\n",            // 11
        );
        assert_eq!(folds(text), vec![(0, 1, None), (3, 8, Some(FoldingRangeKind::Region)), (4, 5, None), (6, 7, None), (9, 11, None),]);
    }
}
