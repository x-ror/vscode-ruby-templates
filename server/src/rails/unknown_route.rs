//! `*_path` and `*_url` calls that the app's routes don't define, found in the Ruby the template compiles to and mapped back
//! to the template. Only a call without a receiver is checked (`admin.users_path` and `main_app.users_path` belong to an
//! engine's routes), and only where it cannot be something else: a Rails helper, a helper of the app, a `direct` route or
//! a local a partial is passed.

use crate::rails::helper_defs;
use crate::rails::paths::{rails_root, ruby_files_under};
use crate::rails::routes::{split_helper, Route};
use crate::ruby::generated::Compiled;
use crate::ruby::strict_locals::Signature;
use crate::util::{self, edit_distance, LineIndex};
use regex::Regex;
use ruby_prism::{CallNode, Node, Visit};
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;
use tower_lsp_server::ls_types::Diagnostic;

pub(crate) const UNKNOWN_ROUTE: &str = "Rails/UnknownRoute";

/// URL and path helpers of Rails and its default gems that no route defines.
const FRAMEWORK_HELPERS: [&str; 24] = [
    "asset_path",
    "asset_url",
    "audio_path",
    "audio_url",
    "compute_asset_path",
    "controller_path",
    "edit_polymorphic_path",
    "edit_polymorphic_url",
    "font_path",
    "font_url",
    "image_path",
    "image_url",
    "javascript_path",
    "javascript_url",
    "new_polymorphic_path",
    "new_polymorphic_url",
    "polymorphic_path",
    "polymorphic_url",
    "public_compute_asset_path",
    "stylesheet_path",
    "stylesheet_url",
    "video_path",
    "video_url",
    "url_path",
];
/// Helpers that Rails, Turbo, Devise and the asset bundlers define besides the routes.
const FRAMEWORK_PREFIXES: [&str; 7] = ["rails_", "turbo_", "action_mailbox_", "omniauth_", "vite_", "asset_pack_", "image_pack_"];
/// A name this many edits from the typed one is offered as the one meant.
const MAX_SUGGESTION_DISTANCE: usize = 2;

static DIRECT_ROUTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bdirect\s*\(?\s*:(\w+)").unwrap());

/// The template being checked.
pub(crate) struct Template<'a> {
    pub path: &'a Path,
    pub partial: bool,
    /// The strict locals the template declares; a partial without them may be passed any local.
    pub strict_locals: Option<&'a Signature>,
}

struct Call {
    name: String,
    /// `users_path` alone, as a local of the same name would be read.
    bare: bool,
    start: usize,
    end: usize,
}

/// The findings in `root`, the parse of `compiled`, the Ruby `text` compiles to. `routes` are all the app's routes.
pub(crate) fn diagnostics(text: &str, template: &Template, routes: &[Route], compiled: &Compiled, root: &Node) -> Vec<Diagnostic> {
    let mut finder = Finder { calls: Vec::new() };
    finder.visit(root);
    let index = LineIndex::new(text);
    // Per name, whether something other than a route may define it: it reads files, so once.
    let mut defined_elsewhere: HashMap<String, bool> = HashMap::new();
    let mut out = Vec::new();
    for call in finder.calls {
        let Some((base, suffix)) = split_helper(&call.name) else { continue };
        if routes.iter().any(|route| route.name == base) || is_framework(&call.name) {
            continue;
        }
        let suggestion = closest(routes, base).map(|near| format!("{near}{suffix}"));
        match (template.strict_locals, template.partial && call.bare) {
            (Some(signature), _) if signature.rest || signature.declares(&call.name) => continue,
            (None, true) if suggestion.is_none() => continue,
            _ => {}
        }
        if *defined_elsewhere.entry(call.name.clone()).or_insert_with(|| defined_by_app(template.path, &call.name)) {
            continue;
        }
        let Some((start, end)) = compiled.template_span(call.start, call.end) else { continue };
        let hint = suggestion.map(|near| format!("; did you mean `{near}`?")).unwrap_or_default();
        let message = format!("`{}` is not a route helper of this app{hint}", call.name);
        out.push(util::coded(util::warning(index.range(start, end), message), UNKNOWN_ROUTE));
    }
    out
}

fn is_framework(name: &str) -> bool {
    FRAMEWORK_HELPERS.contains(&name) || FRAMEWORK_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// The route name closest to `base`, when it is near enough to be the one meant.
fn closest<'a>(routes: &'a [Route], base: &str) -> Option<&'a str> {
    let near = routes
        .iter()
        .filter(|route| !route.name.is_empty())
        .map(|route| (edit_distance(base, &route.name), route.name.as_str()))
        .filter(|&(distance, _)| distance <= MAX_SUGGESTION_DISTANCE && distance < base.chars().count())
        .min_by_key(|&(distance, _)| distance);
    near.map(|(_, name)| name)
}

/// Whether the app's helpers or controllers define `name`, or its route files declare it with `direct`.
fn defined_by_app(template: &Path, name: &str) -> bool {
    if !helper_defs::definitions(template, name).is_empty() {
        return true;
    }
    let Some(config) = rails_root(template).map(|root| root.join("config")) else { return false };
    let mut files = ruby_files_under(&config.join("routes"));
    files.push(config.join("routes.rb"));
    let base = split_helper(name).map_or(name, |(base, _)| base);
    files
        .iter()
        .filter_map(|file| std::fs::read_to_string(file).ok())
        .any(|code| DIRECT_ROUTE.captures_iter(&code).any(|cap| &cap[1] == base))
}

struct Finder {
    calls: Vec<Call>,
}

impl<'pr> Visit<'pr> for Finder {
    fn visit_call_node(&mut self, call: &CallNode<'pr>) {
        let name = String::from_utf8_lossy(call.name().as_slice());
        if let (None, Some(at)) = (call.receiver(), call.message_loc()) {
            if split_helper(&name).is_some() {
                let bare = call.is_variable_call();
                self.calls.push(Call { name: name.into_owned(), bare, start: at.start() as usize, end: at.end() as usize });
            }
        }
        ruby_prism::visit_call_node(self, call);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_context;
    use crate::lang::TemplateLang::{self, Erb, Haml, Slim};
    use crate::rails::routes::parse;

    fn routes() -> Vec<Route> {
        let names = ["users", "user", "edit_user", "root"];
        parse(Path::new("/"), &serde_json::Value::Array(names.iter().map(|name| serde_json::json!({ "name": name })).collect()))
    }

    /// The flagged text and message of each finding in a template at `file` of an app with no helpers.
    fn found(lang: TemplateLang, file: &str, text: &str, strict_locals: Option<&Signature>) -> Vec<(String, String)> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rt-unknown-route-{}-{n}", std::process::id()));
        util::write_files(
            &root,
            &[
                ("config/routes.rb", "direct :homepage do\n  \"/\"\nend\n"),
                ("app/helpers/x_helper.rb", "module XHelper\n  def custom_path\n  end\nend\n"),
            ],
        );
        let path = root.join("app/views").join(file);
        let compiled = code_context::compiled(lang, text, None, &[]).unwrap();
        let options = ruby_prism::Options::default().build();
        let result = compiled.parse(&options).unwrap();
        let template = Template { path: &path, partial: file.contains("/_"), strict_locals };
        let found = diagnostics(text, &template, &routes(), &compiled, &result.node());
        let index = LineIndex::new(text);
        let at = |position| (0..=text.len()).find(|&offset| index.position(offset) == position).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        found.into_iter().map(|d| (text[at(d.range.start)..at(d.range.end)].to_string(), d.message)).collect()
    }

    fn flagged(lang: TemplateLang, file: &str, text: &str) -> Vec<String> {
        found(lang, file, text, None).into_iter().map(|(flagged, _)| flagged).collect()
    }

    #[test]
    fn an_undefined_helper_is_flagged_with_the_route_meant() {
        assert_eq!(
            found(Erb, "users/index.html.erb", "<%= link_to \"Users\", usrs_path %>\n", None),
            [("usrs_path".into(), "`usrs_path` is not a route helper of this app; did you mean `users_path`?".into())]
        );
        assert_eq!(flagged(Haml, "users/index.html.haml", "= link_to \"Users\", usrs_path\n"), ["usrs_path"]);
        assert_eq!(flagged(Slim, "users/index.html.slim", "= link_to \"Users\", usrs_path\n"), ["usrs_path"]);
        assert_eq!(flagged(Erb, "users/index.html.erb", "<%= edit_user_url(@user) %><%= edt_user_url(@user) %>\n"), ["edt_user_url"]);
        assert_eq!(flagged(Slim, "users/index.html.slim", "a href=dashboard_path(1) x\n"), ["dashboard_path"]);
        let far = found(Erb, "users/index.html.erb", "<%= dashboard_path %>\n", None);
        assert_eq!(far[0].1, "`dashboard_path` is not a route helper of this app");
    }

    #[test]
    fn defined_helpers_and_other_helpers_are_quiet() {
        let erb = "<%= users_path %><%= user_url(@user) %><%= root_path %><%= asset_path(\"a.png\") %><%= image_url(\"a\") %>\n\
                   <%= polymorphic_path(@user) %><%= url_for(@user) %><%= rails_blob_path(@b) %><%= turbo_stream_path %>\n\
                   <%= custom_path %><%= homepage_url %><%= @user.avatar_path %><%= main_app.dash_path %><%= admin.users_path %>\n\
                   <%= :dash_path %><%= x(\"dash_path\") %><%= _path %><%= path %><%= users_paths %>\n";
        assert!(flagged(Erb, "users/index.html.erb", erb).is_empty());
        assert!(flagged(Haml, "users/index.html.haml", "= users_path\n= custom_path\n= main_app.dash_path\n").is_empty());
        assert!(flagged(Slim, "users/index.html.slim", "= users_path\n= custom_path\n= main_app.dash_path\n").is_empty());
    }

    #[test]
    fn locals_shadow_helpers() {
        assert!(flagged(Erb, "users/index.html.erb", "<% dash_path = 1 %><%= dash_path %>\n").is_empty());
        assert!(flagged(Erb, "users/index.html.erb", "<% [1].each do |dash_path| %><%= dash_path %><% end %>\n").is_empty());
        assert!(flagged(Haml, "users/index.html.haml", "- dash_path = 1\n= dash_path\n").is_empty());
        assert!(flagged(Slim, "users/index.html.slim", "- dash_path = 1\n= dash_path\n").is_empty());
    }

    #[test]
    fn a_partial_may_be_passed_a_local_that_looks_like_a_helper() {
        let (bare, called) = ("<%= link_to \"x\", dash_path %>\n<%= link_to \"x\", usr_path %>\n", "<%= dash_path(1) %>\n");
        assert_eq!(flagged(Erb, "users/_card.html.erb", bare), ["usr_path"], "only the one with a route near it");
        assert_eq!(flagged(Erb, "users/_card.html.erb", called), ["dash_path"], "a local can't be called");
        assert_eq!(flagged(Erb, "users/index.html.erb", bare), ["dash_path", "usr_path"]);
        let strict = |locals: &[&str], rest| Signature {
            params: String::new(),
            locals: locals.iter().map(|name| (name.to_string(), None)).collect(),
            rest,
        };
        assert!(found(Erb, "users/_card.html.erb", bare, Some(&strict(&["dash_path", "usr_path"], false))).is_empty());
        assert!(found(Erb, "users/_card.html.erb", bare, Some(&strict(&[], true))).is_empty());
        assert_eq!(found(Erb, "users/_card.html.erb", bare, Some(&strict(&["dash_path"], false))).len(), 1);
        assert_eq!(found(Erb, "users/_card.html.erb", bare, Some(&strict(&[], false))).len(), 2);
    }
}
