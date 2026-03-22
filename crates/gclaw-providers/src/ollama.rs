use async_trait::async_trait;
use gclaw_core::config::OllamaConfig;
use gclaw_core::traits::LlmProvider;
use gclaw_core::types::*;
use gclaw_core::{GclawError, Result};
use ollama_rs::generation::chat::request::ChatMessageRequest;
use ollama_rs::generation::chat::{ChatMessage, ChatMessageResponse, MessageRole};
use ollama_rs::models::ModelOptions;
use ollama_rs::Ollama;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::StreamExt;
use tracing::debug;

pub struct OllamaProvider {
    client: Arc<Ollama>,
    default_model: String,
    disable_thinking: bool,
    num_predict: Option<i32>,
}

impl OllamaProvider {
    pub fn new(config: &OllamaConfig) -> Self {
        let (host, port) = parse_url(&config.url);

        let client = if let Some(timeout_secs) = config.timeout_secs {
            let reqwest_client = reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout_secs))
                .build()
                .expect("Failed to build reqwest client");
            Ollama::new_with_client(host, port, reqwest_client)
        } else {
            Ollama::new(host, port)
        };

        Self {
            client: Arc::new(client),
            default_model: config.default_model.clone(),
            disable_thinking: config.disable_thinking,
            num_predict: config.num_predict,
        }
    }

    /// Append /no_think to the first system message when thinking is disabled.
    fn maybe_inject_no_think(disable_thinking: bool, messages: &mut [ChatMessage]) {
        if !disable_thinking {
            return;
        }
        for msg in messages.iter_mut() {
            if msg.role == MessageRole::System {
                if !msg.content.ends_with("/no_think") {
                    msg.content.push_str("\n/no_think");
                }
                break;
            }
        }
    }

    fn build_generation_options(&self, temperature: Option<f32>) -> Option<ModelOptions> {
        let mut opts = ModelOptions::default();
        let mut has_opts = false;

        if let Some(np) = self.num_predict {
            opts = opts.num_predict(np);
            has_opts = true;
        }
        if let Some(temp) = temperature {
            opts = opts.temperature(temp);
            has_opts = true;
        }

        if has_opts {
            Some(opts)
        } else {
            None
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

        let mut messages = to_ollama_messages(&request.messages);
        Self::maybe_inject_no_think(self.disable_thinking, &mut messages);

        let mut chat_req = ChatMessageRequest::new(model.to_string(), messages);
        if let Some(opts) = self.build_generation_options(request.temperature) {
            chat_req = chat_req.options(opts);
        }
        if self.disable_thinking {
            chat_req = chat_req.think(false);
        }

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

        let mut messages = to_ollama_messages(&request.messages);
        Self::maybe_inject_no_think(self.disable_thinking, &mut messages);

        let mut chat_req = ChatMessageRequest::new(model.to_string(), messages);
        if let Some(opts) = self.build_generation_options(request.temperature) {
            chat_req = chat_req.options(opts);
        }
        if self.disable_thinking {
            chat_req = chat_req.think(false);
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_think_injection_appends_to_system() {
        let mut msgs = vec![
            ChatMessage::new(MessageRole::System, "You are helpful.".to_string()),
            ChatMessage::new(MessageRole::User, "Hello".to_string()),
        ];
        OllamaProvider::maybe_inject_no_think(true, &mut msgs);
        assert!(msgs[0].content.ends_with("/no_think"));
        assert_eq!(msgs[1].content, "Hello");
    }

    #[test]
    fn no_think_not_injected_when_disabled() {
        let mut msgs = vec![ChatMessage::new(
            MessageRole::System,
            "You are helpful.".to_string(),
        )];
        OllamaProvider::maybe_inject_no_think(false, &mut msgs);
        assert_eq!(msgs[0].content, "You are helpful.");
    }

    #[test]
    fn no_think_idempotent() {
        let mut msgs = vec![ChatMessage::new(
            MessageRole::System,
            "You are helpful.\n/no_think".to_string(),
        )];
        OllamaProvider::maybe_inject_no_think(true, &mut msgs);
        // Should not double-append
        assert_eq!(msgs[0].content, "You are helpful.\n/no_think");
    }

    #[test]
    fn build_opts_with_num_predict() {
        let provider = OllamaProvider::new(&OllamaConfig {
            num_predict: Some(512),
            ..OllamaConfig::default()
        });
        let opts = provider.build_generation_options(None);
        assert!(opts.is_some());
    }

    #[test]
    fn build_opts_none_when_empty() {
        let provider = OllamaProvider::new(&OllamaConfig::default());
        let opts = provider.build_generation_options(None);
        assert!(opts.is_none());
    }

    #[test]
    fn build_opts_with_temperature() {
        let provider = OllamaProvider::new(&OllamaConfig::default());
        let opts = provider.build_generation_options(Some(0.7));
        assert!(opts.is_some());
    }
}
