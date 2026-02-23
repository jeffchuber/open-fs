use openfs_remote::Vfs;

pub async fn run(vfs: &Vfs, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let content = vfs.read(path).await?;

    // Try to print as UTF-8, fall back to lossy conversion
    match std::str::from_utf8(&content) {
        Ok(s) => print!("{}", s),
        Err(_) => print!("{}", String::from_utf8_lossy(&content)),
    }

    Ok(())
}
