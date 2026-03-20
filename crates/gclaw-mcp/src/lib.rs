pub mod client;
pub mod protocol;
pub mod tool;
pub mod transport;

use client::McpClient;
use gclaw_core::config::McpServerConfig;
use gclaw_core::traits::Tool;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info};

/// Start all configured MCP servers and return their tools.
///
/// Failed servers are logged and skipped — never crash gclaw.
pub async fn start_mcp_servers(servers: &HashMap<String, McpServerConfig>) -> Vec<Arc<dyn Tool>> {
    let mut all_tools = Vec::new();

    for (name, config) in servers {
        info!(server = name, command = %config.command, "Starting MCP server");

        match McpClient::connect(name, &config.command, &config.args, &config.env).await {
            Ok(client) => {
                let client = Arc::new(client);
                let tools = tool::tools_from_client(client);
                info!(server = name, tools = tools.len(), "MCP server ready");
                all_tools.extend(tools);
            }
            Err(e) => {
                error!(server = name, error = %e, "Failed to start MCP server, skipping");
            }
        }
    }

    all_tools
}
