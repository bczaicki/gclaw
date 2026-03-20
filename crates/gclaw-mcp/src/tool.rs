use crate::client::McpClient;
use async_trait::async_trait;
use gclaw_core::traits::Tool;
use gclaw_core::types::{ToolDefinition, ToolResult};
use std::sync::Arc;

/// Wraps a single MCP server tool as a gclaw Tool.
/// Tool name is prefixed as `mcp__{server}__{tool}` to avoid collisions.
pub struct McpTool {
    client: Arc<McpClient>,
    /// The prefixed name: mcp__{server}__{tool}
    prefixed_name: String,
    /// The original tool name on the MCP server
    original_name: String,
    description: String,
    parameters: serde_json::Value,
}

impl McpTool {
    pub fn new(
        client: Arc<McpClient>,
        server_name: &str,
        original_name: &str,
        description: Option<&str>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            client,
            prefixed_name: format!("mcp__{server_name}__{original_name}"),
            original_name: original_name.to_string(),
            description: description.unwrap_or("MCP tool").to_string(),
            parameters,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.prefixed_name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> gclaw_core::Result<ToolResult> {
        self.client.call_tool(&self.original_name, input).await
    }
}

/// Create McpTool wrappers for all tools discovered on a client.
pub fn tools_from_client(client: Arc<McpClient>) -> Vec<Arc<dyn Tool>> {
    let server_name = client.server_name().to_string();
    client
        .tools()
        .iter()
        .map(|info| {
            Arc::new(McpTool::new(
                client.clone(),
                &server_name,
                &info.name,
                info.description.as_deref(),
                info.input_schema.clone(),
            )) as Arc<dyn Tool>
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_prefixing() {
        // Can't create a real McpClient in unit tests, but we can test the naming logic
        let prefixed = format!("mcp__{}__{}", "filesystem", "read_file");
        assert_eq!(prefixed, "mcp__filesystem__read_file");
    }

    #[test]
    fn tool_name_with_special_chars() {
        let prefixed = format!("mcp__{}__{}", "my-server", "list-dir");
        assert_eq!(prefixed, "mcp__my-server__list-dir");
    }
}
