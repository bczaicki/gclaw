use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use gclaw_core::config::TelegramConfig;
use gclaw_core::traits::Channel;
use gclaw_core::types::{InboundMessage, OutboundMessage};
use gclaw_core::{GclawError, Result};
use teloxide::prelude::*;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub struct TelegramChannel {
    bot: Bot,
    chat_ids: Arc<DashMap<String, i64>>,
}

impl TelegramChannel {
    pub fn new(config: &TelegramConfig) -> Self {
        let token = std::env::var("GCLAW_TELEGRAM_TOKEN").unwrap_or_else(|_| config.token.clone());
        let bot = Bot::new(&token);
        Self {
            bot,
            chat_ids: Arc::new(DashMap::new()),
        }
    }

    fn conversation_id(chat_id: i64) -> String {
        format!("telegram-{chat_id}")
    }

    fn parse_chat_id(conversation_id: &str) -> Result<i64> {
        conversation_id
            .strip_prefix("telegram-")
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| {
                GclawError::Channel(format!(
                    "Invalid telegram conversation_id: {conversation_id}"
                ))
            })
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    async fn start(&self, tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        info!("Starting Telegram channel");

        let chat_ids = Arc::clone(&self.chat_ids);
        let bot = self.bot.clone();

        let handler = Update::filter_message().endpoint(move |msg: Message, _bot: Bot| {
            let tx = tx.clone();
            let chat_ids = chat_ids.clone();
            async move {
                let chat_id = msg.chat.id.0;
                let conv_id = TelegramChannel::conversation_id(chat_id);
                chat_ids.insert(conv_id.clone(), chat_id);

                let text = match msg.text() {
                    Some(t) => t.to_owned(),
                    None => return respond(()),
                };

                let sender = msg
                    .from
                    .as_ref()
                    .and_then(|u| u.username.clone().or_else(|| Some(u.first_name.clone())))
                    .unwrap_or_else(|| "unknown".to_owned());

                let inbound = InboundMessage {
                    channel_name: "telegram".to_owned(),
                    conversation_id: conv_id,
                    sender,
                    content: text,
                };

                if let Err(e) = tx.send(inbound).await {
                    error!("Failed to forward Telegram message: {e}");
                }

                respond(())
            }
        });

        tokio::spawn(async move {
            Dispatcher::builder(bot, handler)
                .enable_ctrlc_handler()
                .build()
                .dispatch()
                .await;
            warn!("Telegram dispatcher exited");
        });

        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let chat_id = self
            .chat_ids
            .get(&msg.conversation_id)
            .map(|entry| *entry.value())
            .or_else(|| Self::parse_chat_id(&msg.conversation_id).ok())
            .ok_or_else(|| {
                GclawError::Channel(format!(
                    "Could not resolve chat_id for {}",
                    msg.conversation_id
                ))
            })?;

        self.bot
            .send_message(ChatId(chat_id), &msg.content)
            .await
            .map_err(|e| GclawError::Channel(format!("Failed to send Telegram message: {e}")))?;

        Ok(())
    }

    fn name(&self) -> &str {
        "telegram"
    }
}
