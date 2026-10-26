//! ViewComponent slots: `renders_one :header` / `renders_many :items` in the class, set by callers with
//! `c.with_header` inside `render(CardComponent.new) do |c|`, read in the component's template as `header`.

use crate::partials::{resolve_component, singularize, template_stem};
use crate::project::Project;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static SLOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*renders_(one|many)\s*\(?\s*:(\w+)").unwrap());
static RENDER_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\brender\s*\(?\s*((?:[A-Z]\w*::)*[A-Z]\w*)\.new\b[^\n]*?\bdo\s*\|\s*(\w+)\s*[|,]").unwrap());

#[derive(Debug, PartialEq)]
pub struct Slot {
    pub name: String,
    pub many: bool,
}

pub fn slots(class_source: &str) -> Vec<Slot> {
    SLOT.captures_iter(class_source).map(|cap| Slot { name: cap[2].to_string(), many: &cap[1] == "many" }).collect()
}

/// The setters a caller uses: `with_header`; `with_item` and `with_items` for `renders_many :items`.
pub fn setters(slots: &[Slot]) -> Vec<String> {
    slots
        .iter()
        .flat_map(|s| {
            let one = format!("with_{}", if s.many { singularize(&s.name) } else { s.name.clone() });
            let all = s.many.then(|| format!("with_{}", s.name));
            std::iter::once(one).chain(all)
        })
        .collect()
}

/// The readers a component's template uses: `header`, `header?`.
pub fn readers(slots: &[Slot]) -> Vec<String> {
    slots.iter().flat_map(|s| [s.name.clone(), format!("{}?", s.name)]).collect()
}

/// The component class whose `render … do |var|` block encloses the end of `before`, when `var.` is being typed.
pub fn block_component<'a>(before: &'a str, var: &str) -> Option<&'a str> {
    RENDER_BLOCK.captures_iter(before).filter(|cap| &cap[2] == var).last().map(|cap| cap.get(1).map_or("", |m| m.as_str()))
}

/// The class file of a component's template: `card_component.html.erb` next to it, or one folder up for a
/// sidecar `card_component/card_component.html.erb`.
pub fn class_of_template(project: &Project, template: &Path) -> Option<PathBuf> {
    let stem = template_stem(project, template.file_name()?.to_str()?);
    if !stem.ends_with("_component") {
        return None;
    }
    let dir = template.parent()?;
    let sidecar = dir.file_name().is_some_and(|d| d == stem);
    let class = if sidecar { dir.with_extension("rb") } else { dir.join(format!("{stem}.rb")) };
    class.is_file().then_some(class)
}

/// Slots of the component class `name` as seen from `current`.
pub fn component_slots(project: &Project, current: &Path, name: &str) -> Vec<Slot> {
    let class = resolve_component(project, current, name).into_iter().find(|f| f.extension().is_some_and(|e| e == "rb"));
    class.and_then(|f| std::fs::read_to_string(f).ok()).map(|text| slots(&text)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: &str = "class CardComponent < ViewComponent::Base\n  renders_one :header\n  renders_many :items, ItemComponent\n  renders_many(:people)\nend\n";

    #[test]
    fn setters_and_readers() {
        let slots = slots(CARD);
        assert_eq!(setters(&slots), ["with_header", "with_item", "with_items", "with_person", "with_people"]);
        assert_eq!(readers(&slots)[..2], ["header".to_string(), "header?".to_string()]);
    }

    #[test]
    fn block_variables() {
        let before = "<%= render CardComponent.new(title: \"x\") do |card| %>\n  <% card.with_header do %>\n<%= render(Admin::ListComponent.new) do |c, x| %>\n  <%= c.";
        assert_eq!(block_component(before, "c"), Some("Admin::ListComponent"));
        assert_eq!(block_component(before, "card"), Some("CardComponent"));
        assert_eq!(block_component(before, "f"), None);
    }

    #[test]
    fn template_classes() {
        let root = std::env::temp_dir().join(format!("rt-slots-{}", std::process::id()));
        let components = root.join("app/components");
        std::fs::create_dir_all(components.join("card_component")).unwrap();
        for f in ["card_component.rb", "card_component/card_component.html.erb", "list_component.rb", "list_component.html.slim"] {
            std::fs::write(components.join(f), CARD).unwrap();
        }
        assert_eq!(
            class_of_template(&Project::unknown(), &components.join("card_component/card_component.html.erb")),
            Some(components.join("card_component.rb"))
        );
        assert_eq!(
            class_of_template(&Project::unknown(), &components.join("list_component.html.slim")),
            Some(components.join("list_component.rb"))
        );
        assert_eq!(class_of_template(&Project::unknown(), &root.join("app/views/users/show.html.erb")), None);
        let view = root.join("app/views/home/index.html.erb");
        assert_eq!(component_slots(&Project::unknown(), &view, "CardComponent").len(), 3);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
