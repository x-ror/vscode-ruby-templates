//! Every diagnostic rule, by the `Department/Name` that is its diagnostic code and its name in `.ruby-templates.toml`.

pub mod file;
pub mod resolve;

use crate::lang::TemplateLang;
use TemplateLang::{Erb, Haml, Ruby, Slim};

#[derive(Debug)]
pub struct Rule {
    pub name: &'static str,
    /// Skim counts as Slim.
    pub languages: &'static [TemplateLang],
    /// The code the rule had before 0.11, still accepted in `disabledDiagnostics`.
    pub alias: &'static str,
}

impl Rule {
    pub fn department(&self) -> &'static str {
        self.name.split_once('/').map_or(self.name, |(department, _)| department)
    }

    pub fn applies_to(&self, lang: TemplateLang) -> bool {
        self.languages.contains(&if lang == TemplateLang::Skim { Slim } else { lang })
    }
}

const fn rule(name: &'static str, languages: &'static [TemplateLang], alias: &'static str) -> Rule {
    Rule { name, languages, alias }
}

const ALL: &[TemplateLang] = &[Erb, Haml, Slim, Ruby];
const HAML_SLIM: &[TemplateLang] = &[Haml, Slim];

pub const RULES: &[Rule] = &[
    rule("Syntax/Ruby", ALL, "ruby-syntax"),
    rule("Syntax/Template", HAML_SLIM, "syntax-error"),
    rule("Syntax/UnclosedErbTag", &[Erb], "unclosed-tag"),
    rule("Syntax/InconsistentIndent", &[Haml], "inconsistent-indent"),
    rule("Syntax/TopLevelIndent", &[Haml], "top-level-indent"),
    rule("Syntax/IllegalNesting", &[Haml], "illegal-nesting"),
    rule("Syntax/DoctypeChildren", HAML_SLIM, "doctype-children"),
    rule("Syntax/UnmatchedBranch", &[Haml], "unmatched-branch"),
    rule("Syntax/ExplicitEnd", HAML_SLIM, "explicit-end"),
    rule("Syntax/IllegalElement", &[Haml], "illegal-element"),
    rule("Syntax/UnknownFilter", &[Haml], "unknown-filter"),
    rule("Syntax/SelfClosingContent", &[Haml], "self-closing-content"),
    rule("Syntax/UnclosedAttribute", HAML_SLIM, "unclosed-attribute"),
    rule("Syntax/BadDedent", &[Slim], "bad-dedent"),
    rule("Syntax/UnknownEngine", &[Slim], "unknown-engine"),
    rule("Html/UnexpectedCloseTag", &[Erb], "unexpected-close-tag"),
    rule("Html/MismatchedCloseTag", &[Erb], "mismatched-close-tag"),
    rule("Html/UnclosedElement", &[Erb], "unclosed-element"),
    rule("Html/ImgAlt", &[Erb], "img-missing-alt"),
    rule("Html/DuplicateId", &[Erb], "duplicate-id"),
    rule("Html/VoidChildren", HAML_SLIM, "void-children"),
    rule("Rails/MissingPartial", ALL, "unresolved-render"),
    rule("Rails/StrictLocals", ALL, "strict-locals"),
];

pub fn by_name(name: &str) -> Option<&'static Rule> {
    RULES.iter().find(|rule| rule.name == name)
}

/// The rule a diagnostic code or an old alias names.
pub fn by_code(code: &str) -> Option<&'static Rule> {
    RULES.iter().find(|rule| rule.name == code || rule.alias == code)
}

pub fn departments() -> impl Iterator<Item = &'static str> {
    RULES.iter().map(Rule::department).enumerate().filter(|&(i, d)| RULES[..i].iter().all(|r| r.department() != d)).map(|(_, d)| d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_and_aliases_are_unique_and_shaped() {
        let shape = regex::Regex::new(r"^[A-Z][A-Za-z0-9]*/[A-Z][A-Za-z0-9]*$").unwrap();
        let mut seen = HashSet::new();
        for rule in RULES {
            assert!(shape.is_match(rule.name), "{}", rule.name);
            assert!(seen.insert(rule.name), "{} twice", rule.name);
            assert!(seen.insert(rule.alias), "{} twice", rule.alias);
        }
        assert_eq!(departments().collect::<Vec<_>>(), ["Syntax", "Html", "Rails"]);
    }

    #[test]
    fn codes_resolve_by_name_or_alias() {
        assert_eq!(by_code("img-missing-alt").map(|r| r.name), Some("Html/ImgAlt"));
        assert_eq!(by_code("Html/ImgAlt").map(|r| r.name), Some("Html/ImgAlt"));
        assert!(by_name("img-missing-alt").is_none() && by_code("Html/ImgAltt").is_none());
        assert!(by_name("Syntax/Template").is_some_and(|r| r.applies_to(TemplateLang::Skim) && !r.applies_to(Erb)));
    }

    #[test]
    fn every_diagnostic_code_constant_is_a_rule_and_the_other_way_round() {
        let re = regex::Regex::new(r#"pub(?:\(crate\))? const [A-Z_]+: &str = "([A-Z][A-Za-z]+/[A-Za-z]+)""#).unwrap();
        let sources = [
            include_str!("checks.rs"),
            include_str!("erb.rs"),
            include_str!("erb/tags.rs"),
            include_str!("erb/html_lint.rs"),
            include_str!("haml/indent.rs"),
            include_str!("slim/indent.rs"),
            include_str!("ruby/generated.rs"),
        ];
        let declared: HashSet<&str> = sources.iter().flat_map(|src| re.captures_iter(src).map(|c| c.get(1).unwrap().as_str())).collect();
        let registered: HashSet<&str> = RULES.iter().map(|rule| rule.name).collect();
        assert_eq!(declared, registered);
    }

    /// The site's catalogue lists each registered rule, for the same languages and as a warning that exists.
    #[test]
    fn site_catalogue_lists_the_registered_rules() {
        let catalogue = include_str!("../../site/src/data/rules.ts");
        let field = |entry: &str, key: &str| {
            let re = regex::Regex::new(&format!(r#"\b{key}: (?:\[([^\]]*)\]|"([^"]*)")"#)).unwrap();
            re.captures(entry).map(|c| c.get(1).or(c.get(2)).unwrap().as_str().replace(['"', ' '], ""))
        };
        let language = |lang: &TemplateLang| match lang {
            Erb => "erb",
            Haml => "haml",
            Slim => "slim",
            _ => "jbuilder",
        };
        for rule in RULES.iter().filter(|rule| rule.department() != "Syntax") {
            let marker = format!("name: \"{}\"", rule.name);
            let (_, rest) = catalogue.split_once(&marker).unwrap_or_else(|| panic!("{} is not in rules.ts", rule.name));
            let entry = rest.split("\n  {\n").next().unwrap();
            let listed = field(entry, "availableIn").or_else(|| field(entry, "languages")).unwrap();
            let have: Vec<&str> = rule.languages.iter().map(language).collect();
            assert_eq!(listed, have.join(","), "{} languages", rule.name);
            assert_eq!(field(entry, "type").as_deref(), Some("warning"), "{}", rule.name);
            assert_ne!(field(entry, "status").as_deref(), Some("planned"), "{}", rule.name);
        }
    }
}
