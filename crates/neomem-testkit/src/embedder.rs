use std::hash::{DefaultHasher, Hash, Hasher};

use neomem_core::{Embedder, ModelError, ModelId, Purpose, Vector};

pub const HASH_DIMS: usize = 256;

#[derive(Debug, Clone, Copy, Default)]
pub struct HashEmbedder;

impl HashEmbedder {
    pub fn vector(text: &str) -> Vector {
        let mut values = vec![0.0_f32; HASH_DIMS];
        for word in words(text) {
            values[bucket(&("word", &word))] += 1.0;
            let padded: Vec<char> = format!(" {word} ").chars().collect();
            for trigram in padded.windows(3) {
                values[bucket(&("tri", trigram))] += 0.5;
            }
        }
        Vector::normalized(values)
    }
}

pub fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

fn bucket(value: &impl Hash) -> usize {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    usize::try_from(hasher.finish() % HASH_DIMS as u64).unwrap_or(0)
}

impl Embedder for HashEmbedder {
    fn embed(&self, texts: &[&str], _purpose: Purpose) -> Result<Vec<Vector>, ModelError> {
        Ok(texts.iter().map(|text| Self::vector(text)).collect())
    }

    fn model_id(&self) -> ModelId {
        ModelId::new("fake:hash-256")
    }

    fn dims(&self) -> usize {
        HASH_DIMS
    }
}

#[cfg(test)]
mod tests {
    use super::HashEmbedder;

    #[test]
    fn similar_wording_scores_higher_than_unrelated_text() {
        let query = HashEmbedder::vector("when does Edge ship");
        let close = HashEmbedder::vector("Edge ships on November 1");
        let far = HashEmbedder::vector("Maya is my sister");
        assert!(query.cosine(&close) > query.cosine(&far));
    }

    #[test]
    fn the_same_text_always_embeds_the_same() {
        assert_eq!(HashEmbedder::vector("stable"), HashEmbedder::vector("stable"));
    }
}
