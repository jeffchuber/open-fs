use ax_remote::Vfs;
use std::io::{self, Read};

pub async fn run(
    vfs: &Vfs,
    path: &str,
    content: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = match content {
        Some(c) => c.into_bytes(),
        None => {
            // Read from stdin
            let mut buffer = Vec::new();
            io::stdin().read_to_end(&mut buffer)?;
            buffer
        }
    };

    vfs.append(path, &content).await?;
    println!("Appended {} bytes to {}", content.len(), path);

    Ok(())
}
