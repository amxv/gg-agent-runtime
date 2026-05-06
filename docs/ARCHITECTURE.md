# Architecture

`gg-runtime-server` is the HTTP/SSE control plane around runtime-core.

## Components

- `crates/runtime-server`:
  - bootstraps providers/runtime services
  - exposes HTTP/SSE API surface
- `crates/runtime-core`:
  - session lifecycle
  - event stream/replay
  - team/process/worktree orchestration
- `crates/runtime-provider-codex`:
  - Codex adapter runtime
- `crates/runtime-provider-claude`:
  - Claude adapter runtime + bridge integration
- `sidecars/claude-bridge`:
  - Claude SDK bridge process
- `sidecars/gg-mcp-server`:
  - GG MCP server process

## Provider Auth Model

- Codex:
  - expects local `codex login` on machine
  - runtime can stage auth material from `~/.gg/codex/auth.json`
- Claude:
  - default: `host_machine` (use machine login material)
  - optional: `runtime_managed` (runtime-owned staged/imported files)

## Data + State

Configured by `data.root_dir` in `runtime-server.toml`:

- SQLite state
- provider runtime directories
- process logs
- generated auth token file (if `auth.token` omitted)

## Why Sidecars

- Claude bridge isolates SDK/runtime behavior from the core server process.
- MCP server enables team/runtime tool surface via a stable process boundary.
