use std::fmt::Write;

use neomem_proto::{SearchHit, SearchResults, shown_memory};

pub fn render(results: &SearchResults, budget: usize) -> String {
    let mut context = String::new();
    for (rank, hit) in results.hits.iter().enumerate() {
        let entry = entry(rank + 1, hit);
        if context.chars().count() + entry.chars().count() > budget {
            break;
        }
        context.push_str(&entry);
    }
    context
}

fn entry(rank: usize, hit: &SearchHit) -> String {
    let conflict = if hit.conflict { "[conflict] " } else { "" };
    let mut entry = format!("{rank}. {conflict}{}", shown_memory(&hit.fact, &hit.entities));
    for linked in &hit.linked {
        let _ = writeln!(entry, "  linked: {} {}", linked.kind, linked.fact.content());
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hit_is_shown_with_its_dates_as_neomem_search_prints_it() {
        let results: SearchResults = serde_json::from_value(serde_json::json!({
            "hits": [{
                "fact": {"about":["ent_01M5J6PVMD04ASNCXMR8R93842"],"content":"user: The Osprey bicycle is now blue.",
                    "durability":"ordinary","evidence":[],"id":"fct_01M5J6PVN65J47RFMV3735RVV5","importance":5,
                    "invalid_at":null,"links":[],"recorded_at":1_792_686_124_710_i64,"sources":["int_01M5J6PTGH9KVMD1GY9MHQFDXP"],
                    "tags":[],"updated_at":1_792_686_124_710_i64,"valid_at":1_780_401_600_000_i64},
                "score": 0.9, "similarity": 0.9, "decay": 1.0, "replaced": [], "conflict": false,
                "linked": [], "entities": [{"id": "ent_01M5J6PVMD04ASNCXMR8R93842", "name": "Osprey bicycle", "kind": "thing"}]
            }],
            "model": "m"
        }))
        .unwrap();
        let rendered = render(&results, 10_000);
        assert!(
            rendered
                .starts_with("1. fct_01M5J6PVN65J47RFMV3735RVV5: user: The Osprey bicycle is now blue.\n")
        );
        assert!(rendered.contains("  valid_at: 2026-06-02T12:00:00Z   invalid_at: null (still true)\n"));
        assert!(rendered.contains("  about: ent_01M5J6PVMD04ASNCXMR8R93842 (Osprey bicycle)\n"));
        assert_eq!(render(&results, 20), "");
    }
}
