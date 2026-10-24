//! Hover text for Slim line indicators (`-`, `=`, `==`, `/`, `/!`, `|`, engines, …).

use crate::slim_parser::{Marker, Parsed};

/// Markdown explaining the marker under byte `offset`, if any.
pub fn marker_doc(text: &str, offset: usize, parsed: &Parsed) -> Option<String> {
    let (range, kind) = parsed
        .markers
        .iter()
        .find(|(r, _)| r.start <= offset && offset < r.end)?;
    let marker = &text[range.clone()];
    let doc = match kind {
        Marker::Control => "**`-` control code**: runs the Ruby without output. Lines nested under it are its block; Slim adds the `end`.".into(),
        Marker::Output => {
            let base = if marker.starts_with("==") {
                "**`==` unescaped output**: inserts the value of the Ruby expression without HTML escaping."
            } else {
                "**`=` output**: inserts the HTML-escaped value of the Ruby expression. Lines nested under it are its block."
            };
            format!("{base}{}", whitespace_modifiers(marker))
        }
        Marker::Comment => "**`/` code comment**: this line and the lines nested under it are not rendered.".into(),
        Marker::HtmlComment => "**`/!` HTML comment**: renders the text and the lines nested under it as `<!-- … -->`.".into(),
        Marker::ConditionalComment => {
            "**`/[…]` conditional comment**: wraps the nested lines in `<!--[if …]> … <![endif]-->`.".into()
        }
        Marker::Text => {
            let base = if marker.starts_with('\'') {
                "**`'` verbatim text with a trailing space**: like `|`, then adds a space after the text."
            } else {
                "**`|` verbatim text**: the rest of the line and the lines nested under it are text; `#{}` interpolates Ruby."
            };
            format!("{base}{}", whitespace_modifiers(marker))
        }
        Marker::Engine => format!("**`{marker}:` embedded engine**: {}", engine_doc(marker)),
    };
    Some(doc)
}

fn whitespace_modifiers(marker: &str) -> String {
    let mut notes = String::new();
    if marker.contains('<') {
        notes.push_str("\n\n`<` adds a space before it.");
    }
    if marker.contains('>') {
        notes.push_str("\n\n`>` adds a space after it.");
    }
    if marker.contains('\'') && marker.starts_with('=') {
        notes.push_str("\n\n`'` is accepted but ignored by Slim 5; use `>` for a trailing space.");
    }
    notes
}

fn engine_doc(name: &str) -> &'static str {
    match name {
        "javascript" => "the nested lines are JavaScript in a `<script>` tag; `#{}` interpolates Ruby.",
        "css" => "the nested lines are CSS in a `<style>` tag; `#{}` interpolates Ruby.",
        "ruby" => "the nested lines are Ruby code, run without output.",
        "sass" | "scss" => "the nested lines are compiled to CSS when the template compiles, in a `<style>` tag; `#{}` is Sass interpolation, not Ruby.",
        "coffee" => "the nested lines are compiled to JavaScript when the template compiles, in a `<script>` tag; no Ruby interpolation.",
        "markdown" | "textile" | "rdoc" | "asciidoc" => "the nested lines are rendered to HTML when the template compiles; `#{}` interpolates Ruby.",
        _ => "registered by the application (`rubyTemplates.slim.extraEngines`); the nested lines go to that engine.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slim_parser::parse;

    #[test]
    fn docs_for_markers() {
        let text = "==> raw\np= x\n- y\n/ c\n'< t\ncss:\n  a\nb a = c";
        let parsed = parse(text, &[]);
        let doc = |needle: &str| marker_doc(text, text.find(needle).unwrap(), &parsed);
        assert!(
            doc("==>").unwrap().starts_with("**`==` unescaped")
                && doc("==>").unwrap().contains("space after")
        );
        assert!(doc("= x").unwrap().starts_with("**`=` output"));
        assert!(doc("- y").unwrap().contains("control code"));
        assert!(doc("/ c").unwrap().contains("code comment"));
        assert!(
            doc("'<").unwrap().contains("trailing space")
                && doc("'<").unwrap().contains("space before")
        );
        assert!(doc("css").unwrap().contains("<style>"));
        assert_eq!(doc(" raw"), None);
        assert_eq!(
            doc("= c"),
            None,
            "`b a = c` is the attribute a=c, not output"
        );
    }
}
