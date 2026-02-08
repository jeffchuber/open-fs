use ax_core::Vfs;

pub async fn run(vfs: &Vfs, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let entry = vfs.stat(path).await?;

    println!("Path:     {}", path);
    println!("Name:     {}", entry.name);
    println!("Type:     {}", if entry.is_dir { "directory" } else { "file" });

    if let Some(size) = entry.size {
        println!("Size:     {} bytes", size);
    }

    if let Some(modified) = entry.modified {
        println!("Modified: {}", modified.format("%Y-%m-%d %H:%M:%S UTC"));
    }

    Ok(())
}
