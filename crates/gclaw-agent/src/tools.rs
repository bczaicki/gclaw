use async_trait::async_trait;
use gclaw_core::traits::Tool;
use gclaw_core::types::{ToolDefinition, ToolResult};
use gclaw_core::Result;
use serde_json::json;

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
