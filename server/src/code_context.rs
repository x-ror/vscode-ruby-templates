//! Whether an offset of a template, or a `render` call in it, is Ruby code rather than template text.

use crate::lang::TemplateLang;
use crate::prism::{LocalDefinition, LocalUses};
use crate::strict_locals::{self, Close};
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

/// Where the template declares the local variable at `offset` (a block parameter or an assignment), from the Ruby it
/// compiles to.
pub(crate) fn local_definition(lang: TemplateLang, text: &str, offset: usize, extra_engines: &[String]) -> Option<LocalDefinition> {
    match lang {
        TemplateLang::Erb => erb_ruby::local_definition(text, offset),
        TemplateLang::Slim => slim_ruby::local_definition(&slim_parser::parse(text, extra_engines), offset),
        TemplateLang::Haml => haml_ruby::local_definition(text, &haml_parser::parse_with_ruby(text), offset),
        TemplateLang::Skim | TemplateLang::Ruby => None,
    }
}

/// Where the template reads or assigns the strict local `name`, from the Ruby it compiles to.
pub(crate) fn local_uses(lang: TemplateLang, text: &str, close: Close, name: &str, extra_engines: &[String]) -> LocalUses {
    match lang {
        TemplateLang::Erb => erb_ruby::local_uses(text, close, name),
        TemplateLang::Slim => slim_ruby::local_uses(&slim_parser::parse(text, extra_engines), name),
        TemplateLang::Haml => {
            haml_ruby::local_uses(text, &haml_parser::parse_with_ruby(text), strict_locals::find(text, close).as_ref(), name)
        }
        TemplateLang::Skim | TemplateLang::Ruby => LocalUses::default(),
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
    fn local_uses_in_every_language() {
        let reads = |lang, text: &str| -> Vec<usize> {
            local_uses(lang, text, Close::default(), "user", &[]).spans.into_iter().map(|(start, _)| start).collect()
        };
        let erb = "<%# locals: (user:) %>\n<%= user.name %> <% [1].each do |user| %><%= user %><% end %><%= @user %><% user ||= 1 %>\n";
        assert_eq!(reads(TemplateLang::Erb, erb), [erb.find("user.name").unwrap(), erb.find("user ||=").unwrap()]);
        let slim = "/# locals: (user:)\np = user.name\n- [1].each do |user|\n  = user\n";
        assert_eq!(reads(TemplateLang::Slim, slim), [slim.find("user.name").unwrap()]);
        let haml = "-# locals: (user:)\n%p= user.name\n- [1].each do |user|\n  = user\n";
        assert_eq!(reads(TemplateLang::Haml, haml), [haml.find("user.name").unwrap()]);
    }

    #[test]
    fn local_definitions_in_every_language() {
        fn definition(lang: TemplateLang, text: &str) -> Option<(usize, &str)> {
            let found = local_definition(lang, text, text.find("item.name").unwrap(), &[])?;
            Some((found.start, &text[found.start..found.end]))
        }
        let erb = "<% @items.each do |item| %>\n  <p><%= item.name %></p>\n<% end %>\n";
        assert_eq!(definition(TemplateLang::Erb, erb), Some((erb.find("item|").unwrap(), "item")));
        let slim = "- @items.each do |item|\n  p = item.name\n";
        assert_eq!(definition(TemplateLang::Slim, slim), Some((slim.find("item|").unwrap(), "item")));
        let haml = "- @items.each do |item|\n  %p= item.name\n";
        assert_eq!(definition(TemplateLang::Haml, haml), Some((haml.find("item|").unwrap(), "item")));
        let assigned = "<% item = @items.first %>\n<%= item.name %>\n";
        assert_eq!(definition(TemplateLang::Erb, assigned), Some((3, "item")));
        assert_eq!(definition(TemplateLang::Erb, "<%= item.name %>\n"), None, "a helper call");
    }

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
