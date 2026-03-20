use crate::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tracing::debug;

/// Expand `${env:VAR_NAME}` patterns in a string.
pub fn expand_env_var(value: &str) -> String {
    let mut result = value.to_string();
    while let Some(start) = result.find("${env:") {
        if let Some(end) = result[start..].find('}') {
            let var_name = &result[start + 6..start + end];
            let var_value = std::env::var(var_name).unwrap_or_default();
            result = format!(
                "{}{}{}",
                &result[..start],
                var_value,
                &result[start + end + 1..]
            );
        } else {
            break;
        }
    }
    result
}

/// Serial stdio transport for an MCP server child process.
/// Sends one request at a time and reads the response.
pub struct SerialTransport {
    stdin: ChildStdin,
    reader: tokio::sync::Mutex<tokio::io::Lines<BufReader<ChildStdout>>>,
    child: Option<Child>,
}

impl SerialTransport {
    /// Spawn the child process.
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> gclaw_core::Result<Self> {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        for (key, value) in env {
            let expanded = expand_env_var(value);
            cmd.env(key, expanded);
        }

        let mut child = cmd.spawn().map_err(|e| {
            gclaw_core::GclawError::Provider(format!("Failed to spawn MCP server '{command}': {e}"))
        })?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = BufReader::new(stdout);

        Ok(Self {
            stdin,
            reader: tokio::sync::Mutex::new(reader.lines()),
            child: Some(child),
        })
    }

    /// Send a JSON-RPC request and read the response.
    pub async fn request(&mut self, req: &JsonRpcRequest) -> gclaw_core::Result<JsonRpcResponse> {
        let mut json = serde_json::to_string(req).map_err(|e| {
            gclaw_core::GclawError::Provider(format!("Failed to serialize request: {e}"))
        })?;
        json.push('\n');

        self.stdin.write_all(json.as_bytes()).await.map_err(|e| {
            gclaw_core::GclawError::Provider(format!("Failed to write to MCP server: {e}"))
        })?;
        self.stdin.flush().await.map_err(|e| {
            gclaw_core::GclawError::Provider(format!("Failed to flush MCP server stdin: {e}"))
        })?;

        self.read_response(req.id).await
    }

    /// Send a notification (no response expected).
    pub async fn notify(&mut self, notif: &JsonRpcNotification) -> gclaw_core::Result<()> {
        let mut json = serde_json::to_string(notif).map_err(|e| {
            gclaw_core::GclawError::Provider(format!("Failed to serialize notification: {e}"))
        })?;
        json.push('\n');

        self.stdin.write_all(json.as_bytes()).await.map_err(|e| {
            gclaw_core::GclawError::Provider(format!("Failed to write to MCP server: {e}"))
        })?;
        self.stdin.flush().await.map_err(|e| {
            gclaw_core::GclawError::Provider(format!("Failed to flush MCP server stdin: {e}"))
        })?;
        Ok(())
    }

    /// Read lines until we get a response matching the given request id.
    async fn read_response(&self, expected_id: u64) -> gclaw_core::Result<JsonRpcResponse> {
        let timeout = tokio::time::Duration::from_secs(30);
        let mut reader = self.reader.lock().await;
        let start = tokio::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(gclaw_core::GclawError::Provider(
                    "MCP server response timeout".to_string(),
                ));
            }

            let line = tokio::time::timeout(timeout - start.elapsed(), reader.next_line())
                .await
                .map_err(|_| {
                    gclaw_core::GclawError::Provider("MCP server response timeout".to_string())
                })?
                .map_err(|e| {
                    gclaw_core::GclawError::Provider(format!("Failed to read from MCP server: {e}"))
                })?;

            let line = match line {
                Some(l) => l,
                None => {
                    return Err(gclaw_core::GclawError::Provider(
                        "MCP server stdout closed".to_string(),
                    ));
                }
            };

            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<JsonRpcResponse>(&line) {
                Ok(resp) if resp.id == Some(expected_id) => return Ok(resp),
                Ok(resp) => {
                    debug!(
                        "Ignoring MCP response with unexpected id {:?} (expected {})",
                        resp.id, expected_id
                    );
                }
                Err(_) => {
                    debug!("Ignoring non-response MCP line");
                }
            }
        }
    }

    pub fn kill(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.start_kill();
        }
    }
}

impl Drop for SerialTransport {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_env_var_simple() {
        std::env::set_var("GCLAW_TEST_VAR", "hello");
        assert_eq!(expand_env_var("${env:GCLAW_TEST_VAR}"), "hello");
        std::env::remove_var("GCLAW_TEST_VAR");
    }

    #[test]
    fn expand_env_var_with_prefix() {
        std::env::set_var("GCLAW_TEST_VAR2", "world");
        assert_eq!(
            expand_env_var("prefix_${env:GCLAW_TEST_VAR2}_suffix"),
            "prefix_world_suffix"
        );
        std::env::remove_var("GCLAW_TEST_VAR2");
    }

    #[test]
    fn expand_env_var_missing() {
        assert_eq!(expand_env_var("${env:GCLAW_NONEXISTENT_VAR_12345}"), "");
    }

    #[test]
    fn expand_env_var_no_pattern() {
        assert_eq!(expand_env_var("just a string"), "just a string");
    }

    #[test]
    fn expand_env_var_multiple() {
        std::env::set_var("GCLAW_A", "1");
        std::env::set_var("GCLAW_B", "2");
        assert_eq!(expand_env_var("${env:GCLAW_A}-${env:GCLAW_B}"), "1-2");
        std::env::remove_var("GCLAW_A");
        std::env::remove_var("GCLAW_B");
    }
}
