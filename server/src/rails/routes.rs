//! Route helpers (`users_path`, `edit_user_url`) from the app's routes, which the client reads with
//! `bin/rails runner` and sends in a `rubyTemplates/routes` notification.

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
pub struct Route {
    /// Empty for routes without a helper, like the `POST` sharing `users`' path.
    pub name: String,
    pub verb: String,
    /// Without the optional `(.:format)`.
    pub path: String,
    /// `users#index`; empty for redirects and mounted apps.
    pub action: String,
    /// File and 0-based line of the route's definition (Rails 7.1+).
    pub source: Option<(PathBuf, usize)>,
}

const SUFFIXES: [&str; 2] = ["_path", "_url"];

/// Routes of a `rubyTemplates/routes` payload; `root` resolves relative source locations.
pub fn parse(root: &Path, routes: &Value) -> Vec<Route> {
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let source = |v: &Value| {
        let (file, line) = v.get("source")?.as_str()?.rsplit_once(':')?;
        Some((root.join(file), line.parse::<usize>().ok()?.checked_sub(1)?))
    };
    routes
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| Route {
            name: text(v, "name"),
            verb: text(v, "verb"),
            path: text(v, "path").trim_end_matches("(.:format)").to_string(),
            action: text(v, "action"),
            source: source(v),
        })
        .collect()
}

/// The named route a helper such as `users_path` calls, and every route sharing its path (`GET`, `POST`, …).
pub fn for_helper<'a>(routes: &'a [Route], word: &str) -> Option<(&'a Route, Vec<&'a Route>)> {
    let name = SUFFIXES.iter().find_map(|s| word.strip_suffix(s)).filter(|n| !n.is_empty())?;
    let named = routes.iter().find(|r| r.name == name)?;
    Some((named, routes.iter().filter(|r| r.path == named.path).collect()))
}

pub fn hover(routes: &[Route], word: &str) -> Option<String> {
    let (named, same_path) = for_helper(routes, word)?;
    let mut md = format!("`{word}` → `{}`\n\n", named.path);
    for r in same_path {
        let action = if r.action.is_empty() { String::new() } else { format!(" → `{}`", r.action) };
        md.push_str(&format!("- **{}**{action}\n", if r.verb.is_empty() { "ANY" } else { &r.verb }));
    }
    Some(md)
}

/// `(helper, detail)` for each `_path` / `_url` helper starting with `typed`.
pub fn completions(routes: &[Route], typed: &str) -> Vec<(String, String)> {
    // `name + suffix` starts with `typed`, tested before formatting either: this runs on every word typed in code.
    let typed = typed.as_bytes();
    let helper_starts = |name: &str, suffix: &str| {
        let split = typed.len().min(name.len());
        name.as_bytes().starts_with(&typed[..split]) && suffix.as_bytes().starts_with(&typed[split..])
    };
    routes
        .iter()
        .filter(|r| !r.name.is_empty())
        .flat_map(|r| {
            SUFFIXES
                .into_iter()
                .filter(|s| helper_starts(&r.name, s))
                .map(move |s| (format!("{}{s}", r.name), format!("{} {}", r.verb, r.path)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Route> {
        let json = serde_json::json!([
            { "name": "users", "verb": "GET", "path": "/users(.:format)", "action": "users#index", "source": "config/routes.rb:3" },
            { "name": "", "verb": "POST", "path": "/users(.:format)", "action": "users#create", "source": "config/routes.rb:3" },
            { "name": "user", "verb": "GET", "path": "/users/:id(.:format)", "action": "users#show", "source": "/abs/config/routes/admin.rb:9" },
            { "name": "old", "verb": "", "path": "/old", "action": "" }
        ]);
        parse(Path::new("/app"), &json)
    }

    #[test]
    fn parses_and_resolves_helpers() {
        let routes = sample();
        assert_eq!(routes[0].path, "/users");
        assert_eq!(routes[0].source, Some((PathBuf::from("/app/config/routes.rb"), 2)));
        assert_eq!(routes[2].source, Some((PathBuf::from("/abs/config/routes/admin.rb"), 8)));
        assert_eq!(routes[3].source, None);
        let (named, same) = for_helper(&routes, "users_url").unwrap();
        assert_eq!((named.action.as_str(), same.len()), ("users#index", 2));
        assert!(for_helper(&routes, "users").is_none());
        assert!(for_helper(&routes, "_path").is_none());
        assert_eq!(
            hover(&routes, "users_path").unwrap(),
            "`users_path` → `/users`\n\n- **GET** → `users#index`\n- **POST** → `users#create`\n"
        );
        assert_eq!(hover(&routes, "old_path").unwrap(), "`old_path` → `/old`\n\n- **ANY**\n");
    }

    #[test]
    fn completes_named_routes() {
        let labels: Vec<String> = completions(&sample(), "user").into_iter().map(|(l, _)| l).collect();
        assert_eq!(labels, ["users_path", "users_url", "user_path", "user_url"]);
        assert_eq!(completions(&sample(), "user_p")[0].1, "GET /users/:id");
        let labels = |typed| completions(&sample(), typed).into_iter().map(|(l, _)| l).collect::<Vec<_>>();
        assert_eq!(labels("users_u"), ["users_url"]);
        assert_eq!(labels("").len(), 6);
        assert_eq!(labels("usé"), Vec::<String>::new());
        assert_eq!(labels("old_path_x"), Vec::<String>::new());
    }
}
