use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub container: ContainerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(default)]
    pub ollama: OllamaConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_url")]
    pub url: String,
    #[serde(default = "default_model")]
    pub default_model: String,
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_model() -> String {
    "qwen3.5:9b".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    #[serde(default)]
    pub workspace_dir: Option<String>,
}

fn default_max_iterations() -> usize {
    10
}

fn default_system_prompt() -> String {
    "You are a helpful assistant.".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default)]
    pub slack: SlackConfig,
    #[serde(default)]
    pub whatsapp: WhatsAppConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiscordConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SlackConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Bot token (xoxb-...)
    #[serde(default)]
    pub bot_token: String,
    /// App-level token for Socket Mode (xapp-...)
    #[serde(default)]
    pub app_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Cloud API access token
    #[serde(default)]
    pub access_token: String,
    /// Phone number ID from Meta dashboard
    #[serde(default)]
    pub phone_number_id: String,
    /// Webhook verify token (you choose this)
    #[serde(default)]
    pub verify_token: String,
    /// Port for the webhook HTTP server
    #[serde(default = "default_webhook_port")]
    pub webhook_port: u16,
}

fn default_webhook_port() -> u16 {
    8080
}

impl Default for WhatsAppConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            access_token: String::new(),
            phone_number_id: String::new(),
            verify_token: String::new(),
            webhook_port: default_webhook_port(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_runtime")]
    pub runtime: String,
    #[serde(default = "default_image")]
    pub image: String,
}

fn default_runtime() -> String {
    "docker".to_string()
}

fn default_image() -> String {
    "gclaw-sandbox:latest".to_string()
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            url: default_ollama_url(),
            default_model: default_model(),
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_iterations: default_max_iterations(),
            system_prompt: default_system_prompt(),
            workspace_dir: None,
        }
    }
}

impl Default for ContainerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            runtime: default_runtime(),
            image: default_image(),
        }
    }
}

impl Config {
    pub fn load() -> crate::Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| crate::GclawError::Config(format!("Failed to read config: {e}")))?;
            toml::from_str(&content)
                .map_err(|e| crate::GclawError::Config(format!("Failed to parse config: {e}")))
        } else {
            Ok(Config::default())
        }
    }

    pub fn config_path() -> PathBuf {
        if let Ok(path) = std::env::var("GCLAW_CONFIG") {
            return PathBuf::from(path);
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("gclaw")
            .join("config.toml")
    }
}
