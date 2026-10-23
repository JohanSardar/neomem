use std::fmt::Write;

use neomem_core::{EpochMs, Fact};
use serde::Serialize;

use crate::response::EntityRef;

pub fn shown_memory(fact: &Fact, entities: &[EntityRef]) -> String {
    let about: Vec<String> = fact
        .about()
        .iter()
        .map(|id| match entities.iter().find(|entity| entity.id == *id) {
            Some(entity) => format!("{id} ({})", entity.name),
            None => id.to_string(),
        })
        .collect();
    let invalid = fact.invalid_at().map_or_else(|| "null (still true)".to_owned(), EpochMs::utc_text);
    let links: Vec<String> =
        fact.links().iter().map(|link| format!("{} {}", named(&link.kind), link.to)).collect();
    let mut text = String::new();
    let _ = writeln!(text, "{}: {}", fact.id(), fact.content());
    let _ = writeln!(text, "  valid_at: {}   invalid_at: {invalid}", fact.valid_at().utc_text());
    let _ = writeln!(
        text,
        "  recorded_at: {}   updated_at: {}",
        fact.recorded_at().utc_text(),
        fact.updated_at().utc_text()
    );
    let _ = writeln!(text, "  about: {}", listed(&about));
    let _ = writeln!(
        text,
        "  importance: {}   durability: {}   tags: {}",
        fact.importance().get(),
        named(&fact.durability()),
        listed(&fact.tags().iter().map(ToString::to_string).collect::<Vec<_>>())
    );
    let _ = writeln!(
        text,
        "  sources: {}",
        listed(&fact.sources().iter().map(ToString::to_string).collect::<Vec<_>>())
    );
    let _ = writeln!(text, "  links: {}", listed(&links));
    let _ = writeln!(
        text,
        "  evidence: {}",
        listed(&fact.extra_evidence().iter().copied().map(EpochMs::utc_text).collect::<Vec<_>>())
    );
    text
}

fn listed(items: &[String]) -> String {
    if items.is_empty() { "[]".to_owned() } else { items.join(", ") }
}

fn named(value: &impl Serialize) -> String {
    serde_json::to_value(value).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_is_shown_with_every_field_and_readable_dates() {
        let fact: Fact = serde_json::from_value(serde_json::json!({
            "about": ["ent_01M5J6PVMD04ASNCXMR8R93842"], "content": "The user uses 1 tablespoon of coffee per 5 ounces of water.",
            "durability": "preference", "evidence": [], "id": "fct_01M5J6PVN65J47RFMV3735RVV5", "importance": 4,
            "invalid_at": null, "links": [{"type": "updates", "to": "fct_01M5J6PVMD04ASNCXMR8R93843"}],
            "recorded_at": 1_792_686_124_710_i64, "sources": ["int_01M5J6PTGH9KVMD1GY9MHQFDXP"], "tags": [],
            "updated_at": 1_792_686_124_710_i64, "valid_at": 1_688_083_200_000_i64
        }))
        .unwrap();
        let entities: Vec<EntityRef> = serde_json::from_value(serde_json::json!([
            {"id": "ent_01M5J6PVMD04ASNCXMR8R93842", "name": "the user", "kind": "person"}
        ]))
        .unwrap();
        assert_eq!(
            shown_memory(&fact, &entities),
            "fct_01M5J6PVN65J47RFMV3735RVV5: The user uses 1 tablespoon of coffee per 5 ounces of water.\n\
             \x20 valid_at: 2023-06-30T00:00:00Z   invalid_at: null (still true)\n\
             \x20 recorded_at: 2026-10-22T16:22:04Z   updated_at: 2026-10-22T16:22:04Z\n\
             \x20 about: ent_01M5J6PVMD04ASNCXMR8R93842 (the user)\n\
             \x20 importance: 4   durability: preference   tags: []\n\
             \x20 sources: int_01M5J6PTGH9KVMD1GY9MHQFDXP\n\
             \x20 links: updates fct_01M5J6PVMD04ASNCXMR8R93843\n\
             \x20 evidence: []\n"
        );
    }
}
