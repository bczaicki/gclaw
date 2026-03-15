use crate::types::{InboundMessage, OutboundMessage};
use crate::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

#[async_trait]
pub trait Channel: Send + Sync {
    async fn start(&self, tx: mpsc::Sender<InboundMessage>) -> Result<()>;
    async fn send(&self, msg: OutboundMessage) -> Result<()>;
    fn name(&self) -> &str;
}
