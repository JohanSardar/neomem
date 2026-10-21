use std::fs;
use std::path::Path;

use neomem_core::{EntityId, EpochMs, FactId, IntakeId};

const START_MS: i64 = 1_700_000_000_000;
const DAY_MS: i64 = 86_400_000;
const FACTS_PER_ENTITY: usize = 100;
const FIRST: &[&str] = &[
    "Ada", "Bram", "Cleo", "Dov", "Esme", "Femi", "Gus", "Hana", "Ivo", "Jun", "Kai", "Lena", "Milo", "Nia",
    "Otto", "Pia", "Quin", "Rosa", "Sami", "Tova", "Uma", "Vik", "Wren", "Xia", "Yuri", "Zoe",
];
const LAST: &[&str] = &[
    "Abara",
    "Brandt",
    "Castell",
    "Doshi",
    "Eklund",
    "Farrow",
    "Grieve",
    "Halloran",
    "Ishida",
    "Jansen",
    "Kovac",
    "Lindqvist",
    "Moreau",
    "Nakamura",
    "Okafor",
    "Petrov",
    "Quaid",
    "Rasku",
    "Sorensen",
    "Tanaka",
];
const PROJECTS: &[&str] = &[
    "Atlas", "Beacon", "Cinder", "Delta", "Ember", "Fjord", "Garnet", "Harbor", "Iris", "Juniper", "Kestrel",
    "Lumen", "Meridian", "Nimbus", "Orchid", "Pylon", "Quarry", "Rivet", "Sable", "Tundra",
];
const PLACES: &[&str] =
    &["Lisbon", "Osaka", "Denver", "Tallinn", "Accra", "Hobart", "Quito", "Bergen", "Porto", "Kyoto"];
const THINGS: &[&str] = &[
    "the billing service",
    "the onboarding flow",
    "the iOS widget",
    "the search index",
    "the release pipeline",
    "the pricing page",
    "the sync engine",
    "the export job",
    "the login screen",
    "the analytics dashboard",
];
const VERBS: &[&str] =
    &["rewrote", "reviewed", "shipped", "paused", "migrated", "load-tested", "redesigned", "audited"];
const TOOLS: &[&str] =
    &["Postgres", "Redis", "SQLite", "Kafka", "Terraform", "SwiftUI", "React", "Rust", "Go", "Figma"];
const FOODS: &[&str] = &["ramen", "tapas", "pho", "falafel", "sushi", "dosa", "tacos", "pierogi"];

struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    fn below(&mut self, limit: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(limit.max(1)).unwrap_or(1)).unwrap_or(0)
    }

    fn pick<'list>(&mut self, list: &[&'list str]) -> &'list str {
        list[self.below(list.len())]
    }
}

struct Entity {
    id: EntityId,
    kind: &'static str,
    name: String,
}

fn entities(count: usize, draw: &mut Draw) -> Vec<Entity> {
    (0..count)
        .map(|number| {
            let seed = u128::try_from(number).unwrap_or(0) + 1;
            let id = EntityId::from_parts(EpochMs::from_millis(START_MS), seed);
            match number % 3 {
                0 => Entity {
                    id,
                    kind: "person",
                    name: format!("{} {} {number}", draw.pick(FIRST), draw.pick(LAST)),
                },
                1 => Entity { id, kind: "project", name: format!("{} {number}", draw.pick(PROJECTS)) },
                _ => Entity {
                    id,
                    kind: "organization",
                    name: format!("{} Labs {number}", draw.pick(PROJECTS)),
                },
            }
        })
        .collect()
}

fn sentence(draw: &mut Draw, about: &Entity, other: &Entity, day: i64) -> String {
    let date = jiff::Timestamp::from_millisecond(START_MS + day * DAY_MS)
        .map(|time| time.strftime("%B %-d, %Y").to_string())
        .unwrap_or_default();
    let number = draw.below(900) + 10;
    match draw.below(8) {
        0 => format!("{} {} {} on {date}.", about.name, draw.pick(VERBS), draw.pick(THINGS)),
        1 => format!(
            "{} moved {} to {} after a review on {date}.",
            about.name,
            draw.pick(THINGS),
            draw.pick(TOOLS)
        ),
        2 => format!("{} met {} in {} on {date}.", about.name, other.name, draw.pick(PLACES)),
        3 => format!("{} set the budget for {} at {number} thousand dollars.", about.name, draw.pick(THINGS)),
        4 => {
            format!("{} prefers {} for team lunches in {}.", about.name, draw.pick(FOODS), draw.pick(PLACES))
        }
        5 => format!("{} depends on {} for {}.", about.name, other.name, draw.pick(THINGS)),
        6 => format!("{} measured {number} ms p95 latency on {} on {date}.", about.name, draw.pick(THINGS)),
        _ => format!(
            "{} plans to launch {} in {} with {} people.",
            about.name,
            draw.pick(THINGS),
            draw.pick(PLACES),
            number / 10
        ),
    }
}

fn slug(text: &str) -> String {
    let mut slug: String = text
        .chars()
        .map(|character| if character.is_ascii_alphanumeric() { character.to_ascii_lowercase() } else { '-' })
        .collect();
    slug.truncate(48);
    slug.trim_matches('-').to_owned()
}

pub fn write(home: &Path, facts: usize) -> anyhow::Result<()> {
    let mut draw = Draw(u64::try_from(facts).unwrap_or(0));
    let people = entities((facts / FACTS_PER_ENTITY).max(3), &mut draw);
    for entity in &people {
        let directory = home.join("entities").join(entity.kind);
        fs::create_dir_all(&directory)?;
        fs::write(
            directory.join(format!("{}.md", slug(&entity.name))),
            format!(
                "---\nid: {}\nkind: entity\nentity_kind: {}\nname: {}\naliases: []\nrecorded_at: {START_MS}\nupdated_at: {START_MS}\nmerged_from: []\nmerged_into: null\nrelations: []\nlinks: []\n---\n",
                entity.id, entity.kind, entity.name
            ),
        )?;
    }
    for number in 0..facts {
        let about = &people[draw.below(people.len())];
        let other = &people[draw.below(people.len())];
        let day = i64::try_from(draw.below(1_000)).unwrap_or(0);
        let at = START_MS + day * DAY_MS;
        let seed = u128::try_from(number).unwrap_or(0) + 1;
        let id = FactId::from_parts(EpochMs::from_millis(at), seed);
        let source = IntakeId::from_parts(EpochMs::from_millis(at), seed);
        let content = sentence(&mut draw, about, other, day);
        let month = jiff::Timestamp::from_millisecond(at)
            .map(|time| time.strftime("%Y/%m").to_string())
            .unwrap_or_default();
        let directory = home.join("facts").join(month);
        fs::create_dir_all(&directory)?;
        let importance = draw.below(10) + 1;
        fs::write(
            directory.join(format!("{id}_{}.md", slug(&content))),
            format!(
                "---\nid: {id}\nkind: fact\ntags: [work-session]\nabout: [{}]\nrecorded_at: {at}\nupdated_at: {at}\nvalid_at: {at}\ninvalid_at: null\nimportance: {importance}\ndurability: ordinary\nsources: [{source}]\nlinks: []\nevidence: []\n---\n{content}\n",
                about.id
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_corpus_is_deterministic_and_has_one_file_per_fact() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        write(first.path(), 500).unwrap();
        write(second.path(), 500).unwrap();
        let files = |root: &Path| {
            let mut found: Vec<String> =
                walk(&root.join("facts")).into_iter().map(|path| fs::read_to_string(path).unwrap()).collect();
            found.sort();
            found
        };
        let facts = files(first.path());
        assert_eq!(facts.len(), 500);
        assert_eq!(facts, files(second.path()));
        assert!(
            facts
                .iter()
                .all(|text| text.starts_with("---\nid: fct_") && text.trim_end().lines().count() == 16)
        );
    }

    fn walk(directory: &Path) -> Vec<std::path::PathBuf> {
        fs::read_dir(directory)
            .unwrap()
            .flat_map(|entry| {
                let path = entry.unwrap().path();
                if path.is_dir() { walk(&path) } else { vec![path] }
            })
            .collect()
    }
}
