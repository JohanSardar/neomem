#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod contract;
mod embedder;
pub mod fake_process;
mod judge;
mod llm;
mod server;
mod transport;
mod wire_anthropic;
mod wire_chat;
mod wire_responses;

pub use contract::{embedder_contract, judge_contract, llm_contract};
pub use embedder::HashEmbedder;
pub use fake_process::fake_llm_binary;
pub use judge::{JudgeAnswer, JudgeRule, ScriptedJudge};
pub use llm::{LlmRule, ScriptedAnswer, ScriptedLlm};
pub use server::{FakeModelServer, FakeRules, RecordedCall};
