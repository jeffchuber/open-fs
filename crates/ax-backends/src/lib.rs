mod chroma;
mod error;
mod fs;
mod memory;
mod traits;

#[cfg(feature = "s3")]
mod s3;

#[cfg(feature = "postgres")]
mod postgres;

#[cfg(feature = "webdav")]
mod webdav;

#[cfg(feature = "sftp")]
mod sftp;

#[cfg(feature = "gcs")]
mod gcs;

#[cfg(feature = "azure")]
mod azure;

pub use chroma::{ChromaBackend, QueryResult, SparseEmbedding};
pub use error::BackendError;
pub use fs::FsBackend;
pub use memory::MemoryBackend;
pub use traits::{Backend, Entry};

#[cfg(feature = "s3")]
pub use s3::{S3Backend, S3Config};

#[cfg(feature = "postgres")]
pub use postgres::{PostgresBackend, PostgresConfig};

#[cfg(feature = "webdav")]
pub use webdav::{WebDavBackend, WebDavConfig};

#[cfg(feature = "sftp")]
pub use sftp::{SftpBackend, SftpConfig};

#[cfg(feature = "gcs")]
pub use gcs::{GcsBackend, GcsConfig};

#[cfg(feature = "azure")]
pub use azure::{AzureBlobBackend, AzureBlobConfig};
