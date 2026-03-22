# gclaw

Local-first AI agent gateway. Chat with LLMs through a terminal TUI, or connect WhatsApp, Telegram, Slack, Discord — all from your own machine. Tool execution is optionally sandboxed in Docker/Podman containers.

## Build

```bash
cargo build
cargo test --workspace   # run all tests (unit + integration)
cargo clippy             # lint
cargo fmt --check        # format check
cargo run                # launch TUI
cargo run -- --headless  # headless mode (messaging channels only)
cargo run -- doctor      # check installation health
```

## Architecture

Six workspace crates, all depending on `gclaw-core` for shared traits and types:

```
src/main.rs              CLI entry point, wires everything together
crates/
  gclaw-core/            Config, traits (LlmProvider, Tool, Channel, Memory), types, workspace loader
  gclaw-agent/           Agent loop (ReAct), tool executor, think parser, skills, plugins, model router
  gclaw-providers/       LLM backends: Ollama, OpenAI, Anthropic
  gclaw-channels/        Messaging adapters: Telegram, Discord, Slack, WhatsApp, TUI bridge
  gclaw-tui/             Terminal UI (ratatui): app state, event handling, widgets
  gclaw-mcp/             MCP server integration (stdio transport, JSON-RPC)
```

### Core traits (`gclaw-core/src/traits/`)

- **LlmProvider** — `complete()`, `complete_stream()`, `list_models()`
- **Tool** — `definition()` returns JSON schema, `execute()` runs it
- **Channel** — `start()` begins receiving, `send()` delivers responses
- **Memory** — `store()`, `retrieve()`, `search()`, `list_conversations()`, `delete_conversation()`

### Message flow

```
Channel (Telegram/Discord/Slack/WhatsApp/TUI)
  → InboundMessage → AgentLoop.process()
    → LlmProvider.complete_stream()
    → ThinkParser (strips <think> blocks)
    → ToolExecutor.execute() (if tool calls)
    → loop back to LLM if tools were called
  → OutboundMessage → Channel.send()
```

### Agent loop (`gclaw-agent/src/loop.rs`)

ReAct loop with streaming, retry with exponential backoff, context compression when approaching token limits. Emits `AgentEvent` variants for the TUI: `ThinkStart/Delta/End`, `StreamDelta`, `ToolCallStart`, `ToolResult`, `Done`, `Error`, `Metric`.

### Tools

**Built-in** (always available): `shell_exec`, `file_read`, `file_write`, `list_dir`, `web_fetch`

**Plugins** (`workspace/tools/*.toml`): Shell command templates with `{{param}}` substitution. Example:
```toml
name = "weather"
description = "Get weather for a location"
command = "curl -s 'https://wttr.in/{{location}}?format=3'"
[parameters.properties.location]
type = "string"
[parameters]
required = ["location"]
```

**MCP servers** (`[mcp_servers]` in config): External tools via stdio JSON-RPC. Failed servers are logged and skipped.

### Config (`config.toml`)

Loaded from `~/.config/gclaw/config.toml` (Linux) or `~/Library/Application Support/gclaw/config.toml` (macOS), overridable via `GCLAW_CONFIG` env var or `--config` flag. All fields have defaults.

Key sections: `provider` (ollama/openai/anthropic), `agent` (system_prompt, max_iterations), `channels`, `container` (Docker sandbox), `routing` (model routing rules), `mcp_servers`, `debug`.

### Workspace

Optional directory (`workspace/` or `agent.workspace_dir`) with Markdown files assembled into the system prompt:

- `IDENTITY.md` — name, type, vibe
- `SOUL.md` — personality, communication style
- `AGENTS.md` — operating instructions
- `USER.md` — human context (written by onboarding)
- `TOOLS.md` — environment details (written by onboarding)
- `MEMORY.md` — durable facts across sessions
- `BOOTSTRAP.md` — triggers onboarding wizard, auto-deleted after

Skills live in `workspace/skills/*/SKILL.md` (YAML frontmatter + template body). Invoked via `/skill-name` in the TUI.

## Design principles

- **Local-first**: Data stays on your machine. Models run locally (Ollama) or you bring your own API key.
- **Modular**: Each concern is a separate crate behind a trait. Swap providers, channels, or tools independently.
- **Safe by default**: Container sandbox is opt-in but designed for zero-trust tool execution (no network, read-only root, memory limits).
- **Streaming**: All LLM responses stream token-by-token through `AgentEvent` to the TUI. Think blocks are parsed and displayed separately.
- **Graceful degradation**: MCP server failures, missing config, unreachable providers — warn and continue, never crash.

## License

LGPL-2.1
