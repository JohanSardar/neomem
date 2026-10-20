mod index;
mod snapshot;
mod store;
mod vectors;

pub use index::{Index, Opened};
pub use snapshot::{IndexCounts, Neighbor, Snapshot};
pub use store::Change;
pub use vectors::{Slot, VectorIndex};

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("index.db failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("the vector index failed: {0}")]
    Vectors(String),
    #[error("index.db holds a row that does not parse: {0}")]
    Corrupt(String),
}
