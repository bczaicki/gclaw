//! Integration tests that require a running Ollama instance.
//! Run with: cargo test --test integration -- --ignored
//!
//! These tests are #[ignore]d by default so they don't run in CI
//! without Ollama available.

use gclaw_agent::tools::ShellExecTool;
use gclaw_agent::{ContainerExecutor, ToolExecutor};
use gclaw_core::config::ContainerConfig;
use gclaw_core::traits::{LlmProvider, Memory};
use gclaw_core::types::{CompletionRequest, Message, Role};
use gclaw_core::SqliteMemory;
use gclaw_providers::OllamaProvider;
use std::sync::Arc;

fn make_provider() -> OllamaProvider {
    OllamaProvider::new("http://localhost:11434", "qwen3.5:4b")
}

fn make_memory() -> Arc<SqliteMemory> {
    Arc::new(SqliteMemory::in_memory().unwrap())
}

fn make_executor() -> ContainerExecutor {
    let mut exec = ToolExecutor::new();
    exec.register(Arc::new(ShellExecTool));
    ContainerExecutor::new(ContainerConfig::default(), exec)
}

#[tokio::test]
#[ignore]
async fn ollama_complete_returns_response() {
    let provider = make_provider();
    let request = CompletionRequest {
        model: "qwen3.5:4b".to_string(),
        messages: vec![Message {
            role: Role::User,
            content: "Say hello in exactly 3 words.".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }],
        tools: vec![],
        temperature: Some(0.0),
    };

    let response = provider.complete(request).await.unwrap();
    assert!(!response.message.content.is_empty());
    assert_eq!(response.message.role, Role::Assistant);
}

#[tokio::test]
#[ignore]
async fn ollama_list_models() {
    let provider = make_provider();
    let models = provider.list_models().await.unwrap();
    assert!(!models.is_empty());
    assert!(models.iter().any(|m| m.name.contains("qwen")));
}

#[tokio::test]
#[ignore]
async fn agent_loop_end_to_end() {
    let provider = Arc::new(make_provider());
    let memory = make_memory();
    let executor = make_executor();

    let agent = gclaw_agent::AgentLoop::new(
        provider,
        memory,
        executor,
        "qwen3.5:4b".to_string(),
        5,
        "You are a helpful assistant. Be very brief.".to_string(),
    );

    let response = agent
        .process("test-convo", "What is 2+2?", None, None)
        .await;
    assert!(response.is_ok());
    let text = response.unwrap();
    assert!(text.contains('4'), "Expected '4' in response: {text}");
}

#[tokio::test]
#[ignore]
async fn agent_loop_with_streaming() {
    use tokio::sync::mpsc;

    let provider = Arc::new(make_provider());
    let memory = make_memory();
    let executor = make_executor();

    let agent = gclaw_agent::AgentLoop::new(
        provider,
        memory,
        executor,
        "qwen3.5:4b".to_string(),
        5,
        "You are a helpful assistant. Be very brief.".to_string(),
    );

    let (tx, mut rx) = mpsc::unbounded_channel();
    let response = agent
        .process("test-stream", "Say the word 'hello'", Some(tx), None)
        .await;

    assert!(response.is_ok());

    // Collect events
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }

    // Should have at least a Done event
    assert!(
        events
            .iter()
            .any(|e| matches!(e, gclaw_core::AgentEvent::Done(_))),
        "Expected Done event in: {events:?}"
    );
}

#[tokio::test]
#[ignore]
async fn memory_persists_across_calls() {
    let provider = Arc::new(make_provider());
    let memory = make_memory();
    let executor = make_executor();

    let agent = gclaw_agent::AgentLoop::new(
        provider,
        memory.clone(),
        executor,
        "qwen3.5:4b".to_string(),
        5,
        "You are a helpful assistant. Be very brief.".to_string(),
    );

    // First message
    let _ = agent
        .process("persist-test", "My name is TestUser", None, None)
        .await
        .unwrap();

    // Check memory was stored
    let msgs = memory.retrieve("persist-test", 10).await.unwrap();
    assert!(
        msgs.len() >= 2,
        "Expected stored messages, got {}",
        msgs.len()
    );
    assert!(msgs.iter().any(|m| m.content.contains("TestUser")));
}
