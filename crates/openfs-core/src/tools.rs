use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use openfs_config::VfsConfig;

/// A tool parameter definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParameter {
    /// Parameter name.
    pub name: String,
    /// Parameter description.
    pub description: String,
    /// Parameter type (string, integer, boolean, array, object).
    #[serde(rename = "type")]
    pub param_type: String,
    /// Whether this parameter is required.
    pub required: bool,
    /// Enum values if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    /// Default value if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
}

/// A tool definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// Tool parameters.
    pub parameters: Vec<ToolParameter>,
}

/// Output format for tool definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolFormat {
    /// JSON format (generic).
    #[default]
    Json,
    /// MCP format for Claude/Anthropic.
    Mcp,
    /// OpenAI function calling format.
    OpenAi,
}

impl std::str::FromStr for ToolFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "json" => Ok(ToolFormat::Json),
            "mcp" => Ok(ToolFormat::Mcp),
            "openai" => Ok(ToolFormat::OpenAi),
            _ => Err(format!("Unknown format: {}. Use json, mcp, or openai", s)),
        }
    }
}

/// Generate tool definitions from VFS config.
pub fn generate_tools(config: &VfsConfig) -> Vec<ToolDefinition> {
    let mut tools = Vec::new();

    // Get mount paths for enum values
    let mount_paths: Vec<String> = config.mounts.iter().map(|m| m.path.clone()).collect();

    // Core file operations
    tools.push(ToolDefinition {
        name: "vfs_read".to_string(),
        description: "Read the contents of a file from the virtual filesystem".to_string(),
        parameters: vec![ToolParameter {
            name: "path".to_string(),
            description: "The path to the file to read".to_string(),
            param_type: "string".to_string(),
            required: true,
            enum_values: None,
            default: None,
        }],
    });

    tools.push(ToolDefinition {
        name: "vfs_write".to_string(),
        description: "Write content to a file in the virtual filesystem".to_string(),
        parameters: vec![
            ToolParameter {
                name: "path".to_string(),
                description: "The path to the file to write".to_string(),
                param_type: "string".to_string(),
                required: true,
                enum_values: None,
                default: None,
            },
            ToolParameter {
                name: "content".to_string(),
                description: "The content to write to the file".to_string(),
                param_type: "string".to_string(),
                required: true,
                enum_values: None,
                default: None,
            },
        ],
    });

    tools.push(ToolDefinition {
        name: "vfs_append".to_string(),
        description: "Append content to a file in the virtual filesystem".to_string(),
        parameters: vec![
            ToolParameter {
                name: "path".to_string(),
                description: "The path to the file to append to".to_string(),
                param_type: "string".to_string(),
                required: true,
                enum_values: None,
                default: None,
            },
            ToolParameter {
                name: "content".to_string(),
                description: "The content to append".to_string(),
                param_type: "string".to_string(),
                required: true,
                enum_values: None,
                default: None,
            },
        ],
    });

    tools.push(ToolDefinition {
        name: "vfs_delete".to_string(),
        description: "Delete a file from the virtual filesystem".to_string(),
        parameters: vec![ToolParameter {
            name: "path".to_string(),
            description: "The path to the file to delete".to_string(),
            param_type: "string".to_string(),
            required: true,
            enum_values: None,
            default: None,
        }],
    });

    tools.push(ToolDefinition {
        name: "vfs_list".to_string(),
        description: "List files and directories in a path".to_string(),
        parameters: vec![ToolParameter {
            name: "path".to_string(),
            description: "The directory path to list".to_string(),
            param_type: "string".to_string(),
            required: true,
            enum_values: None,
            default: None,
        }],
    });

    tools.push(ToolDefinition {
        name: "vfs_exists".to_string(),
        description: "Check if a path exists in the virtual filesystem".to_string(),
        parameters: vec![ToolParameter {
            name: "path".to_string(),
            description: "The path to check".to_string(),
            param_type: "string".to_string(),
            required: true,
            enum_values: None,
            default: None,
        }],
    });

    tools.push(ToolDefinition {
        name: "vfs_stat".to_string(),
        description: "Get metadata about a file or directory".to_string(),
        parameters: vec![ToolParameter {
            name: "path".to_string(),
            description: "The path to get metadata for".to_string(),
            param_type: "string".to_string(),
            required: true,
            enum_values: None,
            default: None,
        }],
    });

    // Search tool (if any mount has indexing)
    let has_indexing = config.mounts.iter().any(|m| m.index.is_some());
    if has_indexing {
        tools.push(ToolDefinition {
            name: "vfs_search".to_string(),
            description: "Search for files by content using semantic search".to_string(),
            parameters: vec![
                ToolParameter {
                    name: "query".to_string(),
                    description: "The search query".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                    enum_values: None,
                    default: None,
                },
                ToolParameter {
                    name: "path".to_string(),
                    description: "Optional path prefix to limit search scope".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                    enum_values: None,
                    default: None,
                },
                ToolParameter {
                    name: "limit".to_string(),
                    description: "Maximum number of results to return".to_string(),
                    param_type: "integer".to_string(),
                    required: false,
                    enum_values: None,
                    default: Some(serde_json::json!(10)),
                },
            ],
        });
    }

    // Mount info tool
    if !mount_paths.is_empty() {
        tools.push(ToolDefinition {
            name: "vfs_mounts".to_string(),
            description: "List available mount points in the virtual filesystem".to_string(),
            parameters: vec![],
        });
    }

    tools
}

/// Convert tools to MCP format.
pub fn to_mcp_format(tools: &[ToolDefinition]) -> serde_json::Value {
    let mcp_tools: Vec<serde_json::Value> = tools
        .iter()
        .map(|tool| {
            let properties: HashMap<String, serde_json::Value> = tool
                .parameters
                .iter()
                .map(|p| {
                    let mut prop = serde_json::json!({
                        "type": p.param_type,
                        "description": p.description,
                    });

                    if let Some(enum_vals) = &p.enum_values {
                        prop["enum"] = serde_json::json!(enum_vals);
                    }

                    (p.name.clone(), prop)
                })
                .collect();

            let required: Vec<String> = tool
                .parameters
                .iter()
                .filter(|p| p.required)
                .map(|p| p.name.clone())
                .collect();

            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                }
            })
        })
        .collect();

    serde_json::json!({
        "tools": mcp_tools
    })
}

/// Convert tools to OpenAI function calling format.
pub fn to_openai_format(tools: &[ToolDefinition]) -> serde_json::Value {
    let functions: Vec<serde_json::Value> = tools
        .iter()
        .map(|tool| {
            let properties: HashMap<String, serde_json::Value> = tool
                .parameters
                .iter()
                .map(|p| {
                    let mut prop = serde_json::json!({
                        "type": p.param_type,
                        "description": p.description,
                    });

                    if let Some(enum_vals) = &p.enum_values {
                        prop["enum"] = serde_json::json!(enum_vals);
                    }

                    if let Some(default) = &p.default {
                        prop["default"] = default.clone();
                    }

                    (p.name.clone(), prop)
                })
                .collect();

            let required: Vec<String> = tool
                .parameters
                .iter()
                .filter(|p| p.required)
                .map(|p| p.name.clone())
                .collect();

            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": {
                        "type": "object",
                        "properties": properties,
                        "required": required,
                    }
                }
            })
        })
        .collect();

    serde_json::json!({
        "tools": functions
    })
}

/// Convert tools to JSON format (generic).
pub fn to_json_format(tools: &[ToolDefinition]) -> serde_json::Value {
    serde_json::json!({
        "tools": tools
    })
}

/// Format tools according to the specified format.
pub fn format_tools(tools: &[ToolDefinition], format: ToolFormat) -> serde_json::Value {
    match format {
        ToolFormat::Json => to_json_format(tools),
        ToolFormat::Mcp => to_mcp_format(tools),
        ToolFormat::OpenAi => to_openai_format(tools),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openfs_config::{BackendConfig, FsBackendConfig, MountConfig};
    use indexmap::IndexMap;

    fn test_config() -> VfsConfig {
        let mut backends = IndexMap::new();
        backends.insert(
            "local".to_string(),
            BackendConfig::Fs(FsBackendConfig {
                root: "./data".to_string(),
            }),
        );

        VfsConfig {
            name: Some("test".to_string()),
            version: None,
            backends,
            mounts: vec![MountConfig {
                path: "/workspace".to_string(),
                backend: Some("local".to_string()),
                collection: None,
                mode: None,
                read_only: false,
                index: None,
                sync: None,
                watch: None,
            }],
            defaults: None,
        }
    }

    #[test]
    fn test_generate_tools() {
        let config = test_config();
        let tools = generate_tools(&config);

        // Should have core tools
        assert!(tools.iter().any(|t| t.name == "vfs_read"));
        assert!(tools.iter().any(|t| t.name == "vfs_write"));
        assert!(tools.iter().any(|t| t.name == "vfs_list"));
        assert!(tools.iter().any(|t| t.name == "vfs_delete"));
        assert!(tools.iter().any(|t| t.name == "vfs_mounts"));
    }

    #[test]
    fn test_mcp_format() {
        let config = test_config();
        let tools = generate_tools(&config);
        let mcp = to_mcp_format(&tools);

        assert!(mcp.get("tools").is_some());
        let tools_array = mcp["tools"].as_array().unwrap();
        assert!(!tools_array.is_empty());

        // Check first tool has required fields
        let first = &tools_array[0];
        assert!(first.get("name").is_some());
        assert!(first.get("description").is_some());
        assert!(first.get("input_schema").is_some());
    }

    #[test]
    fn test_openai_format() {
        let config = test_config();
        let tools = generate_tools(&config);
        let openai = to_openai_format(&tools);

        assert!(openai.get("tools").is_some());
        let tools_array = openai["tools"].as_array().unwrap();
        assert!(!tools_array.is_empty());

        // Check first tool has required fields
        let first = &tools_array[0];
        assert_eq!(first["type"], "function");
        assert!(first.get("function").is_some());
        assert!(first["function"].get("name").is_some());
        assert!(first["function"].get("parameters").is_some());
    }

    #[test]
    fn test_tool_format_from_str() {
        assert_eq!("json".parse::<ToolFormat>().unwrap(), ToolFormat::Json);
        assert_eq!("mcp".parse::<ToolFormat>().unwrap(), ToolFormat::Mcp);
        assert_eq!("openai".parse::<ToolFormat>().unwrap(), ToolFormat::OpenAi);
        assert!("invalid".parse::<ToolFormat>().is_err());
    }
}
