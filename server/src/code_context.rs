//! Whether an offset of a template, or a `render` call in it, is Ruby code rather than template text.

use crate::lang::TemplateLang;
use crate::{erb, erb_ruby, haml, haml_indent, haml_parser, haml_ruby, partials, ruby, slim_parser, slim_ruby, util};
use haml_indent::LineRole;

/// `in_ruby_context`, with Slim asked through its parser: its line heuristic also takes `p See (docs)` as Ruby.
pub(crate) fn code_at(lang: TemplateLang, text: &str, offset: usize, extra_engines: &[String]) -> bool {
    match lang {
        TemplateLang::Slim => slim_parser::in_ruby(text, &slim_parser::parse(text, extra_engines), offset),
        _ => in_ruby_context(lang, text, &erb::code_spans(text), offset),
    }
}

/// The block variables in scope at `offset`, from the Ruby the template compiles to.
pub(crate) fn block_locals(lang: TemplateLang, text: &str, offset: usize, extra_engines: &[String]) -> Vec<String> {
    match lang {
        TemplateLang::Erb => erb_ruby::block_locals(text, offset),
        TemplateLang::Slim => slim_ruby::block_locals(&slim_parser::parse(text, extra_engines), offset),
        TemplateLang::Haml => haml_ruby::block_locals(text, &haml_parser::parse_with_ruby(text), offset),
        TemplateLang::Skim | TemplateLang::Ruby => Vec::new(),
    }
}

/// Haml line roles; every other language's lines all start their own construct.
pub(crate) fn line_roles(lang: TemplateLang, text: &str) -> Vec<LineRole> {
    match lang {
        TemplateLang::Haml => haml_indent::haml_line_roles(text),
        _ => vec![LineRole::Start; text.lines().count()],
    }
}

/// Is the `render` whose string starts at byte `start` of the line at `line_start` a call in Ruby code, not template
/// text, a Ruby comment or a Ruby string (`link_to "render 'x'"`)? `spans` are `erb::code_spans(text)`.
fn render_in_code(
    lang: TemplateLang,
    role: LineRole,
    text: &str,
    spans: &[(usize, usize)],
    line_start: usize,
    r: &partials::RenderRef,
) -> bool {
    let line = util::lines_with_offsets(&text[line_start..]).next().map_or("", |(_, line)| line);
    let in_ruby = || in_ruby_context(lang, text, spans, line_start + r.start);
    let code_start = match (lang, role) {
        // A tag opened on an earlier line puts the whole line before `render` in Ruby.
        (TemplateLang::Erb, _) => in_ruby().then(|| line[..r.call].rfind("<%").map_or(0, |open| open + 2)),
        (TemplateLang::Haml, LineRole::Start) => haml_indent::haml_code_start(line).filter(|&code| code <= r.call),
        (TemplateLang::Haml, LineRole::Ruby) => Some(0),
        (TemplateLang::Haml, LineRole::Text) => None,
        (TemplateLang::Slim, _) => {
            let ws = util::leading_ws(line).len();
            let prefix = ["!=", "&=", "=", "-"].into_iter().find(|p| line[ws..].starts_with(p));
            // `==`, `=<`, `=>`, `='` and the like: the rest of the marker isn't Ruby.
            let marker_end =
                |p: &str| ws + p.len() + line[ws + p.len()..].len() - line[ws + p.len()..].trim_start_matches(['=', '<', '>', '\'']).len();
            prefix.filter(|_| in_ruby()).map(marker_end)
        }
        (TemplateLang::Skim, _) => None,
        (TemplateLang::Ruby, _) => in_ruby().then_some(0),
    };
    code_start.is_some_and(|code| code <= r.call && ruby::code_end(&line[code..r.call]) == ruby::CodeEnd::Code)
}

/// `render_in_code` for each render ref in `text`, given its line index and line start; Slim lines
/// also have to be Ruby to Slim's parser.
pub(crate) fn render_filter<'a>(
    lang: TemplateLang,
    text: &'a str,
    extra_engines: &[String],
) -> impl Fn(usize, usize, &partials::RenderRef) -> bool + 'a {
    let roles = line_roles(lang, text);
    let spans = erb::code_spans(text);
    let slim = (lang == TemplateLang::Slim).then(|| slim_parser::parse(text, extra_engines));
    move |i, line_start, r| {
        render_in_code(lang, roles[i], text, &spans, line_start, r)
            && slim.as_ref().is_none_or(|parsed| slim_parser::in_ruby(text, parsed, line_start + r.start))
    }
}

/// Is byte `offset` of `text` in Ruby: inside an ERB code tag (`spans` from `erb::code_spans`, may cross lines)
/// or on a Haml/Slim Ruby line. Slim documents go through `slim_parser` where the whole text is at hand.
pub(crate) fn in_ruby_context(lang: TemplateLang, text: &str, spans: &[(usize, usize)], offset: usize) -> bool {
    let offset = offset.min(text.len());
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    match lang {
        // A Ruby `#` comment ends at the newline, so only this line of the tag matters.
        TemplateLang::Erb => erb::span_at(spans, offset).is_some_and(|code| !ruby::comment_before(&text[code.max(line_start)..offset])),
        TemplateLang::Haml => {
            let line = util::lines_with_offsets(&text[line_start..]).next().map_or("", |(_, line)| line);
            haml::in_ruby(line, offset - line_start)
        }
        // Skim code is CoffeeScript: no Rails helpers, renders or form builders.
        TemplateLang::Skim => false,
        TemplateLang::Ruby => !ruby::comment_before(&text[line_start..offset]),
        TemplateLang::Slim => {
            let before = &text[line_start..offset];
            let t = before.trim_start();
            t.starts_with('-')
                || t.starts_with('=')
                || t.starts_with("!=")
                || t.starts_with("&=")
                || before.contains("#{")
                || before.contains("= ")
                || before.contains('{')
                || before.contains('(')
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slim_code_at_asks_the_parser() {
        let text = "p See (t(\".a\"))\n= t(\".b\")\n";
        assert!(!code_at(TemplateLang::Slim, text, text.find(".a").unwrap(), &[]));
        assert!(code_at(TemplateLang::Slim, text, text.find(".b").unwrap(), &[]));
    }

    #[test]
    fn ruby_context_spans_erb_lines() {
        let text = "<%= link_to \"x\",\n  user_path(u) %> after <%# note %>\n<% # c\n  x %>";
        let at = |needle: &str| text.find(needle).unwrap();
        let spans = erb::code_spans(text);
        assert!(in_ruby_context(TemplateLang::Erb, text, &spans, at("user_path")));
        assert!(!in_ruby_context(TemplateLang::Erb, text, &spans, at("after")));
        assert!(!in_ruby_context(TemplateLang::Erb, text, &spans, at("note")));
        assert!(!in_ruby_context(TemplateLang::Erb, text, &spans, at("# c") + 2));
        assert!(in_ruby_context(TemplateLang::Erb, text, &spans, at("  x") + 2));
        let haml = "%p\n= link_to";
        assert!(in_ruby_context(TemplateLang::Haml, haml, &[], haml.len()));
        assert!(!in_ruby_context(TemplateLang::Haml, haml, &[], 2));
    }
}
