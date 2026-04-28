use async_trait::async_trait;
use gclaw_core::traits::LlmProvider;
use gclaw_core::types::*;
use gclaw_core::{GclawError, Result};
use ollama_rs::generation::chat::request::ChatMessageRequest;
use ollama_rs::generation::chat::{ChatMessage, ChatMessageResponse, MessageRole};
use ollama_rs::generation::tools::{ToolCall as OllamaToolCall, ToolInfo};
use ollama_rs::Ollama;
use serde_json::json;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio_stream::StreamExt;
use tracing::{debug, warn};

pub struct OllamaProvider {
    client: Arc<Ollama>,
    default_model: String,
    call_counter: Arc<AtomicU64>,
}

impl OllamaProvider {
    pub fn new(url: &str, default_model: &str) -> Self {
        let (host, port) = parse_url(url);
        Self {
            client: Arc::new(Ollama::new(host, port)),
            default_model: default_model.to_string(),
            call_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Ollama doesn't return tool-call ids — synthesize stable ones so the
    /// agent loop can match results back to calls.
    fn next_call_id(&self) -> String {
        let n = self.call_counter.fetch_add(1, Ordering::Relaxed);
        format!("ollama_call_{n}")
    }
}

fn parse_url(url: &str) -> (String, u16) {
    if let Some(rest) = url.strip_prefix("http://") {
        if let Some((host, port_str)) = rest.rsplit_once(':') {
            if let Ok(port) = port_str.parse::<u16>() {
                return (format!("http://{host}"), port);
            }
        }
    }
    ("http://localhost".to_string(), 11434)
}

fn to_ollama_role(role: &Role) -> MessageRole {
    match role {
        Role::System => MessageRole::System,
        Role::User => MessageRole::User,
        Role::Assistant => MessageRole::Assistant,
        Role::Tool => MessageRole::Tool,
    }
}

fn from_ollama_role(role: &MessageRole) -> Role {
    match role {
        MessageRole::Assistant => Role::Assistant,
        MessageRole::System => Role::System,
        MessageRole::User => Role::User,
        MessageRole::Tool => Role::Tool,
    }
}

fn to_ollama_messages(messages: &[Message]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|m| {
            let mut chat = ChatMessage::new(to_ollama_role(&m.role), m.content.clone());
            if matches!(m.role, Role::Assistant) && !m.tool_calls.is_empty() {
                chat.tool_calls = m
                    .tool_calls
                    .iter()
                    .map(|tc| OllamaToolCall {
                        function: ollama_rs::generation::tools::ToolCallFunction {
                            name: tc.name.clone(),
                            arguments: tc.arguments.clone(),
                        },
                    })
                    .collect();
            }
            chat
        })
        .collect()
}

/// Convert our generic `ToolDefinition`s into ollama-rs `ToolInfo` by going
/// through JSON. ollama-rs only exposes a `pub(crate)` constructor that
/// requires a compile-time `JsonSchema` type, so we synthesize the same
/// wire shape it would have produced and deserialize it.
///
/// `"type": "Function"` (PascalCase) is what ollama-rs's `ToolType` enum
/// deserializes — using lowercase here causes serde to reject every tool and
/// silently drop it, so the model would receive an empty tools list and just
/// narrate what it would have done. Ollama itself accepts both casings.
fn to_ollama_tools(tools: &[ToolDefinition]) -> Vec<ToolInfo> {
    tools
        .iter()
        .filter_map(|t| {
            let value = json!({
                "type": "Function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                }
            });
            match serde_json::from_value::<ToolInfo>(value) {
                Ok(info) => Some(info),
                Err(e) => {
                    warn!(tool = %t.name, error = %e, "skipping tool: schema not accepted by ollama-rs");
                    None
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn to_ollama_tools_converts_definitions() {
        let defs = vec![ToolDefinition {
            name: "file_write".to_string(),
            description: "Write to a file".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "content"]
            }),
        }];
        let infos = to_ollama_tools(&defs);
        assert_eq!(infos.len(), 1, "tool was dropped during conversion");
        assert_eq!(infos[0].function.name, "file_write");
    }
}

impl OllamaProvider {
    fn convert_response(&self, resp: ChatMessageResponse) -> CompletionResponse {
        let role = from_ollama_role(&resp.message.role);
        let tool_calls = resp
            .message
            .tool_calls
            .iter()
            .map(|tc| ToolCall {
                id: self.next_call_id(),
                name: tc.function.name.clone(),
                arguments: tc.function.arguments.clone(),
            })
            .collect();

        CompletionResponse {
            message: Message {
                role,
                content: resp.message.content.clone(),
                tool_calls,
                tool_call_id: None,
            },
            model: resp.model,
            done: resp.final_data.is_some(),
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let model = if request.model.is_empty() {
            &self.default_model
        } else {
            &request.model
        };

        let messages = to_ollama_messages(&request.messages);
        let tools = to_ollama_tools(&request.tools);
        let chat_req = ChatMessageRequest::new(model.to_string(), messages).tools(tools);

        debug!("Sending chat request to Ollama model: {model}");
        let resp = self
            .client
            .send_chat_messages(chat_req)
            .await
            .map_err(|e| GclawError::Provider(format!("Ollama error: {e}")))?;

        Ok(self.convert_response(resp))
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<StreamDelta>> + Send>>> {
        let model = if request.model.is_empty() {
            &self.default_model
        } else {
            &request.model
        };

        let messages = to_ollama_messages(&request.messages);
        let tools = to_ollama_tools(&request.tools);
        let chat_req = ChatMessageRequest::new(model.to_string(), messages).tools(tools);

        let stream = self
            .client
            .send_chat_messages_stream(chat_req)
            .await
            .map_err(|e| GclawError::Provider(format!("Ollama stream error: {e}")))?;

        let counter = self.call_counter.clone();

        let mapped = stream.map(move |result| {
            result
                .map(|resp| {
                    // Convert each tool_call to our format. The accumulator in
                    // the agent loop expects args as a string fragment, so we
                    // serialize the full args object to JSON: a non-empty id
                    // starts a new builder and the JSON is appended in full.
                    let tool_calls: Vec<ToolCall> = resp
                        .message
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            let n = counter.fetch_add(1, Ordering::Relaxed);
                            let args_json = serde_json::to_string(&tc.function.arguments)
                                .unwrap_or_else(|_| "{}".to_string());
                            ToolCall {
                                id: format!("ollama_call_{n}"),
                                name: tc.function.name.clone(),
                                arguments: serde_json::Value::String(args_json),
                            }
                        })
                        .collect();

                    StreamDelta {
                        content: Some(resp.message.content),
                        tool_calls,
                        done: resp.final_data.is_some(),
                    }
                })
                .map_err(|_| GclawError::Provider("Stream error".to_string()))
        });

        Ok(Box::pin(mapped))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let models = self
            .client
            .list_local_models()
            .await
            .map_err(|e| GclawError::Provider(format!("Failed to list models: {e}")))?;

        Ok(models
            .into_iter()
            .map(|m| ModelInfo {
                name: m.name,
                size: Some(m.size),
                parameters: std::collections::HashMap::new(),
            })
            .collect())
    }

    fn name(&self) -> &str {
        "ollama"
    }
}
