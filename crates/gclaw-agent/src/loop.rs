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
use tracing::{debug, warn};

/// Accumulates tool call deltas from the stream into complete ToolCall objects.
/// OpenAI and Anthropic both stream tool calls incrementally: the first delta
/// for a call carries the id and name, then subsequent deltas append argument
/// fragments. Dispatch is by id — a non-empty id starts a new builder; an
/// empty id appends args to the most recently started builder. Indexing by
/// position within a single SSE delta would be wrong: providers emit one
/// fragment per delta, so multiple parallel tool calls would otherwise all
/// collapse onto the same builder.
#[derive(Default)]
struct ToolCallAccumulator {
    calls: Vec<ToolCallBuilder>,
    current: Option<usize>,
}

struct ToolCallBuilder {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    fn push_delta(&mut self, delta: &ToolCall) {
        if !delta.id.is_empty() {
            self.calls.push(ToolCallBuilder {
                id: delta.id.clone(),
                name: delta.name.clone(),
                arguments: String::new(),
            });
            self.current = Some(self.calls.len() - 1);
        }

        let arg_str = delta.arguments.as_str().unwrap_or("");
        if arg_str.is_empty() {
            return;
        }

        if let Some(idx) = self.current {
            self.calls[idx].arguments.push_str(arg_str);
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
                        let mut stream = provider.complete_stream(req).await?;
                        let mut parser = ThinkParser::new();
                        let mut tool_acc = ToolCallAccumulator::default();

                        while let Some(result) = stream.next().await {
                            match result {
                                Ok(delta) => {
                                    if let Some(ref text) = delta.content {
                                        if !text.is_empty() {
                                            parser.feed(text, &tx);
                                        }
                                    }
                                    for tc in &delta.tool_calls {
                                        tool_acc.push_delta(tc);
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
        acc.push_delta(&ToolCall {
            id: "call_1".to_string(),
            name: "shell_exec".to_string(),
            arguments: serde_json::Value::String(r#"{"command":"ls"}"#.to_string()),
        });
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].id, "call_1");
    }

    #[test]
    fn tool_call_accumulator_multi_delta() {
        let mut acc = ToolCallAccumulator::default();
        // First delta: id + name + partial args
        acc.push_delta(&ToolCall {
            id: "call_1".to_string(),
            name: "shell_exec".to_string(),
            arguments: serde_json::Value::String(r#"{"comma"#.to_string()),
        });
        // Second delta: rest of args
        acc.push_delta(&ToolCall {
            id: String::new(),
            name: String::new(),
            arguments: serde_json::Value::String(r#"nd":"ls"}"#.to_string()),
        });
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].arguments, serde_json::json!({"command": "ls"}));
    }

    #[test]
    fn tool_call_accumulator_multiple_tools() {
        let mut acc = ToolCallAccumulator::default();
        acc.push_delta(&ToolCall {
            id: "call_1".to_string(),
            name: "shell_exec".to_string(),
            arguments: serde_json::Value::String(r#"{"command":"ls"}"#.to_string()),
        });
        acc.push_delta(&ToolCall {
            id: "call_2".to_string(),
            name: "file_read".to_string(),
            arguments: serde_json::Value::String(r#"{"path":"foo.txt"}"#.to_string()),
        });
        let calls = acc.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[1].name, "file_read");
    }

    /// Regression: when the model emits two parallel tool calls, the Anthropic
    /// stream sends ContentBlockStart for each one followed by InputJsonDelta
    /// fragments, all arriving as separate StreamDeltas (each with a single
    /// tool_call entry). Previously the accumulator indexed by position within
    /// the delta's vec, which is always 0, so both calls collapsed onto one
    /// builder and the file_write never actually ran.
    #[test]
    fn tool_call_accumulator_parallel_anthropic_style() {
        let mut acc = ToolCallAccumulator::default();
        // Tool A: ContentBlockStart
        acc.push_delta(&ToolCall {
            id: "tu_A".to_string(),
            name: "file_write".to_string(),
            arguments: serde_json::Value::String(String::new()),
        });
        // Tool A: InputJsonDelta fragments
        acc.push_delta(&ToolCall {
            id: String::new(),
            name: String::new(),
            arguments: serde_json::Value::String(r#"{"path":"a.txt","#.to_string()),
        });
        acc.push_delta(&ToolCall {
            id: String::new(),
            name: String::new(),
            arguments: serde_json::Value::String(r#""content":"A"}"#.to_string()),
        });
        // Tool B: ContentBlockStart
        acc.push_delta(&ToolCall {
            id: "tu_B".to_string(),
            name: "file_write".to_string(),
            arguments: serde_json::Value::String(String::new()),
        });
        // Tool B: InputJsonDelta fragments
        acc.push_delta(&ToolCall {
            id: String::new(),
            name: String::new(),
            arguments: serde_json::Value::String(r#"{"path":"b.txt","content":"B"}"#.to_string()),
        });

        let calls = acc.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "tu_A");
        assert_eq!(calls[0].arguments, serde_json::json!({"path": "a.txt", "content": "A"}));
        assert_eq!(calls[1].id, "tu_B");
        assert_eq!(calls[1].arguments, serde_json::json!({"path": "b.txt", "content": "B"}));
    }

    #[test]
    fn tool_call_accumulator_empty() {
        let acc = ToolCallAccumulator::default();
        assert!(acc.finish().is_empty());
    }
}
