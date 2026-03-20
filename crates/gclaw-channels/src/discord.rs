use std::sync::Arc;

use async_trait::async_trait;
use gclaw_core::config::DiscordConfig;
use gclaw_core::traits::Channel;
use gclaw_core::types::{InboundMessage, OutboundMessage};
use gclaw_core::{GclawError, Result};
use serenity::all::{
    ChannelId, Client, Context, EventHandler, GatewayIntents, Http, Message, Ready,
};
use tokio::sync::{mpsc, OnceCell};
use tracing::{error, info, warn};

pub struct DiscordChannel {
    config: DiscordConfig,
    http: Arc<OnceCell<Arc<Http>>>,
}

impl DiscordChannel {
    pub fn new(config: &DiscordConfig) -> Self {
        Self {
            config: config.clone(),
            http: Arc::new(OnceCell::new()),
        }
    }

    fn resolve_token(&self) -> String {
        std::env::var("GCLAW_DISCORD_TOKEN").unwrap_or_else(|_| self.config.token.clone())
    }
}

struct Handler {
    tx: mpsc::Sender<InboundMessage>,
    http_cell: Arc<OnceCell<Arc<Http>>>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("Discord bot connected as {}", ready.user.name);
        let _ = self.http_cell.set(Arc::clone(&ctx.http));
    }

    async fn message(&self, _ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        let conversation_id = format!("discord-{}", msg.channel_id);
        let inbound = InboundMessage {
            channel_name: "discord".to_string(),
            conversation_id,
            sender: msg.author.name.clone(),
            content: msg.content.clone(),
            skill_context: None,
        };

        if let Err(e) = self.tx.send(inbound).await {
            error!("Failed to forward Discord message: {e}");
        }
    }
}

#[async_trait]
impl Channel for DiscordChannel {
    async fn start(&self, tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        let token = self.resolve_token();

        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;

        let handler = Handler {
            tx,
            http_cell: Arc::clone(&self.http),
        };

        let mut client = Client::builder(&token, intents)
            .event_handler(handler)
            .await
            .map_err(|e| GclawError::Channel(format!("Failed to build Discord client: {e}")))?;

        tokio::spawn(async move {
            if let Err(e) = client.start().await {
                error!("Discord client error: {e}");
            }
        });

        info!("Discord channel started");
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let http = self
            .http
            .get()
            .ok_or_else(|| GclawError::Channel("Discord client not started yet".to_string()))?;

        let channel_id_str = msg
            .conversation_id
            .strip_prefix("discord-")
            .ok_or_else(|| {
                GclawError::Channel(format!(
                    "Invalid Discord conversation_id: {}",
                    msg.conversation_id
                ))
            })?;

        let channel_id: u64 = channel_id_str.parse().map_err(|e| {
            GclawError::Channel(format!(
                "Failed to parse Discord channel id '{channel_id_str}': {e}"
            ))
        })?;

        let channel = ChannelId::new(channel_id);

        // Discord enforces a 2000-character limit
        let content = if msg.content.len() > 2000 {
            warn!(
                "Truncating Discord message from {} to 2000 chars",
                msg.content.len()
            );
            msg.content[..2000].to_string()
        } else {
            msg.content
        };

        channel
            .say(http.as_ref(), &content)
            .await
            .map_err(|e| GclawError::Channel(format!("Failed to send Discord message: {e}")))?;

        Ok(())
    }

    fn name(&self) -> &str {
        "discord"
    }
}
