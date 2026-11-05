//! Literal asset helper paths (`image_tag "logo.png"`, `stylesheet_link_tag "application"`) → files.

use crate::rails::paths::rails_root;
use crate::rails::render_call::MAX_CALL_LINES;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static ASSET_CALL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(image_tag|image_path|asset_path|stylesheet_link_tag|javascript_include_tag|favicon_link_tag)\s*\(?").unwrap()
});
// A string argument with matching quotes; `#` keeps interpolated strings out.
static STRING_ARG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*(?:"([^"'#]+)"|'([^"'#]+)')"#).unwrap());
// What may follow a whole string argument; `"b" + x`, `"b" "c"` and the key `"b": 1` are not just that string.
static ARG_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:$|[,)}\];]|#|-?%>|(?:if|unless|while|until|rescue|and|or|do)\b)").unwrap());
const TAKES_MANY: [&str; 2] = ["stylesheet_link_tag", "javascript_include_tag"];
const ASSET_DIRS: [&str; 6] =
    // Sources before app/assets/builds, so `javascript_include_tag "application"` opens what was written, not the bundle.
    ["app/assets/images", "app/assets/stylesheets", "app/assets/javascripts", "app/javascript", "app/assets/builds", "public"];

pub struct AssetRef<'a> {
    pub helper: &'a str,
    pub path: &'a str,
    /// Byte range of the string literal (without quotes) inside the line.
    pub start: usize,
    pub end: usize,
}

/// The byte range of the string argument that starts `line[from..]`, when it is the whole argument.
fn string_arg(line: &str, from: usize) -> Option<(usize, usize)> {
    let path = STRING_ARG.captures(&line[from..]).and_then(|cap| cap.get(1).or_else(|| cap.get(2)))?;
    let (start, end) = (from + path.start(), from + path.end());
    ARG_END.is_match(&line[end + 1..]).then_some((start, end))
}

fn after_comma(line: &str, from: usize) -> Option<usize> {
    let rest = line[from..].trim_start().strip_prefix(',')?;
    Some(line.len() - rest.len())
}

/// The string arguments back to back from `from`: `"a", 'b'`, stopping at the first thing that isn't one.
fn string_args(line: &str, from: usize) -> Vec<(usize, usize)> {
    let mut args = vec![];
    let mut at = Some(from);
    while let Some((start, end)) = at.and_then(|at| string_arg(line, at)) {
        args.push((start, end));
        at = after_comma(line, end + 1);
    }
    args
}

/// Whether `line[from..]` is string arguments followed by a trailing comma, so the argument list goes on below.
fn list_continues(line: &str, from: usize) -> Option<Vec<(usize, usize)>> {
    let args = string_args(line, from);
    let list_end = args.last().map_or(from, |&(_, end)| end + 1);
    (line[list_end..].trim() == ",").then_some(args)
}

/// The `stylesheet_link_tag`/`javascript_include_tag` whose argument list reaches `line_start` from lines above,
/// each ending in a comma and holding nothing but string arguments after the call.
fn open_call_above(text: &str, line_start: usize) -> Option<&str> {
    let mut start = line_start;
    for _ in 0..MAX_CALL_LINES - 1 {
        let before = text[..start].strip_suffix('\n')?;
        start = before.rfind('\n').map_or(0, |at| at + 1);
        let above = &before[start..];
        let head = ASSET_CALL.captures_iter(above).filter_map(|c| Some((c.get(1)?.as_str(), c.get(0)?.end()))).last();
        match head {
            Some((helper, end)) => {
                let opens_list = above[..end].ends_with('(') && above[end..].trim().is_empty();
                return (TAKES_MANY.contains(&helper) && (opens_list || list_continues(above, end).is_some())).then_some(helper);
            }
            None if list_continues(above, 0).is_none_or(|args| args.is_empty()) => return None,
            None => {}
        }
    }
    None
}

/// The asset paths on `line`, which starts at `line_start` of `text`; a string argument list may begin on lines above.
pub fn asset_refs<'a>(text: &'a str, line_start: usize, line: &'a str) -> Vec<AssetRef<'a>> {
    let mut refs = vec![];
    if let Some(helper) = open_call_above(text, line_start) {
        refs.extend(string_args(line, 0).into_iter().map(|(start, end)| AssetRef { helper, path: &line[start..end], start, end }));
    }
    for call in ASSET_CALL.captures_iter(line) {
        let (Some(helper), Some(whole)) = (call.get(1), call.get(0)) else { continue };
        let helper = helper.as_str();
        let args =
            if TAKES_MANY.contains(&helper) { string_args(line, whole.end()) } else { string_arg(line, whole.end()).into_iter().collect() };
        refs.extend(args.into_iter().map(|(start, end)| AssetRef { helper, path: &line[start..end], start, end }));
    }
    refs
}

/// Asset directories for relative paths, public/ for `/absolute` ones; nothing for URLs or paths leaving those dirs.
pub fn resolve(current: &Path, r: &AssetRef) -> Vec<PathBuf> {
    if r.path.starts_with("//") || r.path.contains(':') || r.path.split(['/', '\\']).any(|s| s == "..") {
        return vec![];
    }
    let Some(rails) = rails_root(current) else { return vec![] };
    let (dirs, rel) = match r.path.strip_prefix('/') {
        Some(rel) => (vec![rails.join("public")], rel),
        None => (ASSET_DIRS.iter().map(|d| rails.join(d)).collect(), r.path),
    };
    // Rails appends the helper's extension unless the path already has it; Sprockets/dartsass compile the sources.
    let names: Vec<String> = match r.helper {
        "stylesheet_link_tag" => {
            let base = rel.strip_suffix(".css").unwrap_or(rel);
            ["css", "scss", "sass"].iter().map(|ext| format!("{base}.{ext}")).collect()
        }
        "javascript_include_tag" => vec![format!("{}.js", rel.strip_suffix(".js").unwrap_or(rel))],
        _ => vec![rel.to_string()],
    };
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).filter(|p| p.is_file()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_refs(line: &str) -> Vec<AssetRef<'_>> {
        asset_refs(line, 0, line)
    }

    #[test]
    fn resolves_asset_helper_paths() {
        let root = std::env::temp_dir().join(format!("rt-assets-{}", std::process::id()));
        for f in [
            "app/assets/images/logo.png",
            "app/assets/stylesheets/application.scss",
            "app/assets/builds/application.css",
            "app/javascript/app.js",
            "app/javascript/application.js",
            "app/assets/builds/application.js",
            "public/favicon.ico",
            "config/master.key",
            "public/robots.txt",
            "app/views/home/index.html.erb",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let current = root.join("app/views/home/index.html.erb");
        let many = r##"stylesheet_link_tag("application", 'admin', "data-turbo-track": "reload"); javascript_include_tag "app", "#{x}", "b"; image_tag "logo.png", "alt""##;
        let paths = |line| line_refs(line).iter().map(|r| r.path.to_string()).collect::<Vec<_>>();
        assert_eq!(paths(many), ["application", "admin", "app", "logo.png"]);
        let line = concat!(
            r#"<%= image_tag "logo.png" %> <%= stylesheet_link_tag "application", "data-turbo-track": "reload" %> "#,
            r#"<%= javascript_include_tag("app") %> <%= favicon_link_tag "favicon.ico" %> <%= asset_path "/robots.txt" %> "#,
            r##"<%= image_tag "https://x.io/a.png" %> <%= image_tag "//cdn/logo.png" %> <%= image_tag "/logo.png" %> <%= image_tag "#{x}.png" %>"##,
        );
        let refs = line_refs(line);
        assert_eq!(
            refs.iter().map(|r| r.path).collect::<Vec<_>>(),
            ["logo.png", "application", "app", "favicon.ico", "/robots.txt", "https://x.io/a.png", "//cdn/logo.png", "/logo.png"]
        );
        assert_eq!(&line[refs[0].start..refs[0].end], "logo.png");
        let found: Vec<Vec<PathBuf>> = refs.iter().map(|r| resolve(&current, r)).collect();
        assert_eq!(found[0], [root.join("app/assets/images/logo.png")]);
        assert_eq!(found[1], [root.join("app/assets/stylesheets/application.scss"), root.join("app/assets/builds/application.css")]);
        assert_eq!(found[2], [root.join("app/javascript/app.js")]);
        assert_eq!(found[3], [root.join("public/favicon.ico")]);
        assert_eq!(found[4], [root.join("public/robots.txt")]);
        assert!(found[5..].iter().all(Vec::is_empty));
        let at = |helper, path| resolve(&current, &AssetRef { helper, path, start: 0, end: 0 });
        assert_eq!(
            at("javascript_include_tag", "application"),
            [root.join("app/javascript/application.js"), root.join("app/assets/builds/application.js")]
        );
        for escape in ["../../config/master.key", "/../config/master.key", "..\\..\\config\\master.key", "x/../../../config/master.key"] {
            assert!(at("asset_path", escape).is_empty(), "{escape}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn each_string_arg_of_stylesheet_and_javascript_tags_is_a_ref() {
        let line = r#"é<%= stylesheet_link_tag "a", 'b', media: "all" %><%= javascript_include_tag("c", "d") %><%= stylesheet_link_tag :application %><%= stylesheet_link_tag "e", nonce: true %>"#;
        let refs = line_refs(line);
        assert_eq!(refs.iter().map(|r| r.path).collect::<Vec<_>>(), ["a", "b", "c", "d", "e"]);
        assert!(refs.iter().all(|r| &line[r.start..r.end] == r.path));
        assert!(line_refs("<%= image_tag :logo %>").is_empty());
    }

    #[test]
    fn closing_brackets_semicolons_and_comments_end_a_string_arg() {
        let paths = |line| line_refs(line).iter().map(|r| r.path.to_string()).collect::<Vec<_>>();
        assert_eq!(paths(r#"%img{src: image_path "a.png"}"#), ["a.png"]);
        assert_eq!(paths(r#"<%= content_tag(:div) { image_tag "a.png" } %>"#), ["a.png"]);
        assert_eq!(paths(r#"<%= [image_path "a.png"] %>"#), ["a.png"]);
        assert_eq!(paths(r#"<%= image_tag "a.png" # logo %>"#), ["a.png"]);
        assert_eq!(paths(r#"<%= image_tag "a.png"; x %>"#), ["a.png"]);
        assert!(paths(r#"image_tag "b" + x"#).is_empty());
        assert!(paths(r#"image_tag "b" "c""#).is_empty());
        assert!(paths(r#"image_tag "b": 1"#).is_empty());
    }

    #[test]
    fn only_whole_string_args_with_matching_quotes_are_refs() {
        let paths = |line| line_refs(line).iter().map(|r| r.path.to_string()).collect::<Vec<_>>();
        assert_eq!(paths(r#"stylesheet_link_tag "a", "b" + x"#), ["a"]);
        assert_eq!(paths(r#"stylesheet_link_tag "a", "b" "c""#), ["a"]);
        assert_eq!(paths(r#"stylesheet_link_tag "a", "b' , 'c""#), ["a"]);
        assert!(paths(r#"stylesheet_link_tag "b' , 'c""#).is_empty());
        assert!(paths(r#"image_tag "a" + x"#).is_empty());
        assert_eq!(paths(r#"image_tag "a" if x"#), ["a"]);
        assert_eq!(paths(r#"stylesheet_link_tag("a", "b")"#), ["a", "b"]);
    }

    #[test]
    fn string_args_on_continuation_lines_are_refs() {
        let text = "<%= stylesheet_link_tag \"application\",\n  \"admin\",\n  'extra',\n  \"data-turbo-track\": \"reload\" %>\n<%= link_to \"x\",\n  \"y\" %>\n";
        let on_line = |n: usize| {
            let (line_start, line) = crate::util::lines_with_offsets(text).nth(n).unwrap();
            asset_refs(text, line_start, line)
                .iter()
                .map(|r| (r.helper.to_string(), r.path.to_string(), &line[r.start..r.end] == r.path))
                .collect::<Vec<_>>()
        };
        assert_eq!(on_line(1), [("stylesheet_link_tag".to_string(), "admin".to_string(), true)]);
        assert_eq!(on_line(2).len(), 1, "a second continuation line");
        assert!(on_line(3).is_empty(), "the options hash ends the list");
        assert!(on_line(5).is_empty(), "other calls' arguments");
    }

    #[test]
    fn continuation_lines_work_in_haml_with_crlf_and_erb_dash_close() {
        let paths = |text: &str, n: usize| {
            let (line_start, line) = crate::util::lines_with_offsets(text).nth(n).unwrap();
            asset_refs(text, line_start, line).iter().map(|r| r.path.to_string()).collect::<Vec<_>>()
        };
        assert_eq!(paths("= stylesheet_link_tag \"a\",\n  \"b\"\n", 1), ["b"]);
        assert_eq!(paths("<%= stylesheet_link_tag \"a\",\r\n  \"b\",\r\n  \"c\" -%>\r\n", 2), ["c"]);
        assert_eq!(paths("<%= javascript_include_tag \"a\", \"b\",\n\"c\", \"d\" %>", 1), ["c", "d"]);
        assert!(paths("<%= image_tag \"a\",\n  \"b\" %>", 1).is_empty(), "image_tag takes one path");
        let opened = "<%= stylesheet_link_tag(\n  \"application\",\n  \"data-turbo-track\": \"reload\"\n) %>";
        assert_eq!(paths(opened, 1), ["application"]);
        assert!(paths(opened, 2).is_empty());
        assert!(paths("<%= stylesheet_link_tag\n  \"a\" %>", 1).is_empty(), "no paren, the call ended");
        assert!(paths("<%= stylesheet_link_tag \"a\",\n\n  \"b\" %>", 2).is_empty(), "a blank line ends the call");
    }

    #[test]
    fn long_runs_of_comma_lines_are_bounded() {
        let text = format!("<%= stylesheet_link_tag \"a\",\n{}  \"z\" %>", "  \"b\",\n".repeat(MAX_CALL_LINES + 5));
        let last = text.lines().count() - 1;
        let (line_start, line) = crate::util::lines_with_offsets(&text).nth(last).unwrap();
        assert!(asset_refs(&text, line_start, line).is_empty());
    }
}
