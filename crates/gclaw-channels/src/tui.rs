use async_trait::async_trait;
use gclaw_core::traits::Channel;
use gclaw_core::types::{InboundMessage, OutboundMessage};
use gclaw_core::Result;
use std::sync::Arc;
use tokio::sync::mpsc;

/// TUI channel adapter - bridges the TUI input/output to the gateway.
/// The actual TUI rendering is handled by gclaw-tui; this adapter just
/// shuttles messages between the TUI and the agent gateway.
pub struct TuiChannel {
    /// Receives user input from the TUI
    input_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<String>>>,
    /// Sends agent responses back to the TUI
    output_tx: mpsc::UnboundedSender<gclaw_core::AgentEvent>,
    conversation_id: String,
}

impl TuiChannel {
    pub fn new(
        input_rx: mpsc::UnboundedReceiver<String>,
        output_tx: mpsc::UnboundedSender<gclaw_core::AgentEvent>,
        conversation_id: String,
    ) -> Self {
        Self {
            input_rx: Arc::new(tokio::sync::Mutex::new(input_rx)),
            output_tx,
            conversation_id,
        }
    }
}

#[async_trait]
impl Channel for TuiChannel {
    async fn start(&self, tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        let input_rx = self.input_rx.clone();
        let conversation_id = self.conversation_id.clone();
        tokio::spawn(async move {
            let mut rx = input_rx.lock().await;
            while let Some(content) = rx.recv().await {
                let msg = InboundMessage {
                    channel_name: "tui".to_string(),
                    conversation_id: conversation_id.clone(),
                    sender: "user".to_string(),
                    content,
                };
                if tx.send(msg).await.is_err() {
                    break;
                }
            }
        });
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let _ = self
            .output_tx
            .send(gclaw_core::AgentEvent::Done(msg.content));
        Ok(())
    }

    fn name(&self) -> &str {
        "tui"
    }
}
