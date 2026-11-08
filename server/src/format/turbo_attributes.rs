//! `Turbo/PreferTurboAttributes`: the Rails UJS attributes `data-confirm`, `data-method` and `data-disable-with` become
//! Turbo's, on links and submit buttons, in markup and in the options of Rails helpers. It runs before the other rules, whose
//! checks then guard the result against this one's text, so everything but the renames stays under them.

use crate::attributes::{self, Attribute, Element, Value};
use crate::erb::Edit;
use crate::lang::TemplateLang;
use std::collections::HashMap;

const CONFIRM: &str = "data-confirm";
const METHOD: &str = "data-method";
const DISABLE_WITH: &str = "data-disable-with";
const TURBO_CONFIRM: &str = "data-turbo-confirm";
const TURBO_METHOD: &str = "data-turbo-method";
const TURBO_SUBMITS_WITH: &str = "data-turbo-submits-with";
const DATA_PREFIX: &str = "data-";

const REFUSED: &str = "Turbo attributes kept: renaming them would change more than attribute names";

/// `text` with the attributes renamed; `Ok(None)` when there is nothing to rename, and a note when the rewrite is refused.
pub(super) fn apply(
    lang: TemplateLang,
    text: &str,
    extra_engines: &[String],
    grammar: ruby_prism::Version,
) -> Result<Option<String>, String> {
    let before = attributes::elements(lang, text, extra_engines, grammar);
    let mut renames: Vec<Rename> = before.iter().flat_map(|element| renames(text, element)).collect();
    renames.sort_by_key(|rename| rename.edit.0.start);
    if renames.is_empty() {
        return Ok(None);
    }
    if renames.windows(2).any(|pair| pair[0].edit.0.end > pair[1].edit.0.start) {
        return Err(REFUSED.to_string());
    }
    let mut renamed = text.to_string();
    for Rename { edit: (range, with), .. } in renames.iter().rev() {
        renamed.replace_range(range.clone(), with);
    }
    let after = attributes::elements(lang, &renamed, extra_engines, grammar);
    let names: HashMap<usize, &str> = renames.iter().map(|rename| (rename.edit.0.start, rename.html_name)).collect();
    if !only_names_differ(&before, &after, &names) {
        return Err(REFUSED.to_string());
    }
    Ok(Some(renamed))
}

struct Rename {
    edit: Edit,
    /// The attribute's new name as HTML spells it.
    html_name: &'static str,
}

/// The renames of one element's attributes.
fn renames(text: &str, element: &Element) -> Vec<Rename> {
    let link = element.name == "a";
    let has = |name: &str| element.attribute(name).is_some();
    let sends_a_method = has(METHOD) || has(TURBO_METHOD);
    element
        .attributes
        .iter()
        .filter(|attribute| !attribute.at.is_empty())
        .filter_map(|attribute| {
            let turbo = match attribute.name.as_str() {
                METHOD if link => TURBO_METHOD,
                // Turbo asks to confirm a link only when the link sends a method.
                CONFIRM if link && sends_a_method => TURBO_CONFIRM,
                DISABLE_WITH if is_submit(text, element) => TURBO_SUBMITS_WITH,
                _ => return None,
            };
            if has(turbo) {
                return None;
            }
            let with = new_spelling(&text[attribute.at.clone()], attribute, turbo)?;
            Some(Rename { edit: (attribute.at.clone(), with), html_name: turbo })
        })
        .collect()
}

/// A submit button: a `<button>` with no type or `type="submit"`, an `<input type="submit">`.
fn is_submit(text: &str, element: &Element) -> bool {
    let kind = element.attribute("type");
    let is_submit = || kind.is_some_and(|k| matches!(&k.value, Value::Text(range) if text[range.clone()].eq_ignore_ascii_case("submit")));
    match element.name.as_str() {
        "button" => kind.is_none() || is_submit(),
        "input" => is_submit(),
        _ => false,
    }
}

/// What replaces the name `written` of `attribute`: the Turbo name, or for a key of a `data:` hash the rest of it, with
/// the separators the key was written with. None when `written` is not the attribute's name spelled in a way this knows.
fn new_spelling(written: &str, attribute: &Attribute, turbo: &str) -> Option<String> {
    if !attribute.in_data {
        return written.eq_ignore_ascii_case(&attribute.name).then(|| turbo.to_string());
    }
    let key = attribute.name.strip_prefix(DATA_PREFIX)?;
    if written.replace('_', "-") != key {
        return None;
    }
    let renamed = turbo.strip_prefix(DATA_PREFIX)?;
    Some(if written.contains('-') { renamed.to_string() } else { renamed.replace('-', "_") })
}

/// Whether `after` reads like `before`, element for element and attribute for attribute, but for the names in `renamed`
/// (by where the attribute started).
fn only_names_differ(before: &[Element], after: &[Element], renamed: &HashMap<usize, &str>) -> bool {
    let kind = |value: &Value| std::mem::discriminant(value);
    before.len() == after.len()
        && before.iter().zip(after).all(|(b, a)| {
            b.name == a.name
                && b.open == a.open
                && b.attributes.len() == a.attributes.len()
                && b.attributes.iter().zip(&a.attributes).all(|(b, a)| {
                    let name = renamed.get(&b.at.start).copied().unwrap_or(&b.name);
                    name == a.name && b.in_data == a.in_data && kind(&b.value) == kind(&a.value)
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use TemplateLang::{Erb, Haml, Slim};

    fn rewrite(lang: TemplateLang, text: &str) -> String {
        let renamed = apply(lang, text, &[], ruby_prism::Version::Latest).unwrap();
        let again = apply(lang, renamed.as_deref().unwrap_or(text), &[], ruby_prism::Version::Latest).unwrap();
        assert_eq!(again, None, "stable: {text}");
        renamed.unwrap_or_else(|| text.to_string())
    }

    #[test]
    fn links_get_turbo_confirm_and_method_in_markup_and_helpers() {
        let erb = "<a href=\"/u/1\" data-confirm=\"Sure?\" data-method=\"delete\">x</a>\n\
                   <%= link_to \"Delete\", user_path(user), data: { confirm: \"Sure?\", method: :delete } %>\n\
                   <%= link_to \"Delete\", user_path(user), \"data-method\" => :delete, data: { \"confirm\": \"Sure?\" } do %>x<% end %>\n\
                   <%= tag.a \"x\", href: y, data: { confirm: \"Sure?\", method: \"post\" } %>\n";
        assert_eq!(
            rewrite(Erb, erb),
            "<a href=\"/u/1\" data-turbo-confirm=\"Sure?\" data-turbo-method=\"delete\">x</a>\n\
             <%= link_to \"Delete\", user_path(user), data: { turbo_confirm: \"Sure?\", turbo_method: :delete } %>\n\
             <%= link_to \"Delete\", user_path(user), \"data-turbo-method\" => :delete, data: { \"turbo_confirm\": \"Sure?\" } do %>x<% end %>\n\
             <%= tag.a \"x\", href: y, data: { turbo_confirm: \"Sure?\", turbo_method: \"post\" } %>\n"
        );
        assert_eq!(
            rewrite(Haml, "%a{href: x, data: {confirm: \"Sure?\", method: :delete}} x\n%a(data-method=\"delete\" data-confirm=\"y\") z\n= link_to \"x\", y, data: { confirm: \"s\", method: :delete }\n"),
            "%a{href: x, data: {turbo_confirm: \"Sure?\", turbo_method: :delete}} x\n%a(data-turbo-method=\"delete\" data-turbo-confirm=\"y\") z\n= link_to \"x\", y, data: { turbo_confirm: \"s\", turbo_method: :delete }\n"
        );
        assert_eq!(
            rewrite(Slim, "a href=\"/x\" data-method=\"delete\" data-confirm=\"Sure?\" x\n= link_to \"x\", y, data: { confirm: \"s\", method: :delete }\n"),
            "a href=\"/x\" data-turbo-method=\"delete\" data-turbo-confirm=\"Sure?\" x\n= link_to \"x\", y, data: { turbo_confirm: \"s\", turbo_method: :delete }\n"
        );
    }

    #[test]
    fn a_confirm_without_a_method_stays_where_turbo_would_ignore_it() {
        for (lang, text) in [
            (Erb, "<a href=\"/x\" data-confirm=\"Sure?\">x</a>\n"),
            (Erb, "<%= link_to \"x\", y, data: { confirm: \"Sure?\" } %>\n"),
            (Haml, "%a{href: x, data: {confirm: \"s\"}} x\n"),
            (Slim, "a href=\"/x\" data-confirm=\"Sure?\" x\n"),
        ] {
            assert_eq!(rewrite(lang, text), text, "{text}");
        }
        assert_eq!(
            rewrite(Erb, "<a data-confirm=\"s\" data-turbo-method=\"post\">x</a>\n"),
            "<a data-turbo-confirm=\"s\" data-turbo-method=\"post\">x</a>\n"
        );
    }

    #[test]
    fn submit_buttons_get_turbo_submits_with() {
        let erb = "<input type=\"submit\" data-disable-with=\"Saving...\">\n\
                   <button data-disable-with=\"Saving...\">x</button>\n\
                   <button type=\"submit\" data-disable-with=\"s\">x</button>\n\
                   <%= form.submit \"Save\", data: { disable_with: \"Saving...\" } %>\n\
                   <%= submit_tag \"Save\", data: { \"disable-with\" => \"Saving...\" } %>\n\
                   <%= button_tag \"Save\", data: { disable_with: \"Saving...\" } %>\n\
                   <%= button_to \"Save\", path, data: { disable_with: \"Saving...\" } %>\n";
        assert_eq!(
            rewrite(Erb, erb),
            "<input type=\"submit\" data-turbo-submits-with=\"Saving...\">\n\
             <button data-turbo-submits-with=\"Saving...\">x</button>\n\
             <button type=\"submit\" data-turbo-submits-with=\"s\">x</button>\n\
             <%= form.submit \"Save\", data: { turbo_submits_with: \"Saving...\" } %>\n\
             <%= submit_tag \"Save\", data: { \"turbo-submits-with\" => \"Saving...\" } %>\n\
             <%= button_tag \"Save\", data: { turbo_submits_with: \"Saving...\" } %>\n\
             <%= button_to \"Save\", path, data: { turbo_submits_with: \"Saving...\" } %>\n"
        );
        assert_eq!(
            rewrite(Haml, "%input{type: \"submit\", data: {disable_with: \"s\"}}\n%button(data-disable-with=\"s\") x\n"),
            "%input{type: \"submit\", data: {turbo_submits_with: \"s\"}}\n%button(data-turbo-submits-with=\"s\") x\n"
        );
        assert_eq!(
            rewrite(Slim, "input type=\"submit\" data-disable-with=\"s\"\nbutton data-disable-with=\"s\" x\n"),
            "input type=\"submit\" data-turbo-submits-with=\"s\"\nbutton data-turbo-submits-with=\"s\" x\n"
        );
    }

    #[test]
    fn everything_else_stays() {
        for (lang, text) in [
            // Other elements and other button types.
            (Erb, "<div data-confirm=\"s\" data-method=\"x\" data-disable-with=\"s\"></div>\n"),
            (Erb, "<input data-disable-with=\"s\">\n<input type=\"text\" data-disable-with=\"s\">\n<input type=\"button\" data-disable-with=\"s\">\n"),
            (Erb, "<button type=\"button\" data-disable-with=\"s\">x</button>\n<button type=\"<%= t %>\" data-disable-with=\"s\">x</button>\n"),
            (Erb, "<a data-disable-with=\"s\">x</a>\n"),
            (Erb, "<form data-confirm=\"s\" data-method=\"post\"></form>\n"),
            // button_to's confirm is Turbo's to read from the form; the method: option is not a data attribute.
            (Erb, "<%= button_to \"x\", y, data: { confirm: \"s\" }, form: { data: { confirm: \"s\" } } %>\n"),
            (Erb, "<%= link_to \"x\", y, method: :delete %>\n"),
            // Text, dynamic keys, other calls, an attribute that is already Turbo's.
            (Erb, "<p>data-confirm=\"x\" data: { confirm: 1 }</p>\n<script>var a = '<a data-method=1>'</script>\n<!-- <a data-method=\"x\"> -->\n"),
            (Erb, "<%= link_to \"x\", y, data: { key => 1, \"#{a}\": 2 } %>\n"),
            (Erb, "<%= render \"x\", data: { confirm: 1, method: :get } %>\n"),
            (Erb, "<%= link_to \"x\", y, data: { method: :delete, turbo_method: :post } %>\n"),
            (Erb, "<a data-method=\"delete\" data-turbo-method=\"post\">x</a>\n"),
            (Erb, "<%= tag.div data: { confirm: 1, method: 2, disable_with: 3 } %>\n"),
            (Haml, "%div{data: {confirm: \"s\", disable_with: \"s\"}}\n%input{data: {disable_with: \"s\"}}\n%a{data: attrs, href: x} x\n"),
            (Haml, "%button{type: x, data: {disable_with: \"s\"}} x\n"),
            (Slim, "div data-confirm=\"s\" data-disable-with=\"s\"\ninput data-disable-with=\"s\"\n"),
        ] {
            assert_eq!(rewrite(lang, text), text, "{text}");
        }
        let open = "<a href=\"/x\" <%= attrs %> data-method=\"delete\">x</a>\n";
        assert_eq!(rewrite(Erb, open), open.replace("data-method", "data-turbo-method"), "a tag with more attributes is still renamed");
    }

    #[test]
    fn a_rewrite_that_changes_more_than_names_is_refused() {
        let before = attributes::elements(Erb, "<a data-method=\"x\" id=\"i\">", &[], ruby_prism::Version::Latest);
        let after = attributes::elements(Erb, "<a data-turbo-method=\"x\">", &[], ruby_prism::Version::Latest);
        let renamed = HashMap::from([(3, TURBO_METHOD)]);
        assert!(!only_names_differ(&before, &after, &renamed), "an attribute vanished");
        let other = attributes::elements(Erb, "<a data-turbo-method=\"x\" id=\"i\">", &[], ruby_prism::Version::Latest);
        assert!(only_names_differ(&before, &other, &renamed));
        assert!(!only_names_differ(&before, &other, &HashMap::new()), "an unannounced rename");
        let spelled = |key: &str, in_data| Attribute { name: key.into(), at: 0..0, in_data, value: Value::Dynamic };
        assert_eq!(new_spelling("confirm", &spelled("data-confirm", true), TURBO_CONFIRM).as_deref(), Some("turbo_confirm"));
        assert_eq!(
            new_spelling("disable-with", &spelled("data-disable-with", true), TURBO_SUBMITS_WITH).as_deref(),
            Some("turbo-submits-with")
        );
        assert_eq!(new_spelling("other", &spelled("data-confirm", true), TURBO_CONFIRM), None);
        assert_eq!(new_spelling("DATA-Confirm", &spelled("data-confirm", false), TURBO_CONFIRM).as_deref(), Some(TURBO_CONFIRM));
    }
}
