use ax_core::Vfs;

pub async fn run(
    vfs: &Vfs,
    path: Option<String>,
    max_depth: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.as_deref().unwrap_or("/");
    let max_depth = max_depth.unwrap_or(usize::MAX);

    println!("{}", path);
    print_tree(vfs, path, "", true, 0, max_depth).await?;

    Ok(())
}

#[async_recursion::async_recursion]
async fn print_tree(
    vfs: &Vfs,
    path: &str,
    prefix: &str,
    _is_last: bool,
    depth: usize,
    max_depth: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    if depth >= max_depth {
        return Ok(());
    }

    let entries = match vfs.list(path).await {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    let count = entries.len();
    for (i, entry) in entries.iter().enumerate() {
        let is_last_entry = i == count - 1;
        let connector = if is_last_entry { "└── " } else { "├── " };

        println!("{}{}{}", prefix, connector, entry.name);

        if entry.is_dir {
            let new_prefix = format!("{}{}", prefix, if is_last_entry { "    " } else { "│   " });
            let child_path = if path == "/" {
                format!("/{}", entry.name)
            } else {
                format!("{}/{}", path, entry.name)
            };
            print_tree(vfs, &child_path, &new_prefix, is_last_entry, depth + 1, max_depth).await?;
        }
    }

    Ok(())
}
