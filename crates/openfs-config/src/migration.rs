use crate::types::VfsConfig;
use crate::ConfigError;

/// The current configuration version.
pub const CURRENT_VERSION: &str = "0.2";

/// Detect the version of a configuration.
pub fn detect_version(config: &VfsConfig) -> String {
    config.version.clone().unwrap_or_else(|| "0.1".to_string())
}

/// Migrate a configuration to the current version.
pub fn migrate(mut config: VfsConfig) -> Result<VfsConfig, ConfigError> {
    let version = detect_version(&config);

    match version.as_str() {
        "0.1" => {
            config = migrate_0_1_to_0_2(config)?;
            Ok(config)
        }
        v if v == CURRENT_VERSION => {
            // Already current, no-op
            Ok(config)
        }
        unknown => Err(ConfigError::InvalidConfig(format!(
            "Unknown config version '{}'. Supported versions: 0.1, {}",
            unknown, CURRENT_VERSION
        ))),
    }
}

/// Migrate from version 0.1 to 0.2.
fn migrate_0_1_to_0_2(mut config: VfsConfig) -> Result<VfsConfig, ConfigError> {
    config.version = Some(CURRENT_VERSION.to_string());
    // Framework for future field transforms:
    // - New fields added via serde defaults are automatically populated
    // - Renamed or restructured fields would be handled here
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BackendConfig, FsBackendConfig, MountConfig};

    fn make_config(version: Option<&str>) -> VfsConfig {
        VfsConfig {
            version: version.map(|v| v.to_string()),
            backends: indexmap::indexmap! {
                "local".to_string() => BackendConfig::Fs(FsBackendConfig {
                    root: "./data".to_string(),
                }),
            },
            mounts: vec![MountConfig {
                path: "/workspace".to_string(),
                backend: Some("local".to_string()),
                collection: None,
                mode: None,
                read_only: false,
                index: None,
                sync: None,
                watch: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn test_detect_version_default() {
        let config = make_config(None);
        assert_eq!(detect_version(&config), "0.1");
    }

    #[test]
    fn test_detect_version_explicit() {
        let config = make_config(Some("0.2"));
        assert_eq!(detect_version(&config), "0.2");
    }

    #[test]
    fn test_migrate_0_1_to_0_2() {
        let config = make_config(None);
        let migrated = migrate(config).unwrap();
        assert_eq!(migrated.version, Some("0.2".to_string()));
    }

    #[test]
    fn test_migrate_already_current() {
        let config = make_config(Some("0.2"));
        let migrated = migrate(config).unwrap();
        assert_eq!(migrated.version, Some("0.2".to_string()));
    }

    #[test]
    fn test_migrate_unknown_version() {
        let config = make_config(Some("99.0"));
        let result = migrate(config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown config version"));
    }
}
