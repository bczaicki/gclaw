use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub message: Message,
    pub model: String,
    pub done: bool,
}

#[derive(Debug, Clone)]
pub struct StreamDelta {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub done: bool,
}

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub name: String,
    pub size: Option<u64>,
    pub parameters: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct InboundMessage {
    pub channel_name: String,
    pub conversation_id: String,
    pub sender: String,
    pub content: String,
    /// When set, injected as a system message for skill invocations.
    pub skill_context: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub channel_name: String,
    pub conversation_id: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    ThinkStart,
    ThinkDelta(String),
    ThinkEnd,
    StreamDelta(String),
    ToolCallStart {
        name: String,
        id: String,
    },
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
    Done(String),
    Error(String),
}
