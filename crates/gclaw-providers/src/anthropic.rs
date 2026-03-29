// Serde deserialization structs have fields that aren't directly read in Rust.
#![allow(dead_code, clippy::enum_variant_names)]

use async_trait::async_trait;
use futures::Stream;
use gclaw_core::traits::LlmProvider;
use gclaw_core::types::*;
use gclaw_core::{GclawError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::time::Duration;
use tracing::debug;

// ---------------------------------------------------------------------------
// Anthropic Messages API serde types
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct CacheControl {
    #[serde(rename = "type")]
    cache_type: String,
}

impl CacheControl {
    fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
        }
    }
}

#[derive(Serialize, Clone)]
struct SystemBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<SystemField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicToolDef>,
}

/// System field can be a plain string or an array of blocks (for prompt caching).
#[derive(Serialize)]
#[serde(untagged)]
enum SystemField {
    Text(String),
    Blocks(Vec<SystemBlock>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct AnthropicMessage {
    role: String,
    content: AnthropicContent,
}

/// Content can be a plain string or an array of content blocks.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

#[derive(Serialize)]
struct AnthropicToolDef {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

// --- Response types ---

#[derive(Deserialize, Debug)]
struct AnthropicResponse {
    #[serde(default)]
    id: String,
    model: String,
    content: Vec<ResponseContentBlock>,
    stop_reason: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
enum ResponseContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

// --- Streaming event types ---

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum StreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: StreamMessageStart },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: usize,
        content_block: StreamContentBlock,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        index: usize,
        delta: StreamDeltaBlock,
    },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },
    #[serde(rename = "message_delta")]
    MessageDelta { delta: MessageDeltaPayload },
    #[serde(rename = "message_stop")]
    MessageStop {},
    #[serde(rename = "ping")]
    Ping {},
    #[serde(rename = "error")]
    Error { error: AnthropicErrorDetail },
}

#[derive(Deserialize, Debug)]
struct StreamMessageStart {
    #[serde(default)]
    model: String,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum StreamContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum StreamDeltaBlock {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
}

#[derive(Deserialize, Debug)]
struct MessageDeltaPayload {
    stop_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
struct AnthropicErrorResponse {
    error: AnthropicErrorDetail,
}

#[derive(Deserialize, Debug)]
struct AnthropicErrorDetail {
    message: String,
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn to_anthropic_messages(
    messages: &[Message],
    prompt_caching: bool,
) -> (Option<SystemField>, Vec<AnthropicMessage>) {
    let mut system_prompt = None;
    let mut result = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {
                // Anthropic uses a top-level system field, not a system message
                if prompt_caching {
                    system_prompt = Some(SystemField::Blocks(vec![SystemBlock {
                        block_type: "text".to_string(),
                        text: msg.content.clone(),
                        cache_control: Some(CacheControl::ephemeral()),
                    }]));
                } else {
                    system_prompt = Some(SystemField::Text(msg.content.clone()));
                }
            }
            Role::User => {
                result.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: AnthropicContent::Text(msg.content.clone()),
                });
            }
            Role::Assistant => {
                if msg.tool_calls.is_empty() {
                    result.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: AnthropicContent::Text(msg.content.clone()),
                    });
                } else {
                    // Assistant message with tool use
                    let mut blocks: Vec<ContentBlock> = Vec::new();
                    if !msg.content.is_empty() {
                        blocks.push(ContentBlock::Text {
                            text: msg.content.clone(),
                        });
                    }
                    for tc in &msg.tool_calls {
                        blocks.push(ContentBlock::ToolUse {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            input: tc.arguments.clone(),
                        });
                    }
                    result.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: AnthropicContent::Blocks(blocks),
                    });
                }
            }
            Role::Tool => {
                // Tool results are user messages with tool_result content blocks
                let tool_use_id = msg.tool_call_id.clone().unwrap_or_default();
                result.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: AnthropicContent::Blocks(vec![ContentBlock::ToolResult {
                        tool_use_id,
                        content: msg.content.clone(),
                        is_error: None,
                    }]),
                });
            }
        }
    }

    (system_prompt, result)
}

fn to_anthropic_tools(tools: &[ToolDefinition], prompt_caching: bool) -> Vec<AnthropicToolDef> {
    let len = tools.len();
    tools
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let cache_control = if prompt_caching && i == len - 1 {
                Some(CacheControl::ephemeral())
            } else {
                None
            };
            AnthropicToolDef {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
                cache_control,
            }
        })
        .collect()
}

fn from_anthropic_response(resp: &AnthropicResponse) -> Message {
    let mut content = String::new();
    let mut tool_calls = Vec::new();

    for block in &resp.content {
        match block {
            ResponseContentBlock::Text { text } => {
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(text);
            }
            ResponseContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: input.clone(),
                });
            }
            ResponseContentBlock::Thinking { thinking } => {
                // Wrap thinking in <think> tags so the think parser can handle it
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str("<think>");
                content.push_str(thinking);
                content.push_str("</think>");
            }
        }
    }

    Message {
        role: Role::Assistant,
        content,
        tool_calls,
        tool_call_id: None,
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 8192;

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
    default_model: String,
    max_tokens: u32,
    prompt_caching: bool,
}

impl AnthropicProvider {
    pub fn new(api_key: &str, base_url: &str, default_model: &str) -> Self {
        let base_url = if base_url.is_empty() {
            ANTHROPIC_API_URL.to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        };

        let client = Client::builder()
            .pool_max_idle_per_host(2)
            .pool_idle_timeout(Duration::from_secs(90))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            api_key: api_key.to_string(),
            base_url,
            default_model: default_model.to_string(),
            max_tokens: DEFAULT_MAX_TOKENS,
            prompt_caching: true,
        }
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_prompt_caching(mut self, enabled: bool) -> Self {
        self.prompt_caching = enabled;
        self
    }

    fn resolve_model(&self, model: &str) -> String {
        if model.is_empty() {
            self.default_model.clone()
        } else {
            model.to_string()
        }
    }

    async fn handle_error_response(&self, resp: reqwest::Response) -> GclawError {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if let Ok(err) = serde_json::from_str::<AnthropicErrorResponse>(&body) {
            GclawError::Provider(format!(
                "Anthropic API error ({status}): {}",
                err.error.message
            ))
        } else {
            GclawError::Provider(format!("Anthropic API error ({status}): {body}"))
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let model = self.resolve_model(&request.model);
        debug!("Anthropic complete: model={model}");

        let (system, messages) = to_anthropic_messages(&request.messages, self.prompt_caching);

        let api_req = AnthropicRequest {
            model: model.clone(),
            max_tokens: self.max_tokens,
            messages,
            system,
            temperature: request.temperature,
            stream: false,
            tools: to_anthropic_tools(&request.tools, self.prompt_caching),
        };

        let mut req_builder = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json");

        if self.prompt_caching {
            req_builder = req_builder.header("anthropic-beta", "prompt-caching-2024-07-31");
        }

        let resp = req_builder
            .json(&api_req)
            .send()
            .await
            .map_err(|e| GclawError::Provider(format!("Anthropic request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(self.handle_error_response(resp).await);
        }

        let api_resp: AnthropicResponse = resp.json().await.map_err(|e| {
            GclawError::Provider(format!("Failed to parse Anthropic response: {e}"))
        })?;

        let done = api_resp.stop_reason.as_deref() == Some("end_turn")
            || api_resp.stop_reason.as_deref() == Some("tool_use");

        Ok(CompletionResponse {
            message: from_anthropic_response(&api_resp),
            model: api_resp.model,
            done,
        })
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamDelta>> + Send>>> {
        let model = self.resolve_model(&request.model);
        debug!("Anthropic stream: model={model}");

        let (system, messages) = to_anthropic_messages(&request.messages, self.prompt_caching);

        let api_req = AnthropicRequest {
            model,
            max_tokens: self.max_tokens,
            messages,
            system,
            temperature: request.temperature,
            stream: true,
            tools: to_anthropic_tools(&request.tools, self.prompt_caching),
        };

        let mut req_builder = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json");

        if self.prompt_caching {
            req_builder = req_builder.header("anthropic-beta", "prompt-caching-2024-07-31");
        }

        let resp = req_builder
            .json(&api_req)
            .send()
            .await
            .map_err(|e| GclawError::Provider(format!("Anthropic stream request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(self.handle_error_response(resp).await);
        }

        let byte_stream = resp.bytes_stream();

        // State: (byte_stream, buffer, in_thinking, active_tool_id, active_tool_name, active_tool_index)
        let stream = futures::stream::unfold(
            (
                byte_stream,
                String::new(),
                false,
                String::new(),
                String::new(),
                0usize,
            ),
            |(
                mut byte_stream,
                mut buffer,
                mut in_thinking,
                mut tool_id,
                mut tool_name,
                mut tool_index,
            )| async move {
                use futures::StreamExt;
                loop {
                    // Extract a complete SSE line from the buffer
                    if let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                        buffer = buffer[newline_pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        // Parse "event: <type>" lines — we use them for context
                        // but the actual data comes in "data: {...}" lines
                        if line.starts_with("event:") {
                            continue;
                        }

                        if let Some(data) = line.strip_prefix("data: ") {
                            let data = data.trim();

                            match serde_json::from_str::<StreamEvent>(data) {
                                Ok(event) => match event {
                                    StreamEvent::ContentBlockDelta { delta, .. } => {
                                        match delta {
                                            StreamDeltaBlock::TextDelta { text } => {
                                                return Some((
                                                    Ok(StreamDelta {
                                                        content: Some(text),
                                                        tool_calls: vec![],
                                                        done: false,
                                                    }),
                                                    (
                                                        byte_stream,
                                                        buffer,
                                                        in_thinking,
                                                        tool_id,
                                                        tool_name,
                                                        tool_index,
                                                    ),
                                                ));
                                            }
                                            StreamDeltaBlock::ThinkingDelta { thinking } => {
                                                // Wrap in <think> tags for the think parser
                                                let text = if !in_thinking {
                                                    in_thinking = true;
                                                    format!("<think>{thinking}")
                                                } else {
                                                    thinking
                                                };
                                                return Some((
                                                    Ok(StreamDelta {
                                                        content: Some(text),
                                                        tool_calls: vec![],
                                                        done: false,
                                                    }),
                                                    (
                                                        byte_stream,
                                                        buffer,
                                                        in_thinking,
                                                        tool_id,
                                                        tool_name,
                                                        tool_index,
                                                    ),
                                                ));
                                            }
                                            StreamDeltaBlock::InputJsonDelta { partial_json } => {
                                                // Stream the partial JSON as a tool call delta
                                                let tc = ToolCall {
                                                    id: String::new(),
                                                    name: String::new(),
                                                    arguments: serde_json::Value::String(
                                                        partial_json,
                                                    ),
                                                };
                                                return Some((
                                                    Ok(StreamDelta {
                                                        content: None,
                                                        tool_calls: vec![tc],
                                                        done: false,
                                                    }),
                                                    (
                                                        byte_stream,
                                                        buffer,
                                                        in_thinking,
                                                        tool_id,
                                                        tool_name,
                                                        tool_index,
                                                    ),
                                                ));
                                            }
                                        }
                                    }
                                    StreamEvent::ContentBlockStop { .. } => {
                                        if in_thinking {
                                            in_thinking = false;
                                            return Some((
                                                Ok(StreamDelta {
                                                    content: Some("</think>".to_string()),
                                                    tool_calls: vec![],
                                                    done: false,
                                                }),
                                                (
                                                    byte_stream,
                                                    buffer,
                                                    in_thinking,
                                                    tool_id,
                                                    tool_name,
                                                    tool_index,
                                                ),
                                            ));
                                        }
                                        continue;
                                    }
                                    StreamEvent::MessageStop {} => {
                                        return Some((
                                            Ok(StreamDelta {
                                                content: None,
                                                tool_calls: vec![],
                                                done: true,
                                            }),
                                            (
                                                byte_stream,
                                                buffer,
                                                in_thinking,
                                                tool_id,
                                                tool_name,
                                                tool_index,
                                            ),
                                        ));
                                    }
                                    StreamEvent::Error { error } => {
                                        return Some((
                                            Err(GclawError::Provider(format!(
                                                "Anthropic stream error: {}",
                                                error.message
                                            ))),
                                            (
                                                byte_stream,
                                                buffer,
                                                in_thinking,
                                                tool_id,
                                                tool_name,
                                                tool_index,
                                            ),
                                        ));
                                    }
                                    StreamEvent::ContentBlockStart { content_block, .. } => {
                                        // Track tool_use block metadata for subsequent InputJsonDelta events
                                        if let StreamContentBlock::ToolUse { id, name } =
                                            content_block
                                        {
                                            // Emit the initial tool call delta with id and name
                                            let tc = ToolCall {
                                                id: id.clone(),
                                                name: name.clone(),
                                                arguments: serde_json::Value::String(String::new()),
                                            };
                                            tool_id = id;
                                            tool_name = name;
                                            tool_index += 1;
                                            return Some((
                                                Ok(StreamDelta {
                                                    content: None,
                                                    tool_calls: vec![tc],
                                                    done: false,
                                                }),
                                                (
                                                    byte_stream,
                                                    buffer,
                                                    in_thinking,
                                                    tool_id,
                                                    tool_name,
                                                    tool_index,
                                                ),
                                            ));
                                        }
                                        continue;
                                    }
                                    // Ping, MessageStart, MessageDelta — skip
                                    _ => continue,
                                },
                                Err(_) => {
                                    // Unparseable event — skip
                                    continue;
                                }
                            }
                        }
                        continue;
                    }

                    // Need more data from the network
                    match byte_stream.next().await {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                        }
                        Some(Err(e)) => {
                            return Some((
                                Err(GclawError::Provider(format!("Stream read error: {e}"))),
                                (
                                    byte_stream,
                                    buffer,
                                    in_thinking,
                                    tool_id,
                                    tool_name,
                                    tool_index,
                                ),
                            ));
                        }
                        None => return None,
                    }
                }
            },
        );

        Ok(Box::pin(stream))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        // Anthropic doesn't have a list models endpoint — return known models
        Ok(vec![
            ModelInfo {
                name: "claude-opus-4-6".to_string(),
                size: None,
                parameters: HashMap::new(),
            },
            ModelInfo {
                name: "claude-sonnet-4-6".to_string(),
                size: None,
                parameters: HashMap::new(),
            },
            ModelInfo {
                name: "claude-haiku-4-5-20251001".to_string(),
                size: None,
                parameters: HashMap::new(),
            },
        ])
    }

    fn name(&self) -> &str {
        "anthropic"
    }
}
