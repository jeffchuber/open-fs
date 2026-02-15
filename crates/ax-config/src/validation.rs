use std::collections::HashSet;

use crate::types::{
    BackendConfig, ChunkConfig, EmbeddingConfig, VfsConfig, WatchConfig,
};
use crate::ConfigError;

impl VfsConfig {
    /// Validate the configuration and return a list of errors.
    pub fn validate(&self) -> Vec<ConfigError> {
        let mut errors = Vec::new();

        // Check for duplicate mount paths
        let mut seen_paths = HashSet::new();
        for mount in &self.mounts {
            if !seen_paths.insert(&mount.path) {
                errors.push(ConfigError::DuplicateMountPath(mount.path.clone()));
            }
        }

        // Check that all mount paths start with /
        for mount in &self.mounts {
            if !mount.path.starts_with('/') {
                errors.push(ConfigError::InvalidMountPath(
                    mount.path.clone(),
                    "Mount path must start with '/'".to_string(),
                ));
            }
        }

        // Check that every mount's backend references a defined backend
        for mount in &self.mounts {
            if let Some(ref backend_name) = mount.backend {
                if !self.backends.contains_key(backend_name) {
                    errors.push(ConfigError::UndefinedBackend(
                        backend_name.clone(),
                        mount.path.clone(),
                    ));
                }
            }
        }

        // Check that no mount path is a prefix of another
        let paths: Vec<_> = self.mounts.iter().map(|m| &m.path).collect();
        for (i, path_a) in paths.iter().enumerate() {
            for (j, path_b) in paths.iter().enumerate() {
                if i != j {
                    let a_normalized = normalize_path(path_a);
                    let b_normalized = normalize_path(path_b);
                    if b_normalized.starts_with(&format!("{}/", a_normalized)) {
                        errors.push(ConfigError::OverlappingMountPaths(
                            (*path_a).clone(),
                            (*path_b).clone(),
                        ));
                    }
                }
            }
        }

        // Check that read-only mounts don't have sync config
        for mount in &self.mounts {
            if mount.read_only && mount.sync.is_some() {
                errors.push(ConfigError::InvalidConfig(format!(
                    "Mount '{}' is read-only but has sync configuration",
                    mount.path
                )));
            }
        }

        // Validate backend-specific configurations
        for (name, backend) in &self.backends {
            match backend {
                BackendConfig::Fs(fs) => validate_fs_config(name, fs, &mut errors),
                BackendConfig::Memory(_memory) => {}
                BackendConfig::S3(s3) => validate_s3_config(name, s3, &mut errors),
                BackendConfig::Postgres(pg) => validate_postgres_config(name, pg, &mut errors),
                BackendConfig::Chroma(chroma) => validate_chroma_config(name, chroma, &mut errors),
            }
        }

        // Validate mount-level index sub-configs
        for mount in &self.mounts {
            if let Some(ref index) = mount.index {
                if let Some(ref chunk) = index.chunk {
                    validate_chunk_config(&mount.path, chunk, &mut errors);
                }
                if let Some(ref embedding) = index.embedding {
                    validate_embedding_config(&mount.path, embedding, &mut errors);
                }
            }
            if let Some(ref watch) = mount.watch {
                validate_watch_config(&mount.path, watch, &mut errors);
            }
        }

        // Validate default-level configs
        if let Some(ref defaults) = self.defaults {
            if let Some(ref chunk) = defaults.chunk {
                validate_chunk_config("defaults", chunk, &mut errors);
            }
            if let Some(ref embedding) = defaults.embedding {
                validate_embedding_config("defaults", embedding, &mut errors);
            }
            if let Some(ref watch) = defaults.watch {
                validate_watch_config("defaults", watch, &mut errors);
            }
        }

        errors
    }

    /// Validate and return Ok(()) if valid, or Err with all errors combined.
    pub fn validate_or_err(&self) -> Result<(), ConfigError> {
        let errors = self.validate();
        if errors.is_empty() {
            Ok(())
        } else {
            let messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            Err(ConfigError::InvalidConfig(format!(
                "{} validation error(s):\n  - {}",
                messages.len(),
                messages.join("\n  - ")
            )))
        }
    }
}

fn validate_fs_config(
    name: &str,
    fs: &crate::types::FsBackendConfig,
    errors: &mut Vec<ConfigError>,
) {
    if fs.root.is_empty() {
        errors.push(ConfigError::InvalidConfig(format!(
            "backends.{}.root: must not be empty",
            name
        )));
    }
}

fn validate_s3_config(
    name: &str,
    s3: &crate::types::S3BackendConfig,
    errors: &mut Vec<ConfigError>,
) {
    if s3.bucket.len() < 3 || s3.bucket.len() > 63 {
        errors.push(ConfigError::InvalidConfig(format!(
            "backends.{}.bucket: must be between 3 and 63 characters (got {})",
            name,
            s3.bucket.len()
        )));
    }
    if let Some(ref endpoint) = s3.endpoint {
        if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
            errors.push(ConfigError::InvalidConfig(format!(
                "backends.{}.endpoint: must start with http:// or https:// (got '{}')",
                name, endpoint
            )));
        }
    }
}

fn validate_postgres_config(
    name: &str,
    pg: &crate::types::PostgresBackendConfig,
    errors: &mut Vec<ConfigError>,
) {
    if !pg.connection_url.expose().starts_with("postgres://")
        && !pg.connection_url.expose().starts_with("postgresql://")
    {
        errors.push(ConfigError::InvalidConfig(format!(
            "backends.{}.connection_url: must start with postgres:// or postgresql://",
            name
        )));
    }
}

fn validate_chroma_config(
    name: &str,
    chroma: &crate::types::ChromaBackendConfig,
    errors: &mut Vec<ConfigError>,
) {
    if !chroma.url.starts_with("http://") && !chroma.url.starts_with("https://") {
        errors.push(ConfigError::InvalidConfig(format!(
            "backends.{}.url: must start with http:// or https:// (got '{}')",
            name, chroma.url
        )));
    }
}

fn validate_chunk_config(context: &str, chunk: &ChunkConfig, errors: &mut Vec<ConfigError>) {
    if chunk.size == 0 {
        errors.push(ConfigError::InvalidConfig(format!(
            "{}.chunk.size: must be greater than 0",
            context
        )));
    }
    if chunk.overlap >= chunk.size {
        errors.push(ConfigError::InvalidConfig(format!(
            "{}.chunk.overlap: must be less than chunk.size ({} >= {})",
            context, chunk.overlap, chunk.size
        )));
    }
    if chunk.size > 100_000 {
        errors.push(ConfigError::InvalidConfig(format!(
            "{}.chunk.size: must be at most 100000 (got {})",
            context, chunk.size
        )));
    }
}

fn validate_embedding_config(
    context: &str,
    embedding: &EmbeddingConfig,
    errors: &mut Vec<ConfigError>,
) {
    if embedding.dimensions == 0 {
        errors.push(ConfigError::InvalidConfig(format!(
            "{}.embedding.dimensions: must be greater than 0",
            context
        )));
    }
    if embedding.dimensions > 4096 {
        errors.push(ConfigError::InvalidConfig(format!(
            "{}.embedding.dimensions: must be at most 4096 (got {})",
            context, embedding.dimensions
        )));
    }
}

fn validate_watch_config(context: &str, watch: &WatchConfig, errors: &mut Vec<ConfigError>) {
    if let Some(ref poll_interval) = watch.poll_interval {
        if poll_interval.as_duration().is_zero() {
            errors.push(ConfigError::InvalidConfig(format!(
                "{}.watch.poll_interval: must be greater than 0",
                context
            )));
        }
    }
    if let Some(ref webhook_url) = watch.webhook_url {
        if !webhook_url.starts_with("http://") && !webhook_url.starts_with("https://") {
            errors.push(ConfigError::InvalidConfig(format!(
                "{}.watch.webhook_url: must start with http:// or https:// (got '{}')",
                context, webhook_url
            )));
        }
    }
}

/// Normalize a path by removing trailing slashes.
fn normalize_path(path: &str) -> &str {
    path.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        BackendConfig, FsBackendConfig, MountConfig, SyncConfig as MountSyncConfig,
        S3BackendConfig, PostgresBackendConfig, ChromaBackendConfig,
        ChunkConfig, EmbeddingConfig, IndexConfig, WatchConfig, HumanDuration, Secret,
    };

    #[test]
    fn test_duplicate_mount_paths() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![
                MountConfig {
                    path: "/workspace".to_string(),
                    backend: Some("local".to_string()),
                    ..default_mount()
                },
                MountConfig {
                    path: "/workspace".to_string(),
                    backend: Some("local".to_string()),
                    ..default_mount()
                },
            ],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|e| matches!(e, ConfigError::DuplicateMountPath(_))));
    }

    #[test]
    fn test_invalid_mount_path() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![MountConfig {
                path: "workspace".to_string(), // Missing leading /
                backend: Some("local".to_string()),
                ..default_mount()
            }],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|e| matches!(e, ConfigError::InvalidMountPath(_, _))));
    }

    #[test]
    fn test_undefined_backend() {
        let config = VfsConfig {
            backends: indexmap::IndexMap::new(),
            mounts: vec![MountConfig {
                path: "/workspace".to_string(),
                backend: Some("nonexistent".to_string()),
                ..default_mount()
            }],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|e| matches!(e, ConfigError::UndefinedBackend(_, _))));
    }

    #[test]
    fn test_overlapping_mount_paths() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![
                MountConfig {
                    path: "/workspace".to_string(),
                    backend: Some("local".to_string()),
                    ..default_mount()
                },
                MountConfig {
                    path: "/workspace/subdir".to_string(),
                    backend: Some("local".to_string()),
                    ..default_mount()
                },
            ],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|e| matches!(e, ConfigError::OverlappingMountPaths(_, _))));
    }

    #[test]
    fn test_readonly_with_sync() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![MountConfig {
                path: "/workspace".to_string(),
                backend: Some("local".to_string()),
                read_only: true,
                sync: Some(MountSyncConfig::default()),
                ..default_mount()
            }],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|e| matches!(e, ConfigError::InvalidConfig(_))));
    }

    #[test]
    fn test_validate_fs_empty_root() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "".to_string(),
                }),
            },
            mounts: vec![],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|e| e.to_string().contains("root: must not be empty")));
    }

    #[test]
    fn test_validate_s3_bucket_length() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "s3".to_string() => BackendConfig::S3(S3BackendConfig {
                    bucket: "ab".to_string(), // too short
                    prefix: None,
                    region: None,
                    endpoint: None,
                    access_key_id: None,
                    secret_access_key: None,
                }),
            },
            mounts: vec![],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|e| e.to_string().contains("bucket: must be between 3 and 63")));
    }

    #[test]
    fn test_validate_s3_bad_endpoint() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "s3".to_string() => BackendConfig::S3(S3BackendConfig {
                    bucket: "my-bucket".to_string(),
                    prefix: None,
                    region: None,
                    endpoint: Some("ftp://bad".to_string()),
                    access_key_id: None,
                    secret_access_key: None,
                }),
            },
            mounts: vec![],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|e| e.to_string().contains("endpoint: must start with http")));
    }

    #[test]
    fn test_validate_postgres_bad_connection_url() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "pg".to_string() => BackendConfig::Postgres(PostgresBackendConfig {
                    connection_url: Secret::new("mysql://localhost/db"),
                    table_name: None,
                    max_connections: None,
                }),
            },
            mounts: vec![],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors.iter().any(|e| e
            .to_string()
            .contains("connection_url: must start with postgres")));
    }

    #[test]
    fn test_validate_chroma_bad_url() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "chroma".to_string() => BackendConfig::Chroma(ChromaBackendConfig {
                    url: "not-a-url".to_string(),
                    collection: None,
                }),
            },
            mounts: vec![],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|e| e.to_string().contains("url: must start with http")));
    }

    #[test]
    fn test_validate_chunk_zero_size() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![MountConfig {
                path: "/workspace".to_string(),
                backend: Some("local".to_string()),
                index: Some(IndexConfig {
                    enabled: true,
                    chunk: Some(ChunkConfig {
                        size: 0,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..default_mount()
            }],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|e| e.to_string().contains("chunk.size: must be greater than 0")));
    }

    #[test]
    fn test_validate_chunk_overlap_gte_size() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![MountConfig {
                path: "/workspace".to_string(),
                backend: Some("local".to_string()),
                index: Some(IndexConfig {
                    enabled: true,
                    chunk: Some(ChunkConfig {
                        size: 100,
                        overlap: 100,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..default_mount()
            }],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors.iter().any(|e| e
            .to_string()
            .contains("chunk.overlap: must be less than chunk.size")));
    }

    #[test]
    fn test_validate_chunk_size_too_large() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![MountConfig {
                path: "/workspace".to_string(),
                backend: Some("local".to_string()),
                index: Some(IndexConfig {
                    enabled: true,
                    chunk: Some(ChunkConfig {
                        size: 200_000,
                        overlap: 10,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..default_mount()
            }],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|e| e.to_string().contains("chunk.size: must be at most 100000")));
    }

    #[test]
    fn test_validate_embedding_zero_dimensions() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![MountConfig {
                path: "/workspace".to_string(),
                backend: Some("local".to_string()),
                index: Some(IndexConfig {
                    enabled: true,
                    embedding: Some(EmbeddingConfig {
                        dimensions: 0,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..default_mount()
            }],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors.iter().any(|e| e
            .to_string()
            .contains("embedding.dimensions: must be greater than 0")));
    }

    #[test]
    fn test_validate_embedding_too_many_dimensions() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![MountConfig {
                path: "/workspace".to_string(),
                backend: Some("local".to_string()),
                index: Some(IndexConfig {
                    enabled: true,
                    embedding: Some(EmbeddingConfig {
                        dimensions: 5000,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..default_mount()
            }],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors.iter().any(|e| e
            .to_string()
            .contains("embedding.dimensions: must be at most 4096")));
    }

    #[test]
    fn test_validate_or_err_shows_all_errors() {
        let config = VfsConfig {
            backends: indexmap::IndexMap::new(),
            mounts: vec![
                MountConfig {
                    path: "bad1".to_string(), // Invalid path
                    backend: Some("missing".to_string()), // Undefined backend
                    ..default_mount()
                },
                MountConfig {
                    path: "bad2".to_string(), // Invalid path
                    ..default_mount()
                },
            ],
            ..Default::default()
        };

        let result = config.validate_or_err();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        // Should contain multiple errors
        assert!(err_msg.contains("validation error(s)"));
        assert!(err_msg.contains("bad1"));
        assert!(err_msg.contains("bad2"));
    }

    #[test]
    fn test_validate_watch_zero_poll_interval() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![MountConfig {
                path: "/workspace".to_string(),
                backend: Some("local".to_string()),
                watch: Some(WatchConfig {
                    poll_interval: Some(HumanDuration(std::time::Duration::from_secs(0))),
                    ..Default::default()
                }),
                ..default_mount()
            }],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors.iter().any(|e| e
            .to_string()
            .contains("watch.poll_interval: must be greater than 0")));
    }

    #[test]
    fn test_validate_watch_bad_webhook_url() {
        let config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![MountConfig {
                path: "/workspace".to_string(),
                backend: Some("local".to_string()),
                watch: Some(WatchConfig {
                    webhook_url: Some("not-a-url".to_string()),
                    ..Default::default()
                }),
                ..default_mount()
            }],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors.iter().any(|e| e
            .to_string()
            .contains("watch.webhook_url: must start with http")));
    }

    fn default_mount() -> MountConfig {
        MountConfig {
            path: String::new(),
            backend: None,
            collection: None,
            mode: None,
            read_only: false,
            index: None,
            sync: None,
            watch: None,
        }
    }
}
