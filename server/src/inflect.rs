//! ActiveSupport's inflections the views need: plurals of model names and the file names of constants.

use std::path::PathBuf;

// Suffix-matched like ActiveSupport's irregulars, so `sales_person` → `sales_people`.
const IRREGULAR: [(&str, &str); 10] = [
    ("person", "people"),
    ("child", "children"),
    ("movie", "movies"),
    ("status", "statuses"),
    ("alias", "aliases"),
    ("cookie", "cookies"),
    ("hero", "heroes"),
    ("quiz", "quizzes"),
    ("cache", "caches"),
    ("analysis", "analyses"),
];
const UNCOUNTABLE: [&str; 9] = ["equipment", "information", "rice", "money", "species", "series", "fish", "sheep", "news"];

/// Matched on the last `_` segment so `prices` isn't read as `rice`.
fn uncountable(word: &str) -> bool {
    UNCOUNTABLE.contains(&word.rsplit('_').next().unwrap_or(word))
}

fn ends_with_consonant_y(word: &str) -> bool {
    word.strip_suffix('y').and_then(|w| w.chars().last()).is_some_and(|c| !"aeiou".contains(c))
}

pub fn pluralize(word: &str) -> String {
    if uncountable(word) {
        return word.to_string();
    }
    if let Some((stem, plural)) = IRREGULAR.iter().find_map(|(s, p)| Some((word.strip_suffix(s)?, p))) {
        return format!("{stem}{plural}");
    }
    if ends_with_consonant_y(word) {
        return format!("{}ies", &word[..word.len() - 1]);
    }
    if ["s", "x", "z", "ch", "sh"].iter().any(|e| word.ends_with(e)) {
        return format!("{word}es");
    }
    format!("{word}s")
}

pub fn singularize(word: &str) -> String {
    if uncountable(word) {
        return word.to_string();
    }
    for (singular, plural) in IRREGULAR {
        if let Some(stem) = word.strip_suffix(plural) {
            return format!("{stem}{singular}");
        }
        if word.ends_with(singular) {
            return word.to_string();
        }
    }
    if let Some(stem) = word.strip_suffix("ies").filter(|s| ends_with_consonant_y(&format!("{s}y"))) {
        return format!("{stem}y");
    }
    if let Some(stem) = word.strip_suffix("es").filter(|s| ["x", "ch", "ss", "sh", "zz"].iter().any(|e| s.ends_with(e))) {
        return stem.to_string();
    }
    // `ss|us|is` end singulars (`address`, `bus`, `axis`); their plurals took `es` above or just miss.
    match word.strip_suffix('s') {
        Some(stem) if !["ss", "us", "is"].iter().any(|e| word.ends_with(e)) => stem.to_string(),
        _ => word.to_string(),
    }
}

/// `user_card` → `UserCard`.
pub fn camel_case(s: &str) -> String {
    s.split('_')
        .flat_map(|w| {
            let mut chars = w.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>())
        })
        .collect()
}

/// ActiveSupport's `underscore` of one constant: `UI` → `ui`, `HTMLCard` → `html_card`, `Card2Component` → `card2_component`.
pub fn snake_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, &ch) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(i + 1);
        // `([a-z\d])([A-Z])` and `([A-Z\d]+)([A-Z][a-z])`: a word starts after a lowercase letter or digit, or
        // where an acronym meets a capitalized word.
        let starts_word = ch.is_ascii_uppercase()
            && prev.is_some_and(|p| {
                p.is_ascii_lowercase() || p.is_ascii_digit() || (p.is_ascii_uppercase() && next.is_some_and(char::is_ascii_lowercase))
            });
        if starts_word {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

/// The file path of a constant without its extension: `Admin::UserCard` → `admin/user_card`.
pub fn constant_path(constant: &str) -> PathBuf {
    constant.split("::").map(snake_case).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflection_round_trips() {
        for (singular, plural) in [
            ("user", "users"),
            ("category", "categories"),
            ("day", "days"),
            ("address", "addresses"),
            ("box", "boxes"),
            ("match", "matches"),
            ("person", "people"),
            ("sales_person", "sales_people"),
            ("child", "children"),
            ("movie", "movies"),
            ("status", "statuses"),
            ("news", "news"),
            ("line_item", "line_items"),
            ("price", "prices"),
            ("cookie", "cookies"),
            ("hero", "heroes"),
            ("quiz", "quizzes"),
            ("cache", "caches"),
            ("analysis", "analyses"),
            ("daily_news", "daily_news"),
        ] {
            assert_eq!(pluralize(singular), plural);
            assert_eq!(singularize(plural), singular);
            assert_eq!(singularize(singular), singular);
        }
        for singular in ["bus", "virus", "axis"] {
            assert_eq!(singularize(singular), singular);
        }
    }

    #[test]
    fn constants_to_file_names() {
        assert_eq!(snake_case("CardComponent"), "card_component");
        for (constant, file) in
            [("UI", "ui"), ("HTMLCardComponent", "html_card_component"), ("Card2Component", "card2_component"), ("APIv2", "ap_iv2")]
        {
            assert_eq!(snake_case(constant), file, "{constant}");
        }
        assert_eq!(constant_path("Admin::HTMLCard"), PathBuf::from("admin/html_card"));
    }
}
