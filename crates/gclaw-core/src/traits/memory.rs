use crate::types::Message;
use crate::Result;
use async_trait::async_trait;

#[async_trait]
pub trait Memory: Send + Sync {
    async fn store(&self, conversation_id: &str, messages: &[Message]) -> Result<()>;
    async fn retrieve(&self, conversation_id: &str, limit: usize) -> Result<Vec<Message>>;
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Message>>;

    /// List all unique conversation IDs, most recent first.
    async fn list_conversations(&self, limit: usize) -> Result<Vec<String>> {
        // Default no-op for backwards compatibility
        let _ = limit;
        Ok(vec![])
    }

    /// Delete all messages for a conversation.
    async fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        // Default no-op
        let _ = conversation_id;
        Ok(())
    }
}
