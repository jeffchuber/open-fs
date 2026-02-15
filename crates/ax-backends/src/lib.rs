mod chroma;
mod error;
mod fs;
mod memory;
mod traits;

#[cfg(feature = "s3")]
mod s3;

#[cfg(feature = "postgres")]
mod postgres;

pub use chroma::{ChromaBackend, QueryResult, SparseEmbedding};
pub use error::BackendError;
pub use fs::FsBackend;
pub use memory::MemoryBackend;
pub use traits::{Backend, Entry};

#[cfg(feature = "s3")]
pub use s3::{S3Backend, S3Config};

#[cfg(feature = "postgres")]
pub use postgres::{PostgresBackend, PostgresConfig};
