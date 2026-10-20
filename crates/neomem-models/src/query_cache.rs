use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use neomem_core::{Embedder, ModelError, ModelId, Purpose, Vector};

pub const QUERY_CACHE_ENTRIES: usize = 1024;

#[derive(Debug)]
pub struct QueryCache {
    embedder: Arc<dyn Embedder>,
    capacity: usize,
    remembered: Mutex<Remembered>,
}

#[derive(Debug, Default)]
struct Remembered {
    vectors: HashMap<String, Vector>,
    oldest_first: VecDeque<String>,
}

impl QueryCache {
    pub fn around(embedder: Arc<dyn Embedder>, capacity: usize) -> Self {
        Self { embedder, capacity, remembered: Mutex::new(Remembered::default()) }
    }

    fn remember(&self, texts: &[&str], vectors: &[Vector]) {
        let mut remembered = self.remembered.lock().unwrap_or_else(PoisonError::into_inner);
        for (text, vector) in texts.iter().zip(vectors) {
            if remembered.vectors.insert((*text).to_owned(), vector.clone()).is_none() {
                remembered.oldest_first.push_back((*text).to_owned());
            }
        }
        while remembered.oldest_first.len() > self.capacity {
            if let Some(oldest) = remembered.oldest_first.pop_front() {
                remembered.vectors.remove(&oldest);
            }
        }
    }
}

impl Embedder for QueryCache {
    fn embed(&self, texts: &[&str], purpose: Purpose) -> Result<Vec<Vector>, ModelError> {
        if purpose == Purpose::Document {
            return self.embedder.embed(texts, purpose);
        }
        let known: Vec<Option<Vector>> = {
            let remembered = self.remembered.lock().unwrap_or_else(PoisonError::into_inner);
            texts.iter().map(|text| remembered.vectors.get(*text).cloned()).collect()
        };
        let missing: Vec<&str> =
            texts.iter().zip(&known).filter(|(_, vector)| vector.is_none()).map(|(text, _)| *text).collect();
        let fresh = self.embedder.embed(&missing, purpose)?;
        if fresh.len() != missing.len() {
            return Err(ModelError::InvalidOutput(format!(
                "asked for {} embeddings, got {}",
                missing.len(),
                fresh.len()
            )));
        }
        self.remember(&missing, &fresh);
        let mut fresh = fresh.into_iter();
        Ok(known.into_iter().map(|vector| vector.or_else(|| fresh.next()).unwrap_or_else(empty)).collect())
    }

    fn model_id(&self) -> ModelId {
        self.embedder.model_id()
    }

    fn dims(&self) -> usize {
        self.embedder.dims()
    }
}

fn empty() -> Vector {
    Vector::normalized(Vec::new())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use neomem_testkit::{HashEmbedder, embedder_contract};

    use super::*;

    #[derive(Debug, Default)]
    struct Counting {
        texts_embedded: AtomicUsize,
    }

    impl Embedder for Counting {
        fn embed(&self, texts: &[&str], purpose: Purpose) -> Result<Vec<Vector>, ModelError> {
            self.texts_embedded.fetch_add(texts.len(), Ordering::SeqCst);
            HashEmbedder.embed(texts, purpose)
        }

        fn model_id(&self) -> ModelId {
            HashEmbedder.model_id()
        }

        fn dims(&self) -> usize {
            HashEmbedder.dims()
        }
    }

    #[test]
    fn the_cache_passes_the_embedder_contract() {
        embedder_contract(&QueryCache::around(Arc::new(HashEmbedder), 4));
    }

    #[test]
    fn repeated_queries_skip_the_model_but_documents_never_do() {
        let counting = Arc::new(Counting::default());
        let cache = QueryCache::around(Arc::clone(&counting) as Arc<dyn Embedder>, 4);
        let first = cache.embed(&["when does Edge ship"], Purpose::Query).unwrap();
        let again = cache.embed(&["when does Edge ship", "who is Maya"], Purpose::Query).unwrap();
        assert_eq!(first[0], again[0]);
        assert_eq!(again[1], HashEmbedder::vector("who is Maya"));
        cache.embed(&["when does Edge ship"], Purpose::Document).unwrap();
        assert_eq!(counting.texts_embedded.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn the_oldest_query_is_forgotten_first() {
        let counting = Arc::new(Counting::default());
        let cache = QueryCache::around(Arc::clone(&counting) as Arc<dyn Embedder>, 2);
        cache.embed(&["one", "two", "three"], Purpose::Query).unwrap();
        cache.embed(&["three", "two"], Purpose::Query).unwrap();
        assert_eq!(counting.texts_embedded.load(Ordering::SeqCst), 3);
        cache.embed(&["one"], Purpose::Query).unwrap();
        assert_eq!(counting.texts_embedded.load(Ordering::SeqCst), 4);
    }
}
