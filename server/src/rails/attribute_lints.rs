//! Lints on the attributes an element is written with: `data-turbo-permanent` without an `id`, a `data-controller` name that
//! no Stimulus controller has, and a target or action that names something its controller doesn't have.

use crate::attribute_insert::empty_attribute;
use crate::attributes::{Element, Value};
use crate::lang::TemplateLang;
use crate::rails::stimulus::{is_name_char, Registered, CONTROLLERS_DIR};
use crate::rails::stimulus_controller::Members;
use crate::util::{self, edit_distance, LineIndex};
use serde_json::json;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use tower_lsp_server::ls_types::{Diagnostic, TextEdit};

pub(crate) const PERMANENT_REQUIRES_ID: &str = "Turbo/PermanentRequiresId";
pub(crate) const UNKNOWN_CONTROLLER: &str = "Stimulus/UnknownController";
pub(crate) const UNKNOWN_TARGET_OR_ACTION: &str = "Stimulus/UnknownTargetOrAction";
/// The codes this module reports.
pub(crate) const LINTS: [&str; 3] = [PERMANENT_REQUIRES_ID, UNKNOWN_CONTROLLER, UNKNOWN_TARGET_OR_ACTION];

const PERMANENT: &str = "data-turbo-permanent";
const CONTROLLER: &str = "data-controller";
const ACTION: &str = "data-action";
const ID: &str = "id";
/// A name this many edits from the typed one is offered as the one meant.
const MAX_SUGGESTION_DISTANCE: usize = 2;

/// The findings in `elements`. `registered` lists the controllers the app has and is asked only when a template names one;
/// none means they can't be told, and no name is reported. `controller` reads what the controller of an identifier declares;
/// none means it can't be read with confidence, and nothing is reported for it.
pub(crate) fn diagnostics(
    lang: TemplateLang,
    text: &str,
    elements: &[Element],
    registered: impl FnOnce() -> Option<Arc<Registered>>,
    controller: impl Fn(&str) -> Option<Arc<Members>>,
) -> Vec<Diagnostic> {
    let index = LineIndex::new(text);
    let message = "`data-turbo-permanent` needs an `id`: Turbo matches the element across pages by it";
    let mut out: Vec<Diagnostic> = elements
        .iter()
        .filter_map(|element| Some((element, permanent_without_id(element)?)))
        .map(|(element, at)| {
            let found = util::coded(util::warning(index.range(at.start, at.end), message), PERMANENT_REQUIRES_ID);
            match empty_attribute(lang, text, element, ID) {
                Some((offset, added)) => util::with_data(found, json!({ "edit": TextEdit::new(index.range(offset, offset), added) })),
                None => found,
            }
        })
        .collect();
    let named: Vec<(Range<usize>, &str)> = elements.iter().flat_map(|element| controller_names(text, element)).collect();
    if let Some(known) = (!named.is_empty()).then(registered).flatten() {
        let unknown = named.into_iter().filter(|(_, name)| !known.knows(name));
        out.extend(unknown.map(|(at, name)| {
            let near = suggestion(name, &known.names);
            let message = format!("No Stimulus controller `{name}` in app/{CONTROLLERS_DIR}{}", did_you_mean(near));
            let found = util::coded(util::warning(index.range(at.start, at.end), message), UNKNOWN_CONTROLLER);
            util::with_data(found, json!({ "controller": name, "suggest": near }))
        }));
    }
    let mut controllers: HashMap<&str, Option<Arc<Members>>> = HashMap::new();
    for mention in elements.iter().flat_map(|element| mentions(text, element)) {
        let members = controllers.entry(mention.controller).or_insert_with(|| controller(mention.controller));
        let Some((message, near)) = members.as_deref().and_then(|members| mention.unknown_message(members)) else { continue };
        let found = util::coded(util::warning(index.range(mention.at.start, mention.at.end), message), UNKNOWN_TARGET_OR_ACTION);
        let kind = if mention.is_target { "target" } else { "method" };
        out.push(util::with_data(found, json!({ "controller": mention.controller, "kind": kind, "name": mention.name, "suggest": near })));
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
            let start = range.start;
            words(&text[range]).map(move |(offset, name)| (start + offset..start + offset + name.len(), name))
        })
        .filter(|(_, name)| name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
        .collect()
}

/// The space-separated words of a value, each with its offset in it.
fn words(value: &str) -> impl Iterator<Item = (usize, &str)> {
    value.split_whitespace().map(move |word| (word.as_ptr() as usize - value.as_ptr() as usize, word))
}

/// The name of `known` nearest to `name`, when one is near enough to be the one meant.
fn suggestion<'a>(name: &str, known: &'a [String]) -> Option<&'a str> {
    let near = known
        .iter()
        .map(|candidate| (edit_distance(name, candidate), candidate))
        .filter(|&(distance, _)| distance <= MAX_SUGGESTION_DISTANCE && distance < name.chars().count())
        .min_by_key(|&(distance, _)| distance);
    near.map(|(_, candidate)| candidate.as_str())
}

fn did_you_mean(near: Option<&str>) -> String {
    near.map_or_else(String::new, |candidate| format!("; did you mean `{candidate}`?"))
}

/// A target or a method an element names for the Stimulus controller `controller`.
struct Mention<'a> {
    controller: &'a str,
    name: &'a str,
    at: Range<usize>,
    is_target: bool,
}

impl Mention<'_> {
    /// What is wrong with the mention, when `members` of its controller lack the name, and the name nearest to it.
    fn unknown_message<'m>(&self, members: &'m Members) -> Option<(String, Option<&'m str>)> {
        let (kind, known, declared) = if self.is_target {
            ("target", members.targets.as_slice(), members.targets.iter().any(|target| target == self.name))
        } else {
            ("method", members.methods.as_slice(), members.has_method(self.name))
        };
        let controller = self.controller;
        let near = suggestion(self.name, known);
        (!declared).then(|| (format!("`{controller}` controller has no {kind} `{}`{}", self.name, did_you_mean(near)), near))
    }
}

/// The targets a static `data-<controller>-target` names and the methods a static `data-action` calls, where each is written.
fn mentions<'a>(text: &'a str, element: &'a Element) -> Vec<Mention<'a>> {
    let mut found = Vec::new();
    for attribute in &element.attributes {
        let Value::Text(range) = &attribute.value else { continue };
        let at = |offset: usize, name: &str| range.start + offset..range.start + offset + name.len();
        let words = words(&text[range.clone()]);
        if let Some(controller) = attribute.name.strip_prefix("data-").and_then(|name| name.strip_suffix("-target")) {
            found.extend(words.map(|(offset, name)| Mention { controller, name, at: at(offset, name), is_target: true }));
        } else if attribute.name == ACTION {
            for (offset, descriptor) in words {
                let Some((controller, method_offset, name)) = action_method(descriptor) else { continue };
                found.push(Mention { controller, name, at: at(offset + method_offset, name), is_target: false });
            }
        }
    }
    found
}

/// The controller identifier and the method of an action descriptor (`click->search#go:prevent`, `search#go`), with the
/// method's offset in the descriptor.
fn action_method(descriptor: &str) -> Option<(&str, usize, &str)> {
    let start = descriptor.find("->").map_or(0, |at| at + "->".len());
    let (controller, after) = descriptor[start..].split_once('#')?;
    let method = after.split(':').next().filter(|method| !method.is_empty() && method.chars().all(is_name_char))?;
    Some((controller, start + controller.len() + "#".len(), method))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::{helper_elements, markup_elements};
    use crate::code_context;
    use crate::lang::TemplateLang::{self, Erb, Haml, Slim};

    fn run(lang: TemplateLang, text: &str, controllers: Option<&[&str]>) -> Vec<(String, String)> {
        run_with(lang, text, controllers, |_| None)
    }

    fn run_with(
        lang: TemplateLang,
        text: &str,
        controllers: Option<&[&str]>,
        members: impl Fn(&str) -> Option<Arc<Members>>,
    ) -> Vec<(String, String)> {
        let version = ruby_prism::Version::Latest;
        let mut elements = markup_elements(lang, text, &[], version);
        let compiled = code_context::compiled(lang, text, None, &[]).unwrap();
        let options = ruby_prism::Options::default().version(version).build();
        let parsed = compiled.parse(&options).unwrap();
        elements.extend(helper_elements(&compiled, &parsed.node()));
        let index = LineIndex::new(text);
        let at = |position| (0..=text.len()).find(|&offset| index.position(offset) == position).unwrap();
        let registered = || controllers.map(|names| Arc::new(Registered::new(names.iter().map(|name| name.to_string()).collect())));
        diagnostics(lang, text, &elements, registered, members)
            .into_iter()
            .map(|d| (format!("{} {}", util::code(&d).unwrap(), d.message), text[at(d.range.start)..at(d.range.end)].to_string()))
            .collect()
    }

    /// The first diagnostic with `code`, with the text the quick fix that its data describes makes of `text`.
    fn fixed_by(lang: TemplateLang, text: &str, code: &str, members: impl Fn(&str) -> Option<Arc<Members>>) -> (Diagnostic, String) {
        let version = ruby_prism::Version::Latest;
        let mut elements = markup_elements(lang, text, &[], version);
        let compiled = code_context::compiled(lang, text, None, &[]).unwrap();
        let options = ruby_prism::Options::default().version(version).build();
        let parsed = compiled.parse(&options).unwrap();
        elements.extend(helper_elements(&compiled, &parsed.node()));
        let registered = || Some(Arc::new(Registered::new(vec!["search".to_string()])));
        let found = diagnostics(lang, text, &elements, registered, members).into_iter().find(|d| util::code(d) == Some(code)).unwrap();
        let edit = util::data_edit(&found);
        let fixed = edit.map_or_else(String::new, |edit| util::edited(text, &edit));
        (found, fixed)
    }

    #[test]
    fn permanent_elements_carry_the_edit_that_adds_an_id() {
        let fix = |lang, text| fixed_by(lang, text, PERMANENT_REQUIRES_ID, |_| None).1;
        assert_eq!(fix(Erb, "<div data-turbo-permanent>\n</div>\n"), "<div id=\"\" data-turbo-permanent>\n</div>\n");
        assert_eq!(
            fix(Erb, "<%= tag.div data: { turbo_permanent: true } do %>x<% end %>"),
            "<%= tag.div data: { turbo_permanent: true }, id: \"\" do %>x<% end %>"
        );
        assert_eq!(fix(Haml, "%div{data: {turbo_permanent: true}}\n"), "%div{data: {turbo_permanent: true}, id: \"\"}\n");
        assert_eq!(fix(Haml, ".card(data-turbo-permanent)\n"), ".card(data-turbo-permanent id=\"\")\n");
        assert_eq!(fix(Slim, "div data-turbo-permanent=true\n"), "div id=\"\" data-turbo-permanent=true\n");
    }

    #[test]
    fn names_carry_what_the_fixes_need() {
        let data = |lang, text, code| fixed_by(lang, text, code, search_only).0.data;
        assert_eq!(
            data(Erb, "<div data-controller=\"serch\"></div>", UNKNOWN_CONTROLLER),
            Some(json!({ "controller": "serch", "suggest": "search" }))
        );
        assert_eq!(
            data(Erb, "<div data-controller=\"zzzzzz\"></div>", UNKNOWN_CONTROLLER),
            Some(json!({ "controller": "zzzzzz", "suggest": null }))
        );
        assert_eq!(
            data(Erb, "<p data-search-target=\"resluts\"></p>", UNKNOWN_TARGET_OR_ACTION),
            Some(json!({ "controller": "search", "kind": "target", "name": "resluts", "suggest": "results" }))
        );
        assert_eq!(
            data(Haml, "%p{data: {action: \"search#submitt\"}}\n", UNKNOWN_TARGET_OR_ACTION),
            Some(json!({ "controller": "search", "kind": "method", "name": "submitt", "suggest": "submit" }))
        );
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

    /// `search` has the targets `query` and `results` and the methods `submit` and `reset`; no other controller can be read.
    fn search_only(identifier: &str) -> Option<Arc<Members>> {
        let list = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
        (identifier == "search").then(|| Arc::new(Members { targets: list(&["query", "results"]), methods: list(&["submit", "reset"]) }))
    }

    fn mentions_in(lang: TemplateLang, text: &str) -> Vec<(String, String)> {
        let found = run_with(lang, text, Some(&["search"]), search_only);
        found.into_iter().filter(|(code, _)| code.starts_with(UNKNOWN_TARGET_OR_ACTION)).collect()
    }

    #[test]
    fn unknown_targets_and_methods_are_flagged_with_the_nearest_name() {
        let found = mentions_in(Erb, "<input data-search-target=\"resluts\" data-action=\"input->search#submitt\">");
        assert_eq!(
            found,
            [
                (
                    "Stimulus/UnknownTargetOrAction `search` controller has no target `resluts`; did you mean `results`?".into(),
                    "resluts".into()
                ),
                (
                    "Stimulus/UnknownTargetOrAction `search` controller has no method `submitt`; did you mean `submit`?".into(),
                    "submitt".into()
                ),
            ]
        );
        let far = mentions_in(Erb, "<p data-search-target=\"zzzzzz\"></p>");
        assert_eq!(far[0].0, "Stimulus/UnknownTargetOrAction `search` controller has no target `zzzzzz`");
        for (lang, text, flagged) in [
            (Erb, "<p data-search-target=\"query other\"></p>", vec!["other"]),
            (Erb, "<p data-action=\"search#go\"></p>", vec!["go"]),
            (Erb, "<p data-action=\"click->search#submit  keydown.enter->search#nope:prevent\"></p>", vec!["nope"]),
            (Erb, "<p data-action='resize@window->search#nope scroll@document->search#reset'></p>", vec!["nope"]),
            (Erb, "<p data-action=\"click->search#go:stop:once\"></p>", vec!["go"]),
            (Erb, "<%= tag.input data: { search_target: \"x\", action: \"search#go\" } %>", vec!["x", "go"]),
            (Erb, "<%= text_field_tag :q, nil, data: { action: \"input->search#go\" } %>", vec![]),
            (Erb, "<%= link_to \"x\", y, data: { action: \"search#go\" } %>", vec!["go"]),
            (Erb, "<%= form_with model: x, data: { search_target: \"x\" } do |f| %><% end %>", vec!["x"]),
            (Haml, "%input{data: {search_target: \"x\", action: \"click->search#go\"}}\n", vec!["x", "go"]),
            (Haml, "%input{\"data-search-target\" => \"x\"}\n", vec!["x"]),
            (Haml, "%input(data-action=\"search#go\")\n", vec!["go"]),
            (Slim, "input data-search-target=\"x\" data-action=\"search#go\"\n", vec!["x", "go"]),
            (Slim, "input[data-search-target=\"x\"]\n", vec!["x"]),
        ] {
            let found = mentions_in(lang, text);
            assert_eq!(found.iter().map(|(_, flagged)| flagged.as_str()).collect::<Vec<_>>(), flagged, "{text}");
        }
    }

    #[test]
    fn known_dynamic_and_unreadable_targets_and_methods_are_left_alone() {
        for (lang, text) in [
            (Erb, "<p data-search-target=\"query results\" data-action=\"click->search#submit search#reset\"></p>"),
            (Erb, "<p data-action=\"click->search#connect search#disconnect search#initialize\"></p>"),
            (Erb, "<p data-search-target=\"<%= name %>\" data-action=\"search#<%= method %>\"></p>"),
            (Erb, "<p data-action=\"search#a<%= b %>\" data-search-target=\"a<%= b %>\"></p>"),
            (Erb, "<%= tag.p data: { search_target: name, action: \"search##{name}\" } %>"),
            (Erb, "<p data-other-target=\"x\" data-action=\"click->other#go\"></p>"),
            (Erb, "<p data-bs-target=\"#modal\" data-bs-toggle=\"modal\"></p>"),
            (Erb, "<p data-action=\"click->search\"></p>"),
            (Erb, "<p data-action=\"click->search#\"></p>"),
            (Erb, "<p data-action=\"search#a-b\"></p>"),
            (Erb, "<p data-action=\"Search#x\" data-Other-target=\"x\"></p>"),
            (Erb, "<p data-search_target=\"x\"></p>"),
            (Erb, "<p data-search-target></p>"),
            (Erb, "<p data-search-targets=\"x\" data-search-target-value=\"x\"></p>"),
            (Erb, "<%= render \"x\", data: { search_target: \"x\", action: \"search#go\" } %>"),
            (Haml, "%input{data: {search_target: name, action: \"click->search##{method}\"}}\n"),
            (Slim, "input data-action=\"search##{method}\" data-search-target=name\n"),
        ] {
            assert_eq!(mentions_in(lang, text), Vec::new(), "{text}");
        }
    }

    #[test]
    fn targets_and_methods_are_not_looked_up_for_controllers_that_cannot_be_read() {
        assert_eq!(run(Erb, "<p data-search-target=\"nope\" data-action=\"search#nope\"></p>", Some(&["search"])), Vec::new());
    }
}
