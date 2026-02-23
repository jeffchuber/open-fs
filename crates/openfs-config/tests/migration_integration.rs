//! Config Migration Integration Tests
//!
//! Tests the config migration pipeline using real YAML files (not programmatic construction).

use openfs_config::migration::{self, CURRENT_VERSION};
use openfs_config::VfsConfig;

#[test]
fn test_load_v01_yaml_and_migrate() {
    let yaml = r#"
name: legacy-workspace
backends:
  local:
    type: fs
    root: ./data
mounts:
  - path: /workspace
    backend: local
"#;
    // v0.1 config has no version field
    let config = VfsConfig::from_yaml(yaml).unwrap();
    assert!(config.version.is_none());

    let version = migration::detect_version(&config);
    assert_eq!(version, "0.1");

    let migrated = migration::migrate(config).unwrap();
    assert_eq!(migrated.version, Some(CURRENT_VERSION.to_string()));
}

#[test]
fn test_load_v02_no_migration_needed() {
    let yaml = r#"
version: "0.2"
name: current-workspace
backends:
  local:
    type: fs
    root: ./data
mounts:
  - path: /workspace
    backend: local
"#;
    let config = VfsConfig::from_yaml(yaml).unwrap();
    assert_eq!(config.version, Some("0.2".to_string()));

    let migrated = migration::migrate(config).unwrap();
    assert_eq!(migrated.version, Some("0.2".to_string()));
}

#[test]
fn test_migrate_preserves_all_fields() {
    let yaml = r#"
name: full-config
backends:
  local:
    type: fs
    root: /tmp/test-data
  mem:
    type: memory
mounts:
  - path: /workspace
    backend: local
    read_only: true
  - path: /scratch
    backend: mem
"#;
    let config = VfsConfig::from_yaml(yaml).unwrap();
    let migrated = migration::migrate(config).unwrap();

    // Version updated
    assert_eq!(migrated.version, Some(CURRENT_VERSION.to_string()));

    // Name preserved
    assert_eq!(migrated.name, Some("full-config".to_string()));

    // Backends preserved
    assert_eq!(migrated.backends.len(), 2);
    assert!(migrated.backends.contains_key("local"));
    assert!(migrated.backends.contains_key("mem"));

    // Mounts preserved
    assert_eq!(migrated.mounts.len(), 2);
    assert_eq!(migrated.mounts[0].path, "/workspace");
    assert!(migrated.mounts[0].read_only);
    assert_eq!(migrated.mounts[1].path, "/scratch");
}

#[test]
fn test_migrate_then_validate() {
    let yaml = r#"
name: validated-config
backends:
  local:
    type: fs
    root: ./data
mounts:
  - path: /workspace
    backend: local
"#;
    let config = VfsConfig::from_yaml(yaml).unwrap();
    let migrated = migration::migrate(config).unwrap();

    let errors = migrated.validate();
    assert!(
        errors.is_empty(),
        "Migrated config should validate without errors, got: {:?}",
        errors
    );
}

#[test]
fn test_unknown_version_fails_gracefully() {
    let yaml = r#"
version: "99.0"
name: future-config
backends:
  local:
    type: fs
    root: ./data
mounts:
  - path: /workspace
    backend: local
"#;
    let config = VfsConfig::from_yaml(yaml).unwrap();
    let result = migration::migrate(config);

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Unknown config version"),
        "Expected 'Unknown config version' in error, got: {}",
        err
    );
    assert!(err.contains("99.0"));
}
