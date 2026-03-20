use crate::protocol::*;
use crate::transport::SerialTransport;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;
use tracing::{debug, info};

/// MCP protocol version we advertise.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// MCP client that manages the lifecycle of one MCP server.
pub struct McpClient {
    server_name: String,
    transport: Mutex<SerialTransport>,
    next_id: AtomicU64,
    tools: Vec<McpToolInfo>,
}

impl McpClient {
    /// Spawn the server process and perform the MCP initialize handshake.
    pub async fn connect(
        server_name: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> gclaw_core::Result<Self> {
        let transport = SerialTransport::spawn(command, args, env)?;
        let mut client = Self {
            server_name: server_name.to_string(),
            transport: Mutex::new(transport),
            next_id: AtomicU64::new(1),
            tools: Vec::new(),
        };

        client.initialize().await?;
        client.tools = client.list_tools().await?;

        info!(
            server = server_name,
            tools = client.tools.len(),
            "MCP server connected"
        );

        Ok(client)
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Perform the initialize handshake.
    async fn initialize(&self) -> gclaw_core::Result<()> {
        let params = InitializeParams {
            protocol_version: PROTOCOL_VERSION.to_string(),
            capabilities: ClientCapabilities {},
            client_info: ClientInfo {
                name: "gclaw".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let req = JsonRpcRequest::new(
            self.next_id(),
            "initialize",
            Some(serde_json::to_value(&params).unwrap()),
        );

        let resp = self.transport.lock().await.request(&req).await?;

        if let Some(err) = resp.error {
            return Err(gclaw_core::GclawError::Provider(format!(
                "MCP initialize error: {} ({})",
                err.message, err.code
            )));
        }

        if let Some(result) = resp.result {
            if let Ok(init_result) = serde_json::from_value::<InitializeResult>(result) {
                let server_name = init_result
                    .server_info
                    .as_ref()
                    .map(|s| s.name.as_str())
                    .unwrap_or("unknown");
                debug!(
                    server = server_name,
                    protocol = %init_result.protocol_version,
                    "MCP server initialized"
                );
            }
        }

        // Send initialized notification
        let notif = JsonRpcNotification::new("notifications/initialized", None);
        self.transport.lock().await.notify(&notif).await?;

        Ok(())
    }

    /// List available tools from the server.
    async fn list_tools(&self) -> gclaw_core::Result<Vec<McpToolInfo>> {
        let req = JsonRpcRequest::new(self.next_id(), "tools/list", None);
        let resp = self.transport.lock().await.request(&req).await?;

        if let Some(err) = resp.error {
            return Err(gclaw_core::GclawError::Provider(format!(
                "MCP tools/list error: {} ({})",
                err.message, err.code
            )));
        }

        let result = resp.result.ok_or_else(|| {
            gclaw_core::GclawError::Provider("MCP tools/list returned no result".to_string())
        })?;

        let tools_result: ToolsListResult = serde_json::from_value(result).map_err(|e| {
            gclaw_core::GclawError::Provider(format!("Failed to parse tools/list result: {e}"))
        })?;

        Ok(tools_result.tools)
    }

    /// Call a tool on the server.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> gclaw_core::Result<gclaw_core::types::ToolResult> {
        let params = ToolsCallParams {
            name: name.to_string(),
            arguments,
        };

        let req = JsonRpcRequest::new(
            self.next_id(),
            "tools/call",
            Some(serde_json::to_value(&params).unwrap()),
        );

        let resp = self.transport.lock().await.request(&req).await?;

        if let Some(err) = resp.error {
            return Ok(gclaw_core::types::ToolResult {
                content: format!("MCP error: {} ({})", err.message, err.code),
                is_error: true,
            });
        }

        let result = resp.result.ok_or_else(|| {
            gclaw_core::GclawError::Provider("MCP tools/call returned no result".to_string())
        })?;

        let call_result: ToolsCallResult = serde_json::from_value(result).map_err(|e| {
            gclaw_core::GclawError::Provider(format!("Failed to parse tools/call result: {e}"))
        })?;

        // Concatenate all text content blocks
        let content = call_result
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(gclaw_core::types::ToolResult {
            content,
            is_error: call_result.is_error,
        })
    }

    /// Return the server name.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Return the discovered tools.
    pub fn tools(&self) -> &[McpToolInfo] {
        &self.tools
    }

    /// Kill the server process.
    pub fn shutdown(&self) {
        // Transport's Drop will kill the child process
    }
}
