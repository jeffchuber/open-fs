use crate::types::{
    BackendConfig, ChunkConfig, ChunkStrategy, EmbeddingConfig, IndexConfig, MountConfig,
    MountMode, SearchMode, VfsConfig,
};

impl VfsConfig {
    /// Apply default inference rules to the configuration.
    /// This mutates the config in place.
    pub fn apply_defaults(&mut self) {
        // Infer implicit backend if only one exists
        let single_backend = if self.backends.len() == 1 {
            self.backends.keys().next().cloned()
        } else {
            None
        };

        for mount in &mut self.mounts {
            // 1. Implicit backend inference
            if mount.backend.is_none() {
                if let Some(ref backend_name) = single_backend {
                    mount.backend = Some(backend_name.clone());
                }
            }

            // 2. Collection name inference from path
            if mount.collection.is_none() {
                mount.collection = Some(derive_collection_name(&mount.path));
            }

            // 3. Mode inference
            if mount.mode.is_none() {
                mount.mode = Some(infer_mode(mount, &self.backends));
            }

            // 4. Indexing inference based on path patterns
            if mount.index.is_none() {
                mount.index = Some(infer_indexing(&mount.path));
            }
        }
    }

    /// Returns a new config with all defaults applied.
    pub fn effective(&self) -> VfsConfig {
        let mut config = self.clone();
        config.apply_defaults();
        config
    }
}

/// Derive collection name from mount path.
/// `/workspace` -> "workspace"
/// `/foo/bar` -> "foo_bar"
fn derive_collection_name(path: &str) -> String {
    path.trim_start_matches('/')
        .trim_end_matches('/')
        .replace('/', "_")
}

/// Infer mount mode based on read_only flag and backend type.
fn infer_mode(
    mount: &MountConfig,
    backends: &indexmap::IndexMap<String, BackendConfig>,
) -> MountMode {
    let backend_config = mount
        .backend
        .as_ref()
        .and_then(|name| backends.get(name));

    let is_remote = backend_config
        .map(|cfg| matches!(cfg, BackendConfig::S3(_) | BackendConfig::Chroma(_) | BackendConfig::Api(_)))
        .unwrap_or(false);

    if mount.read_only {
        if is_remote {
            MountMode::RemoteCached
        } else {
            MountMode::LocalIndexed
        }
    } else if is_remote {
        MountMode::WriteThrough
    } else {
        MountMode::LocalIndexed
    }
}

/// Infer indexing configuration based on path patterns.
fn infer_indexing(path: &str) -> IndexConfig {
    let path_lower = path.to_lowercase();

    // Code paths: /workspace, /code, /src
    if path_lower.contains("workspace")
        || path_lower.contains("/code")
        || path_lower.contains("/src")
    {
        return IndexConfig {
            enabled: true,
            search_modes: vec![SearchMode::Hybrid],
            chunk: Some(ChunkConfig {
                strategy: ChunkStrategy::Ast,
                size: 512,
                overlap: 64,
                ..Default::default()
            }),
            embedding: Some(EmbeddingConfig::default()),
        };
    }

    // Memory/context paths: dense-only prose
    if path_lower.contains("memory") || path_lower.contains("context") {
        return IndexConfig {
            enabled: true,
            search_modes: vec![SearchMode::Dense],
            chunk: Some(ChunkConfig {
                strategy: ChunkStrategy::Recursive,
                size: 1024,
                overlap: 128,
                ..Default::default()
            }),
            embedding: Some(EmbeddingConfig::default()),
        };
    }

    // Scratch/tmp paths: no indexing
    if path_lower.contains("scratch") || path_lower.contains("tmp") || path_lower.contains("temp")
    {
        return IndexConfig {
            enabled: false,
            search_modes: vec![],
            chunk: None,
            embedding: None,
        };
    }

    // Default: basic indexing
    IndexConfig {
        enabled: true,
        search_modes: vec![SearchMode::Dense],
        chunk: Some(ChunkConfig::default()),
        embedding: Some(EmbeddingConfig::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FsBackendConfig;

    #[test]
    fn test_derive_collection_name() {
        assert_eq!(derive_collection_name("/workspace"), "workspace");
        assert_eq!(derive_collection_name("/foo/bar"), "foo_bar");
        assert_eq!(derive_collection_name("/"), "");
    }

    #[test]
    fn test_implicit_backend_inference() {
        let mut config = VfsConfig {
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![MountConfig {
                path: "/workspace".to_string(),
                backend: None,
                collection: None,
                mode: None,
                read_only: false,
                index: None,
                sync: None,
            }],
            ..Default::default()
        };

        config.apply_defaults();

        assert_eq!(config.mounts[0].backend, Some("local".to_string()));
        assert_eq!(config.mounts[0].collection, Some("workspace".to_string()));
        assert_eq!(config.mounts[0].mode, Some(MountMode::LocalIndexed));
    }

    #[test]
    fn test_indexing_inference_code_path() {
        let index = infer_indexing("/workspace");
        assert!(index.enabled);
        assert!(index.search_modes.contains(&SearchMode::Hybrid));
    }

    #[test]
    fn test_indexing_inference_scratch_path() {
        let index = infer_indexing("/scratch");
        assert!(!index.enabled);
    }
}
