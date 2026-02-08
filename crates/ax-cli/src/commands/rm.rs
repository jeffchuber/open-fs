use ax_core::Vfs;

pub async fn run(vfs: &Vfs, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    vfs.delete(path).await?;
    println!("Deleted {}", path);

    Ok(())
}
