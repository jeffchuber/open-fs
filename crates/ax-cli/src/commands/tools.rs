use ax_core::{generate_tools, format_tools, ToolFormat, Vfs};

pub async fn run(
    vfs: &Vfs,
    format: Option<String>,
    pretty: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = vfs.effective_config();

    // Parse format
    let tool_format: ToolFormat = format
        .as_deref()
        .unwrap_or("json")
        .parse()
        .map_err(|e: String| e)?;

    // Generate tools
    let tools = generate_tools(config);

    // Format output
    let output = format_tools(&tools, tool_format);

    // Print
    if pretty {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", serde_json::to_string(&output)?);
    }

    Ok(())
}
