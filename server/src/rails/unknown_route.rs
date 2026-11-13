//! `*_path` and `*_url` calls that the app's routes don't define but come close to one (`usr_path` for `user_path`), found in
//! the Ruby the template compiles to and mapped back to the template. Typos are the point: a name with no route near it may be
//! a helper of a gem, a concern or `lib` that can't be seen. Only a call without a receiver is checked (`admin.users_path` and
//! `main_app.users_path` belong to an engine's routes), not one a local, a helper of the app or a `direct` route can be, and
//! not one under `defined?`.

use crate::rails::helper_defs;
use crate::rails::paths::{app_root, rails_root, ruby_files_under};
use crate::rails::routes::{split_helper, Route};
use crate::ruby::generated::Compiled;
use crate::ruby::strict_locals::Signature;
use crate::util::{self, edit_distance, LineIndex};
use dashmap::DashMap;
use regex::Regex;
use ruby_prism::{CallNode, Node, Visit};
use std::path::Path;
use std::path::PathBuf;
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
/// The URL helpers Devise defines for its controllers' views (`new_session_path`, `edit_password_url`), whichever scope.
const DEVISE_HELPERS: [&str; 10] = [
    "session_path",
    "session_url",
    "registration_path",
    "registration_url",
    "password_path",
    "password_url",
    "confirmation_path",
    "confirmation_url",
    "unlock_path",
    "unlock_url",
];
const DEVISE_ACTION_PREFIXES: [&str; 4] = ["", "new_", "edit_", "cancel_"];
/// Helpers that Rails, Turbo, Devise and the asset bundlers define besides the routes.
const FRAMEWORK_PREFIXES: [&str; 7] = ["rails_", "turbo_", "action_mailbox_", "omniauth_", "vite_", "asset_pack_", "image_pack_"];
/// A name this many edits from the typed one is offered as the one meant.
const MAX_SUGGESTION_DISTANCE: usize = 2;

static DIRECT_ROUTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bdirect\s*\(?\s*:(\w+)").unwrap());

/// The template being checked.
pub(crate) struct Template<'a> {
    pub path: &'a Path,
    /// The strict locals the template declares. Without them any local may be passed to it, so a bare name is left alone.
    pub strict_locals: Option<&'a Signature>,
    /// The app uses Devise, whose views call helpers its controllers define.
    pub devise: bool,
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
    let mut out = Vec::new();
    for call in finder.calls {
        let Some((base, suffix)) = split_helper(&call.name) else { continue };
        if routes.iter().any(|route| route.name == base) || is_framework(&call.name, template.devise) {
            continue;
        }
        let Some(near) = closest(routes, base) else { continue };
        // A bare name is a local unless the signature rules it out.
        let ruled_out = template.strict_locals.is_some_and(|signature| !signature.rest && !signature.declares(&call.name));
        if (call.bare && !ruled_out) || defined_by_app(template.path, &call.name) {
            continue;
        }
        let Some((start, end)) = compiled.template_span(call.start, call.end) else { continue };
        let replacement = format!("{near}{suffix}");
        let message = format!("`{}` is not a route helper of this app; did you mean `{replacement}`?", call.name);
        let found = util::coded(util::warning(index.range(start, end), message), UNKNOWN_ROUTE);
        out.push(util::with_data(found, serde_json::json!({ "replace": replacement })));
    }
    out
}

fn is_framework(name: &str, devise: bool) -> bool {
    let devise_helper =
        devise && DEVISE_ACTION_PREFIXES.iter().any(|action| name.strip_prefix(action).is_some_and(|rest| DEVISE_HELPERS.contains(&rest)));
    devise_helper || FRAMEWORK_HELPERS.contains(&name) || FRAMEWORK_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
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

static DEFINED: LazyLock<DashMap<(PathBuf, String), bool>> = LazyLock::new(DashMap::new);

/// Forgets what `defined_by_app` found; the next call reads the files again.
pub(crate) fn clear_cache() {
    DEFINED.clear();
}

/// Whether the app's helpers or controllers define `name`, or its route files declare it with `direct`. Kept until `clear_cache`.
fn defined_by_app(template: &Path, name: &str) -> bool {
    let Some(root) = rails_root(template) else { return false };
    let key = (root, name.to_string());
    if let Some(found) = DEFINED.get(&key) {
        return *found;
    }
    let found = scan_definitions(template, &key.0, name);
    DEFINED.insert(key, found);
    found
}

fn scan_definitions(template: &Path, root: &Path, name: &str) -> bool {
    if !helper_defs::definitions(template, name).is_empty() || named_by_symbol(template, name) {
        return true;
    }
    let config = root.join("config");
    let mut files = ruby_files_under(&config.join("routes"));
    files.push(config.join("routes.rb"));
    let base = split_helper(name).map_or(name, |(base, _)| base);
    files
        .iter()
        .filter_map(|file| std::fs::read_to_string(file).ok())
        .any(|code| DIRECT_ROUTE.captures_iter(&code).any(|cap| &cap[1] == base))
}

/// Whether a helper or controller names `:name`, as `delegate :name`, `define_method(:name)` and `alias_method` do.
fn named_by_symbol(template: &Path, name: &str) -> bool {
    let Some(app) = app_root(template) else { return false };
    let symbol = format!(":{name}");
    ["helpers", "controllers"]
        .iter()
        .flat_map(|dir| ruby_files_under(&app.join(dir)))
        .any(|file| std::fs::read_to_string(file).is_ok_and(|code| code.contains(&symbol)))
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

    /// `defined?(back_path)` asks whether the helper exists.
    fn visit_defined_node(&mut self, _node: &ruby_prism::DefinedNode<'pr>) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_context;
    use crate::lang::TemplateLang::{self, Erb, Haml, Slim};
    use crate::rails::routes::parse;

    fn routes() -> Vec<Route> {
        let names = ["users", "user", "edit_user", "root", "items", "sessions"];
        parse(Path::new("/"), &serde_json::Value::Array(names.iter().map(|name| serde_json::json!({ "name": name })).collect()))
    }

    /// The findings in a template of an app whose helper defines `custom_path`.
    fn findings(lang: TemplateLang, text: &str, strict_locals: Option<&Signature>, devise: bool) -> Vec<Diagnostic> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rt-unknown-route-{}-{n}", std::process::id()));
        util::write_files(
            &root,
            &[
                ("config/routes.rb", "direct :homepage do\n  \"/\"\nend\n"),
                ("app/helpers/x_helper.rb", "module XHelper\n  def custom_path\n  end\n  delegate :usre_path, to: :helpers\nend\n"),
            ],
        );
        let path = root.join("app/views/users/index.html.erb");
        let compiled = code_context::compiled(lang, text, None, &[]).unwrap();
        let options = ruby_prism::Options::default().build();
        let result = compiled.parse(&options).unwrap();
        let template = Template { path: &path, strict_locals, devise };
        let found = diagnostics(text, &template, &routes(), &compiled, &result.node());
        std::fs::remove_dir_all(&root).unwrap();
        found
    }

    /// The flagged text and message of each finding in a template of an app whose helper defines `custom_path`.
    fn found_in(lang: TemplateLang, text: &str, strict_locals: Option<&Signature>, devise: bool) -> Vec<(String, String)> {
        let index = LineIndex::new(text);
        let at = |position| (0..=text.len()).find(|&offset| index.position(offset) == position).unwrap();
        let found = findings(lang, text, strict_locals, devise);
        found.into_iter().map(|d| (text[at(d.range.start)..at(d.range.end)].to_string(), d.message)).collect()
    }

    fn flagged(lang: TemplateLang, text: &str) -> Vec<String> {
        found_in(lang, text, None, false).into_iter().map(|(flagged, _)| flagged).collect()
    }

    fn strict(locals: &[&str], rest: bool) -> Signature {
        Signature { params: String::new(), locals: locals.iter().map(|name| (name.to_string(), None)).collect(), rest }
    }

    #[test]
    fn a_call_near_a_route_is_flagged_with_the_route_meant() {
        assert_eq!(
            found_in(Erb, "<%= link_to \"x\", usrs_path(page: 1) %>\n", None, false),
            [("usrs_path".into(), "`usrs_path` is not a route helper of this app; did you mean `users_path`?".into())]
        );
        assert_eq!(flagged(Haml, "= link_to \"x\", usr_path(@user)\n"), ["usr_path"]);
        assert_eq!(flagged(Slim, "= link_to \"x\", usr_path(@user)\n"), ["usr_path"]);
        assert_eq!(flagged(Erb, "<%= edit_user_url(@user) %><%= edt_user_url(@user) %>\n"), ["edt_user_url"]);
    }

    #[test]
    fn the_diagnostic_carries_the_route_meant() {
        let found = findings(Erb, "<%= edt_user_url(@user) %>\n", None, false);
        assert_eq!(util::data_str(&found[0], "replace"), Some("edit_user_url"));
    }

    #[test]
    fn a_name_with_no_route_near_it_may_be_a_helper_we_cannot_see() {
        let erb = "<%= dashboard_path(1) %><%= cloudinary_url(@x) %><%= cloudinary_url %>\n";
        assert!(flagged(Erb, erb).is_empty());
        assert!(flagged(Haml, "= dashboard_path(1)\n").is_empty());
        assert!(flagged(Slim, "= dashboard_path(1)\n").is_empty());
    }

    #[test]
    fn defined_helpers_and_other_helpers_are_quiet() {
        let erb = "<%= users_path %><%= user_url(@user) %><%= root_path %><%= asset_path(\"a.png\") %><%= image_url(\"a\") %>\n\
                   <%= polymorphic_path(@user) %><%= url_for(@user) %><%= rails_blob_path(@b) %><%= turbo_stream_path %>\n\
                   <%= custom_path(1) %><%= homepage_url(1) %><%= @user.usrs_path %><%= main_app.usrs_path %><%= admin.usrs_path %>\n\
                   <%= :usrs_path %><%= x(\"usrs_path\") %><%= _path %><%= path %><%= users_paths %>\n";
        assert!(flagged(Erb, erb).is_empty());
        assert!(flagged(Haml, "= users_path\n= custom_path(1)\n= main_app.usrs_path\n").is_empty());
        assert!(flagged(Slim, "= users_path\n= custom_path(1)\n= main_app.usrs_path\n").is_empty());
        // `delegate :usre_path` in a helper.
        assert!(flagged(Erb, "<%= usre_path(@user) %>\n").is_empty());
    }

    #[test]
    fn devise_helpers_are_not_routes_when_the_app_uses_devise() {
        let erb = "<%= session_path(resource_name) %><%= new_session_url(x) %><%= edit_password_url(x) %>\n";
        assert_eq!(found_in(Erb, erb, None, false).len(), 1, "sessions is a route; session_path is near it");
        assert!(found_in(Erb, erb, None, true).is_empty());
    }

    #[test]
    fn locals_and_defined_checks_are_not_calls() {
        assert!(flagged(Erb, "<% usrs_path = 1 %><%= usrs_path %>\n").is_empty());
        assert!(flagged(Erb, "<% [1].each do |usrs_path| %><%= usrs_path %><% end %>\n").is_empty());
        assert!(flagged(Haml, "- usrs_path = 1\n= usrs_path\n").is_empty());
        assert!(flagged(Slim, "- usrs_path = 1\n= usrs_path\n").is_empty());
        assert!(
            flagged(Erb, "<%= usrs_path(1) if defined?(usrs_path) %>\n<%= link_to \"x\", back_path if defined?(back_path) %>\n").len() == 1
        );
        assert!(flagged(Erb, "<% if defined?(usr_path) %><% end %>\n").is_empty());
    }

    #[test]
    fn a_bare_name_is_a_call_only_when_strict_locals_rule_out_a_local() {
        // `item_path` is the local a caller may pass, with `resources :items`.
        let bare = "<%= link_to \"x\", item_path %>\n";
        assert!(flagged(Erb, bare).is_empty(), "no strict locals");
        assert!(found_in(Erb, bare, Some(&strict(&["item_path"], false)), false).is_empty());
        assert!(found_in(Erb, bare, Some(&strict(&[], true)), false).is_empty());
        assert_eq!(found_in(Erb, bare, Some(&strict(&["user"], false)), false).len(), 1);
        assert_eq!(flagged(Erb, "<%= item_path(1) %>\n"), ["item_path"], "a local can't be called");
    }
}
