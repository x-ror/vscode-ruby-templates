//! Output that skips HTML escaping (`raw`, `html_safe`, `safe_concat` and the unescaped output markers), found in the Ruby
//! the template compiles to and mapped back to the template. What is certainly safe, such as a literal or the result of
//! `sanitize`, is not reported.

use crate::lang::TemplateLang;
use crate::rails::output_lints::TAG_BUILDER_ELEMENTS;
use crate::ruby::generated::Compiled;
use crate::util::{self, LineIndex};
use ruby_prism::{CallNode, Node, Visit};
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use tower_lsp_server::ls_types::Diagnostic;

pub(crate) const UNSAFE_OUTPUT: &str = "Rails/UnsafeOutput";

const SANITIZE_HINT: &str = "use `sanitize` if it holds user HTML";
/// Longer code is not quoted in the message.
const MAX_QUOTED_LEN: usize = 40;

/// Helpers whose result is safe markup, or is escaped for the place it goes to.
const SAFE_HELPERS: &[&str] = &[
    "sanitize",
    "safe_join",
    "strip_tags",
    "json_escape",
    "j",
    "escape_javascript",
    "h",
    "html_escape",
    "escape_once",
    "link_to",
    "button_to",
    "mail_to",
    "content_tag",
    "image_tag",
    "simple_format",
    "render",
    "capture",
    "content_for",
    "provide",
];
/// Escaping helpers on modules that are not the view's own: `Markdown.render(x)` is not `render`.
const ESCAPERS: &[(&str, &[&str])] = &[
    ("ERB::Util", &["html_escape", "h", "json_escape", "html_escape_once"]),
    ("::ERB::Util", &["html_escape", "h", "json_escape", "html_escape_once"]),
    ("CGI", &["escapeHTML", "escape_html"]),
    ("Rack::Utils", &["escape_html"]),
    ("ActionController::Base.helpers", &["sanitize", "strip_tags", "safe_join"]),
    ("ApplicationController.helpers", &["sanitize", "strip_tags", "safe_join"]),
];
/// Where a view's JSON is output as a script's code.
const SCRIPT_BLOCKS: &[&str] = &["script", "%script", ":javascript", "javascript:"];
const TRANSLATIONS: &[&str] = &["t", "translate"];
/// Rails marks the translations under these keys as safe, and escapes what is interpolated into them.
const HTML_KEY_ENDINGS: &[&str] = &["_html", ".html"];
/// Unescaped output markers in Haml, which the code follows.
const HAML_MARKERS: &[&str] = &["!=", "!~"];

/// The findings in `root`, the parse of `compiled`, the Ruby `text` compiles to.
pub(crate) fn diagnostics(lang: TemplateLang, text: &str, compiled: &Compiled, root: &Node) -> Vec<Diagnostic> {
    let mut finder = Finder { lang, text, compiled, runs: HashSet::new(), found: BTreeMap::new() };
    finder.visit(root);
    let index = LineIndex::new(text);
    finder
        .found
        .into_iter()
        .map(|(start, (end, message))| util::coded(util::warning(index.range(start, end), message), UNSAFE_OUTPUT))
        .collect()
}

struct Finder<'a> {
    lang: TemplateLang,
    text: &'a str,
    compiled: &'a Compiled,
    /// Where in the template the runs of code already looked at start.
    runs: HashSet<usize>,
    /// By start in the template: the end and the message.
    found: BTreeMap<usize, (usize, String)>,
}

impl Finder<'_> {
    fn record(&mut self, span: (usize, usize), subject: &str, expression: &Node) {
        let json = expression.as_call_node().is_some_and(|call| call.name().as_slice() == b"to_json");
        // ActiveSupport escapes `<`, `>` and `&` in JSON, which is all a script needs.
        if json && in_script(self.lang, self.text, span.0) {
            return;
        }
        let hint = if json {
            format!("`to_json` doesn't escape quotes for an attribute; output it without {subject}")
        } else {
            SANITIZE_HINT.into()
        };
        let message = format!("{subject} outputs {} without escaping; {hint}", self.quoted(expression));
        self.found.entry(span.0).or_insert((span.1, message));
    }

    fn quoted(&self, expression: &Node) -> String {
        let location = expression.location();
        let code = &self.compiled.source[location.start() as usize..location.end() as usize];
        if code.len() > MAX_QUOTED_LEN || code.contains('\n') {
            return "its value".into();
        }
        format!("`{code}`")
    }

    /// A node that is the whole code of an output tag or line, and has an unescaped marker before it.
    fn check_run(&mut self, node: Node) {
        let location = node.location();
        let (start, end) = (location.start() as usize, location.end() as usize);
        if !self.compiled.fills_run(start, end) {
            return;
        }
        let Some((run_start, _)) = self.compiled.template_span(start, end) else { return };
        // The outermost node of a run comes first.
        if !self.runs.insert(run_start) {
            return;
        }
        let Some(marker) = unescaped_marker(self.lang, self.text, run_start) else { return };
        let expression = unwrapped(node);
        if is_safe(&self.compiled.source, &expression) || is_unescaping(&expression) {
            return;
        }
        self.record((marker.start, marker.end), &format!("`{}`", &self.text[marker.clone()]), &expression);
    }
}

impl<'pr> Visit<'pr> for Finder<'_> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        self.check_run(node);
    }

    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        self.check_run(node);
    }

    fn visit_call_node(&mut self, call: &CallNode<'pr>) {
        let name_span = call.message_loc().and_then(|name| self.compiled.template_span(name.start() as usize, name.end() as usize));
        if let (Some(span), Some(target)) = (name_span, unescaped_argument(call)) {
            let target = unwrapped(target);
            // `raw(x).html_safe` is reported once, at `raw`.
            if !is_safe(&self.compiled.source, &target) && !is_unescaping(&target) {
                let subject = format!("`{}`", String::from_utf8_lossy(call.name().as_slice()));
                self.record(span, &subject, &target);
            }
        }
        ruby_prism::visit_call_node(self, call);
    }
}

/// What `raw x`, `x.html_safe` and `safe_concat x` let through unescaped.
fn unescaped_argument<'pr>(call: &CallNode<'pr>) -> Option<Node<'pr>> {
    match call.name().as_slice() {
        b"raw" if call.receiver().is_none() => call.arguments()?.arguments().iter().next(),
        b"safe_concat" => call.arguments()?.arguments().iter().next(),
        b"html_safe" if call.arguments().is_none() => call.receiver(),
        _ => None,
    }
}

fn is_unescaping(expression: &Node) -> bool {
    expression.as_call_node().is_some_and(|call| unescaped_argument(&call).is_some())
}

/// The parentheses and the single statement around an expression removed.
fn unwrapped<'pr>(node: Node<'pr>) -> Node<'pr> {
    if let Some(inner) = node.as_parentheses_node().and_then(|parentheses| parentheses.body()) {
        return unwrapped(inner);
    }
    if let Some(statements) = node.as_statements_node() {
        let mut body = statements.body().iter();
        if let (Some(only), None) = (body.next(), body.next()) {
            return unwrapped(only);
        }
    }
    node
}

/// Whether the expression is certainly safe to output unescaped: a literal, or what a helper that makes safe HTML returns.
fn is_safe(source: &str, expression: &Node) -> bool {
    let adjacent_literals =
        expression.as_interpolated_string_node().is_some_and(|string| string.parts().iter().all(|part| part.as_string_node().is_some()));
    let literal = adjacent_literals
        || expression.as_yield_node().is_some()
        || expression.as_string_node().is_some()
        || expression.as_symbol_node().is_some()
        || expression.as_integer_node().is_some()
        || expression.as_float_node().is_some()
        || expression.as_nil_node().is_some()
        || expression.as_true_node().is_some()
        || expression.as_false_node().is_some();
    let Some(call) = expression.as_call_node() else { return literal };
    let name = String::from_utf8_lossy(call.name().as_slice());
    let Some(receiver) = call.receiver() else {
        return if TRANSLATIONS.contains(&name.as_ref()) { translates_html(&call) } else { SAFE_HELPERS.contains(&name.as_ref()) };
    };
    let location = receiver.location();
    let receiver_code = &source[location.start() as usize..location.end() as usize];
    let escapes = ESCAPERS.iter().any(|(module, names)| *module == receiver_code && names.contains(&name.as_ref()));
    let is_tag_builder = receiver.as_call_node().is_some_and(|r| r.is_variable_call() && r.name().as_slice() == b"tag");
    escapes || (is_tag_builder && TAG_BUILDER_ELEMENTS.contains(&name.as_ref()))
}

/// `t(".name_html")`: the key is a literal that Rails marks safe.
fn translates_html(call: &CallNode) -> bool {
    let Some(key) = call.arguments().and_then(|arguments| arguments.arguments().iter().next()) else { return false };
    let key = key.as_string_node().map(|key| key.unescaped().to_vec()).or_else(|| key.as_symbol_node().map(|key| key.unescaped().to_vec()));
    key.is_some_and(|key| HTML_KEY_ENDINGS.iter().any(|ending| key.ends_with(ending.as_bytes())))
}

/// Whether the template text at `offset` is a script's code: after an unclosed `<script` in ERB, under a script block in
/// Haml and Slim.
fn in_script(lang: TemplateLang, text: &str, offset: usize) -> bool {
    if lang == TemplateLang::Erb {
        let before = text[..offset].to_ascii_lowercase();
        return before.rfind("<script").is_some_and(|open| before.rfind("</script").is_none_or(|close| close < open));
    }
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let current = &text[line_start..offset];
    let indent_of = |line: &str| line.len() - line.trim_start().len();
    let opens_script = |line: &str| {
        let word = line.trim_start().split(|c: char| !(c.is_alphanumeric() || matches!(c, '%' | ':'))).next().unwrap_or("");
        SCRIPT_BLOCKS.contains(&word) || line.trim_start().starts_with("javascript:")
    };
    if opens_script(current) {
        return true;
    }
    let mut limit = indent_of(current);
    for line in text[..line_start].lines().rev().filter(|line| !line.trim().is_empty()) {
        if indent_of(line) < limit {
            if opens_script(line) {
                return true;
            }
            limit = indent_of(line);
        }
    }
    false
}

/// The marker (`<%==`, Haml's `!=`, Slim's `==`) just before the code at `offset`, as a range of the template.
fn unescaped_marker(lang: TemplateLang, text: &str, offset: usize) -> Option<Range<usize>> {
    if lang == TemplateLang::Erb {
        let open = text[..offset].rfind("<%")?;
        return (text[open..offset].trim_end() == "<%==").then_some(open..open + "<%==".len());
    }
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let before = text[line_start..offset].trim_end();
    let (marker, end) = match lang {
        TemplateLang::Haml => (HAML_MARKERS.iter().copied().find(|marker| before.ends_with(marker))?, before.len()),
        // `==>`, `==<` and `=='` keep or trim the space around the output.
        TemplateLang::Slim => {
            let before = before.trim_end_matches(['<', '>', '\'']);
            (before.strip_suffix("==").map(|_| "==")?, before.len())
        }
        _ => return None,
    };
    Some(line_start + end - marker.len()..line_start + end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_context;
    use TemplateLang::{Erb, Haml, Slim};

    /// The flagged text and the message of each finding.
    fn found(lang: TemplateLang, text: &str) -> Vec<(String, String)> {
        let compiled = code_context::compiled(lang, text, None, &[]).unwrap();
        let options = ruby_prism::Options::default().build();
        let result = compiled.parse(&options).unwrap();
        let index = LineIndex::new(text);
        let at = |position| (0..=text.len()).find(|&offset| index.position(offset) == position).unwrap();
        diagnostics(lang, text, &compiled, &result.node())
            .into_iter()
            .map(|d| (text[at(d.range.start)..at(d.range.end)].to_string(), d.message))
            .collect()
    }

    fn flagged(lang: TemplateLang, text: &str) -> Vec<String> {
        found(lang, text).into_iter().map(|(flagged, _)| flagged).collect()
    }

    #[test]
    fn raw_and_html_safe_in_every_language() {
        assert_eq!(
            found(Erb, "<%= raw @user.bio %>\n"),
            [("raw".into(), "`raw` outputs `@user.bio` without escaping; use `sanitize` if it holds user HTML".into())]
        );
        assert_eq!(flagged(Erb, "<%= @user.bio.html_safe %>\n"), ["html_safe"]);
        assert_eq!(flagged(Erb, "<% bio = raw(@bio) %>\n"), ["raw"]);
        assert_eq!(flagged(Haml, "= raw @user.bio\n%p= @user.bio.html_safe\n"), ["raw", "html_safe"]);
        assert_eq!(flagged(Slim, "= raw @user.bio\np = @user.bio.html_safe\n"), ["raw", "html_safe"]);
        assert_eq!(flagged(Erb, "<%= \"<b>#{@name}</b>\".html_safe %>\n"), ["html_safe"]);
        assert_eq!(flagged(Erb, "<%= output_buffer.safe_concat(@x) %>\n"), ["safe_concat"]);
    }

    #[test]
    fn attributes_and_scripts() {
        assert_eq!(flagged(Erb, "<a href=\"<%= raw @user.website %>\">Website</a>\n"), ["raw"]);
        let attribute = found(Erb, "<div data-bio=\"<%= raw @user.bio.to_json %>\">\n");
        assert_eq!(
            attribute[0].1,
            "`raw` outputs `@user.bio.to_json` without escaping; `to_json` doesn't escape quotes for an attribute; output it without `raw`"
        );
    }

    #[test]
    fn unescaped_output_markers() {
        let message = |marker: &str| format!("`{marker}` outputs `@user.bio` without escaping; use `sanitize` if it holds user HTML");
        assert_eq!(found(Erb, "<%== @user.bio %>\n"), [("<%==".into(), message("<%=="))]);
        assert_eq!(found(Haml, "!= @user.bio\n"), [("!=".into(), message("!="))]);
        assert_eq!(flagged(Haml, "%p!= @user.bio\n"), ["!="]);
        assert_eq!(found(Slim, "== @user.bio\n"), [("==".into(), message("=="))]);
        assert_eq!(flagged(Slim, "p == @user.bio\np==> @user.bio\na href==@user.website\n"), ["==", "==", "=="]);
        // The call is the finding when the marker also leaves it unescaped.
        assert_eq!(flagged(Erb, "<%== raw @x %>\n"), ["raw"]);
        assert_eq!(flagged(Slim, "== @x.html_safe\n"), ["html_safe"]);
    }

    #[test]
    fn literals_are_quiet() {
        assert!(flagged(Erb, "<%= \"<br>\".html_safe %>\n<%= raw \"&nbsp;\" %>\n<%== \"<hr>\" %>\n<%= raw(:x) %>\n").is_empty());
        assert!(flagged(Haml, "= raw \"&nbsp;\"\n!= \"<br>\"\n").is_empty());
        assert!(flagged(Slim, "= raw \"&nbsp;\"\n== \"<br>\"\n").is_empty());
    }

    #[test]
    fn html_translations_and_safe_helpers_are_quiet() {
        let erb = "<%= raw t(\".title_html\") %>\n<%= t(\"x.y_html\", name: @n).html_safe %>\n<%== t(:note_html) %>\n<%= raw sanitize(@x) %>\n\
                   <%= sanitize(@x).html_safe %>\n<%= raw safe_join(@xs, \"<br>\") %>\n<%= raw strip_tags(@x) %>\n<%= raw json_escape(@x.to_json) %>\n\
                   <%= raw j(@x) %>\n<%= content_tag(:p, @x).html_safe %>\n<%= tag.div(@x).html_safe %>\n<%== render \"x\" %>\n\
                   <%= raw capture { 1 } %>\n<%= link_to(\"a\", @u).html_safe %>\n<%= raw ERB::Util.html_escape(@x) %>\n";
        assert!(flagged(Erb, erb).is_empty());
        assert!(flagged(Haml, "= raw sanitize(@x)\n!= t(\".a_html\")\n!= render \"x\"\n").is_empty());
        assert!(flagged(Slim, "= raw sanitize(@x)\n== t(\".a_html\")\n== render \"x\"\n").is_empty());
    }

    #[test]
    fn other_calls_and_names_are_not_confused() {
        assert_eq!(flagged(Erb, "<%= raw t(\".title\") %>\n<%= I18n.t(\"a_html\").html_safe %>\n"), ["raw", "html_safe"]);
        assert_eq!(flagged(Erb, "<%= raw %>\n<%= @a == @b %>\n<%= html_safe %>\n"), Vec::<String>::new());
        assert!(flagged(Haml, "- x = a != b\n= a != b\n%p= a == b\n").is_empty());
        assert!(flagged(Slim, "- x = a == b\n= a == b\np = a == b\n").is_empty());
    }

    #[test]
    fn json_is_fine_in_a_script() {
        assert!(flagged(Erb, "<script>var bio = <%= raw @user.bio.to_json %>;</script>\n").is_empty());
        assert!(flagged(Erb, "<script>\n  var a = <%== @a.to_json %>;\n</script>\n").is_empty());
        assert_eq!(flagged(Erb, "<script>1</script>\n<p x=\"<%= raw @a.to_json %>\">\n"), ["raw"]);
        assert!(flagged(Haml, "%script\n  = raw @a.to_json\n").is_empty());
        assert!(flagged(Slim, "script\n  == @a.to_json\n").is_empty());
        assert!(flagged(Slim, "javascript:\n  x\np\n  = raw @a.to_json\n").len() == 1);
        assert_eq!(flagged(Haml, "%p\n  = raw @a.to_json\n"), ["raw"]);
        assert_eq!(flagged(Slim, "div\n  script\n    = 1\n  p = raw @a.to_json\n"), ["raw"]);
    }

    #[test]
    fn layout_content_is_safe() {
        assert!(flagged(
            Erb,
            "<%== yield %>\n<%= raw yield(:head) %>\n<%== content_for(:x) %>\n<%= yield.html_safe %>\n<%= raw provide(:a, 1) %>\n"
        )
        .is_empty());
        assert!(flagged(Haml, "!= yield\n= raw content_for(:x)\n").is_empty());
        assert!(flagged(Slim, "== yield\n= raw content_for(:x)\n").is_empty());
    }

    #[test]
    fn helpers_on_other_modules_are_not_the_views_own() {
        assert_eq!(
            flagged(Erb, "<%= raw MARKDOWN.render(@post.body) %>\n<%= Markdown.render(@x).html_safe %>\n<%= raw Foo::Bar.link_to(@x) %>\n"),
            ["raw", "html_safe", "raw"]
        );
        let safe = "<%= raw ERB::Util.html_escape(@x) %>\n<%= raw ERB::Util.h(@x) %>\n<%= CGI.escapeHTML(@x).html_safe %>\n<%= raw Rack::Utils.escape_html(@x) %>\n\
                    <%= raw ActionController::Base.helpers.sanitize(@x) %>\n<%= ApplicationController.helpers.sanitize(@x).html_safe %>\n";
        assert!(flagged(Erb, safe).is_empty());
    }

    #[test]
    fn nested_unescaping_is_reported_once() {
        assert_eq!(flagged(Erb, "<%= raw(@a).html_safe %>\n"), ["raw"]);
        assert_eq!(flagged(Erb, "<%= @a.html_safe.html_safe %>\n"), ["html_safe"]);
        assert!(flagged(Erb, "<%= \"x\".html_safe.html_safe %>\n").is_empty());
    }

    #[test]
    fn adjacent_literals_are_literals() {
        assert!(flagged(Erb, "<%= raw \"a\" \"<b>\" %>\n<%= (\"a\" 'b').html_safe %>\n").is_empty());
        assert_eq!(flagged(Erb, "<%= raw \"a\" \"#{@b}\" %>\n"), ["raw"]);
    }
}
