use ax_remote::Vfs;

pub async fn run(vfs: &Vfs, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let exists = vfs.exists(path).await?;

    if exists {
        println!("{} exists", path);
        std::process::exit(0);
    } else {
        println!("{} does not exist", path);
        std::process::exit(1);
    }
}
