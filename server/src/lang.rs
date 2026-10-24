//! Which template language a document is, and whether its output is HTML.

use crate::{partials, util};
use std::path::Path;
use tower_lsp::lsp_types::Url;

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
    pub fn from_uri_or_id(uri: &Url, language_id: Option<&str>) -> Option<Self> {
        match language_id {
            Some(id) if id == "erb" || id.starts_with("erb-") => return Some(Self::Erb),
            Some("haml") => return Some(Self::Haml),
            Some("slim") => return Some(Self::Slim),
            Some("skim") => return Some(Self::Skim),
            _ => {}
        }
        let path = uri.path().to_ascii_lowercase();
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
// ponytail: bare `.erb` and untitled documents are skipped; their host (`nginx.conf.erb`…) is unknown.
pub fn html_host(uri: &Url) -> bool {
    let file = file_name(uri).to_ascii_lowercase();
    if file.ends_with(".rhtml") {
        return true;
    }
    if ERB_EXTENSIONS.iter().any(|ext| host_format(&file, ext).is_some_and(is_html_format)) {
        return true;
    }
    // The URI's own path, not `to_file_path`, which a drive-less test URI fails on Windows.
    let in_views = partials::views_root(Path::new(&util::percent_decode(uri.path()))).is_some();
    let original = file_name(uri);
    let parts: Vec<&str> = original.split('.').collect();
    in_views
        && parts.len() == 3
        && partials::is_locale(parts[1])
        && partials::template_format(Path::new(&file)).is_none()
        && ERB_EXTENSIONS.contains(&format!(".{}", parts[2].to_ascii_lowercase()).as_str())
}

/// Like `html_host`, but a bare `x.slim` (and an untitled document) counts as HTML, Slim's default.
pub fn slim_html_host(uri: &Url) -> bool {
    host_format(&file_name(uri).to_ascii_lowercase(), ".slim").is_none_or(is_html_format)
}

/// Decoded last path segment: VS Code sends `show.html%2Bphone.erb` for `show.html+phone.erb`. URIs that aren't
/// file paths here (`untitled:`, a drive-less `file:///v/x` on Windows) are decoded by hand.
pub fn file_name(uri: &Url) -> String {
    uri.to_file_path()
        .ok()
        .and_then(|p| Some(p.file_name()?.to_str()?.to_owned()))
        .unwrap_or_else(|| util::percent_decode(uri.path().rsplit('/').next().unwrap_or_default()))
}

/// The format of `name[.locale][.format].ext`; `None` when `file` doesn't end in `ext` or names no format.
fn host_format<'a>(file: &'a str, ext: &str) -> Option<&'a str> {
    file.ends_with(ext).then(|| partials::template_format(Path::new(file)))?
}

fn is_html_format(format: &str) -> bool {
    format == "html" || format.starts_with("html+") || format == "turbo_stream"
}
