//! An element's attributes as a template spells them, read alike from an ERB start tag, a Haml or Slim tag and the options
//! of a Rails tag helper, for the rules that look for an attribute or rename one.

use crate::code_context;
use crate::erb::html_lint;
use crate::lang::TemplateLang;
use crate::rails::output_lints::TAG_BUILDER_ELEMENTS;
use crate::ruby::generated::Compiled;
use crate::util::BOM;
use crate::{haml, slim};
use ruby_prism::{CallNode, Location, Node, Visit};
use std::ops::Range;

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Element {
    /// Lower-case: `a`, `div`.
    pub name: String,
    /// Where the name is written; empty when it is implied (`.card` is a `div`).
    pub at: Range<usize>,
    pub attributes: Vec<Attribute>,
    /// The text that is all of the element's content, when the template spells it out: no ERB, interpolation or child.
    pub text: Option<Range<usize>>,
    /// Haml and Slim: what follows the tag on its line, text or `=` code, from its first character to its last; none when it
    /// is nothing, when the line goes on or when the tag is nested on a line by `:` (`li: a x`).
    pub content: Option<Range<usize>>,
    /// ERB in the tag, a splat or a hash that isn't a literal may set attributes this reading doesn't see.
    pub open: bool,
    /// A Rails helper called with a block, which writes the element's content.
    pub block: bool,
    /// Markup: where the attribute wrappers or the attributes begin, after the name and the `.class#id` shorthand.
    pub head_end: Option<usize>,
    /// A Rails helper: the end of its last argument, or of the last entry of its braced options, where `, key: value` can follow.
    pub last_argument_end: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Attribute {
    /// As HTML spells it, lower-case: `data-controller`, also for `controller:` in a `data: {}` hash.
    pub name: String,
    /// Where the name is written; empty when the attribute is implied (an object reference sets the id) or can't be placed.
    pub at: Range<usize>,
    /// Written as a key of a `data: {}` hash.
    pub in_data: bool,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    /// No value: `<div hidden>`.
    Empty,
    /// Text the template spells out, without interpolation or ERB.
    Text(Range<usize>),
    /// A Ruby `true` or number: the attribute is set, to nothing that can be read.
    Literal,
    Dynamic,
    /// A Ruby 3.1 `confirm:` with no value, which reads the variable of that name.
    Shorthand,
}

impl Element {
    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes.iter().find(|attribute| attribute.name == name)
    }

    /// Whether attributes may come from somewhere this reading doesn't see; `data` and `aria` attributes are hashes in Slim.
    pub fn may_have_more(&self) -> bool {
        self.open || self.attribute("data").is_some() || self.attribute("aria").is_some()
    }

    /// The same element `by` bytes further on.
    pub fn shifted(mut self, by: usize) -> Self {
        let shift = |range: &Range<usize>| range.start + by..range.end + by;
        self.at = shift(&self.at);
        self.text = self.text.as_ref().map(shift);
        self.content = self.content.as_ref().map(shift);
        self.head_end = self.head_end.map(|end| end + by);
        self.last_argument_end = self.last_argument_end.map(|end| end + by);
        for attribute in &mut self.attributes {
            attribute.at = shift(&attribute.at);
            if let Value::Text(text) = &mut attribute.value {
                *text = shift(text);
            }
        }
        self
    }
}

/// The elements the template writes as markup: an ERB template's HTML start tags, a Haml or Slim template's tags. Tag
/// helpers in Ruby code are `helper_elements`. None for a template its language's parser rejects.
pub(crate) fn markup_elements(lang: TemplateLang, text: &str, extra_engines: &[String], grammar: ruby_prism::Version) -> Vec<Element> {
    match lang {
        TemplateLang::Erb => erb_elements(text),
        TemplateLang::Haml => haml_elements(text, grammar),
        TemplateLang::Slim => slim_elements(text, extra_engines),
        _ => Vec::new(),
    }
}

/// Every element of the template: its markup and its tag helpers.
pub(crate) fn elements(lang: TemplateLang, text: &str, extra_engines: &[String], grammar: ruby_prism::Version) -> Vec<Element> {
    let mut found = markup_elements(lang, text, extra_engines, grammar);
    let Some(compiled) = code_context::compiled(lang, text, None, extra_engines) else { return found };
    let options = ruby_prism::Options::default().version(grammar).build();
    if let Some(parsed) = compiled.parse(&options) {
        found.extend(helper_elements(&compiled, &parsed.node()));
    }
    found
}

fn erb_elements(text: &str) -> Vec<Element> {
    let regions = html_lint::non_markup(text);
    let attribute = |attribute: html_lint::Attribute| {
        let value = match attribute.value {
            None => Value::Empty,
            Some((start, end)) if text[start..end].contains("<%") => Value::Dynamic,
            Some((start, end)) => Value::Text(start..end),
        };
        Attribute { name: attribute.name, at: attribute.at.0..attribute.at.1, in_data: false, value }
    };
    let element = |tag: html_lint::StartTag| Element {
        at: tag.start + "<".len()..tag.start + "<".len() + tag.name.len(),
        text: tag.text.map(|(start, end)| start..end),
        content: None,
        head_end: Some(tag.start + "<".len() + tag.name.len()),
        name: tag.name,
        open: tag.dynamic,
        block: false,
        last_argument_end: None,
        attributes: tag.attrs.into_iter().filter(|attribute| !attribute.name.is_empty()).map(attribute).collect(),
    };
    html_lint::start_tags(text, &regions).into_iter().map(element).collect()
}

fn slim_elements(text: &str, extra_engines: &[String]) -> Vec<Element> {
    let parsed = slim::parser::parse(text, extra_engines);
    if parsed.valid {
        parsed.elements
    } else {
        Vec::new()
    }
}

fn haml_elements(text: &str, grammar: ruby_prism::Version) -> Vec<Element> {
    let parsed = haml::parser::parse_with_ruby(text);
    if parsed.error.is_some() {
        return Vec::new();
    }
    let bom_len = if text.starts_with(BOM) { BOM.len_utf8() } else { 0 };
    let options = ruby_prism::Options::default().version(grammar).build();
    let element = |tag: haml::parser::Tag| {
        let mut element = tag.element.shifted(bom_len);
        element.head_end = (tag.head.end > 0).then_some(tag.head.end + bom_len);
        if !tag.hash.is_empty() {
            let compiled = haml::ruby::attribute_hash(&tag.hash, text);
            read_haml_hash(&mut element, &compiled, &options);
        }
        element
    };
    parsed.tags.into_iter().map(element).collect()
}

fn read_haml_hash(element: &mut Element, compiled: &Compiled, options: &ruby_prism::ParseOptions) {
    let Some(parsed) = compiled.parse(options) else {
        element.open = true;
        return;
    };
    let root = parsed.node();
    let hash = root.as_program_node().and_then(|program| program.statements().body().iter().next());
    match hash.as_ref().and_then(|node| node.as_hash_node()) {
        Some(hash) => read_hash(element, hash.elements().iter(), compiled, None),
        None => element.open = true,
    }
}

/// Hash attributes whose keys are attributes with the hash's name as prefix: `data: { a: 1 }` is `data-a`.
const PREFIXED_HASHES: [&str; 2] = ["data", "aria"];

/// The attributes the entries of a hash or keyword arguments set, by literal keys; `prefix` is the `data` or `aria` hash
/// they are in. A key that is no literal, a `**splat` and a `data:` or `aria:` that is no hash may set any.
fn read_hash<'pr>(element: &mut Element, entries: impl Iterator<Item = Node<'pr>>, compiled: &Compiled, prefix: Option<&str>) {
    for entry in entries {
        let Some(assoc) = entry.as_assoc_node() else {
            element.open = true;
            continue;
        };
        let Some((key, at)) = literal_name(&assoc.key(), compiled) else {
            element.open = true;
            continue;
        };
        let value = assoc.value();
        if let Some(hash) = PREFIXED_HASHES.iter().find(|hash| key == **hash).filter(|_| prefix.is_none()) {
            match value.as_hash_node() {
                Some(nested) => read_hash(element, nested.elements().iter(), compiled, Some(hash)),
                None => element.open = true,
            }
            continue;
        }
        let name = prefix.map_or(key.clone(), |prefix| format!("{prefix}-{}", key.replace('_', "-")));
        let in_data = prefix == Some("data");
        element.attributes.push(Attribute { name: name.to_ascii_lowercase(), at, in_data, value: value_of(&value, compiled) });
    }
}

/// The template range of generated bytes the template wrote.
pub(crate) fn span(compiled: &Compiled, at: &Location) -> Option<Range<usize>> {
    compiled.template_span(at.start() as usize, at.end() as usize).map(|(start, end)| start..end)
}

/// The text of a symbol or string literal and where the template spells it, when the source is the text itself.
pub(crate) fn literal_name(node: &Node, compiled: &Compiled) -> Option<(String, Range<usize>)> {
    let (text, at) = match (node.as_symbol_node(), node.as_string_node()) {
        (Some(symbol), _) => (symbol.unescaped().to_vec(), symbol.content_loc()),
        (_, Some(string)) => (string.unescaped().to_vec(), string.content_loc()),
        _ => return None,
    };
    let range = span(compiled, &at)?;
    (range.len() == text.len()).then(|| (String::from_utf8_lossy(&text).into_owned(), range))
}

fn value_of(node: &Node, compiled: &Compiled) -> Value {
    if node.as_implicit_node().is_some() {
        return Value::Shorthand;
    }
    if let Some((_, range)) = literal_name(node, compiled) {
        return Value::Text(range);
    }
    let is_literal = node.as_true_node().is_some() || node.as_integer_node().is_some() || node.as_float_node().is_some();
    if is_literal {
        Value::Literal
    } else {
        Value::Dynamic
    }
}

/// The elements Rails tag helpers make from the Ruby of a template (`root`, the parse of `compiled`): `tag.div`,
/// `content_tag`, `link_to`, the form and button helpers, with the attributes of their literal options.
pub(crate) fn helper_elements(compiled: &Compiled, root: &Node) -> Vec<Element> {
    let mut helpers = Helpers { compiled, found: Vec::new(), builders: Vec::new() };
    helpers.visit(root);
    helpers.found
}

struct Helpers<'a> {
    compiled: &'a Compiled,
    found: Vec<Element>,
    /// The form builders in scope, innermost last.
    builders: Vec<String>,
}

impl Helpers<'_> {
    fn element(&self, call: &CallNode) -> Option<Element> {
        let name = String::from_utf8_lossy(call.name().as_slice()).into_owned();
        let arguments: Vec<Node> = call.arguments().map(|list| list.arguments().iter().collect()).unwrap_or_default();
        let element_name = match call.receiver() {
            None => match name.as_str() {
                "link_to" => "a".to_string(),
                "image_tag" => "img".to_string(),
                "button_to" | "button_tag" | "submit_tag" => "button".to_string(),
                "form_with" | "form_for" | "form_tag" => "form".to_string(),
                "content_tag" => arguments.first().and_then(|node| literal_name(node, self.compiled).map(|(text, _)| text))?,
                _ => return None,
            },
            Some(receiver) if is_tag_builder(&receiver) && TAG_BUILDER_ELEMENTS.contains(&name.as_str()) => name.clone(),
            Some(receiver) if matches!(name.as_str(), "submit" | "button") && self.is_form_builder(&receiver) => "button".to_string(),
            Some(_) => return None,
        };
        let is_form = element_name == "form" && call.receiver().is_none();
        let is_content_tag = call.receiver().is_none() && name == "content_tag";
        let text_argument = usize::from(is_content_tag);
        let at = if is_content_tag {
            arguments.first().and_then(|node| literal_name(node, self.compiled)).map(|(_, at)| at)
        } else {
            call.message_loc().and_then(|loc| span(self.compiled, &loc))
        };
        let text = arguments.get(text_argument).filter(|_| call.block().is_none()).and_then(|node| literal_name(node, self.compiled));
        let mut element = Element {
            name: element_name.to_ascii_lowercase(),
            at: at.unwrap_or_default(),
            text: text.map(|(_, range)| range),
            block: call.block().is_some_and(|block| block.as_block_node().is_some()),
            last_argument_end: self.last_argument_end(&arguments),
            ..Element::default()
        };
        let options = arguments.last().and_then(|node| {
            let keywords = node.as_keyword_hash_node().map(|hash| hash.elements().iter().collect::<Vec<_>>());
            keywords.or_else(|| node.as_hash_node().map(|hash| hash.elements().iter().collect()))
        });
        // The second argument of `image_tag` is its options: a variable there may hold `alt`.
        element.open = name == "image_tag" && arguments.len() > 1 && options.is_none();
        let entries = options.unwrap_or_default();
        // A form's `html:` options are the form tag's own attributes, and `form_for` always gives the form an id.
        let (html, rest): (Vec<Node>, Vec<Node>) = entries.into_iter().partition(|entry| is_form && self.is_html_option(entry));
        read_hash(&mut element, rest.into_iter(), self.compiled, None);
        for entry in html {
            match entry
                .as_assoc_node()
                .map(|assoc| assoc.value())
                .and_then(|value| value.as_hash_node().map(|hash| hash.elements().iter().collect::<Vec<_>>()))
            {
                Some(nested) => read_hash(&mut element, nested.into_iter(), self.compiled, None),
                None => element.open = true,
            }
        }
        if is_form && name == "form_for" {
            element.attributes.push(Attribute { name: "id".into(), at: 0..0, in_data: false, value: Value::Dynamic });
        }
        Some(element)
    }

    fn last_argument_end(&self, arguments: &[Node]) -> Option<usize> {
        let (start, end) = last_argument_bytes(arguments)?;
        self.compiled.template_span(start, end).map(|(_, end)| end)
    }

    fn is_html_option(&self, entry: &Node) -> bool {
        let key = entry.as_assoc_node().and_then(|assoc| literal_name(&assoc.key(), self.compiled));
        key.is_some_and(|(key, _)| key == "html")
    }

    /// The block parameter of an enclosing form helper: `f` in `form_with … do |f|`.
    fn is_form_builder(&self, receiver: &Node) -> bool {
        let name = receiver.as_local_variable_read_node().map(|read| String::from_utf8_lossy(read.name().as_slice()).into_owned());
        name.is_some_and(|name| self.builders.contains(&name))
    }
}

/// The generated bytes of the last argument, or of the last entry when it is a braced hash.
pub(crate) fn last_argument_bytes(arguments: &[Node]) -> Option<(usize, usize)> {
    let last = arguments.last()?;
    let at = match last.as_hash_node() {
        Some(hash) => hash.elements().iter().last()?.location(),
        None => last.location(),
    };
    Some((at.start() as usize, at.end() as usize))
}

const FORM_HELPERS: [&str; 4] = ["form_with", "form_for", "form_tag", "fields_for"];

/// The names a form helper's block gives the form builder.
fn builder_parameters(call: &CallNode) -> Vec<String> {
    if !FORM_HELPERS.contains(&String::from_utf8_lossy(call.name().as_slice()).as_ref()) {
        return Vec::new();
    }
    let list = call.block().and_then(|block| block.as_block_node()?.parameters()?.as_block_parameters_node()?.parameters());
    let required = list.iter().flat_map(|list| list.requireds().iter());
    required
        .filter_map(|node| node.as_required_parameter_node().map(|p| String::from_utf8_lossy(p.name().as_slice()).into_owned()))
        .collect()
}

/// The `tag` of `tag.div`: the view helper, not a local variable or a call with a receiver.
fn is_tag_builder(node: &Node) -> bool {
    node.as_call_node().is_some_and(|call| call.receiver().is_none() && call.is_variable_call() && call.name().as_slice() == b"tag")
}

impl<'pr> Visit<'pr> for Helpers<'_> {
    fn visit_call_node(&mut self, call: &CallNode<'pr>) {
        self.found.extend(self.element(call));
        let scope = self.builders.len();
        self.builders.extend(builder_parameters(call));
        ruby_prism::visit_call_node(self, call);
        self.builders.truncate(scope);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TemplateLang::{Erb, Haml, Slim};

    fn markup(lang: TemplateLang, text: &str) -> Vec<Element> {
        markup_elements(lang, text, &[], ruby_prism::Version::Latest)
    }

    fn helpers(lang: TemplateLang, text: &str) -> Vec<Element> {
        let compiled = code_context::compiled(lang, text, None, &[]).unwrap();
        let options = ruby_prism::Options::default().version(ruby_prism::Version::Latest).build();
        let parsed = compiled.parse(&options).unwrap();
        helper_elements(&compiled, &parsed.node())
    }

    /// The names of an element's attributes, each with the text it is written as and its value's text.
    fn spelled<'a>(text: &'a str, element: &'a Element) -> Vec<(&'a str, &'a str, Option<&'a str>)> {
        let value = |value: &Value| match value {
            Value::Text(range) => Some(&text[range.clone()]),
            _ => None,
        };
        element.attributes.iter().map(|a| (a.name.as_str(), &text[a.at.clone()], value(&a.value))).collect()
    }

    #[test]
    fn erb_start_tags_keep_where_each_name_is_written() {
        let text = "<div class=\"a\" data-controller=\"x y\" hidden DATA-Turbo=\"<%= v %>\"></div>";
        let elements = markup(Erb, text);
        assert_eq!(elements.len(), 1);
        assert_eq!(
            spelled(text, &elements[0]),
            [
                ("class", "class", Some("a")),
                ("data-controller", "data-controller", Some("x y")),
                ("hidden", "hidden", None),
                ("data-turbo", "DATA-Turbo", None)
            ]
        );
        assert_eq!(elements[0].attributes[3].value, Value::Dynamic);
        assert!(!elements[0].open);
        assert!(markup(Erb, "<div <%= attrs %>>").remove(0).open);
    }

    #[test]
    fn haml_tags_read_the_shorthand_the_list_and_the_hash() {
        let text = "%a#main.x(href=\"/y\" data-confirm=\"Sure?\" hidden title=@t){data: {method: :delete}, \"data-a\" => 1, id: nil}\n";
        let elements = markup(Haml, text);
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].name, "a");
        assert_eq!(
            spelled(text, &elements[0]),
            [
                ("id", "main", Some("main")),
                ("href", "href", Some("/y")),
                ("data-confirm", "data-confirm", Some("Sure?")),
                ("hidden", "hidden", None),
                ("title", "title", None),
                ("data-method", "method", Some("delete")),
                ("data-a", "data-a", None),
                ("id", "id", None),
            ]
        );
        assert!(elements[0].attributes[6..].iter().all(|a| a.value == Value::Literal || a.value == Value::Dynamic));
        assert!(elements[0].attributes[5].in_data && !elements[0].attributes[6].in_data);
        assert!(!elements[0].open);
    }

    #[test]
    fn haml_hashes_that_are_not_literal_are_open() {
        for text in ["%a{**opts}\n", "%a{data: opts}\n", "%a{key => 1}\n", "%a{\"x#{y}\" => 1}\n"] {
            assert!(markup(Haml, text)[0].open, "{text}");
        }
        assert!(!markup(Haml, "%a{x: 1,\n  y: 2}\n")[0].open);
        let wrapped = "%a{x: 1,\n  data: {controller: \"c\"}}\n";
        let elements = markup(Haml, wrapped);
        assert_eq!(spelled(wrapped, &elements[0])[1], ("data-controller", "controller", Some("c")));
        assert!(markup(Haml, "%a(x=1)\n%b\n  %c.d\n").iter().all(|e| !e.open));
        assert!(markup(Haml, "\u{feff}%a{id: 1}\n")[0].attributes[0].at == (6..8));
        assert!(markup(Haml, "%a[@user]\n")[0].attribute("id").is_some());
        assert!(markup(Haml, "%a{\n").is_empty(), "a template Haml rejects");
    }

    #[test]
    fn slim_tags_read_shortcuts_quoted_and_ruby_values() {
        let text = "a#main.x href=\"/y\" data-confirm=\"Sure?\" hidden=true title=@t data-i=\"#{i}\"\n";
        let elements = markup(Slim, text);
        assert_eq!(elements[0].name, "a");
        assert_eq!(
            spelled(text, &elements[0]),
            [
                ("id", "main", Some("main")),
                ("href", "href", Some("/y")),
                ("data-confirm", "data-confirm", Some("Sure?")),
                ("hidden", "hidden", None),
                ("title", "title", None),
                ("data-i", "data-i", None),
            ]
        );
        assert_eq!(elements[0].attributes[3].value, Value::Literal);
        assert_eq!(elements[0].attributes[4].value, Value::Dynamic);
        assert_eq!(elements[0].attributes[5].value, Value::Dynamic);
        assert!(markup(Slim, "div *attrs\n")[0].open);
        assert!(markup(Slim, "div data={a: 1}\n")[0].may_have_more());
        let bracketed = "div[data-turbo-permanent id=\"p\"]\n";
        assert_eq!(markup(Slim, bracketed)[0].attributes[0].value, Value::Empty);
    }

    #[test]
    fn tag_helpers_make_elements_of_their_literal_options() {
        let text = "<%= tag.div data: { turbo_permanent: true, \"controller\": \"a b\" }, id: nil do %>x<% end %>";
        let elements = helpers(Erb, text);
        assert_eq!(elements.len(), 1);
        assert_eq!(
            spelled(text, &elements[0]),
            [("data-turbo-permanent", "turbo_permanent", None), ("data-controller", "controller", Some("a b")), ("id", "id", None)]
        );
        assert!(!elements[0].open);
        let names = |text: &str| helpers(Erb, text).into_iter().map(|e| (e.name, e.open)).collect::<Vec<_>>();
        assert_eq!(names("<%= link_to \"x\", path, **opts %>"), [("a".to_string(), true)]);
        assert_eq!(names("<%= content_tag :section, \"x\", class: \"c\" %>"), [("section".to_string(), false)]);
        assert_eq!(names("<%= form_with model: x do |f| %><%= f.submit \"Save\" %><%= button_to \"x\", y %><% end %>").len(), 3);
        assert_eq!(names("<%= f.submit \"Save\" %><%= f.button %>"), Vec::<(String, bool)>::new(), "f is no form builder here");
        assert_eq!(names("<%= form_with model: x do |f| %><% end %><%= f.submit %>"), [("form".to_string(), false)], "out of scope");
        assert_eq!(names("<%= form_with(model: x, html: y) %>"), [("form".to_string(), true)]);
        for none in ["<%= tag.name %>", "<%= content_tag name %>", "<%= render \"x\", data: { a: 1 } %>", "<% tag = 1 %><%= tag.div %>"] {
            assert_eq!(names(none), Vec::<(String, bool)>::new(), "{none}");
        }
    }
}
