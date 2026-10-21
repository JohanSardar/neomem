use std::fmt::Write as _;
use std::sync::Arc;

use neomem_core::{Choice, Fact, FactId, FactLinkKind, Filter, JudgeRole, Purpose, Vector};
use neomem_index::Snapshot;

use crate::Engine;

const CLOSEST: usize = 5;
const CANDIDATES: usize = 12;
const UNCLEAR_BELOW: f32 = 0.6;
const QUESTION: &str = "How does the new fact relate to the existing facts?";
const RELATIONS: [(&str, FactLinkKind, &str); 3] = [
    ("extends", FactLinkKind::Extends, "adds detail to existing fact {n}; both stay true"),
    ("updates", FactLinkKind::Updates, "replaces existing fact {n}, which is no longer true"),
    (
        "contradicts",
        FactLinkKind::Contradicts,
        "conflicts with existing fact {n} and it is unclear which is true",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settlement {
    Same(FactId),
    Linked(FactLinkKind, FactId),
    New,
    Unclear,
}

impl Engine {
    pub(crate) fn settle(&self, snapshot: &Snapshot, content: &str, vector: &Vector) -> Settlement {
        let closest = closest_current_facts(snapshot, vector);
        if closest.is_empty() {
            return Settlement::New;
        }
        let Ok(checker) = self.models.judge(JudgeRole::Checker) else { return Settlement::Unclear };
        let labels = labels_for(closest.len());
        let options: Vec<Choice<'_>> = labels
            .iter()
            .map(|(label, meaning, _)| Choice { label: label.as_str(), meaning: meaning.as_str() })
            .collect();
        let situation = situation_for(content, &closest);
        let Ok(decision) = checker.decide(&situation, QUESTION, &options) else { return Settlement::Unclear };
        if decision.confidence() < UNCLEAR_BELOW {
            return Settlement::Unclear;
        }
        labels.get(decision.choice()).map_or(Settlement::Unclear, |(_, _, settlement)| match settlement {
            Pick::Same(index) => Settlement::Same(closest[*index].id()),
            Pick::Link(kind, index) => Settlement::Linked(*kind, closest[*index].id()),
            Pick::New => Settlement::New,
        })
    }

    pub(crate) fn embed_one(&self, snapshot: &Snapshot, text: &str) -> Option<Vector> {
        self.models
            .embedder_for(snapshot.model()?)
            .ok()?
            .embed(&[text], Purpose::Document)
            .ok()?
            .into_iter()
            .next()
    }
}

#[derive(Debug, Clone, Copy)]
enum Pick {
    Same(usize),
    Link(FactLinkKind, usize),
    New,
}

fn closest_current_facts(snapshot: &Snapshot, vector: &Vector) -> Vec<Arc<Fact>> {
    let everything = Filter { include_hidden_tags: true, ..Filter::default() };
    let mut closest: Vec<Arc<Fact>> = Vec::new();
    for (seed, _) in snapshot.nearest_facts(vector, &everything, CANDIDATES) {
        let head = snapshot.latest(seed);
        if let Some(fact) = snapshot.fact(head).filter(|fact| fact.invalid_at().is_none())
            && !closest.iter().any(|known| known.id() == head)
        {
            closest.push(Arc::clone(fact));
        }
        if closest.len() == CLOSEST {
            break;
        }
    }
    closest
}

fn labels_for(count: usize) -> Vec<(String, String, Pick)> {
    let mut labels = Vec::new();
    for index in 0..count {
        let number = index + 1;
        labels.push((
            format!("same_{number}"),
            format!("says the same thing as existing fact {number}"),
            Pick::Same(index),
        ));
        for (name, kind, meaning) in RELATIONS {
            labels.push((
                format!("{name}_{number}"),
                meaning.replace("{n}", &number.to_string()),
                Pick::Link(kind, index),
            ));
        }
    }
    labels.push((
        "new".to_owned(),
        "is about something none of the existing facts cover".to_owned(),
        Pick::New,
    ));
    labels
}

fn situation_for(content: &str, closest: &[Arc<Fact>]) -> String {
    let mut text = format!("New fact: {content}\n\nExisting facts:\n");
    for (index, fact) in closest.iter().enumerate() {
        let _ = writeln!(text, "{}. {}", index + 1, fact.content());
    }
    text
}
