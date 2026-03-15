use crate::types::{ToolDefinition, ToolResult};
use crate::Result;
use async_trait::async_trait;

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult>;
}
