use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::token::LlamaToken;
use neomem_core::{Choice, Decision, Judge, ModelError, ModelId};

use crate::llama::{LocalModel, context_threads, failure, local_model_id};

const CONTEXT_TOKENS: u32 = 2048;
const MOST_SITUATION_TOKENS: usize = 768;
const MOST_CANDIDATE_TOKENS: usize = 160;
const MOST_CANDIDATES_PER_BATCH: usize = 32;
const INSTRUCTION_OPENING: &str = "<|im_start|>system\nJudge whether the Document meets the requirements based on the Query and the Instruct provided. Note that the answer can only be \"yes\" or \"no\".<|im_end|>\n<|im_start|>user\n<Instruct>: ";
const ANSWER_OPENING: &str = "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n";

#[derive(Debug, Clone, Copy)]
struct Verdicts {
    yes: LlamaToken,
    no: LlamaToken,
}

#[derive(Debug)]
pub struct LlamaReranker {
    model: LocalModel,
    verdicts: Verdicts,
}

impl LlamaReranker {
    pub fn open(path: &Path) -> Result<Self, ModelError> {
        let (model, verdicts) = LocalModel::load(path, |model| {
            let verdicts = Verdicts { yes: single_token(model, "yes")?, no: single_token(model, "no")? };
            let threads = context_threads();
            let params = LlamaContextParams::default()
                .with_n_ctx(NonZeroU32::new(CONTEXT_TOKENS))
                .with_n_batch(CONTEXT_TOKENS)
                .with_n_ubatch(512)
                .with_n_seq_max(u32::try_from(MOST_CANDIDATES_PER_BATCH).unwrap_or(1))
                .with_kv_unified(true)
                .with_n_threads(threads)
                .with_n_threads_batch(threads);
            Ok((params, verdicts))
        })?;
        Ok(Self { model, verdicts })
    }

    fn yes_probabilities(
        &self,
        situation: &str,
        question: &str,
        documents: Vec<String>,
    ) -> Result<Vec<f32>, ModelError> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }
        let prompt = Prompt { question: question.to_owned(), situation: situation.to_owned(), documents };
        let verdicts = self.verdicts;
        self.model.run(move |model, context| prompt.judge(model, context, verdicts))
    }
}

struct Prompt {
    question: String,
    situation: String,
    documents: Vec<String>,
}

impl Prompt {
    fn judge(
        &self,
        model: &LlamaModel,
        context: &mut LlamaContext<'_>,
        verdicts: Verdicts,
    ) -> Result<Vec<f32>, ModelError> {
        let vocab = model.vocab();
        let mut shared = vocab.tokenize(
            format!("{INSTRUCTION_OPENING}{}\n<Query>:", self.question).as_bytes(),
            false,
            true,
        );
        let situation = vocab.tokenize(format!(" {}", self.situation).as_bytes(), false, false);
        shared.extend_from_slice(&situation[situation.len().saturating_sub(MOST_SITUATION_TOKENS)..]);
        shared.extend(vocab.tokenize(b"\n<Document>:", false, false));
        let closing = vocab.tokenize(ANSWER_OPENING.as_bytes(), false, true);
        let endings: Vec<Vec<LlamaToken>> = self
            .documents
            .iter()
            .map(|document| {
                let mut ending = vocab.tokenize(format!(" {document}").as_bytes(), false, false);
                ending.truncate(MOST_CANDIDATE_TOKENS);
                ending.extend_from_slice(&closing);
                ending
            })
            .collect();
        let budget = usize::try_from(CONTEXT_TOKENS).unwrap_or(2048).saturating_sub(shared.len());
        let mut probabilities = Vec::with_capacity(endings.len());
        for group in grouped(&endings, budget) {
            probabilities.extend(judge_group(context, &shared, group, verdicts)?);
        }
        Ok(probabilities)
    }
}

fn judge_group(
    context: &mut LlamaContext<'_>,
    shared: &[LlamaToken],
    endings: &[Vec<LlamaToken>],
    verdicts: Verdicts,
) -> Result<Vec<f32>, ModelError> {
    context.clear_kv_cache();
    let sequences: Vec<i32> = (0..).take(endings.len()).collect();
    let total = shared.len() + endings.iter().map(Vec::len).sum::<usize>();
    let mut batch = LlamaBatch::new(total, i32::try_from(endings.len()).unwrap_or(1));
    for (position, token) in (0..).zip(shared) {
        batch.add(*token, position, &sequences, false).map_err(|error| failure("batch", error))?;
    }
    let start = i32::try_from(shared.len()).map_err(|error| failure("batch", error))?;
    let mut answer_at = Vec::with_capacity(endings.len());
    for (sequence, ending) in sequences.iter().zip(endings) {
        for (offset, token) in (0..).zip(ending) {
            let last = usize::try_from(offset).is_ok_and(|offset| offset + 1 == ending.len());
            if last {
                answer_at.push(batch.n_tokens());
            }
            batch.add(*token, start + offset, &[*sequence], last).map_err(|error| failure("batch", error))?;
        }
    }
    context.decode(&mut batch).map_err(|error| failure("decode", error))?;
    Ok(answer_at
        .into_iter()
        .map(|index| {
            let logits = context.get_logits_ith(index);
            let logit =
                |token: LlamaToken| usize::try_from(token.0).ok().and_then(|at| logits.get(at)).copied();
            let (yes, no) = (logit(verdicts.yes).unwrap_or(0.0), logit(verdicts.no).unwrap_or(0.0));
            1.0 / (1.0 + (no - yes).exp())
        })
        .collect())
}

fn grouped(endings: &[Vec<LlamaToken>], budget: usize) -> Vec<&[Vec<LlamaToken>]> {
    let mut groups = Vec::new();
    let (mut start, mut tokens) = (0, 0);
    for (index, ending) in endings.iter().enumerate() {
        let full = index - start == MOST_CANDIDATES_PER_BATCH || tokens + ending.len() > budget;
        if full && index > start {
            groups.push(&endings[start..index]);
            (start, tokens) = (index, 0);
        }
        tokens += ending.len();
    }
    groups.push(&endings[start..]);
    groups
}

fn single_token(model: &LlamaModel, word: &str) -> Result<LlamaToken, ModelError> {
    match model.vocab().tokenize(word.as_bytes(), false, false).as_slice() {
        [token] => Ok(*token),
        _ => Err(ModelError::InvalidOutput(format!("the reranker has no single token for {word:?}"))),
    }
}

impl Judge for LlamaReranker {
    fn decide(&self, context: &str, question: &str, options: &[Choice<'_>]) -> Result<Decision, ModelError> {
        let meanings = options.iter().map(|option| option.meaning.to_owned()).collect();
        let fits = self.yes_probabilities(context, question, meanings)?;
        let total: f32 = fits.iter().sum();
        let probabilities = if total > f32::EPSILON {
            fits.iter().map(|fit| fit / total).collect()
        } else {
            let even = 1.0 / f32::from(u16::try_from(options.len().max(1)).unwrap_or(u16::MAX));
            vec![even; options.len()]
        };
        Ok(Decision { probabilities })
    }

    fn score(&self, context: &str, question: &str, candidates: &[&str]) -> Result<Vec<f32>, ModelError> {
        self.yes_probabilities(context, question, candidates.iter().map(ToString::to_string).collect())
    }

    fn model_id(&self) -> ModelId {
        self.model.id()
    }
}

#[derive(Debug)]
pub struct LazyReranker {
    path: PathBuf,
    loaded: OnceLock<Result<LlamaReranker, ModelError>>,
}

impl LazyReranker {
    pub fn at(path: PathBuf) -> Self {
        Self { path, loaded: OnceLock::new() }
    }

    fn reranker(&self) -> Result<&LlamaReranker, ModelError> {
        self.loaded.get_or_init(|| LlamaReranker::open(&self.path)).as_ref().map_err(Clone::clone)
    }
}

impl Judge for LazyReranker {
    fn decide(&self, context: &str, question: &str, options: &[Choice<'_>]) -> Result<Decision, ModelError> {
        self.reranker()?.decide(context, question, options)
    }

    fn score(&self, context: &str, question: &str, candidates: &[&str]) -> Result<Vec<f32>, ModelError> {
        self.reranker()?.score(context, question, candidates)
    }

    fn model_id(&self) -> ModelId {
        local_model_id(&self.path)
    }
}
