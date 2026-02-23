use std::path::Path;

use openfs_config::VfsConfig;

pub async fn run(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let config = VfsConfig::from_file(config_path)?;
    let errors = config.validate();

    if errors.is_empty() {
        println!("Configuration is valid.");
        Ok(())
    } else {
        eprintln!("Configuration has {} error(s):", errors.len());
        for (i, err) in errors.iter().enumerate() {
            eprintln!("  {}: {}", i + 1, err);
        }
        Err(format!("{} validation error(s) found", errors.len()).into())
    }
}
