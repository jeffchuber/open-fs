use std::path::Path;

use openfs_config::VfsConfig;

pub async fn run(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let config = VfsConfig::from_file(config_path)?;

    let version = openfs_config::migration::detect_version(&config);
    println!("Detected config version: {}", version);

    let migrated = openfs_config::migration::migrate(config)?;
    let new_version = migrated.version.as_deref().unwrap_or("unknown");
    println!("Migrated to version: {}", new_version);

    // Write back the migrated config
    let yaml = serde_yaml::to_string(&migrated)?;
    std::fs::write(config_path, yaml)?;
    println!("Configuration updated: {}", config_path.display());

    Ok(())
}
