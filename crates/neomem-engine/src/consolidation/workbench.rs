use std::collections::HashSet;
use std::fmt::Write as _;

use neomem_core::{
    Entity, EntityId, EntityLink, EntityLinkKind, Fact, FactLink, FactLinkKind, Filter, Label, Memory,
    MemoryId, NewEntity, Purpose, Relation,
};
use neomem_state::RawSearch;
use serde_json::Value;

use super::registry::Registry;
use super::tools::Tool;
use crate::Engine;

const READ_LIMIT: usize = 8;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Tally {
    pub(crate) rounds: usize,
    pub(crate) writes: usize,
    pub(crate) rejected: usize,
}

pub(crate) struct Workbench<'engine> {
    pub(crate) engine: &'engine Engine,
    pub(crate) registry: Registry,
    pub(crate) tally: Tally,
}

type Outcome = Result<String, String>;

impl Workbench<'_> {
    pub(crate) fn run(&mut self, tool: Tool, arguments: &Value) -> String {
        let outcome = match tool {
            Tool::SearchFacts => self.search_facts(text(arguments, "query")),
            Tool::RecentFacts => Ok(self.recent_facts(number(arguments, "limit").unwrap_or(10))),
            Tool::SearchIntake => Ok(self.search_intake(text(arguments, "query"))),
            Tool::FollowLinks => self.follow_links(number(arguments, "fact")),
            Tool::Link => self.link(arguments),
            Tool::Invalidate => self.invalidate(number(arguments, "fact"), number(arguments, "replaced_by")),
            Tool::MergeFacts => self.merge_facts(number(arguments, "keep"), number(arguments, "duplicate")),
            Tool::SearchEntities => Ok(self.search_entities(text(arguments, "query"))),
            Tool::EntityFacts => self.entity_facts(number(arguments, "entity")),
            Tool::CreateEntity => self.create_entity(arguments),
            Tool::MergeEntities => {
                self.merge_entities(number(arguments, "keep"), number(arguments, "duplicate"))
            }
            Tool::AddAlias => self.add_alias(number(arguments, "entity"), text(arguments, "alias")),
            Tool::LinkEntity => {
                self.link_entity(number(arguments, "from"), text(arguments, "label"), number(arguments, "to"))
            }
        };
        match outcome {
            Ok(result) => {
                if tool.writes() {
                    self.tally.writes += 1;
                }
                result
            }
            Err(problem) => {
                self.tally.rejected += 1;
                format!("rejected: {problem}")
            }
        }
    }

    pub(crate) fn describe_fact(&mut self, fact: &Fact) -> String {
        let number = self.registry.fact_number(fact.id());
        let snapshot = self.engine.index.snapshot();
        let about: Vec<&str> =
            fact.about().iter().filter_map(|id| snapshot.entity(*id)).map(|entity| entity.name()).collect();
        let state = if fact.invalid_at().is_some() { "no longer valid" } else { "valid" };
        format!("[fact {number}] {} (about {}; {state})", fact.content(), about.join(", "))
    }

    pub(crate) fn describe_entity(&mut self, entity: &Entity) -> String {
        let number = self.registry.entity_number(entity.id());
        let aliases = if entity.aliases().is_empty() {
            String::new()
        } else {
            format!("; also {}", entity.aliases().join(", "))
        };
        format!("[entity {number}] {} ({}{aliases})", entity.name(), entity.kind())
    }

    fn fact(&self, number: Option<i64>, role: &str) -> Result<Fact, String> {
        let id = number
            .and_then(|number| self.registry.fact(number))
            .ok_or_else(|| format!("{role} is not a fact number you were shown"))?;
        self.engine
            .index
            .snapshot()
            .fact(id)
            .map(|fact| Fact::clone(fact))
            .ok_or_else(|| format!("fact {role} no longer exists"))
    }

    fn entity(&self, number: Option<i64>, role: &str) -> Result<Entity, String> {
        let id = number
            .and_then(|number| self.registry.entity(number))
            .ok_or_else(|| format!("{role} is not an entity number you were shown"))?;
        let snapshot = self.engine.index.snapshot();
        snapshot
            .entity(snapshot.survivor(id))
            .map(|entity| Entity::clone(entity))
            .ok_or_else(|| format!("entity {role} no longer exists"))
    }

    fn write(&self, memories: Vec<Memory>) -> Result<(), String> {
        let writing = self.engine.lock_writer();
        self.engine.write_memories(memories, &writing).map_err(|error| error.to_string())
    }

    fn listed(&mut self, facts: Vec<Fact>) -> String {
        if facts.is_empty() {
            return "nothing found".to_owned();
        }
        let mut text = String::new();
        for fact in facts {
            let line = self.describe_fact(&fact);
            let _ = writeln!(text, "{line}");
        }
        text
    }

    fn search_facts(&mut self, query: &str) -> Outcome {
        let snapshot = self.engine.index.snapshot();
        let vector = self
            .engine
            .embed_one_as(&snapshot, query, Purpose::Query)
            .ok_or("the embedder is not available")?;
        let mut found: Vec<Fact> = Vec::new();
        for (seed, _) in snapshot.nearest_facts(&vector, &Filter::default(), READ_LIMIT * 2) {
            let head = snapshot.latest(seed);
            if let Some(fact) = snapshot.fact(head).filter(|fact| fact.invalid_at().is_none())
                && !found.iter().any(|known| known.id() == head)
                && found.len() < READ_LIMIT
            {
                found.push(Fact::clone(fact));
            }
        }
        Ok(self.listed(found))
    }

    fn recent_facts(&mut self, limit: i64) -> String {
        let snapshot = self.engine.index.snapshot();
        let mut recent: Vec<Fact> = snapshot
            .facts()
            .filter(|fact| fact.invalid_at().is_none())
            .map(|fact| Fact::clone(fact))
            .collect();
        recent.sort_by_key(|fact| std::cmp::Reverse(fact.recorded_at()));
        recent.truncate(usize::try_from(limit.clamp(1, 20)).unwrap_or(10));
        self.listed(recent)
    }

    fn search_intake(&mut self, query: &str) -> String {
        let search = RawSearch { text: query, from: None, to: None, source: None, limit: READ_LIMIT };
        let Ok(found) = self.engine.state.raw_search(&search) else {
            return "the intake log is unavailable".to_owned();
        };
        let mut text = String::new();
        for record in found {
            let number = self.registry.record_number(record.logged.id);
            let corrected = if record.corrected { " (logged incorrectly)" } else { "" };
            let content: String = record.logged.record.content.chars().take(300).collect();
            let _ = writeln!(text, "[record {number}] {}{corrected}: {content}", record.logged.record.source);
        }
        if text.is_empty() { "nothing found".to_owned() } else { text }
    }

    fn follow_links(&mut self, fact: Option<i64>) -> Outcome {
        let fact = self.fact(fact, "fact")?;
        let snapshot = self.engine.index.snapshot();
        let mut text = String::new();
        for neighbor in snapshot.neighbors(MemoryId::Fact(fact.id())) {
            if let MemoryId::Fact(other) = neighbor.id
                && let Some(other) = snapshot.fact(other)
            {
                let line = self.describe_fact(other);
                let _ = writeln!(text, "{} {:?}: {line}", neighbor.kind, neighbor.direction);
            }
        }
        Ok(if text.is_empty() { "no linked facts".to_owned() } else { text })
    }

    fn link(&mut self, arguments: &Value) -> Outcome {
        let kind = match text(arguments, "kind") {
            "updates" => FactLinkKind::Updates,
            "extends" => FactLinkKind::Extends,
            "contradicts" => FactLinkKind::Contradicts,
            "part_of" => FactLinkKind::PartOf,
            other => return Err(format!("`{other}` is not a link kind")),
        };
        let mut from = self.fact(number(arguments, "from"), "from")?;
        let mut to = self.fact(number(arguments, "to"), "to")?;
        if from.id() == to.id() {
            return Err("a fact cannot link to itself".to_owned());
        }
        let now = self.engine.clock.now();
        from.link(FactLink { kind, to: to.id() }, now).map_err(|error| error.to_string())?;
        let mut writes = vec![Memory::Fact(from.clone())];
        if kind == FactLinkKind::Updates && to.invalidate(from.valid_at(), now) {
            writes.push(Memory::Fact(to));
        }
        self.write(writes)?;
        Ok("linked".to_owned())
    }

    fn invalidate(&mut self, fact: Option<i64>, replaced_by: Option<i64>) -> Outcome {
        let mut fact = self.fact(fact, "fact")?;
        let mut newer = self.fact(replaced_by, "replaced_by")?;
        if fact.id() == newer.id() {
            return Err("a fact cannot replace itself".to_owned());
        }
        let now = self.engine.clock.now();
        newer
            .link(FactLink { kind: FactLinkKind::Updates, to: fact.id() }, now)
            .map_err(|error| error.to_string())?;
        fact.invalidate(newer.valid_at(), now);
        self.write(vec![Memory::Fact(newer), Memory::Fact(fact)])?;
        Ok("invalidated".to_owned())
    }

    fn merge_facts(&mut self, keep: Option<i64>, duplicate: Option<i64>) -> Outcome {
        let mut keep = self.fact(keep, "keep")?;
        let mut duplicate = self.fact(duplicate, "duplicate")?;
        if keep.id() == duplicate.id() {
            return Err("a fact cannot merge into itself".to_owned());
        }
        let now = self.engine.clock.now();
        for source in duplicate.sources().to_vec() {
            keep.attach_source(source, now);
        }
        keep.link(FactLink { kind: FactLinkKind::Updates, to: duplicate.id() }, now)
            .map_err(|error| error.to_string())?;
        duplicate.invalidate(now, now);
        self.write(vec![Memory::Fact(keep), Memory::Fact(duplicate)])?;
        Ok("merged".to_owned())
    }

    fn search_entities(&mut self, query: &str) -> String {
        let snapshot = self.engine.index.snapshot();
        let mut found: Vec<EntityId> = snapshot.resolve(query);
        if let Some(vector) = self.engine.embed_one_as(&snapshot, query, Purpose::Query) {
            found.extend(snapshot.nearest_entities(&vector, READ_LIMIT).into_iter().map(|(id, _)| id));
        }
        let mut seen = HashSet::new();
        found.retain(|id| seen.insert(*id));
        let mut text = String::new();
        for id in found.into_iter().take(READ_LIMIT) {
            if let Some(entity) = snapshot.entity(id) {
                let line = self.describe_entity(entity);
                let _ = writeln!(text, "{line}");
            }
        }
        if text.is_empty() { "nothing found".to_owned() } else { text }
    }

    fn entity_facts(&mut self, entity: Option<i64>) -> Outcome {
        let entity = self.entity(entity, "entity")?;
        let snapshot = self.engine.index.snapshot();
        let facts: Vec<Fact> = snapshot
            .facts_about(entity.id())
            .iter()
            .filter_map(|id| snapshot.fact(*id))
            .filter(|fact| fact.invalid_at().is_none())
            .map(|fact| Fact::clone(fact))
            .collect();
        Ok(self.listed(facts))
    }

    fn create_entity(&mut self, arguments: &Value) -> Outcome {
        let name = text(arguments, "name").trim().to_owned();
        let cited: Vec<i64> = arguments["facts"]
            .as_array()
            .map(|items| items.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();
        let facts: Vec<Fact> =
            cited.iter().filter_map(|number| self.fact(Some(*number), "fact").ok()).collect();
        if facts.is_empty() {
            return Err("an entity must cite at least one fact number you were shown".to_owned());
        }
        let snapshot = self.engine.index.snapshot();
        if let Some(existing) = snapshot.resolve(&name).first().and_then(|id| snapshot.entity(*id)) {
            let line = self.describe_entity(existing);
            return Err(format!("it already exists as {line}"));
        }
        let now = self.engine.clock.now();
        let aliases: Vec<String> = arguments["aliases"]
            .as_array()
            .map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        let entity = Entity::new(NewEntity {
            id: self.engine.ids.entity(now),
            kind: Label::new(text(arguments, "kind")).unwrap_or_else(|_| Label::thing()),
            name,
            aliases,
            description: String::new(),
            recorded_at: now,
        })
        .map_err(|error| error.to_string())?;
        let id = entity.id();
        let mut writes = vec![Memory::Entity(entity)];
        for mut fact in facts {
            fact.link_entity(id, now);
            writes.push(Memory::Fact(fact));
        }
        self.write(writes)?;
        let number = self.registry.entity_number(id);
        Ok(format!("created entity {number}"))
    }

    fn merge_entities(&mut self, keep: Option<i64>, duplicate: Option<i64>) -> Outcome {
        let mut keep = self.entity(keep, "keep")?;
        let mut duplicate = self.entity(duplicate, "duplicate")?;
        if keep.id() == duplicate.id() {
            return Err("an entity cannot merge into itself".to_owned());
        }
        let now = self.engine.clock.now();
        keep.absorb(&duplicate, now).map_err(|error| error.to_string())?;
        duplicate.retire_into(keep.id(), now).map_err(|error| error.to_string())?;
        self.write(vec![Memory::Entity(keep), Memory::Entity(duplicate)])?;
        Ok("merged".to_owned())
    }

    fn add_alias(&mut self, entity: Option<i64>, alias: &str) -> Outcome {
        let mut entity = self.entity(entity, "entity")?;
        if !entity.add_alias(alias, self.engine.clock.now()) {
            return Err(format!("`{alias}` is empty or already a name of that entity"));
        }
        self.write(vec![Memory::Entity(entity)])?;
        Ok("alias added".to_owned())
    }

    fn link_entity(&mut self, from: Option<i64>, label: &str, to: Option<i64>) -> Outcome {
        let mut from = self.entity(from, "from")?;
        let to = self.entity(to, "to")?;
        let now = self.engine.clock.now();
        let changed = match label {
            "part_of" => from.link(EntityLink { kind: EntityLinkKind::PartOf, to: to.id() }, now),
            "possibly_same" => from.link(EntityLink { kind: EntityLinkKind::PossiblySame, to: to.id() }, now),
            other => {
                let label = Label::new(other).map_err(|error| error.to_string())?;
                from.relate(Relation { label, to: to.id(), valid_at: now, invalid_at: None }, now)
            }
        }
        .map_err(|error| error.to_string())?;
        if changed {
            self.write(vec![Memory::Entity(from)])?;
        }
        Ok("linked".to_owned())
    }
}

fn text<'value>(arguments: &'value Value, name: &str) -> &'value str {
    arguments[name].as_str().unwrap_or_default()
}

fn number(arguments: &Value, name: &str) -> Option<i64> {
    arguments[name].as_i64()
}
