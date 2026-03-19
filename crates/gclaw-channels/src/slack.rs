use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use gclaw_core::config::SlackConfig;
use gclaw_core::traits::Channel;
use gclaw_core::types::{InboundMessage, OutboundMessage};
use gclaw_core::{GclawError, Result};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

pub struct SlackChannel {
    config: SlackConfig,
    http: reqwest::Client,
    seen_ts: Arc<Mutex<HashSet<String>>>,
}

mod api {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct ConversationsListResponse {
        pub ok: bool,
        #[serde(default)]
        pub channels: Vec<SlackChannelInfo>,
        #[serde(default)]
        pub error: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    pub struct SlackChannelInfo {
        pub id: String,
        #[serde(default)]
        pub is_member: bool,
    }

    #[derive(Debug, Deserialize)]
    pub struct ConversationsHistoryResponse {
        pub ok: bool,
        #[serde(default)]
        pub messages: Vec<SlackMessage>,
        #[serde(default)]
        pub error: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    pub struct SlackMessage {
        #[serde(default)]
        pub ts: String,
        #[serde(default)]
        pub text: String,
        #[serde(default)]
        pub user: Option<String>,
        #[serde(default)]
        pub bot_id: Option<String>,
        #[serde(default)]
        pub subtype: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    pub struct ChatPostMessageResponse {
        pub ok: bool,
        #[serde(default)]
        pub error: Option<String>,
    }
}

impl SlackChannel {
    pub fn new(config: &SlackConfig) -> Self {
        Self {
            config: config.clone(),
            http: reqwest::Client::new(),
            seen_ts: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn resolve_bot_token(&self) -> String {
        std::env::var("GCLAW_SLACK_BOT_TOKEN").unwrap_or_else(|_| self.config.bot_token.clone())
    }

    async fn list_joined_channels(
        http: &reqwest::Client,
        token: &str,
    ) -> Result<Vec<api::SlackChannelInfo>> {
        let resp = http
            .get("https://slack.com/api/conversations.list")
            .bearer_auth(token)
            .query(&[
                ("types", "public_channel,private_channel"),
                ("limit", "200"),
            ])
            .send()
            .await
            .map_err(|e| GclawError::Channel(format!("conversations.list failed: {e}")))?;

        let body: api::ConversationsListResponse = resp
            .json()
            .await
            .map_err(|e| GclawError::Channel(format!("conversations.list parse error: {e}")))?;

        if !body.ok {
            let err = body.error.unwrap_or_default();
            return Err(GclawError::Channel(format!(
                "conversations.list API error: {err}"
            )));
        }

        Ok(body.channels.into_iter().filter(|c| c.is_member).collect())
    }

    async fn fetch_history(
        http: &reqwest::Client,
        token: &str,
        channel_id: &str,
        oldest: &str,
    ) -> Result<Vec<api::SlackMessage>> {
        let resp = http
            .get("https://slack.com/api/conversations.history")
            .bearer_auth(token)
            .query(&[("channel", channel_id), ("oldest", oldest), ("limit", "50")])
            .send()
            .await
            .map_err(|e| GclawError::Channel(format!("conversations.history failed: {e}")))?;

        let body: api::ConversationsHistoryResponse = resp
            .json()
            .await
            .map_err(|e| GclawError::Channel(format!("conversations.history parse error: {e}")))?;

        if !body.ok {
            let err = body.error.unwrap_or_default();
            return Err(GclawError::Channel(format!(
                "conversations.history API error: {err}"
            )));
        }

        Ok(body.messages)
    }
}

#[async_trait]
impl Channel for SlackChannel {
    async fn start(&self, tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        let token = self.resolve_bot_token();
        if token.is_empty() {
            return Err(GclawError::Channel(
                "Slack bot token is empty. Set GCLAW_SLACK_BOT_TOKEN or bot_token in config."
                    .to_string(),
            ));
        }

        let http = self.http.clone();
        let seen_ts = Arc::clone(&self.seen_ts);

        let start_ts = format!(
            "{:.6}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
        );

        tokio::spawn(async move {
            info!("Slack polling loop started");
            let mut oldest_per_channel: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();

            loop {
                let channels = match Self::list_joined_channels(&http, &token).await {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("Failed to list Slack channels: {e}");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                };

                for channel in &channels {
                    let oldest = oldest_per_channel
                        .get(&channel.id)
                        .cloned()
                        .unwrap_or_else(|| start_ts.clone());

                    let messages =
                        match Self::fetch_history(&http, &token, &channel.id, &oldest).await {
                            Ok(m) => m,
                            Err(e) => {
                                debug!("Failed to fetch history for {}: {e}", channel.id);
                                continue;
                            }
                        };

                    let mut seen = seen_ts.lock().await;
                    let mut max_ts = oldest.clone();

                    for msg in &messages {
                        if msg.bot_id.is_some() || msg.subtype.is_some() {
                            continue;
                        }
                        if seen.contains(&msg.ts) {
                            continue;
                        }
                        seen.insert(msg.ts.clone());

                        if msg.ts > max_ts {
                            max_ts = msg.ts.clone();
                        }

                        let sender = msg.user.clone().unwrap_or_else(|| "unknown".to_string());
                        let inbound = InboundMessage {
                            channel_name: "slack".to_string(),
                            conversation_id: format!("slack-{}", channel.id),
                            sender,
                            content: msg.text.clone(),
                        };

                        if let Err(e) = tx.send(inbound).await {
                            error!("Failed to forward Slack message: {e}");
                            return;
                        }
                    }

                    if max_ts > oldest {
                        oldest_per_channel.insert(channel.id.clone(), max_ts);
                    }
                }

                {
                    let mut seen = seen_ts.lock().await;
                    if seen.len() > 10_000 {
                        seen.clear();
                    }
                }

                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });

        info!("Slack channel started (polling mode)");
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let token = self.resolve_bot_token();

        let channel_id = msg.conversation_id.strip_prefix("slack-").ok_or_else(|| {
            GclawError::Channel(format!(
                "Invalid Slack conversation_id: {}",
                msg.conversation_id
            ))
        })?;

        let resp = self
            .http
            .post("https://slack.com/api/chat.postMessage")
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "channel": channel_id,
                "text": msg.content,
            }))
            .send()
            .await
            .map_err(|e| GclawError::Channel(format!("chat.postMessage failed: {e}")))?;

        let body: api::ChatPostMessageResponse = resp
            .json()
            .await
            .map_err(|e| GclawError::Channel(format!("chat.postMessage parse error: {e}")))?;

        if !body.ok {
            let err = body.error.unwrap_or_default();
            return Err(GclawError::Channel(format!(
                "chat.postMessage API error: {err}"
            )));
        }

        debug!("Sent message to Slack channel {channel_id}");
        Ok(())
    }

    fn name(&self) -> &str {
        "slack"
    }
}
