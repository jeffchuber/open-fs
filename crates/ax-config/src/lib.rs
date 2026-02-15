mod defaults;
mod env;
pub mod migration;
pub mod types;
mod validation;

use std::path::Path;

pub use types::*;

/// Configuration errors.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse YAML: {0}")]
    YamlError(#[from] serde_yaml::Error),

    #[error("Missing environment variables: {0:?}")]
    MissingEnvVars(Vec<String>),

    #[error("Duplicate mount path: {0}")]
    DuplicateMountPath(String),

    #[error("Invalid mount path '{0}': {1}")]
    InvalidMountPath(String, String),

    #[error("Backend '{0}' referenced by mount '{1}' is not defined")]
    UndefinedBackend(String, String),

    #[error("Overlapping mount paths: '{0}' and '{1}'")]
    OverlappingMountPaths(String, String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

impl VfsConfig {
    /// Parse a VFS configuration from a YAML string.
    /// Environment variables in the format `${VAR_NAME}` will be interpolated.
    pub fn from_yaml(yaml: &str) -> Result<Self, ConfigError> {
        // First, interpolate environment variables
        let interpolated = env::interpolate_env(yaml)?;

        // Then parse the YAML
        let config: VfsConfig = serde_yaml::from_str(&interpolated)?;

        Ok(config)
    }

    /// Load a VFS configuration from a file.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        Self::from_yaml(&content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let yaml = r#"
name: my-workspace
backends:
  local:
    type: fs
    root: ./data
mounts:
  - path: /workspace
    backend: local
"#;

        let config = VfsConfig::from_yaml(yaml).unwrap();
        assert_eq!(config.name, Some("my-workspace".to_string()));
        assert_eq!(config.backends.len(), 1);
        assert_eq!(config.mounts.len(), 1);
        assert_eq!(config.mounts[0].path, "/workspace");
    }

    #[test]
    fn test_parse_with_env_vars() {
        std::env::set_var("TEST_ROOT_PATH", "/tmp/test");

        let yaml = r#"
name: test
backends:
  local:
    type: fs
    root: ${TEST_ROOT_PATH}
mounts:
  - path: /workspace
    backend: local
"#;

        let config = VfsConfig::from_yaml(yaml).unwrap();
        match &config.backends["local"] {
            BackendConfig::Fs(fs) => {
                assert_eq!(fs.root, "/tmp/test");
            }
            _ => panic!("Expected Fs backend"),
        }
    }

    #[test]
    fn test_effective_config() {
        let yaml = r#"
backends:
  local:
    type: fs
    root: ./data
mounts:
  - path: /workspace
"#;

        let config = VfsConfig::from_yaml(yaml).unwrap();
        let effective = config.effective();

        // Should have inferred backend and collection
        assert_eq!(effective.mounts[0].backend, Some("local".to_string()));
        assert_eq!(
            effective.mounts[0].collection,
            Some("workspace".to_string())
        );
        assert_eq!(
            effective.mounts[0].mode,
            Some(types::MountMode::LocalIndexed)
        );
    }

    #[test]
    fn test_validation() {
        let yaml = r#"
backends:
  local:
    type: fs
    root: ./data
mounts:
  - path: /workspace
    backend: local
"#;

        let config = VfsConfig::from_yaml(yaml).unwrap();
        let errors = config.validate();
        assert!(errors.is_empty());
    }
}
