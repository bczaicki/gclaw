use async_trait::async_trait;
use futures::Stream;
use gclaw_core::traits::LlmProvider;
use gclaw_core::types::*;
use gclaw_core::{GclawError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use tracing::debug;

// ---------------------------------------------------------------------------
// OpenAI-compatible serde types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OaiToolDef>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct OaiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct OaiToolCall {
    id: String,
    #[serde(rename = "type")]
    ty: String,
    function: OaiFunction,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct OaiFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct OaiToolDef {
    #[serde(rename = "type")]
    ty: String,
    function: OaiToolFunctionDef,
}

#[derive(Serialize)]
struct OaiToolFunctionDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize, Debug)]
struct OaiResponse {
    model: String,
    choices: Vec<OaiChoice>,
}

#[derive(Deserialize, Debug)]
struct OaiChoice {
    message: OaiMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
struct OaiStreamChunk {
    choices: Vec<OaiStreamChoice>,
}

#[derive(Deserialize, Debug)]
struct OaiStreamChoice {
    delta: OaiStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
struct OaiStreamDelta {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OaiStreamToolCall>>,
}

#[derive(Deserialize, Debug)]
struct OaiStreamToolCall {
    #[allow(dead_code)]
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<OaiStreamFunction>,
}

#[derive(Deserialize, Debug)]
struct OaiStreamFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize, Debug)]
struct OaiModelsResponse {
    data: Vec<OaiModel>,
}

#[derive(Deserialize, Debug)]
struct OaiModel {
    id: String,
}

#[derive(Deserialize, Debug)]
struct OaiErrorResponse {
    error: OaiErrorDetail,
}

#[derive(Deserialize, Debug)]
struct OaiErrorDetail {
    message: String,
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn role_to_str(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn str_to_role(s: &str) -> Role {
    match s {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::Assistant,
    }
}

fn to_oai_messages(messages: &[Message]) -> Vec<OaiMessage> {
    messages
        .iter()
        .map(|m| {
            let tool_calls = if m.tool_calls.is_empty() {
                None
            } else {
                Some(
                    m.tool_calls
                        .iter()
                        .map(|tc| OaiToolCall {
                            id: tc.id.clone(),
                            ty: "function".to_string(),
                            function: OaiFunction {
                                name: tc.name.clone(),
                                arguments: tc.arguments.to_string(),
                            },
                        })
                        .collect(),
                )
            };

            OaiMessage {
                role: role_to_str(&m.role).to_string(),
                content: Some(m.content.clone()),
                tool_calls,
                tool_call_id: m.tool_call_id.clone(),
            }
        })
        .collect()
}

fn to_oai_tools(tools: &[ToolDefinition]) -> Vec<OaiToolDef> {
    tools
        .iter()
        .map(|t| OaiToolDef {
            ty: "function".to_string(),
            function: OaiToolFunctionDef {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            },
        })
        .collect()
}

fn from_oai_message(msg: &OaiMessage) -> Message {
    let tool_calls = msg
        .tool_calls
        .as_ref()
        .map(|tcs| {
            tcs.iter()
                .map(|tc| ToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: serde_json::from_str(&tc.function.arguments)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                })
                .collect()
        })
        .unwrap_or_default();

    Message {
        role: str_to_role(&msg.role),
        content: msg.content.clone().unwrap_or_default(),
        tool_calls,
        tool_call_id: msg.tool_call_id.clone(),
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    base_url: String,
    default_model: String,
}

impl OpenAiProvider {
    pub fn new(api_key: &str, base_url: &str, default_model: &str) -> Self {
        let base_url = if base_url.is_empty() {
            "https://api.openai.com/v1".to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        };

        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
            base_url,
            default_model: default_model.to_string(),
        }
    }

    fn resolve_model(&self, model: &str) -> String {
        if model.is_empty() {
            self.default_model.clone()
        } else {
            model.to_string()
        }
    }

    /// Handle a non-200 response body and return a descriptive error.
    async fn handle_error_response(&self, resp: reqwest::Response) -> GclawError {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if let Ok(err) = serde_json::from_str::<OaiErrorResponse>(&body) {
            GclawError::Provider(format!(
                "OpenAI API error ({}): {}",
                status, err.error.message
            ))
        } else {
            GclawError::Provider(format!("OpenAI API error ({}): {}", status, body))
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let model = self.resolve_model(&request.model);
        debug!("OpenAI complete: model={model}");

        let oai_req = OaiRequest {
            model: model.clone(),
            messages: to_oai_messages(&request.messages),
            stream: false,
            temperature: request.temperature,
            tools: to_oai_tools(&request.tools),
        };

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&oai_req)
            .send()
            .await
            .map_err(|e| GclawError::Provider(format!("OpenAI request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(self.handle_error_response(resp).await);
        }

        let oai_resp: OaiResponse = resp
            .json()
            .await
            .map_err(|e| GclawError::Provider(format!("Failed to parse OpenAI response: {e}")))?;

        let choice = oai_resp
            .choices
            .first()
            .ok_or_else(|| GclawError::Provider("No choices in response".to_string()))?;

        let done = choice.finish_reason.as_deref() == Some("stop")
            || choice.finish_reason.as_deref() == Some("tool_calls");

        Ok(CompletionResponse {
            message: from_oai_message(&choice.message),
            model: oai_resp.model,
            done,
        })
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamDelta>> + Send>>> {
        let model = self.resolve_model(&request.model);
        debug!("OpenAI stream: model={model}");

        let oai_req = OaiRequest {
            model,
            messages: to_oai_messages(&request.messages),
            stream: true,
            temperature: request.temperature,
            tools: to_oai_tools(&request.tools),
        };

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&oai_req)
            .send()
            .await
            .map_err(|e| GclawError::Provider(format!("OpenAI stream request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(self.handle_error_response(resp).await);
        }

        let byte_stream = resp.bytes_stream();

        let stream = futures::stream::unfold(
            (byte_stream, String::new()),
            |(mut byte_stream, mut buffer)| async move {
                use futures::StreamExt;
                loop {
                    // Try to extract a complete SSE line from the buffer.
                    if let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                        buffer = buffer[newline_pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        if let Some(data) = line.strip_prefix("data: ") {
                            let data = data.trim();
                            if data == "[DONE]" {
                                let delta = StreamDelta {
                                    content: None,
                                    tool_calls: vec![],
                                    done: true,
                                };
                                return Some((Ok(delta), (byte_stream, buffer)));
                            }

                            match serde_json::from_str::<OaiStreamChunk>(data) {
                                Ok(chunk) => {
                                    if let Some(choice) = chunk.choices.first() {
                                        // Pass tool call deltas through as-is with raw
                                        // argument fragments — the agent loop's accumulator
                                        // will reconstruct complete tool calls.
                                        let tool_calls = choice
                                            .delta
                                            .tool_calls
                                            .as_ref()
                                            .map(|tcs| {
                                                tcs.iter()
                                                    .filter_map(|tc| {
                                                        let func = tc.function.as_ref()?;
                                                        let name =
                                                            func.name.clone().unwrap_or_default();
                                                        let args = func
                                                            .arguments
                                                            .clone()
                                                            .unwrap_or_default();
                                                        Some(ToolCall {
                                                            id: tc.id.clone().unwrap_or_default(),
                                                            name,
                                                            arguments: serde_json::Value::String(
                                                                args,
                                                            ),
                                                        })
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default();

                                        let done = choice.finish_reason.is_some();

                                        let delta = StreamDelta {
                                            content: choice.delta.content.clone(),
                                            tool_calls,
                                            done,
                                        };
                                        return Some((Ok(delta), (byte_stream, buffer)));
                                    }
                                    continue;
                                }
                                Err(e) => {
                                    return Some((
                                        Err(GclawError::Provider(format!(
                                            "Failed to parse SSE chunk: {e}"
                                        ))),
                                        (byte_stream, buffer),
                                    ));
                                }
                            }
                        }
                        // Not a data: line — skip (e.g., comments, event:, retry:).
                        continue;
                    }

                    // Need more data from the network.
                    match byte_stream.next().await {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                        }
                        Some(Err(e)) => {
                            return Some((
                                Err(GclawError::Provider(format!("Stream read error: {e}"))),
                                (byte_stream, buffer),
                            ));
                        }
                        None => {
                            // Stream ended without [DONE]; treat as done.
                            return None;
                        }
                    }
                }
            },
        );

        Ok(Box::pin(stream))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        debug!("OpenAI list_models");

        let resp = self
            .client
            .get(format!("{}/models", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| GclawError::Provider(format!("OpenAI list models failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(self.handle_error_response(resp).await);
        }

        let oai_models: OaiModelsResponse = resp
            .json()
            .await
            .map_err(|e| GclawError::Provider(format!("Failed to parse models list: {e}")))?;

        Ok(oai_models
            .data
            .into_iter()
            .map(|m| ModelInfo {
                name: m.id,
                size: None,
                parameters: HashMap::new(),
            })
            .collect())
    }

    fn name(&self) -> &str {
        "openai"
    }
}
