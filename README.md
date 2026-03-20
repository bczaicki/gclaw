# gclaw

A local-first AI agent gateway that runs tool calls in containers for security. Chat with LLMs through a TUI, or connect WhatsApp, Telegram, Slack, and Discord -- all from your own machine.

Think of it as a self-hosted alternative to OpenClaw. Your data stays local. Your models run local (or you bring your own API key). Tool execution is sandboxed in Docker.

## What it does

- **Talk to LLMs** from a terminal UI with streaming responses and thinking visualization
- **Connect messaging apps** so people can reach your agent on Telegram, Discord, Slack, or WhatsApp
- **Run tools safely** -- shell commands, file I/O, and web fetches execute inside Docker containers with no network, limited memory, and a read-only filesystem
- **Customize personality** by editing plain Markdown files (`SOUL.md`, `IDENTITY.md`, etc.)
- **Remember conversations** across sessions via SQLite

## Quick start

You need Rust and either [Ollama](https://ollama.com) running locally or an API key for OpenAI / Anthropic.

```bash
git clone https://github.com/bczaicki/gclaw.git
cd gclaw

# Copy and edit the config
cp config.example.toml config.toml
# At minimum: pick a provider and model

# Run
cargo run --release
```

The TUI opens. Type a message, press Enter. First launch runs a short setup wizard.

### Headless mode

If you just want the messaging channels without the TUI:

```bash
cargo run --release -- --headless
```

## Providers

gclaw supports three provider backends. Set `provider.active` in your config:

### Ollama (local, default)

```toml
[provider]
active = "ollama"

[provider.ollama]
url = "http://localhost:11434"
default_model = "qwen3.5:9b"
```

### Anthropic (Claude)

```toml
[provider]
active = "anthropic"

[provider.anthropic]
api_key = ""  # or set ANTHROPIC_API_KEY env var
default_model = "claude-sonnet-4-6"
```

Available models: `claude-opus-4-6`, `claude-sonnet-4-6`, `claude-haiku-4-5-20251001`

### OpenAI (or any compatible API)

Works with OpenAI, Groq, Together, vLLM, or anything that speaks the `/v1/chat/completions` protocol.

```toml
[provider]
active = "openai"

[provider.openai]
base_url = "https://api.openai.com/v1"
api_key = ""  # or set GCLAW_OPENAI_API_KEY env var
default_model = "gpt-4o-mini"
```

## Messaging channels

Enable any combination. Each channel needs its own credentials.

| Channel | Config key | Env var override | How it connects |
|---------|-----------|-----------------|-----------------|
| Telegram | `channels.telegram.token` | `GCLAW_TELEGRAM_TOKEN` | Long-polling bot |
| Discord | `channels.discord.token` | `GCLAW_DISCORD_TOKEN` | Gateway bot |
| Slack | `channels.slack.bot_token` | `GCLAW_SLACK_BOT_TOKEN` | Web API polling |
| WhatsApp | `channels.whatsapp.access_token` | `GCLAW_WHATSAPP_ACCESS_TOKEN` | Webhook server |

Example -- enable Telegram:

```toml
[channels.telegram]
enabled = true
token = "your-bot-token"
```

All channels feed into the same agent loop. Responses route back to the originating channel automatically.

## Container sandbox

When enabled, `shell_exec` tool calls run inside a locked-down container:

- `--network=none` -- no internet access
- `--memory=512m` -- capped memory
- `--cpus=1` -- single CPU
- `--read-only` -- immutable root filesystem
- Per-conversation workspace mounted at `/workspace`

Set it up:

```bash
docker build -t gclaw-sandbox:latest -f Dockerfile.sandbox .
```

```toml
[container]
enabled = true
runtime = "docker"  # or "podman"
image = "gclaw-sandbox:latest"
```

## Workspace personality

The `workspace/` directory contains Markdown files that shape how the agent behaves. Edit them to make it yours.

| File | What it controls |
|------|-----------------|
| `SOUL.md` | Communication style, values, boundaries |
| `IDENTITY.md` | Name, type, vibe |
| `AGENTS.md` | Operating instructions for the agent loop |
| `USER.md` | Your name, timezone, preferences |
| `TOOLS.md` | Environment details (shell, editor, hosts) |
| `MEMORY.md` | Durable facts the agent remembers across sessions |
| `HEARTBEAT.md` | Periodic check-in tasks |

On first run, a `BOOTSTRAP.md` file triggers a setup wizard that fills in `USER.md` and `TOOLS.md` for you, then deletes itself.

## Built-in tools

| Tool | What it does |
|------|-------------|
| `shell_exec` | Run a shell command (containerized when sandbox is on) |
| `file_read` | Read a file |
| `file_write` | Write/create a file |
| `list_dir` | List directory contents |
| `web_fetch` | Fetch a URL and return the response |

## TUI keybindings

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Esc` | Quit |
| `Ctrl+C` | Quit |
| `Ctrl+B` | Toggle conversation sidebar |
| `Ctrl+N` | New conversation |
| `Ctrl+Up/Down` | Switch conversations (sidebar open) |
| `PageUp/Down` | Scroll chat history |

Models that emit `<think>` blocks (Qwen, DeepSeek, Claude with extended thinking) get a distinct visual treatment -- dimmed italic text with a left border, separate from the response.

## CLI

```
gclaw [OPTIONS]

Options:
  -c, --config <PATH>   Path to config file
  -m, --model <MODEL>   Override the default model
      --headless        Run without TUI
  -h, --help            Print help
  -V, --version         Print version
```

## Architecture

Five Rust crates in a Cargo workspace:

| Crate | Role |
|-------|------|
| `gclaw-core` | Traits, types, config, SQLite memory, workspace loader |
| `gclaw-agent` | ReAct agent loop, tool executor, container executor, think parser |
| `gclaw-providers` | Ollama, OpenAI, Anthropic LLM providers |
| `gclaw-channels` | Telegram, Discord, Slack, WhatsApp, TUI channel adapters |
| `gclaw-tui` | Terminal UI (ratatui), onboarding wizard, conversation management |

```
  Telegram ──┐
  Discord ───┤    ┌──────────┐    ┌────────────┐    ┌──────────────┐
  Slack ─────┼──> │ Gateway  │──> │ AgentLoop  │──> │ LLM Provider │
  WhatsApp ──┤    │ (router) │    │ (per-convo)│    └──────────────┘
  TUI ───────┘    └──────────┘    └────────────┘
```

## Development

```bash
cargo build          # build
cargo test           # 37 unit tests
cargo test --test integration -- --ignored  # 5 integration tests (needs Ollama)
cargo clippy         # lint
cargo fmt --check    # format check
```

Logs go to `~/Library/Application Support/gclaw/gclaw.log` (macOS) or `$XDG_DATA_HOME/gclaw/gclaw.log`.

## License

[LGPL-2.1](LICENSE)
