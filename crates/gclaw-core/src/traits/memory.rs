use crate::types::Message;
use crate::Result;
use async_trait::async_trait;

#[async_trait]
pub trait Memory: Send + Sync {
    async fn store(&self, conversation_id: &str, messages: &[Message]) -> Result<()>;
    async fn retrieve(&self, conversation_id: &str, limit: usize) -> Result<Vec<Message>>;
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Message>>;
}
