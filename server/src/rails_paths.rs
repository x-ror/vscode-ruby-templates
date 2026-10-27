//! Rails app layout: the `app` and `app/views` folders, and what a template file name says (locale, format).

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// `Mime::SET` of a fresh Rails app; the app's own formats come from `Project::is_format`.
const KNOWN_FORMATS: [&str; 35] = [
    "html",
    "text",
    "js",
    "css",
    "ics",
    "csv",
    "vcf",
    "png",
    "jpeg",
    "gif",
    "bmp",
    "tiff",
    "svg",
    "webp",
    "avif",
    "mpeg",
    "mp4",
    "webm",
    "ogg",
    "mp3",
    "wav",
    "pdf",
    "zip",
    "gzip",
    "json",
    "xml",
    "rss",
    "atom",
    "yaml",
    "m4a",
    "otf",
    "ttf",
    "woff",
    "woff2",
    "turbo_stream",
];

pub fn is_known_format(segment: &str) -> bool {
    KNOWN_FORMATS.contains(&segment)
}

/// Rails locale shape: `en`, `pt-BR`, `de_AT`.
pub fn is_locale(segment: &str) -> bool {
    let b = segment.as_bytes();
    let region = |r: &[u8]| matches!(r, [b'-' | b'_', x, y] if x.is_ascii_uppercase() && y.is_ascii_uppercase());
    b.len() >= 2 && b[..2].iter().all(u8::is_ascii_lowercase) && (b.len() == 2 || region(&b[2..]))
}

/// Does `fname` name template `basename` (`name[.locale][.format][+variant][.handler]`)?
/// `Some(format)` when it does (`None` inside: matches any format); `_user.v2.html.erb` is not `_user`.
pub fn template_suffix<'a>(is_format: impl Fn(&str) -> bool + Copy, fname: &'a str, basename: &str) -> Option<Option<&'a str>> {
    if fname == basename {
        return Some(None);
    }
    let rest = fname.strip_prefix(basename)?.strip_prefix('.')?;
    let (middle, _handler) = rest.rsplit_once('.').unwrap_or(("", rest));
    let as_format = |segment: &'a str| match segment.split('+').next().unwrap_or("") {
        "" => Some(None),
        format if is_format(format) => Some(Some(format)),
        _ => None,
    };
    match middle.split_once('.') {
        Some((locale, format)) if is_locale(locale) => as_format(format),
        Some(_) => None,
        None if is_locale(middle) && !is_format(middle) => Some(None),
        None => as_format(middle),
    }
}

/// `user` of `user.en.html.erb`, `user.v2` of `user.v2.html.erb`.
pub fn template_stem(is_format: impl Fn(&str) -> bool + Copy, fname: &str) -> &str {
    fname.match_indices('.').map(|(i, _)| &fname[..i]).find(|stem| template_suffix(is_format, fname, stem).is_some()).unwrap_or(fname)
}

/// `html` of `show.html+phone.erb` (variant dropped); `None` for `show.erb` and locale-only `show.de.erb`.
pub fn template_format(is_format: impl Fn(&str) -> bool + Copy, path: &Path) -> Option<&str> {
    let fname = path.file_name()?.to_str()?;
    template_suffix(is_format, fname, template_stem(is_format, fname))?
}

/// Walk up from `file` to find the Rails `app/views` directory.
pub fn views_root(file: &Path) -> Option<PathBuf> {
    let mut dir = file.parent()?;
    loop {
        if dir.file_name().map(|n| n == "views").unwrap_or(false)
            && dir.parent().and_then(|p| p.file_name()).map(|n| n == "app").unwrap_or(false)
        {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// The Rails `app` folder of a file in app/views, app/components or any other app/ folder. Views come first:
/// `app/views/app/x.html.erb` is in a views subfolder named `app`.
pub fn app_root(file: &Path) -> Option<PathBuf> {
    match views_root(file) {
        Some(views) => views.parent().map(Path::to_path_buf),
        None => file.ancestors().skip(1).find(|dir| dir.file_name().is_some_and(|n| n == "app")).map(Path::to_path_buf),
    }
}

/// Every file under `root`; the listing `render_candidates` names come from.
pub fn files_under(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    WalkDir::new(root)
        .follow_links(true)
        .min_depth(1)
        .max_depth(max_depth)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_formats() {
        assert_eq!(template_format(is_known_format, Path::new("index.rss.builder")), Some("rss"));
        assert_eq!(template_format(is_known_format, Path::new("feed.atom.builder")), Some("atom"));
        assert_eq!(template_stem(is_known_format, "_entry.atom.builder"), "_entry");
    }

    #[cfg(unix)]
    #[test]
    fn files_under_follows_symlinked_dirs() {
        let root = std::env::temp_dir().join(format!("rt-symlink-{}", std::process::id()));
        let shared = root.join("shared/cards");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("_card.html.erb"), "").unwrap();
        let views = root.join("app/views");
        std::fs::create_dir_all(&views).unwrap();
        std::os::unix::fs::symlink(root.join("shared"), views.join("shared")).unwrap();
        std::os::unix::fs::symlink(&views, views.join("loop")).unwrap();
        assert_eq!(files_under(&views, usize::MAX), [views.join("shared/cards/_card.html.erb")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn template_file_names() {
        let cases = [
            ("user.html.erb", "user", Some(Some("html"))),
            ("user.html+phone.erb", "user", Some(Some("html"))),
            ("user.+phone.erb", "user", Some(None)),
            ("user.de.erb", "user", Some(None)),
            ("user.pt-BR.html.erb", "user", Some(Some("html"))),
            ("user.de_AT.erb", "user", Some(None)),
            ("user.js.erb", "user", Some(Some("js"))),
            ("user.erb", "user", Some(None)),
            ("user", "user", Some(None)),
            ("user.v2.html.erb", "user", None),
            ("user.en.v2.erb", "user", None),
            ("users.html.erb", "user", None),
            ("user.v2.html.erb", "user.v2", Some(Some("html"))),
        ];
        for (fname, basename, suffix) in cases {
            assert_eq!(template_suffix(is_known_format, fname, basename), suffix, "{fname} as {basename}");
        }
        let stems = [("user.en.html.erb", "user"), ("user.v2.html.erb", "user.v2"), ("user", "user"), ("a.b.c.d", "a.b.c"), (".erb", "")];
        for (fname, stem) in stems {
            assert_eq!(template_stem(is_known_format, fname), stem, "{fname}");
        }
        assert_eq!(template_format(is_known_format, Path::new("show.de.erb")), None);
        assert_eq!(template_format(is_known_format, Path::new("show.en.html.erb")), Some("html"));
    }

    #[test]
    fn suffix_edge_cases() {
        for (fname, suffix) in [
            ("user.ics.erb", Some(Some("ics"))),
            ("user.en.+phone.erb", Some(None)),
            ("user.html.ERB", Some(Some("html"))),
            ("user.EN.erb", None),
            ("user.en.xx.erb", None),
            ("user.erb.erb", None),
        ] {
            assert_eq!(template_suffix(is_known_format, fname, "user"), suffix, "{fname}");
        }
    }
}
