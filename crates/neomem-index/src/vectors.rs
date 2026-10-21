use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{PoisonError, RwLock};

use neomem_core::Vector;
use usearch::{Index as Hnsw, IndexOptions, MetricKind, ScalarKind};

use crate::IndexError;

const CONNECTIVITY: usize = 16;
const EXPANSION_ADD: usize = 128;
const EXPANSION_SEARCH: usize = 64;
const FIRST_CAPACITY: usize = 1024;
const VECTORS_PER_THREAD: usize = 512;

pub type Slot = u64;

pub struct VectorIndex {
    hnsw: RwLock<Hnsw>,
    dims: usize,
    next_slot: AtomicU64,
}

impl std::fmt::Debug for VectorIndex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VectorIndex")
            .field("dims", &self.dims)
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl VectorIndex {
    pub fn new(dims: usize) -> Result<Self, IndexError> {
        let options = IndexOptions {
            dimensions: dims,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F16,
            connectivity: CONNECTIVITY,
            expansion_add: EXPANSION_ADD,
            expansion_search: EXPANSION_SEARCH,
            multi: false,
        };
        let hnsw = Hnsw::new(&options).map_err(|error| IndexError::Vectors(error.to_string()))?;
        hnsw.reserve(FIRST_CAPACITY).map_err(|error| IndexError::Vectors(error.to_string()))?;
        Ok(Self { hnsw: RwLock::new(hnsw), dims, next_slot: AtomicU64::new(0) })
    }

    pub fn dims(&self) -> usize {
        self.dims
    }

    pub fn len(&self) -> usize {
        self.hnsw.read().unwrap_or_else(PoisonError::into_inner).size()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn add(&self, vector: &Vector) -> Result<Slot, IndexError> {
        if vector.dims() != self.dims {
            return Err(IndexError::Vectors(format!(
                "a {}-dim vector cannot join a {}-dim index",
                vector.dims(),
                self.dims
            )));
        }
        self.make_room_for_one()?;
        let slot = self.next_slot.fetch_add(1, Ordering::Relaxed);
        self.hnsw
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .add(slot, vector.as_slice())
            .map_err(|error| IndexError::Vectors(error.to_string()))?;
        Ok(slot)
    }

    pub fn add_all(&self, vectors: &[&Vector]) -> Result<Vec<Slot>, IndexError> {
        if let Some(wrong) = vectors.iter().find(|vector| vector.dims() != self.dims) {
            return Err(IndexError::Vectors(format!(
                "a {}-dim vector cannot join a {}-dim index",
                wrong.dims(),
                self.dims
            )));
        }
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        let threads = vectors.len().div_ceil(VECTORS_PER_THREAD).clamp(1, cores);
        {
            let hnsw = self.hnsw.write().unwrap_or_else(PoisonError::into_inner);
            let needed = (hnsw.size() + vectors.len()).max(hnsw.capacity());
            hnsw.reserve_capacity_and_threads(needed, threads)
                .map_err(|error| IndexError::Vectors(error.to_string()))?;
        }
        let count = u64::try_from(vectors.len()).map_err(|error| IndexError::Vectors(error.to_string()))?;
        let first = self.next_slot.fetch_add(count, Ordering::Relaxed);
        let slots: Vec<Slot> = (first..first + count).collect();
        let hnsw = self.hnsw.read().unwrap_or_else(PoisonError::into_inner);
        let share = vectors.len().div_ceil(threads).max(1);
        let failures: Vec<String> = std::thread::scope(|scope| {
            let workers: Vec<_> = vectors
                .chunks(share)
                .zip(slots.chunks(share))
                .map(|(chunk, chunk_slots)| {
                    let hnsw = &*hnsw;
                    scope.spawn(move || {
                        chunk
                            .iter()
                            .zip(chunk_slots)
                            .filter_map(|(vector, slot)| hnsw.add(*slot, vector.as_slice()).err())
                            .map(|error| error.to_string())
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            workers.into_iter().flat_map(|worker| worker.join().unwrap_or_default()).collect()
        });
        match failures.first() {
            Some(failure) => Err(IndexError::Vectors(failure.clone())),
            None => Ok(slots),
        }
    }

    pub fn nearest(&self, query: &Vector, count: usize, accept: impl Fn(Slot) -> bool) -> Vec<(Slot, f32)> {
        let hnsw = self.hnsw.read().unwrap_or_else(PoisonError::into_inner);
        if hnsw.size() == 0 || count == 0 || query.dims() != self.dims {
            return Vec::new();
        }
        match hnsw.filtered_search(query.as_slice(), count, accept) {
            Ok(matches) => matches
                .keys
                .into_iter()
                .zip(matches.distances)
                .map(|(slot, distance)| (slot, 1.0 - distance))
                .collect(),
            Err(error) => {
                tracing::error!("vector search failed: {error}");
                Vec::new()
            }
        }
    }

    pub fn stored(&self, slot: Slot) -> Option<Vector> {
        let hnsw = self.hnsw.read().unwrap_or_else(PoisonError::into_inner);
        let mut buffer = vec![0.0_f32; self.dims];
        match hnsw.get(slot, &mut buffer) {
            Ok(found) if found > 0 => Some(Vector::normalized(buffer)),
            _ => None,
        }
    }

    fn make_room_for_one(&self) -> Result<(), IndexError> {
        let needs_room = {
            let hnsw = self.hnsw.read().unwrap_or_else(PoisonError::into_inner);
            hnsw.size() + 1 > hnsw.capacity()
        };
        if needs_room {
            let hnsw = self.hnsw.write().unwrap_or_else(PoisonError::into_inner);
            let doubled = (hnsw.capacity() * 2).max(FIRST_CAPACITY);
            hnsw.reserve(doubled).map_err(|error| IndexError::Vectors(error.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neomem_testkit::HashEmbedder;

    #[test]
    fn the_nearest_vector_is_the_most_similar_text() {
        let index = VectorIndex::new(256).unwrap();
        let texts = ["Edge ships on November 1", "Maya is my sister", "The dentist is on Thursday"];
        let slots: Vec<Slot> =
            texts.iter().map(|text| index.add(&HashEmbedder::vector(text)).unwrap()).collect();
        let found = index.nearest(&HashEmbedder::vector("when does Edge ship"), 1, |_| true);
        assert_eq!(found.first().map(|hit| hit.0), Some(slots[0]));
    }

    #[test]
    fn the_filter_hides_slots_it_rejects() {
        let index = VectorIndex::new(256).unwrap();
        let first = index.add(&HashEmbedder::vector("Edge ships on November 1")).unwrap();
        index.add(&HashEmbedder::vector("Edge ships on October 15")).unwrap();
        let found = index.nearest(&HashEmbedder::vector("Edge ships"), 5, |slot| slot != first);
        assert!(found.iter().all(|hit| hit.0 != first));
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn it_grows_past_its_first_capacity() {
        let index = VectorIndex::new(256).unwrap();
        for number in 0..(FIRST_CAPACITY + 10) {
            index.add(&HashEmbedder::vector(&format!("note {number}"))).unwrap();
        }
        assert_eq!(index.len(), FIRST_CAPACITY + 10);
    }
}
