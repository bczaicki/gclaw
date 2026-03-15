use async_trait::async_trait;
use gclaw_core::traits::Channel;
use gclaw_core::types::{InboundMessage, OutboundMessage};
use gclaw_core::Result;
use tokio::sync::mpsc;

pub struct DiscordChannel {
    _token: String,
}

impl DiscordChannel {
    pub fn new(token: &str) -> Self {
        Self {
            _token: token.to_string(),
        }
    }
}

#[async_trait]
impl Channel for DiscordChannel {
    async fn start(&self, _tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        tracing::warn!("Discord channel not yet implemented");
        Ok(())
    }

    async fn send(&self, _msg: OutboundMessage) -> Result<()> {
        tracing::warn!("Discord channel not yet implemented");
        Ok(())
    }

    fn name(&self) -> &str {
        "discord"
    }
}
