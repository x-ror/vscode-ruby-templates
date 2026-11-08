//! Every rule, by the `Department/Name` that is its diagnostic code (lint rules) and its name in `.ruby-templates.toml`.

pub mod file;
pub mod resolve;

use crate::lang::TemplateLang;
use TemplateLang::{Erb, Haml, Ruby, Slim};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Rewrites the template.
    Format,
    /// Reports a diagnostic.
    Lint,
}

/// A whole-number option of a rule's table, 0 or more.
#[derive(Debug)]
pub struct CountOption {
    pub key: &'static str,
    pub default: usize,
}

/// An option of a rule's table that is one of a few words.
#[derive(Debug)]
pub struct ChoiceOption {
    pub key: &'static str,
    pub values: &'static [&'static str],
    pub default: &'static str,
}

#[derive(Debug)]
pub struct Rule {
    pub name: &'static str,
    /// Skim counts as Slim.
    pub languages: &'static [TemplateLang],
    pub kind: Kind,
    /// The languages where the rule runs when no file switches it.
    pub on_by_default: &'static [TemplateLang],
    /// The keys its table takes besides `enabled`, `include` and `exclude`.
    pub options: &'static [CountOption],
    pub choices: &'static [ChoiceOption],
    /// The code a lint rule had before 0.11, still accepted in `disabledDiagnostics`.
    pub alias: Option<&'static str>,
}

impl Rule {
    pub fn department(&self) -> &'static str {
        self.name.split_once('/').map_or(self.name, |(department, _)| department)
    }

    pub fn applies_to(&self, lang: TemplateLang) -> bool {
        self.languages.contains(&if lang == TemplateLang::Skim { Slim } else { lang })
    }

    pub fn is_on_by_default(&self, lang: TemplateLang) -> bool {
        self.on_by_default.contains(&if lang == TemplateLang::Skim { Slim } else { lang })
    }
}

const fn lint(name: &'static str, languages: &'static [TemplateLang], alias: &'static str) -> Rule {
    Rule { alias: Some(alias), ..new_lint(name, languages) }
}

/// A lint rule with no code from before 0.11 to accept.
const fn new_lint(name: &'static str, languages: &'static [TemplateLang]) -> Rule {
    Rule { name, languages, kind: Kind::Lint, on_by_default: languages, options: &[], choices: &[], alias: None }
}

const fn format_rule(
    name: &'static str,
    languages: &'static [TemplateLang],
    on_by_default: &'static [TemplateLang],
    options: &'static [CountOption],
    choices: &'static [ChoiceOption],
) -> Rule {
    Rule { name, languages, kind: Kind::Format, on_by_default, options, choices, alias: None }
}

pub const EMPTY_LINES: &str = "Layout/EmptyLines";
pub const TRAILING_EMPTY_LINES: &str = "Layout/TrailingEmptyLines";
pub const AROUND_BLOCK_BODY: &str = "Layout/EmptyLinesAroundBlockBody";
pub const AROUND_ELEMENT_BODY: &str = "Layout/EmptyLinesAroundElementBody";
pub const AROUND_BRANCHES: &str = "Layout/EmptyLinesAroundBranches";
pub const SPACE_INSIDE_ERB_TAG: &str = "Layout/SpaceInsideErbTag";
pub const SPACE_AFTER_SCRIPT_MARKER: &str = "Layout/SpaceAfterScriptMarker";
pub const ERB_COMMENT_SYNTAX: &str = "Style/ErbCommentSyntax";
pub const SPACE_IN_HTML_TAG: &str = "Layout/SpaceInHtmlTag";
pub const INDENTATION_STYLE: &str = "Layout/IndentationStyle";
pub const TRAILING_WHITESPACE: &str = "Layout/TrailingWhitespace";

pub const NO_EMPTY_LINES: &str = "no_empty_lines";
pub const EMPTY_LINES_STYLE: &str = "empty_lines";
pub const STYLE: ChoiceOption = ChoiceOption { key: "style", values: &[NO_EMPTY_LINES, EMPTY_LINES_STYLE], default: NO_EMPTY_LINES };

pub const SPACES: &str = "spaces";
pub const TABS: &str = "tabs";
/// Without `style` in the file, the editor's or `.editorconfig`'s style applies (`resolve::indent_style`); the default here is for display.
pub const INDENT_STYLE: ChoiceOption = ChoiceOption { key: "style", values: &[SPACES, TABS], default: SPACES };

const ALL: &[TemplateLang] = &[Erb, Haml, Slim, Ruby];
const HAML_SLIM: &[TemplateLang] = &[Haml, Slim];
const TEMPLATES: &[TemplateLang] = &[Erb, Haml, Slim];

pub const RULES: &[Rule] = &[
    lint("Syntax/Ruby", ALL, "ruby-syntax"),
    lint("Syntax/Template", HAML_SLIM, "syntax-error"),
    lint("Syntax/UnclosedErbTag", &[Erb], "unclosed-tag"),
    lint("Syntax/InconsistentIndent", &[Haml], "inconsistent-indent"),
    lint("Syntax/TopLevelIndent", &[Haml], "top-level-indent"),
    lint("Syntax/IllegalNesting", &[Haml], "illegal-nesting"),
    lint("Syntax/DoctypeChildren", HAML_SLIM, "doctype-children"),
    lint("Syntax/UnmatchedBranch", &[Haml], "unmatched-branch"),
    lint("Syntax/ExplicitEnd", HAML_SLIM, "explicit-end"),
    lint("Syntax/IllegalElement", &[Haml], "illegal-element"),
    lint("Syntax/UnknownFilter", &[Haml], "unknown-filter"),
    lint("Syntax/SelfClosingContent", &[Haml], "self-closing-content"),
    lint("Syntax/UnclosedAttribute", HAML_SLIM, "unclosed-attribute"),
    lint("Syntax/BadDedent", &[Slim], "bad-dedent"),
    lint("Syntax/UnknownEngine", &[Slim], "unknown-engine"),
    lint("Html/UnexpectedCloseTag", &[Erb], "unexpected-close-tag"),
    lint("Html/MismatchedCloseTag", &[Erb], "mismatched-close-tag"),
    lint("Html/UnclosedElement", &[Erb], "unclosed-element"),
    lint("Html/ImgAlt", &[Erb], "img-missing-alt"),
    lint("Html/DuplicateId", &[Erb], "duplicate-id"),
    lint("Html/VoidChildren", HAML_SLIM, "void-children"),
    lint("Rails/MissingPartial", ALL, "unresolved-render"),
    lint("Rails/StrictLocals", ALL, "strict-locals"),
    new_lint("Rails/SilentRender", TEMPLATES),
    new_lint("Rails/SilentHelper", TEMPLATES),
    new_lint("Rails/DebugOutput", TEMPLATES),
    format_rule(EMPTY_LINES, TEMPLATES, &[], &[CountOption { key: "max", default: 1 }], &[]),
    // ERB is rendered byte for byte, so its end is output.
    format_rule(TRAILING_EMPTY_LINES, TEMPLATES, HAML_SLIM, &[], &[]),
    format_rule(AROUND_BLOCK_BODY, TEMPLATES, &[], &[], &[STYLE]),
    format_rule(AROUND_ELEMENT_BODY, TEMPLATES, &[], &[], &[STYLE]),
    format_rule(AROUND_BRANCHES, TEMPLATES, &[], &[], &[STYLE]),
    format_rule(SPACE_INSIDE_ERB_TAG, &[Erb], &[], &[], &[]),
    format_rule(SPACE_AFTER_SCRIPT_MARKER, HAML_SLIM, &[], &[], &[]),
    format_rule(ERB_COMMENT_SYNTAX, &[Erb], &[], &[], &[]),
    format_rule(SPACE_IN_HTML_TAG, &[Erb], &[], &[], &[]),
    format_rule(INDENTATION_STYLE, &[Erb], &[Erb], &[], &[INDENT_STYLE]),
    format_rule(TRAILING_WHITESPACE, TEMPLATES, TEMPLATES, &[], &[]),
];

pub fn by_name(name: &str) -> Option<&'static Rule> {
    RULES.iter().find(|rule| rule.name == name)
}

/// The rule a diagnostic code or an old alias names.
pub fn by_code(code: &str) -> Option<&'static Rule> {
    RULES.iter().find(|rule| rule.name == code || rule.alias == Some(code))
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
            assert!(rule.alias.is_none_or(|alias| seen.insert(alias)), "{:?} twice", rule.alias);
        }
        assert_eq!(departments().collect::<Vec<_>>(), ["Syntax", "Html", "Rails", "Layout", "Style"]);
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
            include_str!("rails/output_lints.rs"),
            include_str!("erb.rs"),
            include_str!("erb/tags.rs"),
            include_str!("erb/html_lint.rs"),
            include_str!("haml/indent.rs"),
            include_str!("slim/indent.rs"),
            include_str!("ruby/generated.rs"),
        ];
        let declared: HashSet<&str> = sources.iter().flat_map(|src| re.captures_iter(src).map(|c| c.get(1).unwrap().as_str())).collect();
        let registered: HashSet<&str> = RULES.iter().filter(|rule| rule.kind == Kind::Lint).map(|rule| rule.name).collect();
        assert_eq!(declared, registered);
    }

    /// The site's catalogue lists each registered rule, for the same languages, as the same kind of rule and as one that exists.
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
            let kind = if rule.kind == Kind::Lint { "warning" } else { "autofix" };
            assert_eq!(field(entry, "type").as_deref(), Some(kind), "{}", rule.name);
            assert_ne!(field(entry, "status").as_deref(), Some("planned"), "{}", rule.name);
        }
    }
}
