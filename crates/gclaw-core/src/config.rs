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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub ollama: OllamaConfig,
    #[serde(default)]
    pub openai: OpenAiConfig,
    #[serde(default)]
    pub anthropic: AnthropicConfig,
    /// Which provider to use: "ollama", "openai", or "anthropic"
    #[serde(default = "default_active_provider")]
    pub active: String,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            ollama: OllamaConfig::default(),
            openai: OpenAiConfig::default(),
            anthropic: AnthropicConfig::default(),
            active: default_active_provider(),
        }
    }
}

fn default_active_provider() -> String {
    "ollama".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiConfig {
    #[serde(default = "default_openai_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_openai_model")]
    pub default_model: String,
}

fn default_openai_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_openai_model() -> String {
    "gpt-4o-mini".to_string()
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            base_url: default_openai_url(),
            api_key: String::new(),
            default_model: default_openai_model(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    #[serde(default = "default_anthropic_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_anthropic_model")]
    pub default_model: String,
    #[serde(default = "default_anthropic_max_tokens")]
    pub max_tokens: u32,
}

fn default_anthropic_url() -> String {
    "https://api.anthropic.com".to_string()
}

fn default_anthropic_model() -> String {
    "claude-sonnet-4-6".to_string()
}

fn default_anthropic_max_tokens() -> u32 {
    8192
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            base_url: default_anthropic_url(),
            api_key: String::new(),
            default_model: default_anthropic_model(),
            max_tokens: default_anthropic_max_tokens(),
        }
    }
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

    pub fn load_from_path(path: &std::path::Path) -> crate::Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn load_from_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[agent]
max_iterations = 42
system_prompt = "Be concise."
"#,
        )
        .unwrap();

        let cfg = Config::load_from_path(&path).unwrap();
        assert_eq!(cfg.agent.max_iterations, 42);
        assert_eq!(cfg.agent.system_prompt, "Be concise.");
    }

    #[test]
    fn defaults_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does_not_exist.toml");
        let cfg = Config::load_from_path(&path).unwrap();

        let defaults = Config::default();
        assert_eq!(cfg.agent.max_iterations, defaults.agent.max_iterations);
        assert_eq!(cfg.provider.active, defaults.provider.active);
        assert_eq!(cfg.container.runtime, defaults.container.runtime);
    }

    #[test]
    fn default_config_values() {
        let cfg = Config::default();
        assert_eq!(cfg.provider.ollama.url, "http://localhost:11434");
        assert_eq!(cfg.provider.ollama.default_model, "qwen3.5:9b");
        assert_eq!(cfg.agent.system_prompt, "You are a helpful assistant.");
        assert!(!cfg.container.enabled);
        assert_eq!(cfg.provider.active, "ollama");
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[container]\nenabled = true\n").unwrap();

        let cfg = Config::load_from_path(&path).unwrap();
        assert!(cfg.container.enabled);
        assert_eq!(cfg.container.runtime, "docker");
        assert_eq!(cfg.agent.max_iterations, 10);
    }
}
