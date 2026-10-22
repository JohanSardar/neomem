use std::fmt::Write;

use neomem_proto::{SearchHit, SearchResults};

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
    let fact = &hit.fact;
    let conflict = if hit.conflict { "  [conflict]" } else { "" };
    let ended = fact
        .invalid_at()
        .map(|ended| format!(", invalid since {}", moment(ended.as_millis())))
        .unwrap_or_default();
    let mut entry = String::new();
    let _ = writeln!(entry, "{rank}. {}{conflict}", fact.content());
    let _ = writeln!(entry, "   valid since {}{ended}", moment(fact.valid_at().as_millis()));
    let names: Vec<&str> = hit.entities.iter().map(|entity| entity.name.as_str()).collect();
    if !names.is_empty() {
        let _ = writeln!(entry, "   about {}", names.join(", "));
    }
    for linked in &hit.linked {
        let _ = writeln!(entry, "   {} {}", linked.kind, linked.fact.content());
    }
    entry
}

fn moment(millis: i64) -> String {
    jiff::Timestamp::from_millisecond(millis)
        .map_or_else(|_| millis.to_string(), |moment| moment.strftime("%Y-%m-%dT%H:%M:%SZ").to_string())
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
        assert_eq!(
            render(&results, 10_000),
            "1. user: The Osprey bicycle is now blue.\n   valid since 2026-06-02T12:00:00Z\n   about Osprey bicycle\n"
        );
        assert_eq!(render(&results, 20), "");
    }
}
