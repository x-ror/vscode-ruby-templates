//! `Style/ImplicitDiv`, `Style/ClassesBeforeIds` and `Style/HashAttributes`: rewrites of the tag lines of Haml (and, for the
//! first, Slim). They run before the other rules, like the tag style rules, and stay on the line they are on. Every rewrite is
//! read again and kept only when the template compiles to the same thing.

use super::inline::replaced;
use super::{AttributeStyle, Settings};
use crate::attributes::{self, literal_name, span, Value};
use crate::erb::Edit;
use crate::haml::parser::{parse_with_ruby, Chunk, Parsed, Piece, Tag};
use crate::haml::ruby as haml_ruby;
use crate::lang::TemplateLang;
use crate::slim::parser;
use crate::util::BOM;
use ruby_prism::Node;
use std::ops::Range;

const IMPLICIT_DIV: &str = "div";
const HEAD_REFUSED: &str = "implicit div and class order kept: rewriting them would change the compiled template";
const ATTRIBUTES_REFUSED: &str = "attribute style kept: rewriting it would change the compiled template";

pub(super) fn heads_are_on(settings: &Settings) -> bool {
    settings.implicit_div || settings.classes_before_ids
}

/// `text` without the `div` of tags that a class or id implies and with classes before ids; `Ok(None)` when nothing changes,
/// and a note when the rewrite is refused.
pub(super) fn heads(
    lang: TemplateLang,
    text: &str,
    settings: &Settings,
    extra_engines: &[String],
    grammar: ruby_prism::Version,
) -> Result<Option<String>, String> {
    let kept = match lang {
        TemplateLang::Haml => haml_heads(text, settings, grammar),
        TemplateLang::Slim | TemplateLang::Skim if settings.implicit_div => slim_heads(text, extra_engines),
        _ => return Ok(None),
    };
    kept.ok_or_else(|| HEAD_REFUSED.to_string())
}

/// `text` with the attributes of Haml tags written in `settings.hash_attributes`'s style; `Ok(None)` when nothing changes, and
/// a note when the rewrite is refused.
pub(super) fn attributes(text: &str, settings: &Settings, grammar: ruby_prism::Version) -> Result<Option<String>, String> {
    let Some(style) = settings.hash_attributes else { return Ok(None) };
    let (bom, body) = split_bom(text);
    let options = ruby_prism::Options::default().version(grammar).build();
    let parsed = parse_with_ruby(body);
    if parsed.error.is_some() {
        return Ok(None);
    }
    let edits: Vec<Edit> = parsed.tags.iter().filter_map(|tag| converted(body, tag, style, &options)).collect();
    if edits.is_empty() {
        return Ok(None);
    }
    let before = pairs_of(body, &options);
    let keeps = |edits: &[Edit]| {
        let rewritten = replaced(body, edits);
        (same_tags(body, &rewritten, grammar, false) && before == pairs_of(&rewritten, &options)).then_some(rewritten)
    };
    // One tag the guard refuses must not cost the others their rewrite.
    let kept = keeps(&edits).or_else(|| {
        let passing: Vec<Edit> = edits.into_iter().filter(|edit| keeps(std::slice::from_ref(edit)).is_some()).collect();
        keeps(&passing).filter(|_| !passing.is_empty())
    });
    kept.map(|rewritten| Some(format!("{bom}{rewritten}"))).ok_or_else(|| ATTRIBUTES_REFUSED.to_string())
}

/// `data` and `aria` with a value that is no hash may be one at run time, and Haml then writes `data-*` attributes.
fn is_prefixed_hash_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("data") || name.eq_ignore_ascii_case("aria")
}

pub(super) fn split_bom(text: &str) -> (&str, &str) {
    let body = text.strip_prefix(BOM).unwrap_or(text);
    (&text[..text.len() - body.len()], body)
}

/// `Some(None)` when there is nothing to rewrite, `None` when the rewrite is refused.
fn haml_heads(text: &str, settings: &Settings, grammar: ruby_prism::Version) -> Option<Option<String>> {
    let (bom, body) = split_bom(text);
    let parsed = parse_with_ruby(body);
    if parsed.error.is_some() {
        return Some(None);
    }
    let edits: Vec<Edit> = parsed.tags.iter().flat_map(|tag| head_edits(body, tag, settings)).collect();
    if edits.is_empty() {
        return Some(None);
    }
    let rewritten = replaced(body, &edits);
    same_tags(body, &rewritten, grammar, true).then(|| Some(format!("{bom}{rewritten}")))
}

fn head_edits(text: &str, tag: &Tag, settings: &Settings) -> Vec<Edit> {
    let head = tag.head.clone();
    let shorthand = &text[head.clone()];
    // An unreadable line, and `@`, which Haml reads into the head but not as a class or id.
    if head == (0..0) || shorthand.contains('@') {
        return Vec::new();
    }
    let mut edits = Vec::new();
    let name = tag.element.at.clone();
    let percent_before_name = name.start.checked_sub(1).filter(|&at| text.as_bytes()[at] == b'%');
    // `%%div` is the element `div` too, but its first `%` would stay.
    let lone_percent = percent_before_name.filter(|&at| at == 0 || text.as_bytes()[at - 1] != b'%');
    if let (true, Some(percent), true) = (settings.implicit_div, lone_percent, shorthand.starts_with(['.', '#'])) {
        if &text[name.clone()] == IMPLICIT_DIV {
            edits.push((percent..name.end, String::new()));
        }
    }
    if settings.classes_before_ids {
        let reordered = classes_first(shorthand);
        if reordered != shorthand {
            edits.push((head, reordered));
        }
    }
    edits
}

/// A head's classes, then its ids, each in the order written.
fn classes_first(shorthand: &str) -> String {
    let (classes, ids): (Vec<&str>, Vec<&str>) = split_head(shorthand).into_iter().partition(|part| part.starts_with('.'));
    classes.concat() + &ids.concat()
}

/// The `.class` and `#id` parts of a head, each with its sign.
fn split_head(shorthand: &str) -> Vec<&str> {
    let starts: Vec<usize> = shorthand.match_indices(['.', '#']).map(|(at, _)| at).collect();
    starts.iter().zip(starts.iter().skip(1).copied().chain([shorthand.len()])).map(|(&from, to)| &shorthand[from..to]).collect()
}

/// What Haml makes of each tag, apart from where it is written: the generated Ruby, then per tag its element and its head.
/// `attribute_hashes` false leaves out the hashes Haml builds attributes from, which a change of style rewrites.
fn same_tags(before: &str, after: &str, grammar: ruby_prism::Version, attribute_hashes: bool) -> bool {
    let (before_parsed, after_parsed) = (parse_with_ruby(before), parse_with_ruby(after));
    if before_parsed.error.is_some() || after_parsed.error.is_some() || before_parsed.roles != after_parsed.roles {
        return false;
    }
    if generated(before, before_parsed, attribute_hashes) != generated(after, after_parsed, attribute_hashes) {
        return false;
    }
    signatures(before, grammar) == signatures(after, grammar)
}

/// The Ruby Haml generates; without `attribute_hashes` the arguments of `AttributeBuilder.build` are left out, and the call
/// with them when only they made it: a hash of literals makes one that a `()` list of literals does not.
fn generated(text: &str, mut parsed: Parsed, attribute_hashes: bool) -> String {
    if !attribute_hashes {
        parsed.pieces.retain_mut(|piece| match piece {
            Piece::Attributes { object, arguments } => {
                arguments.clear();
                !object.is_empty()
            }
            _ => true,
        });
    }
    haml_ruby::compiled(text, &parsed, None).source
}

/// A tag as the markup shows it: name, written class and id parts, and each attribute's name with its text when it is static.
type Signature<'a> = (String, bool, Option<&'a str>, Vec<&'a str>, Vec<(String, bool, Option<&'a str>)>);

fn signatures(text: &str, grammar: ruby_prism::Version) -> Vec<Signature<'_>> {
    let parsed = parse_with_ruby(text);
    let elements = attributes::markup_elements(TemplateLang::Haml, text, &[], grammar);
    parsed
        .tags
        .iter()
        .zip(elements)
        .map(|(tag, element)| {
            let mut attributes: Vec<_> = element
                .attributes
                .iter()
                .map(|a| (a.name.clone(), a.in_data, if let Value::Text(range) = &a.value { Some(&text[range.clone()]) } else { None }))
                .collect();
            attributes.sort_by(|a, b| a.0.cmp(&b.0));
            // The class parts in order, then the ids in order: the same whichever way round they were written.
            let parts = split_head(&text[tag.head.clone()]);
            let (classes, ids): (Vec<&str>, Vec<&str>) = parts.into_iter().partition(|part| part.starts_with('.'));
            (element.name, element.open, element.text.map(|range| &text[range]), [classes, ids].concat(), attributes)
        })
        .collect()
}

/// Slim reads `.card` as a `div` with that class, so `div.card` and `.card` give the same pieces.
fn slim_heads(text: &str, extra_engines: &[String]) -> Option<Option<String>> {
    let parsed = parser::parse(text, extra_engines);
    if !parsed.valid {
        return Some(None);
    }
    let edits: Vec<Edit> = parsed
        .elements
        .iter()
        .filter(|element| &text[element.at.clone()] == IMPLICIT_DIV)
        .filter(|element| shortcut_follows(&text[element.at.end..]))
        .map(|element| (element.at.clone(), String::new()))
        .collect();
    if edits.is_empty() {
        return Some(None);
    }
    let rewritten = replaced(text, &edits);
    crate::slim::ruby::same_compilation_ignoring_lines(text, &rewritten, extra_engines).then_some(Some(rewritten))
}

/// Whether a `.class` or `#id` with a name starts `rest`: `div#{x}` is no id.
fn shortcut_follows(rest: &str) -> bool {
    let mut chars = rest.chars();
    matches!(chars.next(), Some('.' | '#')) && chars.next().is_some_and(|name| name.is_alphanumeric() || matches!(name, '_' | '-'))
}

/// What an attribute is set to, as both ways of writing it can say: a string literal with its quotes, a variable or number
/// as written, or just present (`checked`, `checked: true`).
#[derive(Debug, PartialEq)]
enum AttributeValue {
    Quoted(String),
    Bare(String),
    Present,
}

type Pair = (String, AttributeValue);

/// One `()` list or `{}` hash on a tag, with the attributes it sets.
struct Group {
    range: Range<usize>,
    pairs: Vec<Pair>,
    hash: bool,
}

/// The edit that writes the tag's group in `style`, when it is written the other way and every attribute can be said in both.
fn converted(text: &str, tag: &Tag, style: AttributeStyle, options: &ruby_prism::ParseOptions) -> Option<Edit> {
    let group = group_of(text, tag, options)?;
    if group.hash == (style == AttributeStyle::Html) {
        let written = if group.hash { html_list(&group.pairs) } else { ruby_hash(&group.pairs) };
        return Some((group.range, written));
    }
    None
}

/// What each tag's group sets, `None` for a tag without one this reads.
fn pairs_of(text: &str, options: &ruby_prism::ParseOptions) -> Vec<Option<Vec<Pair>>> {
    parse_with_ruby(text).tags.iter().map(|tag| group_of(text, tag, options).map(|group| group.pairs)).collect()
}

fn html_list(pairs: &[Pair]) -> String {
    let attributes: Vec<String> = pairs
        .iter()
        .map(|(name, value)| match value {
            AttributeValue::Present => name.clone(),
            AttributeValue::Quoted(value) | AttributeValue::Bare(value) => format!("{name}={value}"),
        })
        .collect();
    format!("({})", attributes.join(" "))
}

fn ruby_hash(pairs: &[Pair]) -> String {
    let entries: Vec<String> = pairs
        .iter()
        .map(|(name, value)| {
            let key = if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { name.clone() } else { format!("\"{name}\"") };
            let value = match value {
                AttributeValue::Present => "true",
                AttributeValue::Quoted(value) | AttributeValue::Bare(value) => value,
            };
            format!("{key}: {value}")
        })
        .collect();
    format!("{{{}}}", entries.join(", "))
}

/// The `()` list or the `{}` hash that follows a tag's head, when it is the only one, is on one line and sets nothing but
/// attributes both syntaxes can write.
fn group_of(text: &str, tag: &Tag, options: &ruby_prism::ParseOptions) -> Option<Group> {
    if tag.head == (0..0) {
        return None;
    }
    let at = tag.head.end;
    match text[at..].chars().next()? {
        '(' if tag.hash.is_empty() => {
            let line = &text[at..at + text[at..].find(['\n', '\r']).unwrap_or(text.len() - at)];
            let (len, pairs) = read_list(line)?;
            Some(Group { range: at..at + len, pairs, hash: false })
        }
        '{' => read_hash(text, at, &tag.hash, options),
        _ => None,
    }
}

/// The attributes of the `(...)` that starts `line` and its length: names, then bare, quoted or no values, on one line.
fn read_list(line: &str) -> Option<(usize, Vec<Pair>)> {
    let mut pairs: Vec<Pair> = Vec::new();
    let mut at = 1;
    loop {
        at += line[at..].len() - line[at..].trim_start_matches(' ').len();
        if line[at..].starts_with(')') {
            return (!pairs.is_empty()).then_some((at + 1, pairs));
        }
        let name_len = attribute_name_len(&line[at..])?;
        let name = line[at..at + name_len].to_string();
        at += name_len;
        let value = if line[at..].starts_with('=') {
            let value_len = value_len(&line[at + 1..])?;
            let value = &line[at + 1..at + 1 + value_len];
            at += 1 + value_len;
            classify(value)
        } else {
            AttributeValue::Present
        };
        // A value runs on to a blank or the closing parenthesis; anything else is something this does not read.
        if !line[at..].starts_with([' ', ')']) || is_prefixed_hash_name(&name) || pairs.iter().any(|(seen, _)| *seen == name) {
            return None;
        }
        pairs.push((name, value));
    }
}

/// A name HTML and Ruby can both spell: `href`, `data-turbo`, `xlink:href`.
fn attribute_name_len(text: &str) -> Option<usize> {
    let first = text.chars().next().filter(|c| c.is_ascii_alphabetic() || *c == '_')?;
    Some(
        first.len_utf8() + text[1..].find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':'))).unwrap_or(text.len() - 1),
    )
}

/// The length of the value at the start of `text`: a string with nothing in it that either syntax would read differently,
/// a variable, or a number.
fn value_len(text: &str) -> Option<usize> {
    if let Some(quote @ ('"' | '\'')) = text.chars().next() {
        let len = 1 + text[1..].find(quote)?;
        return plain_string(&text[1..len]).then_some(len + 1);
    }
    let name = ["@@", "@", "$"].iter().find_map(|prefix| text.strip_prefix(prefix)).unwrap_or(text);
    let word_len = name.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(name.len());
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        let is_number = text.len() == name.len() && name[..word_len].bytes().all(|b| b.is_ascii_digit());
        return is_number.then_some(word_len);
    }
    (word_len > 0).then_some(text.len() - name.len() + word_len)
}

/// Whether Ruby and Haml's `()` both take a string's content as it stands: no escape, no interpolation (`#{x}`, `#@x`, `#$x`).
fn plain_string(content: &str) -> bool {
    !content.contains('\\') && !["#{", "#@", "#$"].iter().any(|opening| content.contains(opening))
}

fn classify(value: &str) -> AttributeValue {
    if value.starts_with(['"', '\'']) {
        AttributeValue::Quoted(value.to_string())
    } else if value == "true" {
        AttributeValue::Present
    } else {
        AttributeValue::Bare(value.to_string())
    }
}

/// The attributes of the `{...}` at `at` when its code is one run on the line, with Ruby's reading of each entry.
fn read_hash(text: &str, at: usize, code: &[Chunk], options: &ruby_prism::ParseOptions) -> Option<Group> {
    let [chunk] = code else { return None };
    let start = chunk.offset.filter(|&start| start == at + 1)?;
    let end = start + chunk.text.len();
    // A `(` list after the braces is a second group; a newline in the code is a hash over several lines.
    if text.as_bytes().get(end) != Some(&b'}') || chunk.text.contains(['\n', '\r']) || text[end + 1..].starts_with('(') {
        return None;
    }
    let compiled = haml_ruby::attribute_hash(code, text);
    let parsed = compiled.parse(options)?;
    let root = parsed.node();
    let statement = root.as_program_node()?.statements().body().iter().next()?;
    let mut pairs: Vec<Pair> = Vec::new();
    for entry in statement.as_hash_node()?.elements().iter() {
        let assoc = entry.as_assoc_node()?;
        let (name, _) = literal_name(&assoc.key(), &compiled)?;
        let well_named = attribute_name_len(&name) == Some(name.len());
        let value = hash_value(&assoc.value(), &compiled, text)?;
        if !well_named || is_prefixed_hash_name(&name) || pairs.iter().any(|(seen, _)| *seen == name) {
            return None;
        }
        pairs.push((name, value));
    }
    (!pairs.is_empty()).then_some(Group { range: at..end + 1, pairs, hash: true })
}

/// A value the `()` syntax can say the same: a plain string literal, `true`, a number, a variable or a bare method name.
fn hash_value(node: &Node, compiled: &crate::ruby::generated::Compiled, text: &str) -> Option<AttributeValue> {
    if node.as_true_node().is_some() {
        return Some(AttributeValue::Present);
    }
    let written = &text[span(compiled, &node.location())?];
    let is_variable = node.as_instance_variable_read_node().is_some()
        || node.as_class_variable_read_node().is_some()
        || node.as_global_variable_read_node().is_some()
        || node.as_call_node().is_some_and(|call| call.is_variable_call());
    let is_literal = node.as_integer_node().is_some() || node.as_false_node().is_some() || node.as_nil_node().is_some();
    let is_string = node.as_string_node().is_some() && written.starts_with(['"', '\'']);
    ((is_string || is_variable || is_literal) && value_len(written) == Some(written.len())).then(|| classify(written))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heads_of(lang: TemplateLang, text: &str, settings: &Settings) -> Result<String, String> {
        let once = heads(lang, text, settings, &[], ruby_prism::Version::Latest)?.unwrap_or_else(|| text.to_string());
        assert_eq!(heads(lang, &once, settings, &[], ruby_prism::Version::Latest)?, None, "stable: {text}");
        Ok(once)
    }

    fn div(lang: TemplateLang, text: &str) -> String {
        heads_of(lang, text, &Settings { implicit_div: true, ..Settings::default() }).unwrap()
    }

    fn order(text: &str) -> String {
        heads_of(TemplateLang::Haml, text, &Settings { classes_before_ids: true, ..Settings::default() }).unwrap()
    }

    fn hash_style(text: &str, style: AttributeStyle) -> String {
        let settings = Settings { hash_attributes: Some(style), ..Settings::default() };
        let once = attributes(text, &settings, ruby_prism::Version::Latest).unwrap().unwrap_or_else(|| text.to_string());
        let again = attributes(&once, &settings, ruby_prism::Version::Latest).unwrap();
        assert_eq!(again, None, "stable: {text}");
        once
    }

    fn ruby_hash_of(text: &str) -> String {
        hash_style(text, AttributeStyle::RubyHash)
    }

    fn html_of(text: &str) -> String {
        hash_style(text, AttributeStyle::Html)
    }

    #[test]
    fn haml_div_goes_where_a_class_or_id_follows_it() {
        assert_eq!(div(TemplateLang::Haml, "%div.card\n  %div#details\n    %p= user.bio\n"), ".card\n  #details\n    %p= user.bio\n");
        assert_eq!(div(TemplateLang::Haml, "%div.a.b#c{x: 1} text\n%div#d(y=\"2\")= @z\n"), ".a.b#c{x: 1} text\n#d(y=\"2\")= @z\n");
        assert_eq!(div(TemplateLang::Haml, "\u{feff}%div.card/\n"), "\u{feff}.card/\n");
    }

    #[test]
    fn haml_div_stays_without_a_class_or_id_right_after_it() {
        for text in [
            "%div\n",
            "%div{class: \"a\"}\n",
            "%div(class=\"a\")\n",
            "%div[@user]\n",
            "%DIV.card\n",
            "%div:x.card\n",
            "%divider.card\n",
            "%%div.card\n",
            ".card\n",
            "%p.card\n",
            "%div.card x |\n  y |\n",
        ] {
            assert_eq!(div(TemplateLang::Haml, text), text, "{text:?}");
        }
    }

    #[test]
    fn haml_classes_come_before_ids() {
        assert_eq!(order("%section#profile.card.wide\n  %p= user.bio\n"), "%section.card.wide#profile\n  %p= user.bio\n");
        assert_eq!(order("#a.b.c\n%p#x.y#z.w text\n"), ".b.c#a\n%p.y.w#x#z text\n");
        assert_eq!(order("%p.lead#intro\n%p#only\n%p.only\n%p\n"), "%p.lead#intro\n%p#only\n%p.only\n%p\n");
        assert_eq!(order("%a#x.y{href: \"/\"} Go\n"), "%a.y#x{href: \"/\"} Go\n");
    }

    #[test]
    fn haml_div_and_class_order_combine() {
        let both = Settings { implicit_div: true, classes_before_ids: true, ..Settings::default() };
        assert_eq!(heads_of(TemplateLang::Haml, "%div#main.wide.tall\n  %div#inner.x\n", &both).unwrap(), ".wide.tall#main\n  .x#inner\n");
    }

    #[test]
    fn slim_div_goes_where_a_class_or_id_follows_it() {
        let cases = [
            ("div.card\n  div#details\n    p = user.bio\n", ".card\n  #details\n    p = user.bio\n"),
            ("div.card text\ndiv#a.b = x\ndiv.c=y\n", ".card text\n#a.b = x\n.c=y\n"),
            ("div.a: span b\ndiv#c class=\"d\" data-x=1\n", ".a: span b\n#c class=\"d\" data-x=1\n"),
            ("div.a\n  | text\ndiv.b == raw\n", ".a\n  | text\n.b == raw\n"),
        ];
        for (before, after) in cases {
            assert_eq!(div(TemplateLang::Slim, before), after);
        }
    }

    #[test]
    fn slim_div_stays_without_a_shortcut_right_after_it() {
        for text in [
            "div\n",
            "div class=\"a\"\n",
            "div .card\n",
            "div#{x}\n",
            "DIV.card\n",
            "divider.card\n",
            ".card\n",
            "p.card\n",
            "div(class=\"a\")\n",
            "div.\n",
        ] {
            assert_eq!(div(TemplateLang::Slim, text), text, "{text:?}");
        }
    }

    #[test]
    fn a_template_haml_or_slim_rejects_is_left_alone() {
        let on =
            Settings { implicit_div: true, classes_before_ids: true, hash_attributes: Some(AttributeStyle::Html), ..Settings::default() };
        assert_eq!(heads(TemplateLang::Haml, "%div.card\n  %p\n %b\n", &on, &[], ruby_prism::Version::Latest), Ok(None));
        assert_eq!(heads(TemplateLang::Slim, "div.card\n    p\n  b\n", &on, &[], ruby_prism::Version::Latest), Ok(None));
        assert_eq!(attributes("%a{href: \"x\"}\n  %p\n %b\n", &on, ruby_prism::Version::Latest), Ok(None));
    }

    #[test]
    fn html_attributes_become_a_hash() {
        assert_eq!(
            ruby_hash_of("%a.btn(href=\"/users\" target=\"_blank\") Users\n"),
            "%a.btn{href: \"/users\", target: \"_blank\"} Users\n"
        );
        assert_eq!(
            ruby_hash_of("%a(title=@t data-n=@n href=url tabindex=0) x\n"),
            "%a{title: @t, \"data-n\": @n, href: url, tabindex: 0} x\n"
        );
        assert_eq!(
            ruby_hash_of("%input(type='checkbox' checked disabled=true)\n"),
            "%input{type: 'checkbox', checked: true, disabled: true}\n"
        );
        assert_eq!(ruby_hash_of("%a( href=\"#top\"  xlink:href=\"y\" )/\n"), "%a{href: \"#top\", \"xlink:href\": \"y\"}/\n");
        assert_eq!(ruby_hash_of("#x(a=\"1\")= @y\n"), "#x{a: \"1\"}= @y\n");
        assert_eq!(ruby_hash_of("%a(href=\"x\")[@user] y\n"), "%a{href: \"x\"}[@user] y\n");
    }

    #[test]
    fn a_hash_becomes_html_attributes() {
        assert_eq!(html_of("%a.btn{href: \"/users\", target: \"_blank\"} Users\n"), "%a.btn(href=\"/users\" target=\"_blank\") Users\n");
        assert_eq!(
            html_of("%a{:href => '/x', \"data-n\" => @n, 'title': t, tabindex: 0}\n"),
            "%a(href='/x' data-n=@n title=t tabindex=0)\n"
        );
        assert_eq!(html_of("%input{ type: \"checkbox\", checked: true,}\n"), "%input(type=\"checkbox\" checked)\n");
        assert_eq!(html_of("%a{title: 'it\"s'}\n"), "%a(title='it\"s')\n");
        assert_eq!(html_of("%a{disabled: false, hidden: nil, href: \"#\"}\n"), "%a(disabled=false hidden=nil href=\"#\")\n");
    }

    #[test]
    fn attributes_that_the_other_syntax_cannot_say_stay() {
        for text in [
            "%a{href: x.y}\n",
            "%a{href: \"a#{b}\"}\n",
            "%a{href: \"a\\\"b\"}\n",
            "%a{**opts}\n",
            "%a{data: @h}\n",
            "%a{\"aria\" => x}\n",
            "%a{data: {a: 1}}\n",
            "%a{href: \"x\", href: \"y\"}\n",
            "%a{href: @x ? 1 : 2}\n",
            "%a{\"a b\" => 1}\n",
            "%a{href: \"x\"}(title=\"y\")\n",
            "%a(title=\"y\"){href: \"x\"}\n",
            "%a{href: \"x\",\n    title: \"y\"}\n",
            "%a{}\n",
            "%a{href: [1]}\n",
            "%a{key => 1}\n",
            "%a{href: :x}\n",
            "%a{href: %q(x)}\n",
        ] {
            assert_eq!(html_of(text), text, "{text:?}");
        }
        for text in [
            "%a(href=\"#{x}\")\n",
            "%a(href=x.y)\n",
            "%a(href=\"it\\'s\")\n",
            "%a(href=\"x\" href=\"y\")\n",
            "%a(href=\"x\" title=\"#@y\")\n",
            "%a(href = \"x\")\n",
            "%a(href=\"x\"\n   title=\"y\")\n",
            "%a(href=x2y.z)\n",
            "%a()\n",
            "%a(data=@h)\n",
            "%a(ARIA=x)\n",
            "%a(a=\"b\"c=\"d\")\n",
            "%a(href=\"x\"){title: \"y\"}\n",
            "%a(href=1x)\n",
        ] {
            assert_eq!(ruby_hash_of(text), text, "{text:?}");
        }
    }

    #[test]
    fn attribute_style_only_changes_the_other_style() {
        assert_eq!(html_of("%a(href=\"x\")\n"), "%a(href=\"x\")\n");
        assert_eq!(ruby_hash_of("%a{href: \"x\"}\n"), "%a{href: \"x\"}\n");
        assert_eq!(ruby_hash_of("%a(href=\"x\")\n%b{y: 1}\n%c(z)\n"), "%a{href: \"x\"}\n%b{y: 1}\n%c{z: true}\n");
    }

    #[test]
    fn a_rewrite_that_changes_a_tag_is_not_the_same() {
        let latest = ruby_prism::Version::Latest;
        assert!(same_tags("%p#a.b x\n", "%p.b#a x\n", latest, true));
        assert!(same_tags("%div.b= @x\n", ".b= @x\n", latest, true));
        for (before, after) in [
            ("%p#a.b x\n", "%p.c#a x\n"),
            ("%p#a.b x\n", "%p.b x\n"),
            ("%div.b\n", "%p.b\n"),
            ("%p.b{x: 1}\n", "%p.b{x: 2}\n"),
            ("%p.b= @x\n", "%p.b= @y\n"),
            ("%p.b\n", "%p.b\n%p\n"),
            ("%p(a=\"1\")\n", "%p(a=\"2\")\n"),
        ] {
            assert!(!same_tags(before, after, latest, true), "{before:?} {after:?}");
        }
        let options = ruby_prism::Options::default().build();
        assert_ne!(pairs_of("%p(a=@x)\n", &options), pairs_of("%p{a: @y}\n", &options));
    }

    #[test]
    fn a_tag_that_stays_does_not_stop_the_others() {
        let text = "%a{data: @h, href: \"/\"}\n%b{href: \"/b\"}\n";
        assert_eq!(html_of(text), "%a{data: @h, href: \"/\"}\n%b(href=\"/b\")\n");
        assert_eq!(ruby_hash_of("%a(data=@h)\n%b(x=\"1\")\n"), "%a(data=@h)\n%b{x: \"1\"}\n");
    }

    #[test]
    fn the_three_rules_combine() {
        let all =
            Settings { implicit_div: true, classes_before_ids: true, hash_attributes: Some(AttributeStyle::Html), ..Settings::default() };
        let text = "%div#main.wide{role: \"main\", tabindex: 0} Hi\n%div#a.b{role: \"x\"}\n";
        let first = heads(TemplateLang::Haml, text, &all, &[], ruby_prism::Version::Latest).unwrap().unwrap();
        assert_eq!(first, ".wide#main{role: \"main\", tabindex: 0} Hi\n.b#a{role: \"x\"}\n");
        assert_eq!(
            attributes(&first, &all, ruby_prism::Version::Latest).unwrap().unwrap(),
            ".wide#main(role=\"main\" tabindex=0) Hi\n.b#a(role=\"x\")\n"
        );
    }
}
