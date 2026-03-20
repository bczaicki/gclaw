use async_trait::async_trait;
use gclaw_core::traits::Tool;
use gclaw_core::types::{ToolDefinition, ToolResult};
use gclaw_core::Result;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, warn};

/// A tool defined by a TOML manifest file.
///
/// Manifest format (`workspace/tools/<name>.toml`):
/// ```toml
/// name = "weather"
/// description = "Get current weather for a location"
/// command = "curl -s 'https://wttr.in/{{location}}?format=3'"
///
/// [parameters.properties.location]
/// type = "string"
/// description = "City name"
///
/// [parameters]
/// required = ["location"]
/// ```
#[derive(Deserialize, Debug, Clone)]
struct ToolManifest {
    name: String,
    description: String,
    command: String,
    #[serde(default)]
    parameters: serde_json::Value,
}

/// A tool loaded from a manifest that executes a shell command.
struct CommandTool {
    manifest: ToolManifest,
}

#[async_trait]
impl Tool for CommandTool {
    fn definition(&self) -> ToolDefinition {
        let params = if self.manifest.parameters.is_null() {
            serde_json::json!({
                "type": "object",
                "properties": {},
            })
        } else {
            let mut params = self.manifest.parameters.clone();
            if let Some(obj) = params.as_object_mut() {
                obj.entry("type")
                    .or_insert(serde_json::Value::String("object".to_string()));
            }
            params
        };

        ToolDefinition {
            name: self.manifest.name.clone(),
            description: self.manifest.description.clone(),
            parameters: params,
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        // Substitute {{param}} placeholders in the command template
        let mut command = self.manifest.command.clone();
        if let Some(obj) = input.as_object() {
            for (key, value) in obj {
                let placeholder = format!("{{{{{key}}}}}");
                let replacement = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                command = command.replace(&placeholder, &replacement);
            }
        }

        debug!(
            "Plugin tool '{}' executing: {}",
            self.manifest.name, command
        );

        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .output()
            .await
            .map_err(|e| gclaw_core::GclawError::ToolExecution(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(ToolResult {
                content: stdout,
                is_error: false,
            })
        } else {
            Ok(ToolResult {
                content: format!("{stdout}\n{stderr}").trim().to_string(),
                is_error: true,
            })
        }
    }
}

/// Load all tool plugins from a directory of TOML manifests.
pub fn load_plugins(dir: &Path) -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();

    if !dir.exists() {
        return tools;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!("Failed to read plugins directory {}: {e}", dir.display());
            return tools;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str::<ToolManifest>(&content) {
                Ok(manifest) => {
                    debug!(
                        "Loaded plugin tool '{}' from {}",
                        manifest.name,
                        path.display()
                    );
                    tools.push(Arc::new(CommandTool { manifest }));
                }
                Err(e) => {
                    warn!("Failed to parse tool manifest {}: {e}", path.display());
                }
            },
            Err(e) => {
                warn!("Failed to read tool manifest {}: {e}", path.display());
            }
        }
    }

    tools
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_plugins_from_directory() {
        let dir = TempDir::new().unwrap();
        let manifest = r#"
name = "greet"
description = "Say hello"
command = "echo Hello {{name}}"

[parameters.properties.name]
type = "string"
description = "Name to greet"

[parameters]
required = ["name"]
"#;
        let path = dir.path().join("greet.toml");
        std::fs::write(&path, manifest).unwrap();

        let tools = load_plugins(dir.path());
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].definition().name, "greet");
    }

    #[test]
    fn load_plugins_skips_invalid() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("bad.toml"), "not valid toml [[[").unwrap();
        std::fs::write(dir.path().join("readme.md"), "# Plugins").unwrap();

        let tools = load_plugins(dir.path());
        assert!(tools.is_empty());
    }

    #[test]
    fn load_plugins_empty_dir() {
        let dir = TempDir::new().unwrap();
        let tools = load_plugins(dir.path());
        assert!(tools.is_empty());
    }

    #[test]
    fn load_plugins_nonexistent_dir() {
        let tools = load_plugins(Path::new("/nonexistent/path"));
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn command_tool_executes() {
        let manifest = ToolManifest {
            name: "test".to_string(),
            description: "test".to_string(),
            command: "echo hello {{name}}".to_string(),
            parameters: serde_json::json!({}),
        };
        let tool = CommandTool { manifest };
        let result = tool
            .execute(serde_json::json!({"name": "world"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("hello world"));
    }

    #[tokio::test]
    async fn command_tool_reports_failure() {
        let manifest = ToolManifest {
            name: "fail".to_string(),
            description: "test".to_string(),
            command: "exit 1".to_string(),
            parameters: serde_json::json!({}),
        };
        let tool = CommandTool { manifest };
        let result = tool.execute(serde_json::json!({})).await.unwrap();
        assert!(result.is_error);
    }
}
