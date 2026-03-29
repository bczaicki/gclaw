use crate::container::ContainerExecutor;
use crate::context::ConversationContext;
use crate::think::ThinkParser;
use futures::StreamExt;
use gclaw_core::traits::{LlmProvider, Memory};
use gclaw_core::types::*;
use gclaw_core::{GclawError, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Accumulates tool call deltas from the stream into complete ToolCall objects.
/// OpenAI and Anthropic both stream tool calls incrementally — first delta has
/// the id and name, subsequent deltas append to the arguments string.
/// Merges deltas by index to reconstruct complete calls.
#[derive(Default)]
struct ToolCallAccumulator {
    calls: Vec<ToolCallBuilder>,
}

struct ToolCallBuilder {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    fn push_delta(&mut self, delta: &ToolCall, index: usize) {
        // Grow the vec if needed
        while self.calls.len() <= index {
            self.calls.push(ToolCallBuilder {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
        }

        let builder = &mut self.calls[index];
        if !delta.id.is_empty() {
            builder.id = delta.id.clone();
        }
        if !delta.name.is_empty() {
            builder.name = delta.name.clone();
        }
        // Accumulate arguments — they arrive as string fragments for OpenAI,
        // or as partial_json for Anthropic
        let arg_str = delta.arguments.as_str().unwrap_or("");
        if !arg_str.is_empty() {
            builder.arguments.push_str(arg_str);
        }
    }

    fn finish(self) -> Vec<ToolCall> {
        self.calls
            .into_iter()
            .filter(|b| !b.name.is_empty())
            .map(|b| ToolCall {
                id: b.id,
                name: b.name,
                arguments: serde_json::from_str(&b.arguments)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
            })
            .collect()
    }
}

const MAX_RETRIES: usize = 3;
const INITIAL_BACKOFF_MS: u64 = 500;
const MAX_CONTEXT_TOKENS: usize = 100_000; // ~100k tokens before compression
const KEEP_RECENT_MESSAGES: usize = 10; // Keep last 10 messages uncompressed

/// Check if an error is likely transient and worth retrying.
fn is_transient_error(err: &GclawError) -> bool {
    match err {
        GclawError::Provider(msg) => {
            let lower = msg.to_lowercase();
            lower.contains("connection")
                || lower.contains("timeout")
                || lower.contains("reset")
                || lower.contains("429")
                || lower.contains("500")
                || lower.contains("502")
                || lower.contains("503")
                || lower.contains("504")
        }
        _ => false,
    }
}

pub struct AgentLoop {
    provider: Arc<dyn LlmProvider>,
    memory: Arc<dyn Memory>,
    executor: ContainerExecutor,
    model: String,
    max_iterations: usize,
    system_prompt: String,
}

impl AgentLoop {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        memory: Arc<dyn Memory>,
        executor: ContainerExecutor,
        model: String,
        max_iterations: usize,
        system_prompt: String,
    ) -> Self {
        Self {
            provider,
            memory,
            executor,
            model,
            max_iterations,
            system_prompt,
        }
    }

    pub async fn process(
        &self,
        conversation_id: &str,
        user_input: &str,
        event_tx: Option<mpsc::UnboundedSender<AgentEvent>>,
        skill_context: Option<&str>,
    ) -> Result<String> {
        let system = if let Some(ctx_str) = skill_context {
            format!("{}\n\n{}", self.system_prompt, ctx_str)
        } else {
            self.system_prompt.clone()
        };
        let mut ctx = ConversationContext::new(system);
        ctx.set_tools(self.executor.definitions());

        // Load history and compress if needed
        let history = self.memory.retrieve(conversation_id, 50).await?;
        ctx.load_history(history);
        ctx.compress_if_needed(MAX_CONTEXT_TOKENS, KEEP_RECENT_MESSAGES);

        // Add user message
        let user_msg = Message {
            role: Role::User,
            content: user_input.to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        };
        ctx.add_message(user_msg.clone());

        let mut iterations = 0;

        loop {
            if iterations >= self.max_iterations {
                return Err(GclawError::MaxIterationsReached(self.max_iterations));
            }
            iterations += 1;

            let request = CompletionRequest {
                model: self.model.clone(),
                messages: ctx.messages_with_system(),
                tools: ctx.tool_definitions().to_vec(),
                temperature: None,
            };

            debug!("Sending completion request, iteration {iterations}");

            // Get response with retry — streaming or non-streaming
            let (assistant_content, tool_calls) = if let Some(ref tx) = event_tx {
                self.with_retry(|provider| {
                    let req = request.clone();
                    let tx = tx.clone();
                    async move {
                        let t0 = Instant::now();
                        let mut stream = provider.complete_stream(req).await?;
                        let t_stream = t0.elapsed();
                        debug!(elapsed_ms = t_stream.as_millis() as u64, "Stream created");

                        let mut parser = ThinkParser::new();
                        let mut tool_acc = ToolCallAccumulator::default();
                        let mut first_token_time: Option<std::time::Duration> = None;

                        while let Some(result) = stream.next().await {
                            match result {
                                Ok(delta) => {
                                    if let Some(ref text) = delta.content {
                                        if !text.is_empty() {
                                            if first_token_time.is_none() {
                                                first_token_time = Some(t0.elapsed());
                                            }
                                            parser.feed(text, &tx);
                                        }
                                    }
                                    for (i, tc) in delta.tool_calls.iter().enumerate() {
                                        tool_acc.push_delta(tc, i);
                                    }
                                    if delta.done {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    let _ = tx.send(AgentEvent::Error(e.to_string()));
                                    break;
                                }
                            }
                        }

                        let total = t0.elapsed();
                        let ttft = first_token_time.unwrap_or(total);
                        info!(
                            ttft_ms = ttft.as_millis() as u64,
                            stream_created_ms = t_stream.as_millis() as u64,
                            total_ms = total.as_millis() as u64,
                            "Generation metrics"
                        );
                        let _ = tx.send(AgentEvent::Metrics {
                            ttft_ms: ttft.as_millis() as u64,
                            total_ms: total.as_millis() as u64,
                            stream_created_ms: t_stream.as_millis() as u64,
                        });

                        parser.flush(&tx);
                        Ok((parser.content().to_string(), tool_acc.finish()))
                    }
                })
                .await?
            } else {
                self.with_retry(|provider| {
                    let req = request.clone();
                    async move {
                        let response = provider.complete(req).await?;
                        Ok((response.message.content, response.message.tool_calls))
                    }
                })
                .await?
            };

            let assistant_msg = Message {
                role: Role::Assistant,
                content: assistant_content.clone(),
                tool_calls: tool_calls.clone(),
                tool_call_id: None,
            };
            ctx.add_message(assistant_msg.clone());

            // No tool calls → done
            if assistant_msg.tool_calls.is_empty() {
                self.memory
                    .store(conversation_id, &[user_msg, assistant_msg.clone()])
                    .await?;

                if let Some(ref tx) = event_tx {
                    let _ = tx.send(AgentEvent::Done(assistant_content.clone()));
                }

                return Ok(assistant_content);
            }

            // Execute tool calls
            for call in &assistant_msg.tool_calls {
                if let Some(ref tx) = event_tx {
                    let _ = tx.send(AgentEvent::ToolCallStart {
                        name: call.name.clone(),
                        id: call.id.clone(),
                    });
                }

                debug!("Executing tool: {} ({})", call.name, call.id);
                let result = self.executor.execute(call, Some(conversation_id)).await?;

                if let Some(ref tx) = event_tx {
                    let _ = tx.send(AgentEvent::ToolResult {
                        id: call.id.clone(),
                        content: result.content.clone(),
                        is_error: result.is_error,
                    });
                }

                ctx.add_message(Message {
                    role: Role::Tool,
                    content: result.content,
                    tool_calls: vec![],
                    tool_call_id: Some(call.id.clone()),
                });
            }
        }
    }

    /// Retry a provider call with exponential backoff for transient errors.
    async fn with_retry<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: Fn(Arc<dyn LlmProvider>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut last_err = None;
        for attempt in 0..MAX_RETRIES {
            match f(self.provider.clone()).await {
                Ok(val) => return Ok(val),
                Err(e) => {
                    if attempt + 1 < MAX_RETRIES && is_transient_error(&e) {
                        let delay = INITIAL_BACKOFF_MS * 2u64.pow(attempt as u32);
                        warn!(
                            "Provider error (attempt {}/{}), retrying in {}ms: {}",
                            attempt + 1,
                            MAX_RETRIES,
                            delay,
                            e
                        );
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        last_err = Some(e);
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        Err(last_err.unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_accumulator_single_delta() {
        let mut acc = ToolCallAccumulator::default();
        acc.push_delta(
            &ToolCall {
                id: "call_1".to_string(),
                name: "shell_exec".to_string(),
                arguments: serde_json::json!(r#"{"command":"ls"}"#),
            },
            0,
        );
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].id, "call_1");
    }

    #[test]
    fn tool_call_accumulator_multi_delta() {
        let mut acc = ToolCallAccumulator::default();
        // First delta: id + name + partial args
        acc.push_delta(
            &ToolCall {
                id: "call_1".to_string(),
                name: "shell_exec".to_string(),
                arguments: serde_json::Value::String(r#"{"comma"#.to_string()),
            },
            0,
        );
        // Second delta: rest of args
        acc.push_delta(
            &ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: serde_json::Value::String(r#"nd":"ls"}"#.to_string()),
            },
            0,
        );
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].arguments, serde_json::json!({"command": "ls"}));
    }

    #[test]
    fn tool_call_accumulator_multiple_tools() {
        let mut acc = ToolCallAccumulator::default();
        acc.push_delta(
            &ToolCall {
                id: "call_1".to_string(),
                name: "shell_exec".to_string(),
                arguments: serde_json::Value::String(r#"{"command":"ls"}"#.to_string()),
            },
            0,
        );
        acc.push_delta(
            &ToolCall {
                id: "call_2".to_string(),
                name: "file_read".to_string(),
                arguments: serde_json::Value::String(r#"{"path":"foo.txt"}"#.to_string()),
            },
            1,
        );
        let calls = acc.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[1].name, "file_read");
    }

    #[test]
    fn tool_call_accumulator_empty() {
        let acc = ToolCallAccumulator::default();
        assert!(acc.finish().is_empty());
    }
}
