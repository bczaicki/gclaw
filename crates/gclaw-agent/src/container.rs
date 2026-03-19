use crate::executor::ToolExecutor;
use gclaw_core::config::ContainerConfig;
use gclaw_core::types::{ToolCall, ToolDefinition, ToolResult};
use gclaw_core::GclawError;
use gclaw_core::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Default timeout for containerized command execution.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Wraps a `ToolExecutor` and optionally routes `shell_exec` calls through
/// Docker (or Podman) containers for sandboxed execution.
pub struct ContainerExecutor {
    config: ContainerConfig,
    inner: ToolExecutor,
    timeout: Duration,
}

impl ContainerExecutor {
    /// Create a new `ContainerExecutor`.
    ///
    /// If `config.enabled` is `false`, all calls are delegated directly to the
    /// inner `ToolExecutor` without any containerization.
    pub fn new(config: ContainerConfig, inner: ToolExecutor) -> Self {
        Self {
            config,
            inner,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Override the default execution timeout for containerized commands.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Register a tool with the inner executor.
    pub fn register(&mut self, tool: Arc<dyn gclaw_core::traits::Tool>) {
        self.inner.register(tool);
    }

    /// Return tool definitions from the inner executor.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.inner.definitions()
    }

    /// Ensure the per-conversation workspace directory exists and return its path.
    ///
    /// The workspace is created at `/tmp/gclaw-workspaces/{conversation_id}/`.
    pub fn ensure_workspace(&self, conversation_id: &str) -> Result<PathBuf> {
        let workspace = PathBuf::from("/tmp/gclaw-workspaces").join(conversation_id);
        std::fs::create_dir_all(&workspace)?;
        debug!(path = %workspace.display(), "ensured workspace directory");
        Ok(workspace)
    }

    /// Execute a tool call, routing `shell_exec` through a container when enabled.
    ///
    /// * If containerization is disabled, delegates directly to the inner executor.
    /// * If the tool is not `shell_exec`, delegates directly to the inner executor.
    /// * Otherwise, runs the command inside a sandboxed container.
    pub async fn execute(
        &self,
        call: &ToolCall,
        conversation_id: Option<&str>,
    ) -> Result<ToolResult> {
        // Bypass container for non-shell tools or when containers are disabled.
        if !self.config.enabled || call.name != "shell_exec" {
            return self.inner.execute(call).await;
        }

        let conversation_id = conversation_id.unwrap_or("default");
        let workspace = self.ensure_workspace(conversation_id)?;

        let command = call
            .arguments
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GclawError::ToolExecution("Missing 'command' parameter".to_string()))?;

        info!(
            runtime = %self.config.runtime,
            image = %self.config.image,
            conversation_id = %conversation_id,
            "executing shell command in container"
        );
        debug!(command = %command, "container command");

        let result =
            tokio::time::timeout(self.timeout, self.run_in_container(command, &workspace)).await;

        match result {
            Ok(inner_result) => inner_result,
            Err(_) => {
                warn!(
                    timeout_secs = self.timeout.as_secs(),
                    "container execution timed out"
                );
                Ok(ToolResult {
                    content: format!("Command timed out after {} seconds", self.timeout.as_secs()),
                    is_error: true,
                })
            }
        }
    }

    /// Spawn the container runtime process and collect output.
    async fn run_in_container(
        &self,
        command: &str,
        workspace: &std::path::Path,
    ) -> Result<ToolResult> {
        let workspace_mount = format!("{}:/workspace", workspace.display());

        let output = tokio::process::Command::new(&self.config.runtime)
            .arg("run")
            .arg("--rm")
            .arg("--network=none")
            .arg("--memory=512m")
            .arg("--cpus=1")
            .arg("--read-only")
            .arg("-v")
            .arg(&workspace_mount)
            .arg("-w")
            .arg("/workspace")
            .arg(&self.config.image)
            .arg("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await;

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let content = if stderr.is_empty() {
                    stdout.to_string()
                } else {
                    format!("{stdout}\nSTDERR:\n{stderr}")
                };
                debug!(
                    exit_code = output.status.code(),
                    "container execution finished"
                );
                Ok(ToolResult {
                    content,
                    is_error: !output.status.success(),
                })
            }
            Err(e) => {
                warn!(error = %e, "failed to start container runtime");
                Ok(ToolResult {
                    content: format!("Failed to execute container command: {e}"),
                    is_error: true,
                })
            }
        }
    }
}
