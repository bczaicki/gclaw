use async_trait::async_trait;
use gclaw_core::traits::LlmProvider;
use gclaw_core::types::*;
use gclaw_core::{GclawError, Result};
use ollama_rs::generation::chat::request::ChatMessageRequest;
use ollama_rs::generation::chat::{ChatMessage, ChatMessageResponse, MessageRole};
use ollama_rs::Ollama;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::StreamExt;
use tracing::debug;

pub struct OllamaProvider {
    client: Arc<Ollama>,
    default_model: String,
}

impl OllamaProvider {
    pub fn new(url: &str, default_model: &str) -> Self {
        let (host, port) = parse_url(url);
        Self {
            client: Arc::new(Ollama::new(host, port)),
            default_model: default_model.to_string(),
        }
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

fn to_ollama_messages(messages: &[Message]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|m| ChatMessage::new(to_ollama_role(&m.role), m.content.clone()))
        .collect()
}

fn from_ollama_response(resp: ChatMessageResponse) -> CompletionResponse {
    let role = match resp.message.role {
        MessageRole::Assistant => Role::Assistant,
        MessageRole::System => Role::System,
        MessageRole::User => Role::User,
        MessageRole::Tool => Role::Tool,
    };

    CompletionResponse {
        message: Message {
            role,
            content: resp.message.content.clone(),
            tool_calls: vec![],
            tool_call_id: None,
        },
        model: resp.model,
        done: resp.final_data.is_some(),
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
        let chat_req = ChatMessageRequest::new(model.to_string(), messages);

        debug!("Sending chat request to Ollama model: {model}");
        let resp = self
            .client
            .send_chat_messages(chat_req)
            .await
            .map_err(|e| GclawError::Provider(format!("Ollama error: {e}")))?;

        Ok(from_ollama_response(resp))
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
        let chat_req = ChatMessageRequest::new(model.to_string(), messages);

        let stream = self
            .client
            .send_chat_messages_stream(chat_req)
            .await
            .map_err(|e| GclawError::Provider(format!("Ollama stream error: {e}")))?;

        let mapped = stream.map(|result| {
            result
                .map(|resp| StreamDelta {
                    content: Some(resp.message.content),
                    tool_calls: vec![],
                    done: resp.final_data.is_some(),
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
