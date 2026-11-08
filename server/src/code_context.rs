//! Whether an offset of a template, or a `render` call in it, is Ruby code rather than template text.

use crate::lang::TemplateLang;
use crate::rails::render_refs;
use crate::ruby::generated::Compiled;
use crate::ruby::prism::{LocalDefinition, LocalToken, LocalUses};
use crate::ruby::strict_locals;
use crate::ruby::strict_locals::Close;
use crate::slim;
use crate::{erb, haml, ruby, util};
use haml::indent::LineRole;

/// `in_ruby_context`, with Slim asked through its parser: its line heuristic also takes `p See (docs)` as Ruby.
pub(crate) fn code_at(lang: TemplateLang, text: &str, offset: usize, extra_engines: &[String]) -> bool {
    match lang {
        TemplateLang::Slim => slim::parser::in_ruby(text, &slim::parser::parse(text, extra_engines), offset),
        _ => in_ruby_context(lang, text, &erb::code_spans(text), offset),
    }
}

/// The Ruby the template compiles to; with `close`, the strict locals comment makes the method's parameters.
pub(crate) fn compiled(lang: TemplateLang, text: &str, close: Option<Close>, extra_engines: &[String]) -> Option<Compiled> {
    match lang {
        TemplateLang::Erb => Some(erb::ruby::compiled(text, close)),
        TemplateLang::Slim => Some(slim::ruby::compiled(&slim::parser::parse(text, extra_engines))),
        TemplateLang::Haml => {
            let comment = close.and_then(|close| strict_locals::find(text, close));
            Some(haml::ruby::compiled(text, &haml::parser::parse_with_ruby(text), comment.as_ref()))
        }
        TemplateLang::Skim | TemplateLang::Ruby | TemplateLang::Liquid => None,
    }
}

/// The block variables in scope at `offset`, from the Ruby the template compiles to.
pub(crate) fn block_locals(lang: TemplateLang, text: &str, offset: usize, extra_engines: &[String]) -> Vec<String> {
    compiled(lang, text, None, extra_engines).map_or_else(Vec::new, |compiled| compiled.block_locals(offset))
}

/// Where the template declares the local variable at `offset` (a block parameter or an assignment), from the Ruby it
/// compiles to.
pub(crate) fn local_definition(lang: TemplateLang, text: &str, offset: usize, extra_engines: &[String]) -> Option<LocalDefinition> {
    compiled(lang, text, None, extra_engines)?.local_definition(offset)
}

/// Where the template reads or assigns the strict local `name`, from the Ruby it compiles to.
pub(crate) fn local_uses(lang: TemplateLang, text: &str, close: Close, name: &str, extra_engines: &[String]) -> LocalUses {
    compiled(lang, text, Some(close), extra_engines).map_or_else(LocalUses::default, |compiled| compiled.local_uses(name))
}

/// The local variables the template declares or uses, from the Ruby it compiles to; a strict local is a parameter.
pub(crate) fn local_tokens(lang: TemplateLang, text: &str, close: Close, extra_engines: &[String]) -> Vec<LocalToken> {
    compiled(lang, text, Some(close), extra_engines).map_or_else(Vec::new, |compiled| compiled.local_tokens())
}

/// Haml line roles; every other language's lines all start their own construct.
pub(crate) fn line_roles(lang: TemplateLang, text: &str) -> Vec<LineRole> {
    match lang {
        TemplateLang::Haml => haml::indent::haml_line_roles(text),
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
    r: &render_refs::RenderRef,
) -> bool {
    let line = util::lines_with_offsets(&text[line_start..]).next().map_or("", |(_, line)| line);
    let in_ruby = || in_ruby_context(lang, text, spans, line_start + r.start);
    let code_start = match (lang, role) {
        // A tag opened on an earlier line puts the whole line before `render` in Ruby.
        (TemplateLang::Erb, _) => in_ruby().then(|| line[..r.call].rfind("<%").map_or(0, |open| open + 2)),
        (TemplateLang::Haml, LineRole::Start) => haml::indent::haml_code_start(line).filter(|&code| code <= r.call),
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
        (TemplateLang::Skim | TemplateLang::Liquid, _) => None,
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
) -> impl Fn(usize, usize, &render_refs::RenderRef) -> bool + 'a {
    let roles = line_roles(lang, text);
    let spans = erb::code_spans(text);
    let slim = (lang == TemplateLang::Slim).then(|| slim::parser::parse(text, extra_engines));
    move |i, line_start, r| {
        render_in_code(lang, roles[i], text, &spans, line_start, r)
            && slim.as_ref().is_none_or(|parsed| slim::parser::in_ruby(text, parsed, line_start + r.start))
    }
}

/// Is byte `offset` of `text` in Ruby: inside an ERB code tag (`spans` from `erb::code_spans`, may cross lines)
/// or on a Haml/Slim Ruby line. Slim documents go through `slim::parser` where the whole text is at hand.
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
        TemplateLang::Skim | TemplateLang::Liquid => false,
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
    use crate::rails::project::Project;
    use crate::rails::render_call::HamlPipes;

    #[test]
    fn liquid_render_is_not_a_partial_reference() {
        let root = std::env::temp_dir().join(format!("rt-liquid-{}", std::process::id()));
        let views = root.join("app/views/pages");
        std::fs::create_dir_all(&views).unwrap();
        std::fs::write(views.join("_card.html.slim"), "").unwrap();
        let current = views.join("show.liquid");
        let text = "{% render \"card\" %}\n";
        let refs = |keep: bool| {
            let in_code = render_filter(TemplateLang::Liquid, text, &[]);
            render_refs::render_refs_in(text, 0, text.trim_end(), &HamlPipes::new(TemplateLang::Liquid), ruby_prism::Version::Latest, |r| {
                !keep || in_code(0, 0, r)
            })
        };
        let resolved: Vec<_> = refs(false).iter().flat_map(|r| crate::rails::partials::resolve(&Project::unknown(), &current, r)).collect();
        let filtered = refs(true);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(resolved, [views.join("_card.html.slim")]);
        assert!(filtered.is_empty());
        assert!(!in_ruby_context(TemplateLang::Liquid, text, &[], 4));
    }

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
