use ax_remote::Vfs;

pub async fn run(vfs: &Vfs) -> Result<(), Box<dyn std::error::Error>> {
    let config = vfs.effective_config();

    // Print as YAML for readability
    let yaml = serde_yaml::to_string(config)?;
    println!("{}", yaml);

    Ok(())
}
