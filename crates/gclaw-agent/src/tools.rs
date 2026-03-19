use async_trait::async_trait;
use gclaw_core::traits::Tool;
use gclaw_core::types::{ToolDefinition, ToolResult};
use gclaw_core::Result;
use serde_json::json;

// === ShellExecTool === (keep existing)

pub struct ShellExecTool;

#[async_trait]
impl Tool for ShellExecTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "shell_exec".to_string(),
            description: "Execute a shell command and return its output.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                gclaw_core::GclawError::ToolExecution("Missing 'command' parameter".to_string())
            })?;

        match tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let content = if stderr.is_empty() {
                    stdout.to_string()
                } else {
                    format!("{stdout}\nSTDERR:\n{stderr}")
                };
                Ok(ToolResult {
                    content,
                    is_error: !output.status.success(),
                })
            }
            Err(e) => Ok(ToolResult {
                content: format!("Failed to execute command: {e}"),
                is_error: true,
            }),
        }
    }
}

// === FileReadTool ===

pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "file_read".to_string(),
            description: "Read the contents of a file.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let path = input.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            gclaw_core::GclawError::ToolExecution("Missing 'path' parameter".to_string())
        })?;

        match tokio::fs::read_to_string(path).await {
            Ok(content) => Ok(ToolResult {
                content,
                is_error: false,
            }),
            Err(e) => Ok(ToolResult {
                content: format!("Failed to read file: {e}"),
                is_error: true,
            }),
        }
    }
}

// === FileWriteTool ===

pub struct FileWriteTool;

#[async_trait]
impl Tool for FileWriteTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "file_write".to_string(),
            description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let path = input.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            gclaw_core::GclawError::ToolExecution("Missing 'path' parameter".to_string())
        })?;
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                gclaw_core::GclawError::ToolExecution("Missing 'content' parameter".to_string())
            })?;

        // Create parent directories if needed
        if let Some(parent) = std::path::Path::new(path).parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(ToolResult {
                    content: format!("Failed to create directories: {e}"),
                    is_error: true,
                });
            }
        }

        match tokio::fs::write(path, content).await {
            Ok(()) => Ok(ToolResult {
                content: format!("Wrote {} bytes to {path}", content.len()),
                is_error: false,
            }),
            Err(e) => Ok(ToolResult {
                content: format!("Failed to write file: {e}"),
                is_error: true,
            }),
        }
    }
}

// === WebFetchTool ===
// Fetches a URL and returns its text content.

pub struct WebFetchTool {
    client: reqwest::Client,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "web_fetch".to_string(),
            description: "Fetch the content of a URL. Returns the response body as text."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch"
                    }
                },
                "required": ["url"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let url = input.get("url").and_then(|v| v.as_str()).ok_or_else(|| {
            gclaw_core::GclawError::ToolExecution("Missing 'url' parameter".to_string())
        })?;

        match self.client.get(url).send().await {
            Ok(resp) => {
                let status = resp.status();
                match resp.text().await {
                    Ok(body) => {
                        // Truncate to 50KB to avoid blowing up context
                        let truncated = if body.len() > 50_000 {
                            format!(
                                "{}...\n[truncated, {} total bytes]",
                                &body[..50_000],
                                body.len()
                            )
                        } else {
                            body
                        };
                        Ok(ToolResult {
                            content: format!("HTTP {status}\n\n{truncated}"),
                            is_error: !status.is_success(),
                        })
                    }
                    Err(e) => Ok(ToolResult {
                        content: format!("Failed to read response body: {e}"),
                        is_error: true,
                    }),
                }
            }
            Err(e) => Ok(ToolResult {
                content: format!("Request failed: {e}"),
                is_error: true,
            }),
        }
    }
}

// === ListDirTool ===

pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_dir".to_string(),
            description: "List the contents of a directory.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the directory (default: current directory)"
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        match tokio::fs::read_dir(path).await {
            Ok(mut entries) => {
                let mut items = Vec::new();
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let meta = entry.metadata().await;
                    let suffix = match meta {
                        Ok(m) if m.is_dir() => "/",
                        Ok(m) if m.is_symlink() => "@",
                        _ => "",
                    };
                    items.push(format!("{name}{suffix}"));
                }
                items.sort();
                Ok(ToolResult {
                    content: items.join("\n"),
                    is_error: false,
                })
            }
            Err(e) => Ok(ToolResult {
                content: format!("Failed to list directory: {e}"),
                is_error: true,
            }),
        }
    }
}
