use ax_remote::Vfs;
use regex::Regex;

pub async fn run(
    vfs: &Vfs,
    pattern: &str,
    path: Option<String>,
    recursive: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.as_deref().unwrap_or("/");
    let regex = Regex::new(pattern)?;

    if recursive {
        grep_recursive(vfs, path, &regex).await?;
    } else {
        // Single file
        grep_file(vfs, path, &regex).await?;
    }

    Ok(())
}

async fn grep_file(vfs: &Vfs, path: &str, pattern: &Regex) -> Result<(), Box<dyn std::error::Error>> {
    let content = match vfs.read(path).await {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    let text = match std::str::from_utf8(&content) {
        Ok(t) => t,
        Err(_) => return Ok(()), // Skip binary files
    };

    for (line_num, line) in text.lines().enumerate() {
        if pattern.is_match(line) {
            println!("{}:{}:{}", path, line_num + 1, line);
        }
    }

    Ok(())
}

#[async_recursion::async_recursion]
async fn grep_recursive(
    vfs: &Vfs,
    path: &str,
    pattern: &Regex,
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = match vfs.list(path).await {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries {
        let full_path = if path == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", path, entry.name)
        };

        if entry.is_dir {
            grep_recursive(vfs, &full_path, pattern).await?;
        } else {
            grep_file(vfs, &full_path, pattern).await?;
        }
    }

    Ok(())
}
