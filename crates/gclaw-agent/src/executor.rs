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
