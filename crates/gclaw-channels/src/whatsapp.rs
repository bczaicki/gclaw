use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use gclaw_core::config::WhatsAppConfig;
use gclaw_core::traits::Channel;
use gclaw_core::types::{InboundMessage, OutboundMessage};
use gclaw_core::{GclawError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

// ---------------------------------------------------------------------------
// Webhook payload types (minimal subset of the WhatsApp Cloud API schema)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct WhatsAppWebhook {
    #[serde(default)]
    pub entry: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub changes: Vec<Change>,
}

#[derive(Debug, Deserialize)]
pub struct Change {
    pub value: Value,
}

#[derive(Debug, Deserialize)]
pub struct Value {
    #[serde(default)]
    pub messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub from: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub text: Option<Text>,
}

#[derive(Debug, Deserialize)]
pub struct Text {
    pub body: String,
}

// ---------------------------------------------------------------------------
// Outbound request body
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct SendMessageRequest<'a> {
    messaging_product: &'a str,
    to: &'a str,
    #[serde(rename = "type")]
    msg_type: &'a str,
    text: SendText<'a>,
}

#[derive(Debug, Serialize)]
struct SendText<'a> {
    body: &'a str,
}

// ---------------------------------------------------------------------------
// Webhook verification query parameters
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct VerifyQuery {
    #[serde(rename = "hub.mode")]
    hub_mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    hub_verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    hub_challenge: Option<String>,
}

// ---------------------------------------------------------------------------
// Shared state passed into axum handlers
// ---------------------------------------------------------------------------

struct WebhookState {
    verify_token: String,
    tx: mpsc::Sender<InboundMessage>,
}

// ---------------------------------------------------------------------------
// WhatsAppChannel
// ---------------------------------------------------------------------------

pub struct WhatsAppChannel {
    config: WhatsAppConfig,
    access_token: String,
    client: Client,
}

impl WhatsAppChannel {
    pub fn new(config: &WhatsAppConfig) -> Self {
        let config = config.clone();
        let access_token = std::env::var("GCLAW_WHATSAPP_ACCESS_TOKEN")
            .unwrap_or_else(|_| config.access_token.clone());

        Self {
            config,
            access_token,
            client: Client::new(),
        }
    }
}

#[async_trait]
impl Channel for WhatsAppChannel {
    async fn start(&self, tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        let state = Arc::new(WebhookState {
            verify_token: self.config.verify_token.clone(),
            tx,
        });

        let app = Router::new()
            .route("/webhook", get(handle_verify))
            .route("/webhook", post(handle_incoming))
            .with_state(state);

        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], self.config.webhook_port));
        info!(
            port = self.config.webhook_port,
            "WhatsApp webhook server starting"
        );

        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            error!(error = %e, "Failed to bind WhatsApp webhook listener");
            GclawError::Channel(format!("Failed to bind WhatsApp webhook listener: {e}"))
        })?;

        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                error!(error = %e, "WhatsApp webhook server error");
            }
        });

        info!(
            port = self.config.webhook_port,
            "WhatsApp webhook server running"
        );
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let phone = msg
            .conversation_id
            .strip_prefix("whatsapp-")
            .ok_or_else(|| {
                GclawError::Channel(format!(
                    "Invalid WhatsApp conversation_id (expected whatsapp-<phone>): {}",
                    msg.conversation_id
                ))
            })?;

        let url = format!(
            "https://graph.facebook.com/v21.0/{}/messages",
            self.config.phone_number_id
        );

        let body = SendMessageRequest {
            messaging_product: "whatsapp",
            to: phone,
            msg_type: "text",
            text: SendText { body: &msg.content },
        };

        debug!(to = phone, "Sending WhatsApp message");

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.access_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to send WhatsApp message");
                GclawError::Channel(format!("WhatsApp send failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            error!(status = %status, body = %text, "WhatsApp API returned error");
            return Err(GclawError::Channel(format!(
                "WhatsApp API error (HTTP {status}): {text}"
            )));
        }

        debug!(to = phone, "WhatsApp message sent successfully");
        Ok(())
    }

    fn name(&self) -> &str {
        "whatsapp"
    }
}

// ---------------------------------------------------------------------------
// Axum handlers
// ---------------------------------------------------------------------------

async fn handle_verify(
    State(state): State<Arc<WebhookState>>,
    Query(params): Query<VerifyQuery>,
) -> std::result::Result<String, StatusCode> {
    let mode = params.hub_mode.as_deref().unwrap_or_default();
    let token = params.hub_verify_token.as_deref().unwrap_or_default();
    let challenge = params.hub_challenge.unwrap_or_default();

    if mode == "subscribe" && token == state.verify_token {
        info!("WhatsApp webhook verified");
        Ok(challenge)
    } else {
        warn!("WhatsApp webhook verification failed");
        Err(StatusCode::FORBIDDEN)
    }
}

async fn handle_incoming(
    State(state): State<Arc<WebhookState>>,
    Json(payload): Json<WhatsAppWebhook>,
) -> StatusCode {
    for entry in &payload.entry {
        for change in &entry.changes {
            for message in &change.value.messages {
                let text = match &message.text {
                    Some(t) => &t.body,
                    None => {
                        debug!(
                            msg_type = %message.msg_type,
                            "Ignoring non-text WhatsApp message"
                        );
                        continue;
                    }
                };

                let inbound = InboundMessage {
                    channel_name: "whatsapp".to_string(),
                    conversation_id: format!("whatsapp-{}", message.from),
                    sender: message.from.clone(),
                    content: text.clone(),
                    skill_context: None,
                };

                debug!(from = %message.from, "Received WhatsApp message");

                if let Err(e) = state.tx.send(inbound).await {
                    error!(error = %e, "Failed to forward inbound WhatsApp message");
                }
            }
        }
    }

    StatusCode::OK
}
