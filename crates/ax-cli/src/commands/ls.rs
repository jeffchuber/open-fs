use ax_remote::Vfs;

pub async fn run(vfs: &Vfs, path: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.as_deref().unwrap_or("/");

    let entries = vfs.list(path).await?;

    if entries.is_empty() {
        println!("(empty)");
        return Ok(());
    }

    for entry in entries {
        let type_indicator = if entry.is_dir { "d" } else { "-" };
        let size = entry
            .size
            .map(format_size)
            .unwrap_or_else(|| "-".to_string());

        println!("{} {:>8}  {}", type_indicator, size, entry.name);
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1}G", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{}B", bytes)
    }
}
