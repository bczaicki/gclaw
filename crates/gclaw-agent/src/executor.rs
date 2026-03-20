use gclaw_core::traits::Tool;
use gclaw_core::types::{ToolCall, ToolDefinition, ToolResult};
use gclaw_core::Result;
use std::collections::HashMap;
use std::sync::Arc;

pub struct ToolExecutor {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolExecutor {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let def = tool.definition();
        self.tools.insert(def.name.clone(), tool);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    pub async fn execute(&self, call: &ToolCall) -> Result<ToolResult> {
        match self.tools.get(&call.name) {
            Some(tool) => tool.execute(call.arguments.clone()).await,
            None => Ok(ToolResult {
                content: format!("Unknown tool: {}", call.name),
                is_error: true,
            }),
        }
    }
}

impl Default for ToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use gclaw_core::traits::Tool;
    use gclaw_core::types::{ToolCall, ToolDefinition, ToolResult};
    use std::sync::Arc;

    /// A simple mock tool that echoes back the input JSON.
    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "echo".to_string(),
                description: "Echoes input".to_string(),
                parameters: serde_json::json!({}),
            }
        }

        async fn execute(&self, input: serde_json::Value) -> gclaw_core::Result<ToolResult> {
            Ok(ToolResult {
                content: input.to_string(),
                is_error: false,
            })
        }
    }

    #[test]
    fn register_adds_tool() {
        let mut executor = ToolExecutor::new();
        assert!(executor.definitions().is_empty());

        executor.register(Arc::new(EchoTool));
        let defs = executor.definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "echo");
    }

    #[tokio::test]
    async fn execute_registered_tool() {
        let mut executor = ToolExecutor::new();
        executor.register(Arc::new(EchoTool));

        let call = ToolCall {
            id: "call-1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({"msg": "hello"}),
        };

        let result = executor.execute(&call).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("hello"));
    }

    #[tokio::test]
    async fn execute_unknown_tool_returns_error() {
        let executor = ToolExecutor::new();

        let call = ToolCall {
            id: "call-2".to_string(),
            name: "nonexistent".to_string(),
            arguments: serde_json::json!({}),
        };

        let result = executor.execute(&call).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Unknown tool"));
        assert!(result.content.contains("nonexistent"));
    }
}
