use clap::Parser;
use gclaw_agent::tools::{FileReadTool, FileWriteTool, ListDirTool, ShellExecTool, WebFetchTool};
use gclaw_agent::{load_plugins, AgentLoop, ContainerExecutor, ToolExecutor};
use gclaw_channels::TuiChannel;
use gclaw_core::traits::{Channel, LlmProvider};
use gclaw_core::types::{AgentEvent, InboundMessage, OutboundMessage};
use gclaw_core::workspace::{resolve_workspace_dir, Workspace};
use gclaw_core::{Config, SqliteMemory};
use gclaw_providers::{AnthropicProvider, OllamaProvider, OpenAiProvider};
use gclaw_tui::app::App;
use gclaw_tui::event::EventHandler;
use gclaw_tui::Tui;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "gclaw", version, about = "Local-first AI agent gateway")]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<String>,

    /// Model to use
    #[arg(short, long)]
    model: Option<String>,

    /// Run without TUI (headless mode for messaging channels only)
    #[arg(long)]
    headless: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Load config
    if let Some(ref path) = cli.config {
        std::env::set_var("GCLAW_CONFIG", path);
    }
    let config = Config::load()?;

    // Set up tracing to file (not stdout, that's the TUI)
    let log_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("gclaw");
    std::fs::create_dir_all(&log_dir)?;
    let log_file = std::fs::File::create(log_dir.join("gclaw.log"))?;
    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_env_filter("gclaw=debug")
        .init();

    info!("gclaw starting");

    // Validate config
    let api_key_overrides = [
        ("openai", std::env::var("GCLAW_OPENAI_API_KEY").ok()),
        ("anthropic", std::env::var("ANTHROPIC_API_KEY").ok()),
    ];
    for warning in config.validate() {
        // Suppress API key warnings if the env var is set
        let suppress = api_key_overrides.iter().any(|(provider, key)| {
            key.is_some() && config.provider.active == *provider && warning.contains("API key")
        });
        if !suppress {
            warn!("Config: {warning}");
            eprintln!("Warning: {warning}");
        }
    }

    // Build tokio runtime
    let rt = tokio::runtime::Runtime::new()?;

    // Ollama availability check
    if config.provider.active == "ollama" {
        let url = config.provider.ollama.url.clone();
        let check = rt.block_on(async {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap();
            client.get(format!("{url}/api/tags")).send().await
        });
        match check {
            Ok(resp) if resp.status().is_success() => {
                info!("Ollama is reachable at {}", config.provider.ollama.url);
            }
            Ok(resp) => {
                warn!(
                    "Ollama returned status {} — it may not be ready",
                    resp.status()
                );
                eprintln!(
                    "Warning: Ollama at {} returned status {} — is it running?",
                    config.provider.ollama.url,
                    resp.status()
                );
            }
            Err(e) => {
                warn!("Cannot reach Ollama at {}: {e}", config.provider.ollama.url);
                eprintln!(
                    "Warning: Cannot reach Ollama at {} — is it running?\n  Error: {e}",
                    config.provider.ollama.url
                );
            }
        }
    }

    // Provider selection
    let (provider, model): (Arc<dyn LlmProvider>, String) = match config.provider.active.as_str() {
        "openai" => {
            let api_key = std::env::var("GCLAW_OPENAI_API_KEY")
                .unwrap_or_else(|_| config.provider.openai.api_key.clone());
            let m = cli
                .model
                .unwrap_or_else(|| config.provider.openai.default_model.clone());
            info!(
                "Using OpenAI-compatible provider at {}",
                config.provider.openai.base_url
            );
            (
                Arc::new(OpenAiProvider::new(
                    &api_key,
                    &config.provider.openai.base_url,
                    &m,
                )),
                m,
            )
        }
        "anthropic" => {
            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .unwrap_or_else(|_| config.provider.anthropic.api_key.clone());
            let m = cli
                .model
                .unwrap_or_else(|| config.provider.anthropic.default_model.clone());
            info!(
                "Using Anthropic provider at {}",
                config.provider.anthropic.base_url
            );
            (
                Arc::new(
                    AnthropicProvider::new(&api_key, &config.provider.anthropic.base_url, &m)
                        .with_max_tokens(config.provider.anthropic.max_tokens),
                ),
                m,
            )
        }
        _ => {
            let m = cli
                .model
                .unwrap_or_else(|| config.provider.ollama.default_model.clone());
            info!("Using Ollama provider at {}", config.provider.ollama.url);
            (
                Arc::new(OllamaProvider::new(&config.provider.ollama.url, &m)),
                m,
            )
        }
    };

    // Memory
    let db_path = log_dir.join("memory.db");
    let memory = Arc::new(SqliteMemory::new(db_path.to_str().unwrap_or("memory.db"))?);

    // Tools
    let mut executor = ToolExecutor::new();
    executor.register(Arc::new(ShellExecTool));
    executor.register(Arc::new(FileReadTool));
    executor.register(Arc::new(FileWriteTool));
    executor.register(Arc::new(ListDirTool));
    executor.register(Arc::new(WebFetchTool::new()));

    // Wrap in container executor if enabled (plugins loaded below after workspace resolution)
    let mut executor = ContainerExecutor::new(config.container.clone(), executor);

    // Load workspace (SOUL.md, IDENTITY.md, AGENTS.md, etc.)
    let workspace_dir = resolve_workspace_dir(config.agent.workspace_dir.as_deref());
    let workspace = Workspace::load(&workspace_dir);
    let system_prompt = workspace.build_system_prompt(&config.agent.system_prompt);
    if workspace.is_loaded() {
        info!("Workspace loaded from {}", workspace_dir.display());
    } else {
        info!("No workspace found, using default system prompt");
    }

    // Load plugin tools from workspace/tools/
    let tools_dir = workspace_dir.join("tools");
    let plugins = load_plugins(&tools_dir);
    if !plugins.is_empty() {
        info!(
            "Loaded {} plugin tool(s) from {}",
            plugins.len(),
            tools_dir.display()
        );
    }
    for plugin in plugins {
        executor.register(plugin);
    }

    // Agent loop
    let agent = Arc::new(AgentLoop::new(
        provider,
        memory,
        executor,
        model.clone(),
        config.agent.max_iterations,
        system_prompt,
    ));

    // Gateway message channel — all channels send InboundMessages here
    let (gateway_tx, mut gateway_rx) = mpsc::channel::<InboundMessage>(100);

    // Collect all active channels for response routing
    let (channel_map, channel_errors) = rt.block_on(start_channels(&config, gateway_tx.clone()));
    let channel_map = Arc::new(channel_map);

    // Spawn gateway message processor for messaging channels
    let agent_clone = agent.clone();
    let channels = channel_map.clone();
    rt.spawn(async move {
        while let Some(msg) = gateway_rx.recv().await {
            let agent = agent_clone.clone();
            let channels = channels.clone();
            tokio::spawn(async move {
                let convo_id = msg.conversation_id.clone();
                let channel_name = msg.channel_name.clone();

                match agent.process(&convo_id, &msg.content, None).await {
                    Ok(response) => {
                        if let Some(channel) = channels.get(&channel_name) {
                            let out = OutboundMessage {
                                channel_name: channel_name.clone(),
                                conversation_id: convo_id,
                                content: response,
                            };
                            if let Err(e) = channel.send(out).await {
                                error!("Failed to send response on {channel_name}: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Agent error for {convo_id}: {e}");
                        if let Some(channel) = channels.get(&channel_name) {
                            let out = OutboundMessage {
                                channel_name: channel_name.clone(),
                                conversation_id: convo_id,
                                content: format!("Error: {e}"),
                            };
                            let _ = channel.send(out).await;
                        }
                    }
                }
            });
        }
    });

    if cli.headless {
        info!("Running in headless mode (no TUI)");
        rt.block_on(async {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to listen for ctrl-c");
        });
    } else {
        // TUI mode — gets its own event channel for streaming
        let (tui_input_tx, tui_input_rx) = mpsc::unbounded_channel::<String>();
        let (agent_event_tx, agent_event_rx) = mpsc::unbounded_channel::<AgentEvent>();
        let conversation_id = format!("tui-{}", std::process::id());

        let tui_channel = TuiChannel::new(
            tui_input_rx,
            agent_event_tx.clone(),
            conversation_id.clone(),
        );

        let (tui_gw_tx, mut tui_gw_rx) = mpsc::channel::<InboundMessage>(100);
        rt.block_on(async {
            tui_channel
                .start(tui_gw_tx)
                .await
                .expect("Failed to start TUI channel");
        });

        // TUI gateway processor (supports streaming via event_tx)
        let tui_agent = agent.clone();
        let tui_event_tx = agent_event_tx.clone();
        rt.spawn(async move {
            while let Some(msg) = tui_gw_rx.recv().await {
                let agent = tui_agent.clone();
                let event_tx = tui_event_tx.clone();
                tokio::spawn(async move {
                    match agent
                        .process(&msg.conversation_id, &msg.content, Some(event_tx.clone()))
                        .await
                    {
                        Ok(_) => {}
                        Err(e) => {
                            let _ = event_tx.send(AgentEvent::Error(e.to_string()));
                        }
                    }
                });
            }
        });

        let mut tui = Tui::new()?;
        let mut app = App::new(model, conversation_id);
        if !channel_errors.is_empty() {
            app = app.with_startup_warnings(channel_errors);
        }
        if workspace.bootstrap.is_some() {
            app = app.with_onboarding(workspace_dir);
        }
        let mut events = EventHandler::new(agent_event_rx);

        let result = tui.run(&mut app, &mut events, &tui_input_tx);
        tui.restore()?;
        result?;
    }

    info!("gclaw shutting down");
    Ok(())
}

/// Start all enabled messaging channels and return a map for response routing
/// plus any startup errors for display.
async fn start_channels(
    config: &Config,
    gateway_tx: mpsc::Sender<InboundMessage>,
) -> (HashMap<String, Arc<dyn Channel>>, Vec<String>) {
    let mut map: HashMap<String, Arc<dyn Channel>> = HashMap::new();
    let mut errors = Vec::new();

    if config.channels.telegram.enabled {
        let ch = Arc::new(gclaw_channels::TelegramChannel::new(
            &config.channels.telegram,
        ));
        match ch.start(gateway_tx.clone()).await {
            Ok(()) => {
                info!("Telegram channel started");
                map.insert("telegram".to_string(), ch);
            }
            Err(e) => {
                let msg = format!("Failed to start Telegram: {e}");
                error!("{msg}");
                errors.push(msg);
            }
        }
    }

    if config.channels.discord.enabled {
        let ch = Arc::new(gclaw_channels::DiscordChannel::new(
            &config.channels.discord,
        ));
        match ch.start(gateway_tx.clone()).await {
            Ok(()) => {
                info!("Discord channel started");
                map.insert("discord".to_string(), ch);
            }
            Err(e) => {
                let msg = format!("Failed to start Discord: {e}");
                error!("{msg}");
                errors.push(msg);
            }
        }
    }

    if config.channels.slack.enabled {
        let ch = Arc::new(gclaw_channels::SlackChannel::new(&config.channels.slack));
        match ch.start(gateway_tx.clone()).await {
            Ok(()) => {
                info!("Slack channel started");
                map.insert("slack".to_string(), ch);
            }
            Err(e) => {
                let msg = format!("Failed to start Slack: {e}");
                error!("{msg}");
                errors.push(msg);
            }
        }
    }

    if config.channels.whatsapp.enabled {
        let ch = Arc::new(gclaw_channels::WhatsAppChannel::new(
            &config.channels.whatsapp,
        ));
        match ch.start(gateway_tx.clone()).await {
            Ok(()) => {
                info!("WhatsApp channel started");
                map.insert("whatsapp".to_string(), ch);
            }
            Err(e) => {
                let msg = format!("Failed to start WhatsApp: {e}");
                error!("{msg}");
                errors.push(msg);
            }
        }
    }

    (map, errors)
}
