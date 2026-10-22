use std::num::NonZeroU32;
use std::path::Path;

use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::{LlamaContextParams, LlamaPoolingType};
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::token::LlamaToken;
use neomem_core::{Embedder, ModelError, ModelId, Purpose, Vector};

use crate::f16_copy::fastest_weights;
use crate::llama::{LocalModel, context_threads, failure};

const BATCH_TOKENS: u32 = 2048;
const MOST_TOKENS_PER_TEXT: usize = 512;
const MOST_TEXTS_PER_BATCH: usize = 32;
const FILLER_WORD: &str = "warm ";
const QWEN3_QUERY_INSTRUCTION: &str =
    "Instruct: Given a web search query, retrieve relevant passages that answer the query\nQuery:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbeddingFamily {
    EmbeddingGemma,
    Qwen3Embedding,
    Plain,
}

impl EmbeddingFamily {
    fn of(architecture: &str) -> Self {
        match architecture {
            "gemma-embedding" => Self::EmbeddingGemma,
            "qwen3" => Self::Qwen3Embedding,
            _ => Self::Plain,
        }
    }

    fn pooling(self) -> LlamaPoolingType {
        match self {
            Self::EmbeddingGemma => LlamaPoolingType::Mean,
            Self::Qwen3Embedding => LlamaPoolingType::Last,
            Self::Plain => LlamaPoolingType::Unspecified,
        }
    }

    fn prompt(self, text: &str, purpose: Purpose) -> String {
        match (self, purpose) {
            (Self::EmbeddingGemma, Purpose::Query) => format!("task: search result | query: {text}"),
            (Self::EmbeddingGemma, Purpose::Document) => format!("title: none | text: {text}"),
            (Self::Qwen3Embedding, Purpose::Query) => format!("{QWEN3_QUERY_INSTRUCTION}{text}"),
            (Self::Qwen3Embedding | Self::Plain, _) => text.to_owned(),
        }
    }
}

#[derive(Debug)]
pub struct LlamaEmbedder {
    model: LocalModel,
    family: EmbeddingFamily,
    dims: usize,
}

impl LlamaEmbedder {
    pub fn open(path: &Path) -> Result<Self, ModelError> {
        let (model, (family, dims)) = LocalModel::load(&fastest_weights(path), |model| {
            let architecture = model.meta_val_str("general.architecture").unwrap_or_default();
            let family = EmbeddingFamily::of(&architecture);
            let dims = usize::try_from(model.n_embd_out())
                .map_err(|_| ModelError::InvalidOutput("the model reports no embedding size".to_owned()))?;
            let threads = context_threads();
            let params = LlamaContextParams::default()
                .with_embeddings(true)
                .with_pooling_type(family.pooling())
                .with_n_ctx(NonZeroU32::new(BATCH_TOKENS))
                .with_n_batch(BATCH_TOKENS)
                .with_n_ubatch(BATCH_TOKENS)
                .with_n_seq_max(u32::try_from(MOST_TEXTS_PER_BATCH).unwrap_or(1))
                .with_kv_unified(true)
                .with_n_threads(threads)
                .with_n_threads_batch(threads);
            Ok((params, (family, dims)))
        })?;
        let embedder = Self { model, family, dims };
        if cfg!(feature = "vulkan") {
            embedder.grow_gpu_buffers_to_the_longest_query()?;
        }
        Ok(embedder)
    }

    fn grow_gpu_buffers_to_the_longest_query(&self) -> Result<(), ModelError> {
        self.embed(&[&FILLER_WORD.repeat(MOST_TOKENS_PER_TEXT)], Purpose::Query).map(drop)
    }
}

impl Embedder for LlamaEmbedder {
    fn embed(&self, texts: &[&str], purpose: Purpose) -> Result<Vec<Vector>, ModelError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let prompts: Vec<String> = texts.iter().map(|text| self.family.prompt(text, purpose)).collect();
        self.model.run(move |model, context| embed_prompts(model, context, &prompts))
    }

    fn model_id(&self) -> ModelId {
        self.model.id()
    }

    fn dims(&self) -> usize {
        self.dims
    }
}

fn embed_prompts(
    model: &LlamaModel,
    context: &mut LlamaContext<'_>,
    prompts: &[String],
) -> Result<Vec<Vector>, ModelError> {
    let vocab = model.vocab();
    let token_lists: Vec<Vec<LlamaToken>> =
        prompts.iter().map(|prompt| clipped(vocab.tokenize(prompt.as_bytes(), true, false))).collect();
    if token_lists.iter().any(Vec::is_empty) {
        return Err(ModelError::InvalidOutput("a text produced no tokens".to_owned()));
    }
    let mut vectors = Vec::with_capacity(prompts.len());
    for group in batched(&token_lists) {
        context.clear_kv_cache();
        let mut batch = LlamaBatch::new(usize::try_from(BATCH_TOKENS).unwrap_or(2048), 1);
        for (sequence, tokens) in (0..).zip(group) {
            batch.add_sequence(tokens, sequence, true).map_err(|error| failure("batch", error))?;
        }
        context.decode(&mut batch).map_err(|error| failure("decode", error))?;
        for sequence in (0..).take(group.len()) {
            let values =
                context.embeddings_seq_ith(sequence).map_err(|error| failure("embeddings", error))?;
            vectors.push(Vector::normalized(values.to_vec()));
        }
    }
    Ok(vectors)
}

fn clipped(mut tokens: Vec<LlamaToken>) -> Vec<LlamaToken> {
    if tokens.len() > MOST_TOKENS_PER_TEXT
        && let Some(last) = tokens.last().copied()
    {
        tokens.truncate(MOST_TOKENS_PER_TEXT - 1);
        tokens.push(last);
    }
    tokens
}

fn batched(token_lists: &[Vec<LlamaToken>]) -> Vec<&[Vec<LlamaToken>]> {
    let budget = usize::try_from(BATCH_TOKENS).unwrap_or(2048);
    let mut groups = Vec::new();
    let (mut start, mut tokens) = (0, 0);
    for (index, list) in token_lists.iter().enumerate() {
        let full = index - start == MOST_TEXTS_PER_BATCH || tokens + list.len() > budget;
        if full && index > start {
            groups.push(&token_lists[start..index]);
            (start, tokens) = (index, 0);
        }
        tokens += list.len();
    }
    groups.push(&token_lists[start..]);
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrieval_models_get_their_own_query_and_document_prompts() {
        let gemma = EmbeddingFamily::of("gemma-embedding");
        assert_eq!(gemma.prompt("when", Purpose::Query), "task: search result | query: when");
        assert_eq!(gemma.prompt("Edge", Purpose::Document), "title: none | text: Edge");
        let qwen = EmbeddingFamily::of("qwen3");
        assert!(qwen.prompt("when", Purpose::Query).ends_with("\nQuery:when"));
        assert_eq!(qwen.prompt("Edge", Purpose::Document), "Edge");
        assert_eq!(qwen.pooling(), LlamaPoolingType::Last);
    }

    #[test]
    fn long_texts_keep_their_closing_token_and_batches_respect_both_limits() {
        let long: Vec<LlamaToken> = (0..600).map(LlamaToken).collect();
        let kept = clipped(long);
        assert_eq!(kept.len(), MOST_TOKENS_PER_TEXT);
        assert_eq!(kept.last(), Some(&LlamaToken(599)));
        let lists: Vec<Vec<LlamaToken>> = (0..40).map(|_| vec![LlamaToken(1); 100]).collect();
        let groups = batched(&lists);
        assert!(groups.iter().all(|group| group.len() <= MOST_TEXTS_PER_BATCH && group.len() * 100 <= 2048));
        assert_eq!(groups.iter().map(|group| group.len()).sum::<usize>(), 40);
    }
}
