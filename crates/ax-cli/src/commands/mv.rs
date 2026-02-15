use ax_remote::Vfs;

pub async fn run(vfs: &Vfs, src: &str, dst: &str) -> Result<(), Box<dyn std::error::Error>> {
    let content = vfs.read(src).await?;
    vfs.write(dst, &content).await?;
    vfs.delete(src).await?;
    println!("Moved {} -> {}", src, dst);

    Ok(())
}
