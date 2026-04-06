# Contributing to gclaw

Thanks for your interest in contributing! This guide will help you get started.

## Getting started

### Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- [just](https://github.com/casey/just) (task runner, optional but recommended)
- Docker or Podman (for container sandbox features)
- [Ollama](https://ollama.com) (for integration tests)

### Setup

```bash
git clone https://github.com/bczaicki/gclaw.git
cd gclaw

# Install the git hooks (runs fmt + clippy before each commit)
git config core.hooksPath .githooks

# Copy the example config
cp config.example.toml config.toml

# Build and test
just check        # or: cargo fmt --all --check && cargo clippy --all-targets --workspace -- -D warnings && cargo test --workspace
```

## Development workflow

1. Fork the repo and create a feature branch from `main`.
2. Make your changes.
3. Run `just check` (or `cargo fmt && cargo clippy && cargo test`) to make sure everything passes.
4. Commit using [Conventional Commits](#commit-messages).
5. Open a pull request against `main`.

### Commit messages

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add Matrix channel adapter
fix: prevent panic when config file is missing
docs: add WhatsApp webhook setup guide
refactor: extract tool parsing into shared module
test: add unit tests for memory persistence
chore: update dependencies
```

The prefix tells reviewers (and future changelog tooling) what kind of change this is.

### Running tests

```bash
cargo test --workspace                              # unit tests
cargo test --test integration -- --ignored          # integration tests (needs Ollama running)
```

### Code style

- **Format**: `cargo fmt --all` (we use default rustfmt settings with `max_width = 100`)
- **Lint**: `cargo clippy --all-targets --workspace -- -D warnings` (zero warnings policy)
- Both checks run in CI and in the pre-commit hook

### Pre-commit hook

The repo includes a git hook at `.githooks/pre-commit` that runs format and lint checks before each commit. Enable it with:

```bash
git config core.hooksPath .githooks
```

If a commit is blocked, run `cargo fmt --all` to fix formatting, then commit again.

## What to work on

Check the [issue tracker](https://github.com/bczaicki/gclaw/issues) for issues labeled **`good first issue`** or **`help wanted`**. If you want to work on something, leave a comment on the issue so others know.

For larger changes, open an issue first to discuss the approach before writing code.

## Common contribution types

### Adding a new channel adapter

Channel adapters live in `crates/gclaw-channels/src/`. Each channel is behind a feature flag.

1. Create a new module in `crates/gclaw-channels/src/` (e.g. `matrix.rs`).
2. Implement the `Channel` trait from `gclaw-core`.
3. Add a feature flag in `crates/gclaw-channels/Cargo.toml` and gate the module behind it.
4. Add the feature to `all-channels`.
5. Wire it up in the channel initialization code.
6. Document the config keys in `README.md`.

### Adding a new LLM provider

Providers live in `crates/gclaw-providers/src/`.

1. Create a new module (e.g. `mistral.rs`).
2. Implement the `Provider` trait from `gclaw-core`.
3. Add config parsing in `gclaw-core` for the new provider section.
4. Document the config keys in `README.md`.

### Adding a new tool

Tools live in `crates/gclaw-agent/src/tools/`.

1. Create a new module for the tool.
2. Implement the `Tool` trait.
3. Register it in the tool executor.
4. Document it in the "Built-in tools" section of `README.md`.

## Project structure

```
gclaw/
├── src/main.rs              # Binary entrypoint
├── crates/
│   ├── gclaw-core/          # Traits, types, config, SQLite memory
│   ├── gclaw-agent/         # Agent loop, tools, container executor
│   ├── gclaw-providers/     # Ollama, OpenAI, Anthropic
│   ├── gclaw-channels/      # Telegram, Discord, Slack, WhatsApp
│   ├── gclaw-tui/           # Terminal UI (ratatui)
│   └── gclaw-mcp/           # MCP client/protocol
├── workspace/               # Agent personality Markdown files
├── tests/                   # Integration tests
└── config.example.toml      # Example configuration
```

## Reporting bugs

Use the [bug report template](https://github.com/bczaicki/gclaw/issues/new?template=bug_report.yml). Include:

- Your OS and Rust version (`rustc --version`)
- The provider you're using (Ollama, Anthropic, OpenAI)
- Steps to reproduce
- What you expected vs what happened

## Requesting features

Use the [feature request template](https://github.com/bczaicki/gclaw/issues/new?template=feature_request.yml). Describe the problem you're trying to solve, not just the solution you have in mind.

## Security issues

**Do not open a public issue for security vulnerabilities.** See [SECURITY.md](SECURITY.md) for responsible disclosure instructions.

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). By participating, you agree to uphold it.

## License

By contributing, you agree that your contributions will be licensed under the [LGPL-2.1](LICENSE) license.
