use clap::Parser;
use gclaw_agent::{AgentLoop, ShellExecTool, ToolExecutor};
use gclaw_channels::TuiChannel;
use gclaw_core::traits::Channel;
use gclaw_core::types::{AgentEvent, InboundMessage};
use gclaw_core::workspace::{resolve_workspace_dir, Workspace};
use gclaw_core::{Config, SqliteMemory};
use gclaw_providers::OllamaProvider;
use gclaw_tui::app::App;
use gclaw_tui::event::EventHandler;
use gclaw_tui::Tui;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

#[derive(Parser)]
#[command(name = "gclaw", version, about = "Local-first AI agent gateway")]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<String>,

    /// Model to use
    #[arg(short, long)]
    model: Option<String>,
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

    let model = cli
        .model
        .unwrap_or_else(|| config.provider.ollama.default_model.clone());

    // Build tokio runtime
    let rt = tokio::runtime::Runtime::new()?;

    // Provider
    let provider = Arc::new(OllamaProvider::new(&config.provider.ollama.url, &model));

    // Memory
    let db_path = log_dir.join("memory.db");
    let memory = Arc::new(SqliteMemory::new(db_path.to_str().unwrap_or("memory.db"))?);

    // Tools
    let mut executor = ToolExecutor::new();
    executor.register(Arc::new(ShellExecTool));

    // Load workspace (SOUL.md, IDENTITY.md, AGENTS.md, etc.)
    let workspace_dir = resolve_workspace_dir(config.agent.workspace_dir.as_deref());
    let workspace = Workspace::load(&workspace_dir);
    let system_prompt = workspace.build_system_prompt(&config.agent.system_prompt);
    if workspace.is_loaded() {
        info!("Workspace loaded from {}", workspace_dir.display());
    } else {
        info!("No workspace found, using default system prompt");
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

    // Channels for TUI <-> Gateway communication
    let (tui_input_tx, tui_input_rx) = mpsc::unbounded_channel::<String>();
    let (agent_event_tx, agent_event_rx) = mpsc::unbounded_channel::<AgentEvent>();

    let conversation_id = format!("tui-{}", std::process::id());

    // Gateway message channel
    let (gateway_tx, mut gateway_rx) = mpsc::channel::<InboundMessage>(100);

    // TUI channel adapter
    let tui_channel = TuiChannel::new(
        tui_input_rx,
        agent_event_tx.clone(),
        conversation_id.clone(),
    );

    // Start TUI channel in background
    rt.block_on(async {
        tui_channel
            .start(gateway_tx)
            .await
            .expect("Failed to start TUI channel");
    });

    // Spawn gateway message processor
    let agent_clone = agent.clone();
    let event_tx_clone = agent_event_tx.clone();
    rt.spawn(async move {
        while let Some(msg) = gateway_rx.recv().await {
            let agent = agent_clone.clone();
            let event_tx = event_tx_clone.clone();
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

    // Run TUI on main thread
    let mut tui = Tui::new()?;
    let mut app = App::new(model, conversation_id);
    if workspace.bootstrap.is_some() {
        app = app.with_onboarding(workspace_dir);
    }
    let mut events = EventHandler::new(agent_event_rx);

    let result = tui.run(&mut app, &mut events, &tui_input_tx);
    tui.restore()?;
    result?;

    info!("gclaw shutting down");
    Ok(())
}
