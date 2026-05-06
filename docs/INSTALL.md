# Install Guide

## Fast Path (Release Artifact)

Prereqs on host:
- `curl`
- `tar`
- provider CLIs you want to use (`codex`, `claude`)

Install latest release to `~/.local`:

```bash
./scripts/install-runtime.sh latest
```

Or run directly from GitHub without cloning:

```bash
curl -fsSL https://raw.githubusercontent.com/amxv/gg-agent-runtime/main/scripts/install-runtime.sh | \
  bash -s -- latest
```

Install a pinned version:

```bash
./scripts/install-runtime.sh v0.1.0
```

Only set `GG_RUNTIME_REPO` if you want to install from a fork or alternate repo:

```bash
GG_RUNTIME_REPO=owner/repo ./scripts/install-runtime.sh latest
```

Then:

```bash
export PATH="$HOME/.local/bin:$PATH"
cp "$HOME/.local/runtime-server.toml.example" ./runtime-server.toml
codex login
claude login
gg-runtime-server --config ./runtime-server.toml
```

## Source Install (No Release Needed)

```bash
cargo build --release --bin gg-runtime-server
cargo build --release --manifest-path sidecars/gg-mcp-server/Cargo.toml --bin gg-mcp-server
bun install --cwd sidecars/claude-bridge
bun build sidecars/claude-bridge/src/main.ts --compile --target bun-linux-x64 --outfile sidecars/claude-bridge/claude-bridge
```

Copy into install root:

```bash
mkdir -p ~/.local/bin ~/.local/sidecars/claude-bridge ~/.local/sidecars/gg-mcp-server
cp target/release/gg-runtime-server ~/.local/bin/gg-runtime-server
cp target/release/gg-mcp-server ~/.local/sidecars/gg-mcp-server/gg-mcp-server
cp sidecars/claude-bridge/claude-bridge ~/.local/sidecars/claude-bridge/claude-bridge
chmod +x ~/.local/bin/gg-runtime-server ~/.local/sidecars/gg-mcp-server/gg-mcp-server ~/.local/sidecars/claude-bridge/claude-bridge
```

## Install Layout

Runtime expects this relative layout by default:

```text
<install-root>/
  bin/gg-runtime-server
  sidecars/claude-bridge/claude-bridge
  sidecars/gg-mcp-server/gg-mcp-server
```

This allows starting `gg-runtime-server` without additional bridge path overrides.
