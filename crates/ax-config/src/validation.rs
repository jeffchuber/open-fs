use std::collections::HashSet;

use crate::types::VfsConfig;
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

        errors
    }

    /// Validate and return Ok(()) if valid, or Err with the first error.
    pub fn validate_or_err(&self) -> Result<(), ConfigError> {
        let errors = self.validate();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.into_iter().next().unwrap())
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
    use crate::types::{BackendConfig, FsBackendConfig, MountConfig, SyncConfig};

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
                sync: Some(SyncConfig::default()),
                ..default_mount()
            }],
            ..Default::default()
        };

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|e| matches!(e, ConfigError::InvalidConfig(_))));
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
        }
    }
}
