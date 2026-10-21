use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use neomem_core::{Fact, JudgeRole, LlmMessage, LlmRequest, LlmRole, redact_secrets};
use neomem_proto::{RecallMemory, RecallQuery, Response, SearchQuery};
use neomem_state::{RecallLog, RecallScore};

use crate::Engine;

#[derive(Debug, Clone)]
pub struct RecallConfig {
    pub planner_timeout: Duration,
    pub hard_timeout: Duration,
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self { planner_timeout: Duration::from_millis(2500), hard_timeout: Duration::from_millis(3500) }
    }
}

struct Prepared {
    memories: Vec<RecallMemory>,
    log: RecallLog,
}

struct RecallSearch {
    queries: HashSet<String>,
    seen: HashSet<neomem_core::FactId>,
    candidates: Vec<Fact>,
    started: Instant,
    deadline: Instant,
}

enum Progress {
    Log(RecallLog),
    Complete(Prepared),
}

struct ActiveRecall(Arc<Engine>);

impl Drop for ActiveRecall {
    fn drop(&mut self) {
        self.0.recall_busy.store(false, Ordering::Release);
    }
}

enum Planned {
    Line(String),
    Finished(bool),
}

impl Engine {
    pub(crate) fn recall(self: &Arc<Self>, mut query: RecallQuery) -> Response {
        if self.recall_busy.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            return Response::Recalled(Vec::new());
        }
        let active = ActiveRecall(Arc::clone(self));
        self.models.record_read();
        query.slice = redact_secrets(&query.slice.chars().take(16_000).collect::<String>());
        query.message = redact_secrets(&query.message.chars().take(16_000).collect::<String>());
        query.error =
            query.error.map(|error| redact_secrets(&error.chars().take(16_000).collect::<String>()));
        let started = Instant::now();
        let budget = self.recall_config.hard_timeout;
        let deadline = started + budget.saturating_sub(Duration::from_millis(10).min(budget / 10));
        let engine = Arc::clone(self);
        let input = query.clone();
        let (send, receive) = mpsc::channel();
        let spawned = thread::Builder::new().name("neomem-recall".to_owned()).spawn(move || {
            let active_recall = active;
            let prepared = engine.prepare_recall(&input, deadline, &send);
            drop(active_recall);
            let _ = send.send(Progress::Complete(prepared));
        });
        let mut prepared = Prepared { memories: Vec::new(), log: RecallLog::default() };
        if spawned.is_ok() {
            loop {
                match receive.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(Progress::Log(log)) => prepared.log = log,
                    Ok(Progress::Complete(result)) => {
                        prepared = result;
                        break;
                    }
                    Err(_) => {
                        prepared
                            .log
                            .timings
                            .push(("timeout_ms".to_owned(), started.elapsed().as_secs_f64() * 1000.0));
                        break;
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            prepared.memories.clear();
            prepared.log.injected.clear();
        }
        prepared.log.timings.push(("total_ms".to_owned(), started.elapsed().as_secs_f64() * 1000.0));
        if self
            .state
            .finish_recall(
                self.clock.now(),
                &query.agent,
                &query.session,
                query.turn.as_deref(),
                &mut prepared.log,
            )
            .is_err()
        {
            return Response::Recalled(Vec::new());
        }
        prepared.memories.retain(|memory| prepared.log.injected.contains(&memory.id));
        Response::Recalled(prepared.memories)
    }

    fn prepare_recall(
        &self,
        query: &RecallQuery,
        deadline: Instant,
        progress: &mpsc::Sender<Progress>,
    ) -> Prepared {
        let mut prepared = Prepared { memories: Vec::new(), log: RecallLog::default() };
        let context = format!(
            "Session:\n{}\nNew message:\n{}\nNew error:\n{}",
            query.slice,
            query.message,
            query.error.as_deref().unwrap_or("")
        );
        let Ok(judge) = self.models.judge(JudgeRole::Judge) else { return prepared };
        prepared.log.models.push(judge.model_id().to_string());
        let _ = progress.send(Progress::Log(prepared.log.clone()));
        let gate_started = Instant::now();
        let current =
            query.error.as_deref().filter(|error| !error.trim().is_empty()).unwrap_or(&query.message);
        let Ok(scores) = judge.score_with_model(
            &context,
            "Would long-term memory help with the new message or error?",
            &[current],
        ) else {
            return prepared;
        };
        prepared.log.models.push(scores.model.to_string());
        let gate = scores.probabilities.first().copied().filter(|score| (0.0..=1.0).contains(score));
        prepared.log.gate = gate;
        prepared.log.timings.push(("gate_ms".to_owned(), gate_started.elapsed().as_secs_f64() * 1000.0));
        let _ = progress.send(Progress::Log(prepared.log.clone()));
        if !gate.is_some_and(|score| score >= 0.5) || Instant::now() >= deadline {
            return prepared;
        }
        let candidates = self.recall_candidates(query, &context, deadline, &mut prepared.log, progress);
        if candidates.is_empty() || Instant::now() >= deadline {
            return prepared;
        }
        prepared.log.candidates =
            candidates.iter().map(|fact| RecallScore { id: fact.id(), probability: None }).collect();
        let _ = progress.send(Progress::Log(prepared.log.clone()));
        let texts: Vec<&str> = candidates.iter().map(Fact::content).collect();
        let approval_started = Instant::now();
        let Ok(scores) = judge.score_with_model(
            &context,
            "Would this memory help with what is happening right now?",
            &texts,
        ) else {
            return prepared;
        };
        prepared
            .log
            .timings
            .push(("approval_ms".to_owned(), approval_started.elapsed().as_secs_f64() * 1000.0));
        prepared.log.models.push(scores.model.to_string());
        let scores = scores.probabilities;
        if scores.len() != candidates.len() || Instant::now() >= deadline {
            return prepared;
        }
        for (candidate, probability) in prepared.log.candidates.iter_mut().zip(&scores) {
            candidate.probability = (0.0..=1.0).contains(probability).then_some(*probability);
        }
        let _ = progress.send(Progress::Log(prepared.log.clone()));
        let mut approved: Vec<_> =
            candidates.into_iter().zip(scores).filter(|(_, score)| (0.7..=1.0).contains(score)).collect();
        approved.sort_by(|left, right| right.1.total_cmp(&left.1));
        let snapshot = self.index.snapshot();
        for (fact, _) in approved.into_iter().take(5) {
            if snapshot.latest(fact.id()) != fact.id()
                || snapshot.fact(fact.id()).is_none_or(|current| current.invalid_at().is_some())
            {
                continue;
            }
            prepared.log.injected.push(fact.id());
            prepared.memories.push(RecallMemory { id: fact.id(), content: fact.content().to_owned() });
        }
        prepared
    }

    fn recall_candidates(
        &self,
        query: &RecallQuery,
        context: &str,
        deadline: Instant,
        log: &mut RecallLog,
        progress: &mpsc::Sender<Progress>,
    ) -> Vec<Fact> {
        let plan_started = Instant::now();
        let plan_deadline = (plan_started + self.recall_config.planner_timeout).min(deadline);
        let receive = self.start_recall_planner(context, log);
        let mut search = RecallSearch {
            queries: HashSet::new(),
            seen: HashSet::new(),
            candidates: Vec::new(),
            started: plan_started,
            deadline,
        };
        let mut completed = false;
        "planner".clone_into(&mut log.source);
        while let Some(receive) = receive.as_ref().filter(|_| Instant::now() < plan_deadline) {
            match receive.recv_timeout(plan_deadline.saturating_duration_since(Instant::now())) {
                Ok(Planned::Line(line)) if search.queries.len() < 5 => {
                    self.search_recall_line(query, &line, &mut search, log);
                    let _ = progress.send(Progress::Log(log.clone()));
                }
                Ok(Planned::Finished(success)) => {
                    completed = success;
                    break;
                }
                Ok(Planned::Line(_)) => {}
                Err(_) => break,
            }
        }
        if !completed || search.queries.len() < 3 {
            if search.queries.is_empty() { "fallback" } else { "planner+fallback" }
                .clone_into(&mut log.source);
            for line in fallback_queries(query) {
                self.search_recall_line(query, &line, &mut search, log);
                let _ = progress.send(Progress::Log(log.clone()));
            }
        }
        log.timings.push(("plan_ms".to_owned(), plan_started.elapsed().as_secs_f64() * 1000.0));
        let _ = progress.send(Progress::Log(log.clone()));
        search.candidates.truncate(15);
        search.candidates
    }

    fn start_recall_planner(&self, context: &str, log: &mut RecallLog) -> Option<mpsc::Receiver<Planned>> {
        let (send, receive) = mpsc::sync_channel(8);
        let llm = self.models.llm(LlmRole::Planner, None);

        if let Ok(llm) = llm {
            log.models.push(llm.model_id().to_string());
            let request = LlmRequest {
                system: "Rewrite the session into 3 to 5 self-contained fact-shaped memory search queries. Output one query per line, no numbering or prose.".to_owned(),
                messages: vec![LlmMessage::User(context.to_owned())], response_schema: None, tools: Vec::new(), max_output_tokens: 256,
            };
            let spawned = thread::Builder::new()
                .name("neomem-recall-planner".to_owned())
                .spawn(move || {
                    let mut buffer = String::new();
                    let mut received = 0;
                    let mut emitted = 0;
                    let reply = llm.complete(&request, &mut |delta| {
                        let chunk: String =
                            delta.chars().take(16_000_usize.saturating_sub(received)).collect();
                        received += chunk.chars().count();
                        buffer.push_str(&chunk);
                        while let Some(end) = buffer.find('\n') {
                            let line: String = buffer.drain(..=end).collect();
                            if emitted < 5 {
                                let _ = send.try_send(Planned::Line(line));
                                emitted += 1;
                            }
                        }
                    });
                    if reply.is_ok() && emitted < 5 && !buffer.trim().is_empty() {
                        let _ = send.try_send(Planned::Line(buffer));
                    }
                    let _ = send.try_send(Planned::Finished(reply.is_ok()));
                })
                .is_ok();
            return spawned.then_some(receive);
        }
        None
    }

    fn search_recall_line(
        &self,
        query: &RecallQuery,
        line: &str,
        search: &mut RecallSearch,
        log: &mut RecallLog,
    ) {
        let text = line.trim().trim_start_matches(['-', '*']).trim();
        if text.is_empty()
            || search.queries.len() >= 8
            || Instant::now() >= search.deadline
            || !search.queries.insert(text.to_owned())
        {
            return;
        }
        log.queries = search.queries.len();
        if log.queries == 1 {
            log.timings.push(("first_search_ms".to_owned(), search.started.elapsed().as_secs_f64() * 1000.0));
        }
        let started = Instant::now();
        if let Response::SearchResults(results) =
            self.search(&SearchQuery { text: text.to_owned(), k: Some(10), ..SearchQuery::default() })
        {
            for fact in results
                .hits
                .into_iter()
                .flat_map(|hit| std::iter::once(hit.fact).chain(hit.linked.into_iter().map(|link| link.fact)))
            {
                if search.seen.insert(fact.id())
                    && !self.state.was_injected(&query.agent, &query.session, fact.id()).unwrap_or(true)
                {
                    search.candidates.push(fact);
                }
            }
        }
        log.timings.push(("search_ms".to_owned(), started.elapsed().as_secs_f64() * 1000.0));
    }
}

fn fallback_queries(query: &RecallQuery) -> Vec<String> {
    let mut queries = vec![query.message.clone()];
    if let Some(error) =
        query.error.as_deref().and_then(|error| error.lines().rev().find(|line| !line.trim().is_empty()))
    {
        queries.push(error.to_owned());
    } else if let Some(error) =
        query.slice.lines().rev().find(|line| line.to_ascii_lowercase().contains("error"))
    {
        queries.push(error.to_owned());
    }
    let mut seen = HashSet::new();
    let names: Vec<_> = query
        .message
        .split_whitespace()
        .chain(query.error.as_deref().unwrap_or("").split_whitespace())
        .filter(|word| {
            word.chars().next().is_some_and(char::is_uppercase)
                && !query.slice.contains(word)
                && seen.insert(*word)
        })
        .collect();
    if !names.is_empty() {
        queries.push(names.join(" "));
    }
    queries
}
