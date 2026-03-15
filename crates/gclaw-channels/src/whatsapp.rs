use async_trait::async_trait;
use gclaw_core::traits::Channel;
use gclaw_core::types::{InboundMessage, OutboundMessage};
use gclaw_core::Result;
use tokio::sync::mpsc;

pub struct WhatsAppChannel {
    _token: String,
}

impl WhatsAppChannel {
    pub fn new(token: &str) -> Self {
        Self {
            _token: token.to_string(),
        }
    }
}

#[async_trait]
impl Channel for WhatsAppChannel {
    async fn start(&self, _tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        tracing::warn!("WhatsApp channel not yet implemented");
        Ok(())
    }

    async fn send(&self, _msg: OutboundMessage) -> Result<()> {
        tracing::warn!("WhatsApp channel not yet implemented");
        Ok(())
    }

    fn name(&self) -> &str {
        "whatsapp"
    }
}
