//! `Layout/AttributesPerLine`: an ERB start tag that stands first on a line longer than the limit gets one attribute per line,
//! one indent level in, with its `>` after the last attribute. It runs on the re-indented text, so lines are measured where
//! they will stay, and adds lines. Whitespace between attributes is not content, so the guard reads every tag again and accepts
//! nothing but that.

use super::blank_lines::dominant_ending;
use super::element_content::SENSITIVE;
use super::inline::replaced;
use super::tag_style::same_erb_tags;
use super::{same_non_whitespace, Settings};
use crate::erb::ruby::{self, ErbTag};
use crate::erb::tag_spacing::{name_range, read, tag_spans};
use crate::erb::{tags, Edit};
use crate::lang::TemplateLang;
use crate::line_length::width;
use crate::util::leading_ws;
use std::ops::Range;

const REFUSED: &str = "attributes kept on their line: moving them would change more than whitespace";
/// A start tag with fewer attributes has nothing to split.
const MIN_ATTRIBUTES: usize = 2;

pub(super) fn is_on(lang: TemplateLang, settings: &Settings) -> bool {
    lang == TemplateLang::Erb && settings.attributes_per_line.is_some()
}

/// `text` with the attributes of the long start tags on lines of their own; `Ok(None)` when nothing changes, and a note when
/// the rewrite is refused.
pub(super) fn apply(text: &str, settings: &Settings) -> Result<Option<String>, String> {
    let Some(max) = settings.attributes_per_line else { return Ok(None) };
    let edits = split_edits(text, max, &settings.indent_unit());
    if edits.is_empty() {
        return Ok(None);
    }
    let split = replaced(text, &edits);
    if same_non_whitespace(text, &split) && same_erb_tags(text, &split, &Settings::default()) {
        Ok(Some(split))
    } else {
        Err(REFUSED.to_string())
    }
}

fn split_edits(text: &str, max: usize, unit: &str) -> Vec<Edit> {
    let Some(spans) = tag_spans(text) else { return Vec::new() };
    let erb = ruby::tags(text);
    let elements = tags::elements(text);
    let eol = dominant_ending(text);
    let mut edits = Vec::new();
    for span in &spans {
        let line_start = text[..span.start].rfind('\n').map_or(0, |at| at + 1);
        let line = text[line_start..].lines().next().unwrap_or_default();
        let indent = leading_ws(line);
        let first_on_line = line_start + indent.len() == span.start;
        let inside_sensitive = elements.iter().any(|element| {
            element.open_end <= span.start
                && span.end <= element.close_start
                && SENSITIVE.iter().any(|name| element.name.eq_ignore_ascii_case(name))
        });
        let one_line = !text[span.clone()].contains(['\n', '\r']);
        if !first_on_line || !one_line || inside_sensitive || width(line) <= max || text[span.start + 1..].starts_with('/') {
            continue;
        }
        let Some((_, parsed)) = read(text, span, &erb) else { continue };
        let values: Vec<&Range<usize>> = parsed.written.iter().filter_map(|attribute| attribute.value.as_ref()).collect();
        let erb_only_in_values = erb
            .iter()
            .filter(|tag| tag.start >= span.start && tag.end <= span.end)
            .all(|tag| prints(tag) && values.iter().any(|value| value.start <= tag.start && tag.end <= value.end));
        if parsed.written.len() < MIN_ATTRIBUTES || !erb_only_in_values {
            continue;
        }
        let separator = format!("{eol}{indent}{unit}");
        let mut previous_end = name_range(text, span).end;
        for attribute in &parsed.written {
            let end = attribute.value.as_ref().map_or(attribute.name.end, |value| value.end);
            let gap = previous_end..attribute.name.start;
            if text[gap.clone()].bytes().all(|b| b.is_ascii_whitespace()) {
                edits.push((gap, separator.clone()));
            }
            previous_end = end;
        }
    }
    edits
}

/// Whether the tag only prints: `<%= %>` can't be control flow.
fn prints(tag: &ErbTag) -> bool {
    tag.indicator.is_some_and(|indicator| indicator.contains('='))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{format, IndentStyle, Quote, VoidClosing};

    const LONG: &str =
        r#"<input type="email" name="user[email]" id="user_email" class="form-control form-control-lg" placeholder="Email" required>"#;
    const SPLIT: &str = "<input\n  type=\"email\"\n  name=\"user[email]\"\n  id=\"user_email\"\n  class=\"form-control form-control-lg\"\n  placeholder=\"Email\"\n  required>";

    fn settings(max: usize) -> Settings {
        Settings { attributes_per_line: Some(max), ..Settings::default() }
    }

    fn run(text: &str, settings: &Settings) -> String {
        let uri = "file:///app/views/users/index.html.erb".parse().unwrap();
        let once = format(&uri, TemplateLang::Erb, false, text, settings, &[]).unwrap();
        let twice = format(&uri, TemplateLang::Erb, false, &once.text, settings, &[]).unwrap();
        assert_eq!(twice.text, once.text, "stable: {text}");
        assert!(twice.notes.is_empty() && once.notes.is_empty(), "{:?}", once.notes);
        once.text
    }

    fn split(text: &str) -> String {
        run(text, &settings(100))
    }

    #[test]
    fn a_long_start_tag_gets_one_attribute_per_line() {
        assert_eq!(split(&format!("{LONG}\n")), format!("{SPLIT}\n"));
        assert_eq!(
            split("<div>\n  <a href=\"/users\" class=\"btn btn-primary btn-lg\" data-turbo-method=\"delete\" data-turbo-confirm=\"Are you sure?\">x</a>\n</div>\n"),
            "<div>\n  <a\n    href=\"/users\"\n    class=\"btn btn-primary btn-lg\"\n    data-turbo-method=\"delete\"\n    data-turbo-confirm=\"Are you sure?\">x</a>\n</div>\n"
        );
    }

    #[test]
    fn the_closing_stays_after_the_last_attribute() {
        let split_slash =
            split("<input type=\"email\" name=\"user[email]\" id=\"user_email\" class=\"form-control form-control-lg\" required />\n");
        assert_eq!(
            split_slash,
            "<input\n  type=\"email\"\n  name=\"user[email]\"\n  id=\"user_email\"\n  class=\"form-control form-control-lg\"\n  required />\n"
        );
        assert!(run(
            "<input type=\"email\" name=\"user[email]\" id=\"user_email\" class=\"form-control form-control-lg\"/>\n",
            &settings(80)
        )
        .ends_with("\n  class=\"form-control form-control-lg\"/>\n"));
    }

    #[test]
    fn the_limit_decides() {
        let fits = "<input type=\"email\" name=\"user[email]\" id=\"user_email\">\n";
        assert_eq!(run(fits, &settings(100)), fits);
        assert_eq!(run(fits, &settings(fits.trim_end().chars().count())), fits, "a line at the limit stays");
        assert_ne!(run(fits, &settings(fits.trim_end().chars().count() - 1)), fits);
        let accents = "<p title=\"éééé\" id=\"éééé\">\n";
        assert_eq!(run(accents, &settings(26)), accents, "characters, not bytes");
        assert_eq!(run(accents, &settings(25)), "<p\n  title=\"éééé\"\n  id=\"éééé\">\n");
    }

    #[test]
    fn the_whole_line_counts_and_the_indentation_is_the_files() {
        assert_eq!(
            run("<ul>\n    <li class=\"a\" id=\"b\">x</li>\n</ul>\n", &settings(29)),
            "<ul>\n  <li class=\"a\" id=\"b\">x</li>\n</ul>\n"
        );
        assert_eq!(
            run("<ul>\n    <li class=\"a\" id=\"b\">xxxxxxxxxx</li>\n</ul>\n", &settings(30)),
            "<ul>\n  <li\n    class=\"a\"\n    id=\"b\">xxxxxxxxxx</li>\n</ul>\n"
        );
        let four = Settings { indent_size: 4, ..settings(30) };
        assert_eq!(
            run("<ul>\n<li class=\"a\" id=\"b\">xxxxxxxxxx</li>\n</ul>\n", &four),
            "<ul>\n    <li\n        class=\"a\"\n        id=\"b\">xxxxxxxxxx</li>\n</ul>\n"
        );
    }

    #[test]
    fn tabs_and_crlf_are_kept() {
        let tabs = Settings { indent_style: IndentStyle::Tabs, ..settings(30) };
        assert_eq!(
            run("<ul>\n  <li class=\"a\" id=\"b\" title=\"c\">x</li>\n</ul>\n", &tabs),
            "<ul>\n\t<li\n\t\tclass=\"a\"\n\t\tid=\"b\"\n\t\ttitle=\"c\">x</li>\n</ul>\n"
        );
        assert_eq!(
            run("<ul>\r\n  <li class=\"a\" id=\"b\" title=\"c\">x</li>\r\n</ul>\r\n", &settings(30)),
            "<ul>\r\n  <li\r\n    class=\"a\"\r\n    id=\"b\"\r\n    title=\"c\">x</li>\r\n</ul>\r\n"
        );
    }

    #[test]
    fn values_with_output_tags_are_kept_whole() {
        assert_eq!(
            split("<a href=\"<%= user_path(@user) %>\" class=\"btn btn-primary <%= 'active' if current %>\" data-id=\"<%= @user.id %>\" title='x y'>x</a>\n"),
            "<a\n  href=\"<%= user_path(@user) %>\"\n  class=\"btn btn-primary <%= 'active' if current %>\"\n  data-id=\"<%= @user.id %>\"\n  title='x y'>x</a>\n"
        );
    }

    #[test]
    fn spaces_around_the_equals_sign_stay_inside_the_attribute() {
        assert_eq!(
            run("<a href = \"/users/index\"   class =  \"btn btn-primary\"   id=\"main-link\">x</a>\n", &settings(40)),
            "<a\n  href = \"/users/index\"\n  class =  \"btn btn-primary\"\n  id=\"main-link\">x</a>\n"
        );
    }

    fn untouched(text: &str, max: usize) {
        assert_eq!(run(text, &settings(max)), text, "{text}");
    }

    #[test]
    fn tags_that_cannot_be_split_safely_stay() {
        // Control flow or an output tag between attributes, or in a name.
        untouched("<div <%= attrs %> class=\"card card-body card-lg\" id=\"main-card-element\" data-controller=\"toggle\">\n</div>\n", 40);
        untouched("<div class=\"card card-body\" <%= 'hidden' if @x %> id=\"main-card-element\" data-controller=\"toggle\">\n</div>\n", 40);
        untouched(
            "<div <% if a %>class=\"a\"<% end %> id=\"main-card-element-long\" data-controller=\"toggle\" data-x=\"y\">\n</div>\n",
            40,
        );
        untouched(
            "<div class=\"<% if a %>big<% else %>small<% end %>\" id=\"main-card-element-long\" data-controller=\"toggle\">\n</div>\n",
            40,
        );
        untouched("<div data-<%= name %>=\"a\" id=\"main-card-element-long\" data-controller=\"toggle\" data-x=\"yyyyy\">\n</div>\n", 40);
        // Already over lines, or a value over lines.
        untouched(
            "<div class=\"card card-body\"\n  id=\"main-card-element-long\" data-controller=\"toggle\" data-x=\"yyyyy\">\n</div>\n",
            40,
        );
        untouched(
            "<div class=\"card\n  card-body\" id=\"main-card-element-long\" data-controller=\"toggle\" data-x=\"yyyyy\">\n</div>\n",
            40,
        );
        // A single attribute, or none.
        untouched("<a href=\"/a/very/long/path/that/goes/on/and/on/and/on/and/on/and/on/and/on\">x</a>\n", 40);
        // Not a start tag that begins its line.
        untouched("<ul><li class=\"a-long-class\" id=\"main-card-element-long\" data-controller=\"toggle\">x</li></ul>\n", 40);
        untouched(
            "<p>text</p> <a href=\"/a/long/path\" class=\"a-long-class\" id=\"main-card-element-long\" data-controller=\"toggle\">x</a>\n",
            40,
        );
        // Not markup.
        untouched(
            "<!-- <a href=\"/a/long/path\" class=\"a-long-class\" id=\"main-card-element-long\" data-controller=\"toggle\"> -->\n",
            40,
        );
        untouched(
            "<%= '<a href=\"/a/long/path\" class=\"a-long-class\" id=\"main-card-element-long\" data-controller=\"toggle\">' %>\n",
            40,
        );
        untouched("<script>\n  var a = '<a href=\"/a/long/path\" class=\"a-long-class\" id=\"main-card-element-long\" data-controller=\"toggle\">';\n</script>\n", 40);
        untouched(
            "<![CDATA[<a href=\"/a/long/path\" class=\"a-long-class\" id=\"main-card-element-long\" data-controller=\"toggle\">]]>\n",
            40,
        );
        // Whitespace is content in these.
        untouched("<pre>\n<code class=\"language-ruby\" id=\"main-card-element-long\" data-controller=\"toggle\" data-x=\"yyyyy\">x</code></pre>\n", 40);
        untouched("<textarea>\n<a href=\"/a/long/path\" class=\"a-long-class\" id=\"main-card-element-long\" data-controller=\"toggle\">x</textarea>\n", 40);
        // Unreadable tags.
        untouched("<a href=\"/a/long/path class=\"a-long-class\" id=\"main-card-element-long\" data-controller=\"toggle\">x</a>\n", 40);
    }

    #[test]
    fn the_start_tags_of_script_and_style_are_split_and_end_tags_are_not() {
        assert_eq!(
            run("<script src=\"/assets/application.js\" defer async type=\"module\" nonce=\"abc\">\n  var a;\n</script>\n", &settings(40)),
            "<script\n  src=\"/assets/application.js\"\n  defer\n  async\n  type=\"module\"\n  nonce=\"abc\">\n  var a;\n</script>\n"
        );
        untouched("<svg viewBox=\"0 0 10 10\"/>\n", 5);
    }

    #[test]
    fn foreign_content_is_split_too() {
        assert_eq!(
            run("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\" width=\"10\" height=\"10\">\n</svg>\n", &settings(40)),
            "<svg\n  xmlns=\"http://www.w3.org/2000/svg\"\n  viewBox=\"0 0 10 10\"\n  width=\"10\"\n  height=\"10\">\n</svg>\n"
        );
    }

    #[test]
    fn inside_an_erb_block_the_lines_follow_the_block() {
        assert_eq!(
            run("<% if admin? %>\n<a href=\"/admin/users/index\" class=\"btn btn-primary btn-lg\" data-turbo-frame=\"modal-frame\">x</a>\n<% end %>\n", &settings(80)),
            "<% if admin? %>\n  <a\n    href=\"/admin/users/index\"\n    class=\"btn btn-primary btn-lg\"\n    data-turbo-frame=\"modal-frame\">x</a>\n<% end %>\n"
        );
    }

    #[test]
    fn it_combines_with_the_other_rules() {
        let all = Settings {
            attributes_per_line: Some(60),
            space_in_html_tag: true,
            attribute_quotes: Some(Quote::Double),
            boolean_attributes: true,
            void_tag_closing: Some(VoidClosing::NoSlash),
            tag_name_case: true,
            content_on_own_line: Some(vec!["div".to_string()]),
            ..Settings::default()
        };
        assert_eq!(
            run("<DIV  class='card card-body'  id=main-card  data-controller='toggle'  hidden='hidden'><%= x %></DIV>\n<INPUT  type='checkbox'  name='user[admin]'  id='user_admin'  checked='checked'  disabled  />\n", &all),
            "<div\n  class=\"card card-body\"\n  id=\"main-card\"\n  data-controller=\"toggle\"\n  hidden>\n  <%= x %>\n</div>\n<input\n  type=\"checkbox\"\n  name=\"user[admin]\"\n  id=\"user_admin\"\n  checked\n  disabled>\n"
        );
        let tabs = Settings { indent_style: IndentStyle::Tabs, ..all };
        assert_eq!(
            run("<ul>\n<li class='a-long-class' id='main-card-element' data-controller='toggle' hidden>x</li>\n</ul>\n", &tabs),
            "<ul>\n\t<li\n\t\tclass=\"a-long-class\"\n\t\tid=\"main-card-element\"\n\t\tdata-controller=\"toggle\"\n\t\thidden>x</li>\n</ul>\n"
        );
    }

    #[test]
    fn nothing_changes_while_the_rule_is_off() {
        assert!(!is_on(TemplateLang::Erb, &Settings::default()));
        assert!(!is_on(TemplateLang::Haml, &settings(10)));
        assert_eq!(apply(&format!("{LONG}\n"), &Settings::default()), Ok(None));
    }

    #[test]
    fn a_rewrite_that_changes_more_than_whitespace_is_refused() {
        let text = format!("{LONG}\n");
        assert_eq!(apply(&text, &settings(100)), Ok(Some(format!("{SPLIT}\n"))));
        // The guard on its own: a changed value, a dropped attribute or a changed slash is not whitespace.
        let same = |after: &str| same_non_whitespace(&text, after) && same_erb_tags(&text, after, &Settings::default());
        assert!(same(&format!("{SPLIT}\n")));
        assert!(!same(&format!("{SPLIT}\n").replace("form-control-lg", "form-control-sm")));
        assert!(!same(&format!("{SPLIT}\n").replace("\n  required>", ">")));
        assert!(!same(&format!("{SPLIT}\n").replace("required>", "required />")));
    }
}
