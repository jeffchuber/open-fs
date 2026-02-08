use ax_core::Vfs;
use regex::Regex;

pub async fn run(
    vfs: &Vfs,
    path: Option<String>,
    pattern: &str,
    file_type: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.as_deref().unwrap_or("/");
    let regex = Regex::new(pattern)?;
    let type_filter = file_type.as_deref();

    find_recursive(vfs, path, &regex, type_filter).await?;

    Ok(())
}

#[async_recursion::async_recursion]
async fn find_recursive(
    vfs: &Vfs,
    path: &str,
    pattern: &Regex,
    type_filter: Option<&str>,
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

        let matches_type = match type_filter {
            Some("f") | Some("file") => !entry.is_dir,
            Some("d") | Some("dir") => entry.is_dir,
            _ => true,
        };

        if matches_type && pattern.is_match(&entry.name) {
            println!("{}", full_path);
        }

        if entry.is_dir {
            find_recursive(vfs, &full_path, pattern, type_filter).await?;
        }
    }

    Ok(())
}
