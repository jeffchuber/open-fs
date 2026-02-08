use std::collections::HashMap;
use std::time::Duration;

use ax_core::Vfs;

pub async fn run(
    vfs: &Vfs,
    path: Option<String>,
    interval_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.unwrap_or_else(|| "/".to_string());
    let interval = Duration::from_secs(interval_secs);

    println!("Watching {} for changes (interval: {}s)", path, interval_secs);
    println!("Press Ctrl+C to stop");
    println!();

    // Track file states
    let mut file_states: HashMap<String, (u64, Option<chrono::DateTime<chrono::Utc>>)> = HashMap::new();

    // Initial scan
    scan_directory(vfs, &path, &mut file_states).await?;
    println!("Initial scan: {} files", file_states.len());
    println!();

    loop {
        tokio::time::sleep(interval).await;

        let mut new_states: HashMap<String, (u64, Option<chrono::DateTime<chrono::Utc>>)> = HashMap::new();
        scan_directory(vfs, &path, &mut new_states).await?;

        // Check for changes
        let mut changes = Vec::new();

        // New or modified files
        for (file_path, (size, modified)) in &new_states {
            if let Some((old_size, old_modified)) = file_states.get(file_path) {
                if size != old_size || modified != old_modified {
                    changes.push(format!("modified: {}", file_path));
                }
            } else {
                changes.push(format!("created: {}", file_path));
            }
        }

        // Deleted files
        for file_path in file_states.keys() {
            if !new_states.contains_key(file_path) {
                changes.push(format!("deleted: {}", file_path));
            }
        }

        // Report changes
        if !changes.is_empty() {
            let now = chrono::Local::now().format("%H:%M:%S");
            for change in &changes {
                println!("[{}] {}", now, change);
            }
        }

        file_states = new_states;
    }
}

#[async_recursion::async_recursion]
async fn scan_directory(
    vfs: &Vfs,
    dir_path: &str,
    states: &mut HashMap<String, (u64, Option<chrono::DateTime<chrono::Utc>>)>,
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = vfs.list(dir_path).await?;

    for entry in entries {
        if entry.is_dir {
            scan_directory(vfs, &entry.path, states).await?;
        } else {
            let size = entry.size.unwrap_or(0);
            states.insert(entry.path.clone(), (size, entry.modified));
        }
    }

    Ok(())
}
