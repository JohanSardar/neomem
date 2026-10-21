mod registry;
mod tools;
mod workbench;

use std::collections::HashSet;
use std::fmt::Write as _;
use std::time::Instant;

use neomem_core::{Entity, EpochMs, Fact, LlmMessage, LlmRequest, LlmRole, ModelError, Purpose};
use neomem_proto::{DreamReport, FailureKind, Response, SpecialistReport};
use neomem_state::RunRecord;

use self::registry::Registry;
use self::tools::{ENTITY_KEEPER_TOOLS, RECONCILER_TOOLS, Tool};
use self::workbench::{Tally, Workbench};
use crate::Engine;
use crate::reply::failed;

const RECONCILER: &str = include_str!("../../prompts/reconciler.md");
const ENTITY_KEEPER: &str = include_str!("../../prompts/entity_keeper.md");
const RECONCILER_ROUNDS: usize = 12;
const ENTITY_KEEPER_ROUNDS: usize = 8;
const MOST_FOCUS: usize = 30;
const CLOSEST_PER_FACT: usize = 3;
const DUPLICATE_NAME_SIMILARITY: f32 = 0.8;
pub(crate) const NEW_FACTS_TRIGGER: usize = 50;

struct Specialist {
    name: &'static str,
    instructions: &'static str,
    tools: &'static [Tool],
    rounds: usize,
}

const FACT_RECONCILER: Specialist = Specialist {
    name: "fact reconciler",
    instructions: RECONCILER,
    tools: RECONCILER_TOOLS,
    rounds: RECONCILER_ROUNDS,
};
const ENTITY_KEEPER_SPECIALIST: Specialist = Specialist {
    name: "entity keeper",
    instructions: ENTITY_KEEPER,
    tools: ENTITY_KEEPER_TOOLS,
    rounds: ENTITY_KEEPER_ROUNDS,
};

impl Engine {
    pub(crate) fn dream_now(&self) -> Response {
        match self.dream("manual") {
            Ok(report) => Response::Dreamed(report),
            Err(error) => failed(FailureKind::Unavailable, format!("consolidation failed: {error}")),
        }
    }

    pub(crate) fn dream(&self, trigger: &str) -> Result<DreamReport, ModelError> {
        let _consolidation = self.consolidation.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let started = Instant::now();
        let started_at = self.clock.now();
        let since =
            self.state.last_run_started("consolidation").ok().flatten().unwrap_or(EpochMs::from_millis(0));
        let model = self.models.llm(LlmRole::Consolidator, None)?.model_id().to_string();
        let mut bench = Workbench { engine: self, registry: Registry::default(), tally: Tally::default() };
        let (opening, focus) = self.reconciler_opening(&mut bench, since);
        let reconciler = self.run_specialist(&FACT_RECONCILER, opening, focus, &mut bench)?;
        let mut keeper_bench =
            Workbench { engine: self, registry: Registry::default(), tally: Tally::default() };
        let (opening, focus) = self.entity_keeper_opening(&mut keeper_bench, since);
        let entity_keeper =
            self.run_specialist(&ENTITY_KEEPER_SPECIALIST, opening, focus, &mut keeper_bench)?;
        if let Err(error) = self.state.finish_rechecks(self.clock.now()) {
            tracing::error!("finishing re-checks failed: {error}");
        }
        let report = DreamReport {
            trigger: trigger.to_owned(),
            reconciler,
            entity_keeper,
            took_ms: i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
        };
        self.log_dream(started_at, &model, &report);
        Ok(report)
    }

    pub(crate) fn consolidate_when_due(&self) {
        let rechecks = self.state.pending_rechecks().is_ok_and(|pending| !pending.is_empty());
        let trigger = if rechecks {
            "late data or a correction"
        } else if self.facts_since_last_dream() >= NEW_FACTS_TRIGGER {
            "50 new facts"
        } else {
            return;
        };
        if let Err(error) = self.dream(trigger) {
            tracing::warn!("consolidation after {trigger} failed: {error}");
        }
    }

    pub(crate) fn facts_since_last_dream(&self) -> usize {
        let since =
            self.state.last_run_started("consolidation").ok().flatten().unwrap_or(EpochMs::from_millis(0));
        self.index.snapshot().facts().filter(|fact| fact.recorded_at() > since).count()
    }

    fn run_specialist(
        &self,
        specialist: &Specialist,
        opening: String,
        focus: usize,
        bench: &mut Workbench<'_>,
    ) -> Result<SpecialistReport, ModelError> {
        if focus == 0 {
            return Ok(SpecialistReport::default());
        }
        let llm = self.models.llm(LlmRole::Consolidator, None)?;
        let tools: Vec<_> = specialist.tools.iter().map(|tool| tool.spec()).collect();
        let mut messages = vec![LlmMessage::User(opening)];
        for _ in 0..specialist.rounds {
            let request = LlmRequest {
                system: specialist.instructions.to_owned(),
                messages: messages.clone(),
                response_schema: None,
                tools: tools.clone(),
                max_output_tokens: 2048,
            };
            let reply = llm.complete(&request, &mut |_| {})?;
            bench.tally.rounds += 1;
            if reply.tool_calls.is_empty() {
                break;
            }
            messages.push(LlmMessage::Assistant {
                text: reply.text.clone(),
                tool_calls: reply.tool_calls.clone(),
            });
            for call in reply.tool_calls {
                let result = if let Some(tool) = Tool::named(&call.name, specialist.tools) {
                    let arguments = serde_json::from_str(&call.arguments).unwrap_or_default();
                    bench.run(tool, &arguments)
                } else {
                    bench.tally.rejected += 1;
                    format!("rejected: the {} has no tool named {}", specialist.name, call.name)
                };
                messages.push(LlmMessage::ToolResult { call_id: call.id, content: result });
            }
        }
        let tally = bench.tally;
        Ok(SpecialistReport { focus, rounds: tally.rounds, writes: tally.writes, rejected: tally.rejected })
    }

    fn reconciler_opening(&self, bench: &mut Workbench<'_>, since: EpochMs) -> (String, usize) {
        let snapshot = self.index.snapshot();
        let rechecked: HashSet<_> = self.state.pending_rechecks().unwrap_or_default().into_iter().collect();
        let mut focus: Vec<Fact> = snapshot
            .facts()
            .filter(|fact| fact.invalid_at().is_none())
            .filter(|fact| {
                fact.recorded_at() > since || fact.sources().iter().any(|source| rechecked.contains(source))
            })
            .map(|fact| Fact::clone(fact))
            .collect();
        focus.sort_by_key(|fact| std::cmp::Reverse(fact.recorded_at()));
        focus.truncate(MOST_FOCUS);
        let mut text = String::from("Facts to reconcile (new since the last run or flagged):\n");
        let mut close: Vec<Fact> = Vec::new();
        for fact in &focus {
            let corrected = fact.sources().iter().any(|source| rechecked.contains(source));
            let flag = if corrected { "  [a source was logged incorrectly]" } else { "" };
            let line = bench.describe_fact(fact);
            let _ = writeln!(text, "{line}{flag}");
            if let Some(vector) = self.embed_one_as(&snapshot, fact.content(), Purpose::Query) {
                for (seed, _) in
                    snapshot.nearest_facts(&vector, &neomem_core::Filter::default(), CLOSEST_PER_FACT + 1)
                {
                    let head = snapshot.latest(seed);
                    if head != fact.id()
                        && !focus.iter().any(|known| known.id() == head)
                        && !close.iter().any(|known| known.id() == head)
                        && let Some(other) = snapshot.fact(head)
                    {
                        close.push(Fact::clone(other));
                    }
                }
            }
        }
        text.push_str("\nClose existing facts:\n");
        for fact in &close {
            let line = bench.describe_fact(fact);
            let _ = writeln!(text, "{line}");
        }
        (text, focus.len())
    }

    fn entity_keeper_opening(&self, bench: &mut Workbench<'_>, since: EpochMs) -> (String, usize) {
        let snapshot = self.index.snapshot();
        let active: Vec<Entity> = snapshot
            .entities()
            .filter(|entity| entity.merged_into().is_none())
            .map(|entity| Entity::clone(entity))
            .collect();
        let mut recent: Vec<&Entity> = active.iter().filter(|entity| entity.updated_at() > since).collect();
        recent.sort_by_key(|entity| entity.name().to_lowercase());
        let mut text = String::from("Entities that are new or changed since the last run:\n");
        for entity in &recent {
            let line = bench.describe_entity(entity);
            let _ = writeln!(text, "{line}");
        }
        text.push_str("\nPairs that may be the same entity:\n");
        let mut pairs = 0;
        for entity in &recent {
            let Some(vector) = self.embed_one_as(&snapshot, entity.name(), Purpose::Query) else { continue };
            for (other, similarity) in snapshot.nearest_entities(&vector, 3) {
                if other != entity.id()
                    && similarity >= DUPLICATE_NAME_SIMILARITY
                    && let Some(other) = snapshot.entity(other)
                {
                    let left = bench.describe_entity(entity);
                    let right = bench.describe_entity(other);
                    let _ = writeln!(text, "{left}  ~  {right}  (similarity {similarity:.2})");
                    pairs += 1;
                }
            }
        }
        (text, recent.len() + pairs)
    }

    fn log_dream(&self, started_at: EpochMs, model: &str, report: &DreamReport) {
        let run = RunRecord {
            kind: "consolidation",
            started_at,
            finished_at: self.clock.now(),
            model: Some(model),
            changes: serde_json::json!({
                "trigger": report.trigger,
                "reconciler": { "focus": report.reconciler.focus, "rounds": report.reconciler.rounds, "writes": report.reconciler.writes, "rejected": report.reconciler.rejected },
                "entity_keeper": { "focus": report.entity_keeper.focus, "rounds": report.entity_keeper.rounds, "writes": report.entity_keeper.writes, "rejected": report.entity_keeper.rejected },
            }),
        };
        if let Err(error) = self.state.log_run(&run) {
            tracing::error!("recording the consolidation run failed: {error}");
        }
    }
}
