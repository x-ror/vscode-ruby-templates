//! Which template language a document is, and whether its output is HTML.

use crate::html;
use crate::rails_paths::{is_known_format, is_locale, template_format, views_root};
use crate::util;
use std::path::Path;
use tower_lsp_server::ls_types::Uri;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TemplateLang {
    Erb,
    Haml,
    Slim,
    Skim,
    /// Jbuilder and Builder views: plain Ruby, attached through the `ruby` language id.
    Ruby,
}

impl TemplateLang {
    /// The `source` of this language's diagnostics; Skim is Slim syntax.
    pub fn diagnostic_source(self) -> &'static str {
        match self {
            Self::Erb => "erb",
            Self::Haml => "haml",
            Self::Slim | Self::Skim => "slim",
            Self::Ruby => "ruby",
        }
    }

    pub fn from_uri_or_id(uri: &Uri, language_id: Option<&str>) -> Option<Self> {
        match language_id {
            Some(id) if id == "erb" || id.starts_with("erb-") => return Some(Self::Erb),
            Some("haml") => return Some(Self::Haml),
            Some("slim") => return Some(Self::Slim),
            Some("skim") => return Some(Self::Skim),
            _ => {}
        }
        Self::from_file_name(uri.path().as_str())
    }

    /// The language of a file named `name` (a bare name or a whole path), by its extension.
    pub fn from_file_name(name: &str) -> Option<Self> {
        let path = name.to_ascii_lowercase();
        if path.ends_with(".jbuilder") || path.ends_with(".builder") {
            return Some(Self::Ruby);
        }
        if ERB_EXTENSIONS.iter().any(|ext| path.ends_with(ext)) || path.ends_with(".rhtml") {
            Some(Self::Erb)
        } else if path.ends_with(".haml") {
            Some(Self::Haml)
        } else if path.ends_with(".slim") {
            Some(Self::Slim)
        } else if path.ends_with(".skim") {
            Some(Self::Skim)
        } else {
            None
        }
    }
}

pub const ERB_EXTENSIONS: [&str; 3] = [".erb", ".erubi", ".erubis"];

/// `x.html.erb`, `x.html+phone.erb`, `x.turbo_stream.erb`, `x.html.erubi`, `x.rhtml`, and a localized view with no
/// format (`app/views/pages/about.de.erb`), which is HTML in practice.
/// A bare `app/views/**/x.erb` counts when its first text outside ERB tags is markup; elsewhere a bare `.erb`
/// (`nginx.conf.erb`, `Dockerfile.erb`) has an unknown host.
pub fn html_host(uri: &Uri, plain_erb: bool, text: &str) -> bool {
    let file = file_name(uri).to_ascii_lowercase();
    // An untitled document names no host; the plain `erb` language id (not `erb-js`, ...) is HTML ERB, as for Slim.
    if file.ends_with(".rhtml") || (plain_erb && !file.contains('.')) {
        return true;
    }
    if ERB_EXTENSIONS.iter().any(|ext| host_format(&file, ext).is_some_and(is_html_format)) {
        return true;
    }
    // The URI's own path, not `file_path`, which a drive-less test URI fails on Windows.
    let in_views = views_root(Path::new(&util::percent_decode(uri.path().as_str()))).is_some();
    let original = file_name(uri);
    let parts: Vec<&str> = original.split('.').collect();
    let erb_extension = |part: &str| ERB_EXTENSIONS.contains(&format!(".{}", part.to_ascii_lowercase()).as_str());
    if !in_views {
        return false;
    }
    match parts[..] {
        [_, locale, ext] => is_locale(locale) && template_format(is_known_format, Path::new(&file)).is_none() && erb_extension(ext),
        [_, ext] => erb_extension(ext) && starts_with_markup(text),
        _ => false,
    }
}

/// Whether the first text outside ERB tags opens an HTML element (or custom element), a comment or a doctype; other
/// XML (`<rss>`) isn't HTML.
fn starts_with_markup(text: &str) -> bool {
    let mut rest = text.trim_start_matches('\u{feff}').trim_start();
    while let Some(after_open) = rest.strip_prefix("<%") {
        let Some((_, after_close)) = after_open.split_once("%>") else { return false };
        rest = after_close.trim_start();
    }
    let Some(after_lt) = rest.strip_prefix('<') else { return false };
    if after_lt.starts_with('!') {
        return true;
    }
    let name_len = after_lt.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(after_lt.len());
    let name = after_lt[..name_len].to_ascii_lowercase();
    html::TAGS.contains(&name.as_str()) || (name.contains('-') && name.starts_with(|c: char| c.is_ascii_alphabetic()))
}

/// Like `html_host`, but a bare `x.slim` (and an untitled document) counts as HTML, Slim's default.
pub fn slim_html_host(uri: &Uri) -> bool {
    host_format(&file_name(uri).to_ascii_lowercase(), ".slim").is_none_or(is_html_format)
}

pub fn is_partial(uri: &Uri) -> bool {
    uri.path().as_str().rsplit('/').next().is_some_and(|f| f.starts_with('_'))
}

/// Decoded last path segment: VS Code sends `show.html%2Bphone.erb` for `show.html+phone.erb`. URIs that aren't
/// file paths here (`untitled:`, a drive-less `file:///v/x` on Windows) are decoded by hand.
pub fn file_name(uri: &Uri) -> String {
    util::file_path(uri)
        .and_then(|p| Some(p.file_name()?.to_str()?.to_owned()))
        .unwrap_or_else(|| util::percent_decode(uri.path().as_str().rsplit('/').next().unwrap_or_default()))
}

/// The format of `name[.locale][.format].ext`; `None` when `file` doesn't end in `ext` or names no format.
fn host_format<'a>(file: &'a str, ext: &str) -> Option<&'a str> {
    // A file name alone doesn't tell the app's registered formats; only Rails' own count here.
    file.ends_with(ext).then(|| template_format(is_known_format, Path::new(file)))?
}

fn is_html_format(format: &str) -> bool {
    format == "html" || format.starts_with("html+") || format == "turbo_stream"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_host_by_file_name() {
        for (path, html) in [
            ("/v/users/show.html.erb", true),
            ("/v/users/show.html+phone.erb", true),
            ("/v/users/show.html%2Bphone.erb", true),
            ("/v/users/create.turbo_stream.erb", true),
            ("/v/old.rhtml", true),
            ("/v/users/SHOW.HTML.ERB", true),
            ("/v/users/index.js.erb", false),
            ("/v/mailer/welcome.text.erb", false),
            ("/v.html/config.yml.erb", false),
            ("/v.html/nginx.erb", false),
            ("/v/users/show.de.erb", false),
            ("/r/app/views/pages/about.de.erb", true),
            ("/r/app/views/pages/about.pt-BR.erb", true),
            ("/r/app/views/pages/about.yml.erb", false),
            ("/r/app/views/pages/index.js.erb", false),
            ("/r/app/views/pages/feed.rss.erb", false),
            ("/v/users/show.de.html.erb", true),
            ("/v/show.html.erubi", true),
            ("/v/show.html.erubis", true),
            ("/v/feed.xml.erubi", false),
        ] {
            assert_eq!(html_host(&format!("file://{path}").parse::<Uri>().unwrap(), false, ""), html, "{path}");
        }
    }

    #[test]
    fn bare_erb_view_is_html_when_it_starts_with_markup() {
        let host = |path: &str, text: &str| html_host(&format!("file://{path}").parse::<Uri>().unwrap(), false, text);
        let view = "/r/app/views/pages/about.erb";
        for text in
            ["<div>", "  \n<p>x", "<%# c %>\n<!DOCTYPE html>", "<% x = 1 %><%= y %><!-- c -->", "<h1>", "<turbo-frame id=\"x\">", "<DIV>"]
        {
            assert!(host(view, text), "{text}");
        }
        for text in [
            "",
            "<rss version=\"2.0\"><channel><link><%= root_url %></link>",
            "<feed>",
            "Hello <b>",
            "<%= x %>",
            "<% if a %>",
            "<?xml version=\"1.0\"?>",
            "< div>",
            "<",
        ] {
            assert!(!host(view, text), "{text}");
        }
        assert!(host(view, "\u{feff}<div>"));
        assert!(!host(view, "\u{feff}<rss>"));
        assert!(!host("/r/config/about.erb", "<div>"));
        assert!(!host("/r/app/views/pages/nginx.conf.erb", "<div>"));
    }

    #[test]
    fn file_name_decodes_what_vscode_encodes() {
        for (sent, name) in [
            ("file:///v/show.html%2Bphone.erb", "show.html+phone.erb"),
            ("file:///v/my%20app/_caf%C3%A9%40x.erb", "_caf\u{e9}@x.erb"),
            ("untitled:Untitled-1", "Untitled-1"),
        ] {
            assert_eq!(file_name(&sent.parse::<Uri>().unwrap()), name, "{sent}");
        }
        assert!(is_partial(&"file:///v/my%20app/_caf%C3%A9.erb".parse::<Uri>().unwrap()));
    }

    #[test]
    fn untitled_erb_is_html() {
        let untitled = "untitled:Untitled-1".parse::<Uri>().unwrap();
        assert!(html_host(&untitled, true, ""));
        assert!(!html_host(&untitled, false, ""));
        assert!(!html_host(&"file:///v/nginx.erb".parse::<Uri>().unwrap(), true, ""));
    }

    #[test]
    fn host_specific_erb_ids() {
        let untitled = "untitled:Untitled-1".parse::<Uri>().unwrap();
        assert_eq!(TemplateLang::from_uri_or_id(&untitled, Some("erb-js")), Some(TemplateLang::Erb));
        assert_eq!(TemplateLang::from_uri_or_id(&untitled, Some("erbx")), None);
        let js_erb = "file:///app/views/users/index.js.erb".parse::<Uri>().unwrap();
        assert_eq!(TemplateLang::from_uri_or_id(&js_erb, None), Some(TemplateLang::Erb));
        let upper = "file:///app/views/users/INDEX.HTML.ERB".parse::<Uri>().unwrap();
        assert_eq!(TemplateLang::from_uri_or_id(&upper, None), Some(TemplateLang::Erb));
        for path in ["file:///views/x.erubi", "file:///views/x.html.ERUBIS"] {
            assert_eq!(TemplateLang::from_uri_or_id(&path.parse::<Uri>().unwrap(), None), Some(TemplateLang::Erb), "{path}");
        }
    }
}
