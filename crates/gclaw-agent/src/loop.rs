use crate::container::ContainerExecutor;
use crate::context::ConversationContext;
use crate::think::ThinkParser;
use futures::StreamExt;
use gclaw_core::traits::{LlmProvider, Memory};
use gclaw_core::types::*;
use gclaw_core::{GclawError, Result};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::debug;

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
    ) -> Result<String> {
        let mut ctx = ConversationContext::new(self.system_prompt.clone());
        ctx.set_tools(self.executor.definitions());

        // Load history
        let history = self.memory.retrieve(conversation_id, 50).await?;
        ctx.load_history(history);

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

            debug!("Sending streaming completion request, iteration {iterations}");

            // Use streaming to get real-time tokens with think-tag parsing
            let assistant_content = if let Some(ref tx) = event_tx {
                self.stream_response(request, tx).await?
            } else {
                // No event channel — fall back to non-streaming
                let response = self.provider.complete(request).await?;
                response.message.content
            };

            let assistant_msg = Message {
                role: Role::Assistant,
                content: assistant_content.clone(),
                tool_calls: vec![],
                tool_call_id: None,
            };
            ctx.add_message(assistant_msg.clone());

            // No tool calls → done (streaming doesn't produce tool calls currently)
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

    /// Stream the response from the provider, parsing think tags in real-time.
    /// Returns the full content (non-thinking) text once the stream completes.
    async fn stream_response(
        &self,
        request: CompletionRequest,
        tx: &mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<String> {
        let mut stream = self.provider.complete_stream(request).await?;
        let mut parser = ThinkParser::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(delta) => {
                    if let Some(ref text) = delta.content {
                        if !text.is_empty() {
                            parser.feed(text, tx);
                        }
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

        parser.flush(tx);
        Ok(parser.content().to_string())
    }
}
