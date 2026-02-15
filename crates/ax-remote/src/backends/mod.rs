pub mod fs;
pub mod memory;

#[cfg(feature = "s3")]
pub mod s3;

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "webdav")]
pub mod webdav;

#[cfg(feature = "sftp")]
pub mod sftp;

#[cfg(feature = "gcs")]
pub mod gcs;

#[cfg(feature = "azure")]
pub mod azure;

pub use fs::FsBackend;
pub use memory::MemoryBackend;

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
