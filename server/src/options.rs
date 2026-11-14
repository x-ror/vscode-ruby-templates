//! `initializationOptions`, from the `rubyTemplates.{slim,erb,haml,jbuilder}.*` settings.

use crate::lang::TemplateLang;

/// The settings sections with a `disabledDiagnostics` list, and the languages each covers.
const SECTIONS: [(&str, &[TemplateLang]); 4] = [
    ("slim", &[TemplateLang::Slim, TemplateLang::Skim]),
    ("erb", &[TemplateLang::Erb]),
    ("haml", &[TemplateLang::Haml]),
    ("jbuilder", &[TemplateLang::Ruby]),
];
/// The sections whose `diagnostics.enabled` and `format.enabled` switch their languages off.
const SWITCHED_SECTIONS: [&str; 3] = ["erb", "haml", "slim"];

#[derive(Default, Debug)]
pub(crate) struct Options {
    pub extra_engines: Vec<String>,
    /// Diagnostic codes turned off, by language.
    disabled: Vec<(TemplateLang, Vec<String>)>,
    diagnostics_off: Vec<TemplateLang>,
    format_off: Vec<TemplateLang>,
    /// `erb.format.alignComments`.
    pub align_erb_comments: bool,
}

impl Options {
    /// Reads each field on its own so one malformed field keeps the others; returns what was ignored.
    pub(crate) fn from_init(init: Option<&serde_json::Value>) -> (Self, Vec<String>) {
        let mut ignored = Vec::new();
        let mut strings = |section: &str, name: &str| -> Vec<String> {
            let Some(value) = init.and_then(|v| v.get(section)?.get(name)) else {
                return vec![];
            };
            let Some(items) = value.as_array() else {
                ignored.push(format!("{section}.{name}: expected an array, got {value}"));
                return vec![];
            };
            items
                .iter()
                .filter_map(|item| {
                    let s = item.as_str().map(String::from);
                    if s.is_none() {
                        ignored.push(format!("{section}.{name}: expected a string, got {item}"));
                    }
                    s
                })
                .collect()
        };
        let mut options = Self { extra_engines: strings("slim", "extraEngines"), ..Self::default() };
        for (section, langs) in SECTIONS {
            let codes = strings(section, "disabledDiagnostics");
            options.disabled.extend(langs.iter().map(|&lang| (lang, codes.clone())));
        }
        for section in SWITCHED_SECTIONS {
            let langs = SECTIONS.iter().find(|(name, _)| *name == section).map_or(&[][..], |(_, langs)| langs);
            for (group, off) in [("diagnostics", &mut options.diagnostics_off), ("format", &mut options.format_off)] {
                let Some(value) = init.and_then(|v| v.get(section)?.get(group)?.get("enabled")) else { continue };
                match value.as_bool() {
                    Some(false) => off.extend(langs),
                    Some(true) => {}
                    None => ignored.push(format!("{section}.{group}.enabled: expected a boolean, got {value}")),
                }
            }
        }
        if let Some(value) = init.and_then(|v| v.get("erb")?.get("format")?.get("alignComments")) {
            match value.as_bool() {
                Some(align) => options.align_erb_comments = align,
                None => ignored.push(format!("erb.format.alignComments: expected a boolean, got {value}")),
            }
        }
        (options, ignored)
    }

    pub(crate) fn diagnostics_enabled(&self, lang: TemplateLang) -> bool {
        !self.diagnostics_off.contains(&lang)
    }

    pub(crate) fn format_enabled(&self, lang: TemplateLang) -> bool {
        !self.format_off.contains(&lang)
    }

    /// Diagnostic codes turned off for `lang`; Skim shares Slim's setting.
    pub(crate) fn disabled(&self, lang: TemplateLang) -> &[String] {
        self.disabled.iter().find(|(of, _)| *of == lang).map_or(&[], |(_, codes)| codes.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_keep_valid_fields() {
        let init = serde_json::json!({
            "slim": { "extraEngines": "mjml", "disabledDiagnostics": ["bad-dedent", 3] },
            "erb": { "disabledDiagnostics": ["Html/UnclosedElement"] },
            "haml": { "disabledDiagnostics": "illegal-nesting" }
        });
        let (options, ignored) = Options::from_init(Some(&init));
        assert!(options.extra_engines.is_empty());
        assert_eq!(options.disabled(TemplateLang::Slim), ["bad-dedent"]);
        assert_eq!(options.disabled(TemplateLang::Skim), ["bad-dedent"]);
        assert_eq!(options.disabled(TemplateLang::Erb), ["Html/UnclosedElement"]);
        assert!(options.disabled(TemplateLang::Haml).is_empty() && options.disabled(TemplateLang::Liquid).is_empty());
        assert_eq!(ignored.len(), 3, "{ignored:?}");
        let (none, ignored) = Options::from_init(None);
        assert!(ignored.is_empty() && none.extra_engines.is_empty() && none.format_enabled(TemplateLang::Slim));
        assert!(SECTIONS.iter().flat_map(|(_, langs)| *langs).all(|&lang| none.disabled(lang).is_empty()));
    }

    #[test]
    fn languages_switch_diagnostics_and_formatting_off() {
        let init = serde_json::json!({
            "erb": { "diagnostics": { "enabled": false } },
            "slim": { "format": { "enabled": false }, "diagnostics": { "enabled": "no" } },
            "haml": { "diagnostics": { "enabled": true }, "format": {} },
            "jbuilder": { "diagnostics": { "enabled": false } }
        });
        let (options, ignored) = Options::from_init(Some(&init));
        assert_eq!(ignored.len(), 1, "{ignored:?}");
        assert!(!options.align_erb_comments);
        let align = |value| Options::from_init(Some(&serde_json::json!({ "erb": { "format": { "alignComments": value } } })));
        assert!(align(serde_json::json!(true)).0.align_erb_comments);
        assert_eq!(align(serde_json::json!("yes")).1.len(), 1);
        assert!(!options.diagnostics_enabled(TemplateLang::Erb));
        assert!(options.diagnostics_enabled(TemplateLang::Haml) && options.diagnostics_enabled(TemplateLang::Slim));
        assert!(options.diagnostics_enabled(TemplateLang::Ruby), "Jbuilder has no switch");
        let formats = |lang| options.format_enabled(lang);
        assert!(formats(TemplateLang::Erb) && formats(TemplateLang::Haml) && formats(TemplateLang::Ruby));
        assert!(!formats(TemplateLang::Slim) && !formats(TemplateLang::Skim));
    }
}
