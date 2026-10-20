use std::time::Duration;

use neomem_core::{Embedder, ModelError, ModelId, Vector};
use serde::{Deserialize, Serialize};

use crate::http;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct OpenAiEmbedder {
    agent: ureq::Agent,
    url: String,
    key: Option<String>,
    model: String,
    dims: usize,
}

#[derive(Serialize)]
struct EmbeddingRequest<'texts> {
    model: &'texts str,
    input: &'texts [&'texts str],
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Deserialize)]
struct EmbeddingItem {
    index: usize,
    embedding: Vec<f32>,
}

impl OpenAiEmbedder {
    pub fn connect(base_url: &str, key: Option<String>, model: &str) -> Result<Self, ModelError> {
        let mut embedder = Self {
            agent: http::agent(TIMEOUT),
            url: format!("{}/embeddings", base_url.trim_end_matches('/')),
            key,
            model: model.to_owned(),
            dims: 0,
        };
        let probe = embedder.request(&["dimension probe"])?;
        embedder.dims = probe.first().map_or(0, Vector::dims);
        Ok(embedder)
    }

    fn request(&self, texts: &[&str]) -> Result<Vec<Vector>, ModelError> {
        let body = EmbeddingRequest { model: &self.model, input: texts };
        let mut response: EmbeddingResponse =
            http::post_json(&self.agent, &self.url, self.key.as_deref(), &body, TIMEOUT)?;
        if response.data.len() != texts.len() {
            return Err(ModelError::InvalidOutput(format!(
                "asked for {} embeddings, got {}",
                texts.len(),
                response.data.len()
            )));
        }
        response.data.sort_by_key(|item| item.index);
        Ok(response.data.into_iter().map(|item| Vector::normalized(item.embedding)).collect())
    }
}

impl Embedder for OpenAiEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vector>, ModelError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.request(texts)
    }

    fn model_id(&self) -> ModelId {
        ModelId::new(format!("openai:{}", self.model))
    }

    fn dims(&self) -> usize {
        self.dims
    }
}
