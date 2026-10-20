use std::path::{Path, PathBuf};

use neomem_core::{Entity, Fact};

use crate::calendar;

pub const FACTS_DIR: &str = "facts";
pub const ENTITIES_DIR: &str = "entities";
const LONGEST_SLUG: usize = 48;

pub fn fact_path(root: &Path, fact: &Fact) -> PathBuf {
    let (year, month) = calendar::year_and_month(fact.recorded_at());
    root.join(FACTS_DIR).join(format!("{year:04}")).join(format!("{month:02}")).join(format!(
        "{}_{}.md",
        fact.id(),
        slug(fact.content(), "fact")
    ))
}

pub fn entity_path(root: &Path, entity: &Entity, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let directory = root.join(ENTITIES_DIR).join(entity.kind().as_str());
    let plain = directory.join(format!("{}.md", slug(entity.name(), "entity")));
    if !taken(&plain) {
        return plain;
    }
    let id = entity.id().to_string();
    let suffix = &id[id.len().saturating_sub(6)..];
    directory.join(format!("{}-{}.md", slug(entity.name(), "entity"), suffix.to_lowercase()))
}

pub fn slug(text: &str, fallback: &str) -> String {
    let mut slug = String::new();
    for word in
        text.split(|character: char| !character.is_ascii_alphanumeric()).filter(|word| !word.is_empty())
    {
        if slug.len() + word.len() + 1 > LONGEST_SLUG {
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&word.to_ascii_lowercase());
    }
    if slug.is_empty() { fallback.to_owned() } else { slug }
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn slugs_are_short_lowercase_words() {
        assert_eq!(
            slug("Edge's ship date is November 1, 2026.", "fact"),
            "edge-s-ship-date-is-november-1-2026"
        );
        assert_eq!(slug("Sam Kim", "entity"), "sam-kim");
        assert_eq!(slug("日本", "entity"), "entity");
        assert!(slug(&"word ".repeat(40), "fact").len() <= 48);
    }
}
