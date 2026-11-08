//! Lints on the attributes an element is written with: `data-turbo-permanent` without an `id`, and a `data-controller` name
//! that no Stimulus controller has.

use crate::attributes::{Element, Value};
use crate::rails::stimulus::{Registered, CONTROLLERS_DIR};
use crate::util::{self, edit_distance, LineIndex};
use std::ops::Range;
use std::sync::Arc;
use tower_lsp_server::ls_types::Diagnostic;

pub(crate) const PERMANENT_REQUIRES_ID: &str = "Turbo/PermanentRequiresId";
pub(crate) const UNKNOWN_CONTROLLER: &str = "Stimulus/UnknownController";

const PERMANENT: &str = "data-turbo-permanent";
const CONTROLLER: &str = "data-controller";
const ID: &str = "id";
/// A name this many edits from the typed one is offered as the one meant.
const MAX_SUGGESTION_DISTANCE: usize = 2;

/// The findings in `elements`. `registered` lists the controllers the app has and is asked only when a template names one;
/// none means they can't be told, and no name is reported.
pub(crate) fn diagnostics(text: &str, elements: &[Element], registered: impl FnOnce() -> Option<Arc<Registered>>) -> Vec<Diagnostic> {
    let index = LineIndex::new(text);
    let message = "`data-turbo-permanent` needs an `id`: Turbo matches the element across pages by it";
    let mut out: Vec<Diagnostic> = elements
        .iter()
        .filter_map(permanent_without_id)
        .map(|at| util::coded(util::warning(index.range(at.start, at.end), message), PERMANENT_REQUIRES_ID))
        .collect();
    let named: Vec<(Range<usize>, &str)> = elements.iter().flat_map(|element| controller_names(text, element)).collect();
    if let Some(known) = (!named.is_empty()).then(registered).flatten() {
        let unknown = named.into_iter().filter(|(_, name)| !known.knows(name));
        out.extend(unknown.map(|(at, name)| {
            let message = unknown_message(name, &known.names);
            util::coded(util::warning(index.range(at.start, at.end), message), UNKNOWN_CONTROLLER)
        }));
    }
    out
}

/// Where the element names `data-turbo-permanent` when it can't have an id.
fn permanent_without_id(element: &Element) -> Option<Range<usize>> {
    let permanent = element
        .attribute(PERMANENT)
        .filter(|attribute| !matches!(attribute.value, Value::Dynamic | Value::Shorthand) && !attribute.at.is_empty())?;
    (element.attribute(ID).is_none() && !element.may_have_more()).then(|| permanent.at.clone())
}

/// The controllers a static `data-controller` names, with where each is written. A name with anything but letters, digits,
/// `-` and `_` is skipped.
fn controller_names<'a>(text: &'a str, element: &Element) -> Vec<(Range<usize>, &'a str)> {
    let values = element.attributes.iter().filter(|attribute| attribute.name == CONTROLLER);
    let ranges = values.filter_map(|attribute| if let Value::Text(range) = &attribute.value { Some(range.clone()) } else { None });
    ranges
        .flat_map(|range| {
            let value = &text[range.clone()];
            value.split_whitespace().map(move |name| {
                let start = range.start + (name.as_ptr() as usize - value.as_ptr() as usize);
                (start..start + name.len(), name)
            })
        })
        .filter(|(_, name)| name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
        .collect()
}

fn unknown_message(name: &str, known: &[String]) -> String {
    let mut message = format!("No Stimulus controller `{name}` in app/{CONTROLLERS_DIR}");
    let near = known
        .iter()
        .map(|candidate| (edit_distance(name, candidate), candidate))
        .filter(|&(distance, _)| distance <= MAX_SUGGESTION_DISTANCE && distance < name.chars().count())
        .min_by_key(|&(distance, _)| distance);
    if let Some((_, candidate)) = near {
        message.push_str(&format!("; did you mean `{candidate}`?"));
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::{helper_elements, markup_elements};
    use crate::code_context;
    use crate::lang::TemplateLang::{self, Erb, Haml, Slim};

    fn run(lang: TemplateLang, text: &str, controllers: Option<&[&str]>) -> Vec<(String, String)> {
        let version = ruby_prism::Version::Latest;
        let mut elements = markup_elements(lang, text, &[], version);
        let compiled = code_context::compiled(lang, text, None, &[]).unwrap();
        let options = ruby_prism::Options::default().version(version).build();
        let parsed = compiled.parse(&options).unwrap();
        elements.extend(helper_elements(&compiled, &parsed.node()));
        let index = LineIndex::new(text);
        let at = |position| (0..=text.len()).find(|&offset| index.position(offset) == position).unwrap();
        diagnostics(text, &elements, || {
            controllers.map(|names| Arc::new(Registered::new(names.iter().map(|name| name.to_string()).collect())))
        })
        .into_iter()
        .map(|d| (format!("{} {}", util::code(&d).unwrap(), d.message), text[at(d.range.start)..at(d.range.end)].to_string()))
        .collect()
    }

    fn permanent(lang: TemplateLang, text: &str) -> Vec<String> {
        let found = run(lang, text, None);
        found.into_iter().filter(|(code, _)| code.starts_with(PERMANENT_REQUIRES_ID)).map(|(_, flagged)| flagged).collect()
    }

    #[test]
    fn permanent_elements_without_an_id_are_flagged_on_the_attribute() {
        assert_eq!(permanent(Erb, "<div data-turbo-permanent>\n</div>\n"), ["data-turbo-permanent"]);
        assert_eq!(permanent(Erb, "<div class=\"a\" data-turbo-permanent=\"true\"></div>"), ["data-turbo-permanent"]);
        assert_eq!(permanent(Haml, "%div{data: {turbo_permanent: true}}\n"), ["turbo_permanent"]);
        assert_eq!(permanent(Haml, "%div{\"data-turbo-permanent\" => true}\n"), ["data-turbo-permanent"]);
        assert_eq!(permanent(Haml, "%div(data-turbo-permanent)\n"), ["data-turbo-permanent"]);
        assert_eq!(permanent(Slim, "div data-turbo-permanent=true\n"), ["data-turbo-permanent"]);
        assert_eq!(permanent(Slim, "div[data-turbo-permanent]\n"), ["data-turbo-permanent"]);
        assert_eq!(permanent(Erb, "<%= tag.div data: { turbo_permanent: true } do %>x<% end %>"), ["turbo_permanent"]);
        assert_eq!(permanent(Erb, "<%= form_with model: @user, data: { turbo_permanent: true } do |f| %><% end %>"), ["turbo_permanent"]);
        assert_eq!(
            permanent(Erb, "<%= form_with model: @user, html: { data: { turbo_permanent: true } } do |f| %><% end %>"),
            ["turbo_permanent"]
        );
        assert_eq!(permanent(Erb, "<%= content_tag :div, \"x\", data: { turbo_permanent: \"\" } %>"), ["turbo_permanent"]);
        assert_eq!(permanent(Haml, "= link_to \"x\", y, data: { turbo_permanent: true }\n"), ["turbo_permanent"]);
        assert_eq!(
            run(Erb, "<div data-turbo-permanent></div>", None)[0].0,
            "Turbo/PermanentRequiresId `data-turbo-permanent` needs an `id`: Turbo matches the element across pages by it"
        );
    }

    #[test]
    fn permanent_elements_with_an_id_or_that_may_have_one_are_left_alone() {
        for (lang, text) in [
            (Erb, "<div id=\"player\" data-turbo-permanent></div>"),
            (Erb, "<div data-turbo-permanent ID='p'></div>"),
            (Erb, "<div id=\"<%= dom_id(x) %>\" data-turbo-permanent></div>"),
            (Erb, "<div <%= tag_options %> data-turbo-permanent></div>"),
            (Erb, "<div data-turbo-permanent<%= x %>></div>"),
            (Erb, "<%= tag.div id: \"p\", data: { turbo_permanent: true } %>"),
            (Erb, "<%= tag.div data: { turbo_permanent: true }, **attrs %>"),
            (Erb, "<%= tag.div data: attrs %>"),
            (Erb, "<%= tag.div data: { turbo_permanent: flag } %>"),
            (Erb, "<%= tag.div data: { turbo_permanent: nil } %>"),
            (Erb, "<%= tag.div data: { turbo_permanent: } %>"),
            (Erb, "<%= form_for @user, html: { id: \"f\" }, data: { turbo_permanent: true } do |f| %><% end %>"),
            (Erb, "<%= form_for @user, data: { turbo_permanent: true } do |f| %><% end %>"),
            (Erb, "<%= form_with model: @user, html: opts, data: { turbo_permanent: true } do |f| %><% end %>"),
            (Erb, "<%= form_with model: @user, html: { id: \"f\", data: { turbo_permanent: true } } do |f| %><% end %>"),
            (Erb, "<%= render \"x\", data: { turbo_permanent: true } %>"),
            (Erb, "<%= link_to \"x\", y, id: \"l\", data: { turbo_permanent: true } %>"),
            (Erb, "<div data-turbo-permanent=\"<%= x %>\" id=\"p\"></div>"),
            (Erb, "<div><!-- <p data-turbo-permanent> --></div>"),
            (Erb, "<script>var s = '<p data-turbo-permanent>'</script>"),
            (Erb, "<div data-turbo-permanent=\"<%= x %>\"></div>"),
            (Haml, "#player{data: {turbo_permanent: true}}\n"),
            (Haml, "%div#player(data-turbo-permanent)\n"),
            (Haml, "%div{id: \"p\", data: {turbo_permanent: true}}\n"),
            (Haml, "%div{\"id\" => x, data: {turbo_permanent: true}}\n"),
            (Haml, "%div(id=\"p\" data-turbo-permanent)\n"),
            (Haml, "%div(id=p data-turbo-permanent)\n"),
            (Haml, "%div[@player]{data: {turbo_permanent: true}}\n"),
            (Haml, "%div{data: {turbo_permanent: true}, **attrs}\n"),
            (Haml, "%div{data: attrs}\n"),
            (Haml, "%div{data: {turbo_permanent: flag}}\n"),
            (Haml, "%div{data: {turbo_permanent: false}}\n"),
            (Haml, "%div{data: {turbo: {permanent: true}}}\n"),
            (Slim, "#player data-turbo-permanent=true\n"),
            (Slim, "div id=\"player\" data-turbo-permanent=true\n"),
            (Slim, "div *attrs data-turbo-permanent=true\n"),
            (Slim, "div data={turbo_permanent: true}\n"),
            (Slim, "div data-turbo-permanent=flag\n"),
            (Slim, "p data-turbo-permanent=\"#{x}\"\n"),
        ] {
            assert_eq!(permanent(lang, text), Vec::<String>::new(), "{text}");
        }
    }

    fn controllers(lang: TemplateLang, text: &str, known: &[&str]) -> Vec<(String, String)> {
        run(lang, text, Some(known)).into_iter().filter(|(code, _)| code.starts_with(UNKNOWN_CONTROLLER)).collect()
    }

    #[test]
    fn unknown_controllers_are_flagged_with_the_nearest_name() {
        let found = controllers(Erb, "<div data-controller=\"serch hello\"></div>", &["hello", "search"]);
        assert_eq!(
            found,
            [(
                "Stimulus/UnknownController No Stimulus controller `serch` in app/javascript/controllers; did you mean `search`?".into(),
                "serch".into()
            )]
        );
        let far = controllers(Erb, "<div data-controller=\"x\"></div>", &["hello"]);
        assert_eq!(far[0].0, "Stimulus/UnknownController No Stimulus controller `x` in app/javascript/controllers");
        for (lang, text) in [
            (Haml, "%div{data: {controller: \"serch\"}}\n"),
            (Haml, "%div{\"data-controller\" => \"serch\"}\n"),
            (Haml, "%div(data-controller=\"serch\")\n"),
            (Slim, "div data-controller=\"serch\"\n"),
            (Erb, "<%= tag.div data: { controller: \"serch\" } %>"),
            (Erb, "<%= content_tag :div, data: { controller: :serch } %>"),
            (Erb, "<%= form_with model: x, data: { controller: \"serch\" } do |f| %><% end %>"),
        ] {
            let found = controllers(lang, text, &["search"]);
            assert_eq!(found.iter().map(|(_, flagged)| flagged.as_str()).collect::<Vec<_>>(), ["serch"], "{text}");
        }
    }

    #[test]
    fn known_dynamic_and_unreadable_controller_names_are_left_alone() {
        for (lang, text) in [
            (Erb, "<div data-controller=\"search\"></div>"),
            (Erb, "<div data-controller=\"hello  admin--users\"></div>"),
            (Erb, "<div data-controller=\"<%= name %> search\"></div>"),
            (Erb, "<div data-controller=\"<%= admin ? 'a' : 'b' %>\"></div>"),
            (Erb, "<div <%= attrs %> data-controller=\"search\"></div>"),
            (Erb, "<%= tag.div data: { controller: name } %>"),
            (Erb, "<%= tag.div data: { controller: \"a#{b}\" } %>"),
            (Erb, "<%= render \"x\", data: { controller: \"nope\" } %>"),
            (Erb, "<%= link_to \"x\", y, controller: \"users\" %>"),
            (Erb, "<p data-action=\"nope#go\" data-nope-target=\"x\"></p>"),
            (Erb, "<div data-controller=\"a:b\"></div>"),
            (Haml, "%div{data: {controller: \"search\"}}\n"),
            (Haml, "%div{data: {controller: \"s#{x}\"}}\n"),
            (Haml, "%div{data: {controller: name}}\n"),
            (Slim, "div data-controller=\"s#{x}\"\n"),
            (Slim, "div data-controller=name\n"),
        ] {
            assert_eq!(controllers(lang, text, &["search", "hello", "admin--users"]), Vec::new(), "{text}");
        }
    }

    #[test]
    fn nothing_is_reported_when_the_controllers_cannot_be_told() {
        assert_eq!(run(Erb, "<div data-controller=\"nope\"></div>", None), Vec::new());
    }
}
