use clap::{Parser, Subcommand};
use gclaw_agent::tools::{FileReadTool, FileWriteTool, ListDirTool, ShellExecTool, WebFetchTool};
use gclaw_agent::{load_plugins, AgentLoop, ContainerExecutor, SkillRegistry, ToolExecutor};
use gclaw_core::traits::{Channel, LlmProvider};
use gclaw_core::types::{AgentEvent, InboundMessage, OutboundMessage};
use gclaw_core::workspace::{resolve_workspace_dir, Workspace};
use gclaw_core::{Config, SqliteMemory};
use gclaw_providers::{AnthropicProvider, OllamaProvider, OpenAiProvider};
use gclaw_tui::app::{App, SubmitResult};
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
    #[arg(short, long, global = true)]
    config: Option<String>,

    /// Model to use
    #[arg(short, long)]
    model: Option<String>,

    /// Run without TUI (headless mode for messaging channels only)
    #[arg(long)]
    headless: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Check the health of your gclaw installation
    Doctor,
    /// Start from scratch — delete memory, logs, and (optionally) config
    Reset {
        /// Also delete the config file
        #[arg(long)]
        include_config: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Load config
    if let Some(ref path) = cli.config {
        std::env::set_var("GCLAW_CONFIG", path);
    }

    // Handle subcommands before full startup
    match cli.command {
        Some(Command::Doctor) => return run_doctor(),
        Some(Command::Reset { include_config }) => return run_reset(include_config),
        None => {}
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
            let mut ollama_config = config.provider.ollama.clone();
            if let Some(ref model) = cli.model {
                ollama_config.default_model = model.clone();
            }
            let m = ollama_config.default_model.clone();
            info!("Using Ollama provider at {}", ollama_config.url);
            (Arc::new(OllamaProvider::new(&ollama_config)), m)
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

    // Start MCP servers
    if !config.mcp_servers.is_empty() {
        let mcp_tools = rt.block_on(gclaw_mcp::start_mcp_servers(&config.mcp_servers));
        if !mcp_tools.is_empty() {
            info!("Registered {} MCP tool(s)", mcp_tools.len());
        }
        for tool in mcp_tools {
            executor.register(tool);
        }
    }

    // Discover skills
    let skill_registry = SkillRegistry::discover(&workspace_dir);
    let skill_registry = Arc::new(skill_registry);

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

                match agent
                    .process(&convo_id, &msg.content, None, msg.skill_context.as_deref())
                    .await
                {
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
        let (tui_input_tx, mut tui_input_rx) = mpsc::unbounded_channel::<SubmitResult>();
        let (agent_event_tx, agent_event_rx) = mpsc::unbounded_channel::<AgentEvent>();
        let conversation_id = format!("tui-{}", std::process::id());

        // TUI gateway processor (supports streaming via event_tx + skill dispatch)
        let tui_agent = agent.clone();
        let tui_event_tx = agent_event_tx.clone();
        let tui_skills = skill_registry.clone();
        let tui_convo_id = conversation_id.clone();
        rt.spawn(async move {
            while let Some(submit) = tui_input_rx.recv().await {
                let agent = tui_agent.clone();
                let event_tx = tui_event_tx.clone();
                let skills = tui_skills.clone();
                let convo_id = tui_convo_id.clone();
                tokio::spawn(async move {
                    let (content, skill_context) = match submit {
                        SubmitResult::Message(text) => (text, None),
                        SubmitResult::SkillInvocation { name, args } => {
                            if let Some(skill) = skills.get(&name) {
                                let rendered = skill.render(&args);
                                (args, Some(rendered))
                            } else {
                                let _ = event_tx
                                    .send(AgentEvent::Error(format!("Unknown skill: {name}")));
                                return;
                            }
                        }
                        SubmitResult::None => return,
                    };
                    match agent
                        .process(
                            &convo_id,
                            &content,
                            Some(event_tx.clone()),
                            skill_context.as_deref(),
                        )
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
        let skill_names = skill_registry.user_invocable_names();
        let mut app = App::new(model, conversation_id).with_skill_names(skill_names);
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

// ---------------------------------------------------------------------------
// gclaw doctor
// ---------------------------------------------------------------------------

fn run_doctor() -> anyhow::Result<()> {
    println!("gclaw doctor\n");

    let mut ok_count = 0u32;
    let mut warn_count = 0u32;
    let mut fail_count = 0u32;

    macro_rules! pass {
        ($($arg:tt)*) => { ok_count += 1; println!("  [ok]   {}", format!($($arg)*)); };
    }
    macro_rules! warning {
        ($($arg:tt)*) => { warn_count += 1; println!("  [warn] {}", format!($($arg)*)); };
    }
    macro_rules! fail {
        ($($arg:tt)*) => { fail_count += 1; println!("  [FAIL] {}", format!($($arg)*)); };
    }

    // --- Config ---
    println!("Config");
    let config_path = Config::config_path();
    if config_path.exists() {
        pass!("Config file: {}", config_path.display());
    } else {
        warning!(
            "No config file at {} (using defaults)",
            config_path.display()
        );
    }

    let config = match Config::load() {
        Ok(c) => {
            pass!("Config parses OK");
            Some(c)
        }
        Err(e) => {
            fail!("Config parse error: {e}");
            None
        }
    };

    if let Some(ref cfg) = config {
        let warnings = cfg.validate();
        if warnings.is_empty() {
            pass!("Config validation: no issues");
        } else {
            for w in &warnings {
                warning!("Config: {w}");
            }
        }
    }

    // --- Data directory ---
    println!("\nData");
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("gclaw");
    if data_dir.exists() {
        pass!("Data dir: {}", data_dir.display());
    } else {
        warning!(
            "Data dir missing: {} (will be created on first run)",
            data_dir.display()
        );
    }

    let db_path = data_dir.join("memory.db");
    if db_path.exists() {
        let meta = std::fs::metadata(&db_path);
        match meta {
            Ok(m) => {
                let size_kb = m.len() / 1024;
                pass!("Memory DB: {} ({size_kb} KB)", db_path.display());
                // Try opening it
                match SqliteMemory::new(db_path.to_str().unwrap_or("")) {
                    Ok(_) => {
                        pass!("Memory DB: opens OK");
                    }
                    Err(e) => {
                        fail!("Memory DB: cannot open — {e}");
                    }
                }
            }
            Err(e) => {
                fail!("Memory DB: cannot stat — {e}");
            }
        }
    } else {
        warning!("Memory DB: not yet created (will be created on first run)");
    }

    let log_path = data_dir.join("gclaw.log");
    if log_path.exists() {
        let meta = std::fs::metadata(&log_path);
        match meta {
            Ok(m) => {
                let size_kb = m.len() / 1024;
                pass!("Log file: {} ({size_kb} KB)", log_path.display());
            }
            Err(e) => {
                warning!("Log file: cannot stat — {e}");
            }
        }
    } else {
        warning!("Log file: not yet created");
    }

    // --- Provider ---
    if let Some(ref cfg) = config {
        println!("\nProvider");
        pass!("Active provider: {}", cfg.provider.active);

        match cfg.provider.active.as_str() {
            "ollama" => {
                pass!("Ollama URL: {}", cfg.provider.ollama.url);
                pass!("Ollama model: {}", cfg.provider.ollama.default_model);
                // Connectivity check
                let rt = tokio::runtime::Runtime::new()?;
                let url = cfg.provider.ollama.url.clone();
                let check = rt.block_on(async {
                    let client = reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(5))
                        .build()
                        .unwrap();
                    client.get(format!("{url}/api/tags")).send().await
                });
                match check {
                    Ok(resp) if resp.status().is_success() => {
                        pass!("Ollama: reachable");
                    }
                    Ok(resp) => {
                        warning!("Ollama: returned status {}", resp.status());
                    }
                    Err(e) => {
                        fail!("Ollama: unreachable — {e}");
                    }
                }
            }
            "openai" => {
                let has_key = std::env::var("GCLAW_OPENAI_API_KEY").is_ok()
                    || !cfg.provider.openai.api_key.is_empty();
                if has_key {
                    pass!("OpenAI API key: set");
                } else {
                    fail!("OpenAI API key: missing (set GCLAW_OPENAI_API_KEY or provider.openai.api_key)");
                }
                pass!("OpenAI model: {}", cfg.provider.openai.default_model);
                pass!("OpenAI base URL: {}", cfg.provider.openai.base_url);
            }
            "anthropic" => {
                let has_key = std::env::var("ANTHROPIC_API_KEY").is_ok()
                    || !cfg.provider.anthropic.api_key.is_empty();
                if has_key {
                    pass!("Anthropic API key: set");
                } else {
                    fail!("Anthropic API key: missing (set ANTHROPIC_API_KEY or provider.anthropic.api_key)");
                }
                pass!("Anthropic model: {}", cfg.provider.anthropic.default_model);
                pass!("Anthropic base URL: {}", cfg.provider.anthropic.base_url);
            }
            other => {
                fail!("Unknown provider: {other}");
            }
        }

        // --- Container ---
        if cfg.container.enabled {
            println!("\nContainer");
            pass!(
                "Container: enabled (runtime: {}, image: {})",
                cfg.container.runtime,
                cfg.container.image
            );
            // Check if runtime binary exists
            let which = std::process::Command::new("which")
                .arg(&cfg.container.runtime)
                .output();
            match which {
                Ok(o) if o.status.success() => {
                    pass!("{}: found", cfg.container.runtime);
                }
                _ => {
                    fail!("{}: not found in PATH", cfg.container.runtime);
                }
            }
        }

        // --- Channels ---
        println!("\nChannels");
        let channels_enabled: Vec<&str> = [
            cfg.channels.telegram.enabled.then_some("telegram"),
            cfg.channels.discord.enabled.then_some("discord"),
            cfg.channels.slack.enabled.then_some("slack"),
            cfg.channels.whatsapp.enabled.then_some("whatsapp"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if channels_enabled.is_empty() {
            pass!("No messaging channels enabled (TUI only)");
        } else {
            pass!("Enabled channels: {}", channels_enabled.join(", "));
        }

        // --- MCP ---
        if !cfg.mcp_servers.is_empty() {
            println!("\nMCP Servers");
            for (name, server) in &cfg.mcp_servers {
                let which = std::process::Command::new("which")
                    .arg(&server.command)
                    .output();
                match which {
                    Ok(o) if o.status.success() => {
                        pass!("{name}: command '{}' found", server.command);
                    }
                    _ => {
                        fail!("{name}: command '{}' not found in PATH", server.command);
                    }
                }
            }
        }
    }

    // --- Workspace ---
    println!("\nWorkspace");
    let workspace_dir = config
        .as_ref()
        .and_then(|c| c.agent.workspace_dir.as_deref())
        .map(|_| ())
        .is_some();
    let ws_dir = resolve_workspace_dir(
        config
            .as_ref()
            .and_then(|c| c.agent.workspace_dir.as_deref()),
    );
    let workspace = Workspace::load(&ws_dir);
    if workspace.is_loaded() {
        pass!("Workspace: {}", ws_dir.display());
    } else if workspace_dir {
        warning!("Workspace dir configured but no SOUL.md/IDENTITY.md found");
    } else {
        pass!("No workspace configured (using default system prompt)");
    }

    // Skills
    let skills = SkillRegistry::discover(&ws_dir);
    let skill_names = skills.user_invocable_names();
    if skill_names.is_empty() {
        pass!("Skills: none discovered");
    } else {
        pass!("Skills: {}", skill_names.join(", "));
    }

    // Plugins
    let tools_dir = ws_dir.join("tools");
    let plugins = load_plugins(&tools_dir);
    if plugins.is_empty() {
        pass!("Plugins: none loaded");
    } else {
        pass!(
            "Plugins: {} tool(s) from {}",
            plugins.len(),
            tools_dir.display()
        );
    }

    // --- Summary ---
    println!();
    println!(
        "Summary: {} ok, {} warnings, {} failures",
        ok_count, warn_count, fail_count
    );
    if fail_count > 0 {
        println!("\nFix the failures above before running gclaw.");
        std::process::exit(1);
    } else if warn_count > 0 {
        println!("\ngclaw should work, but check the warnings above.");
    } else {
        println!("\nAll good!");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// gclaw reset
// ---------------------------------------------------------------------------

fn run_reset(include_config: bool) -> anyhow::Result<()> {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("gclaw");

    let db_path = data_dir.join("memory.db");
    let log_path = data_dir.join("gclaw.log");
    let config_path = Config::config_path();

    // Resolve workspace so we can reset onboarding
    let config = Config::load().ok();
    let workspace_dir = resolve_workspace_dir(
        config
            .as_ref()
            .and_then(|c| c.agent.workspace_dir.as_deref()),
    );
    let user_md = workspace_dir.join("USER.md");
    let tools_md = workspace_dir.join("TOOLS.md");
    let bootstrap_md = workspace_dir.join("BOOTSTRAP.md");

    println!("gclaw reset\n");
    println!("This will delete:");
    if db_path.exists() {
        println!("  - Memory database: {}", db_path.display());
    }
    if log_path.exists() {
        println!("  - Log file: {}", log_path.display());
    }
    // Also remove WAL/SHM files that SQLite may leave behind
    let wal_path = data_dir.join("memory.db-wal");
    let shm_path = data_dir.join("memory.db-shm");
    if wal_path.exists() {
        println!("  - WAL file: {}", wal_path.display());
    }
    if shm_path.exists() {
        println!("  - SHM file: {}", shm_path.display());
    }
    if include_config && config_path.exists() {
        println!("  - Config file: {}", config_path.display());
    }

    // Onboarding reset
    if user_md.exists() {
        println!("  - Onboarding: {}", user_md.display());
    }
    if tools_md.exists() {
        println!("  - Onboarding: {}", tools_md.display());
    }
    let will_recreate_bootstrap = workspace_dir.exists() && !bootstrap_md.exists();
    if will_recreate_bootstrap {
        println!("  + Recreate:   {}", bootstrap_md.display());
    }

    let nothing = !(db_path.exists()
        || log_path.exists()
        || wal_path.exists()
        || shm_path.exists()
        || (include_config && config_path.exists())
        || user_md.exists()
        || tools_md.exists()
        || will_recreate_bootstrap);

    if nothing {
        println!("  (nothing to do)");
        return Ok(());
    }

    // Prompt for confirmation
    eprint!("\nProceed? [y/N] ");
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if !answer.trim().eq_ignore_ascii_case("y") {
        println!("Aborted.");
        return Ok(());
    }

    let mut deleted = 0u32;
    for path in [&db_path, &log_path, &wal_path, &shm_path] {
        if path.exists() {
            match std::fs::remove_file(path) {
                Ok(()) => {
                    println!("  Deleted {}", path.display());
                    deleted += 1;
                }
                Err(e) => {
                    eprintln!("  Failed to delete {}: {e}", path.display());
                }
            }
        }
    }

    if include_config && config_path.exists() {
        match std::fs::remove_file(&config_path) {
            Ok(()) => {
                println!("  Deleted {}", config_path.display());
                deleted += 1;
            }
            Err(e) => {
                eprintln!("  Failed to delete {}: {e}", config_path.display());
            }
        }
    }

    // Reset onboarding: delete USER.md/TOOLS.md, recreate BOOTSTRAP.md
    for path in [&user_md, &tools_md] {
        if path.exists() {
            match std::fs::remove_file(path) {
                Ok(()) => {
                    println!("  Deleted {}", path.display());
                    deleted += 1;
                }
                Err(e) => {
                    eprintln!("  Failed to delete {}: {e}", path.display());
                }
            }
        }
    }
    if will_recreate_bootstrap {
        match std::fs::write(&bootstrap_md, "first-run\n") {
            Ok(()) => {
                println!("  Created {}", bootstrap_md.display());
            }
            Err(e) => {
                eprintln!("  Failed to create {}: {e}", bootstrap_md.display());
            }
        }
    }

    println!("\nDone — deleted {deleted} file(s). gclaw is back to a clean slate.");
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
