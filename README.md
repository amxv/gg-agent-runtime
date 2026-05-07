# GG Runtime

`gg-runtime-server` is a standalone agent runtime you can deploy on a laptop, a VPS, or a dedicated machine and then drive from any frontend over HTTP and SSE.

The interesting part is not "another AI wrapper." The interesting part is that this repo treats agent execution as real systems infrastructure:

- agents run against actual provider CLIs and bridges, not toy mocks
- session state and events are durable and replayable through SQLite
- frontends connect over a stable network contract instead of embedding provider logic
- team messaging, worktrees, processes, and MCP tooling live in the runtime instead of being reimplemented per app
- Codex and Claude share one runtime model, so a UI can target one backend instead of two separate stacks

If you want to build agent products where the UI is replaceable but the runtime is persistent, observable, and deployable, that is what this project is for.

## Why This Is Cool

Most agent apps mix three concerns into one codebase:

- frontend state
- provider integration
- machine orchestration

That gets messy fast. This project splits those apart cleanly.

`gg-runtime-server` is the machine-side control plane. It owns:

- provider sessions
- turn execution
- event streaming
- auth staging
- team communication
- process execution
- managed worktrees
- MCP tool plumbing

That means you can build multiple clients on top of the same runtime:

- a desktop UI
- a web app
- an internal ops dashboard
- a CLI
- a thin mobile companion

without rewriting the hard part each time.

## What You Get

### Unified provider runtime

Today the runtime supports:

- Codex
- Claude

Both providers map into the same runtime concepts for sessions, turns, events, approvals, terminal assistant output, and recovery.

### Durable event model

Events are stored in SQLite and exposed over HTTP/SSE, which gives you:

- reconnectable UIs
- replayable session history
- a clean separation between backend execution and frontend rendering
- a usable debugging trail when something goes wrong

### Real machine orchestration

This is built for agents that do actual work on a machine, including:

- spawning provider-backed sessions
- running MCP tools through a dedicated sidecar
- team messaging and delivery tracking
- process execution
- worktree allocation and lifecycle management

### Deployable product boundary

The runtime is shipped as one bundle:

- `bin/gg-runtime-server`
- `sidecars/claude-bridge/claude-bridge`
- `sidecars/gg-mcp-server/gg-mcp-server`

That bundle is the backend product. Your UI is a separate concern.

## Architecture

High level shape:

- `crates/runtime-server`: HTTP/SSE server and bootstrap layer
- `crates/runtime-core`: runtime state machine, orchestration, teams, events
- `crates/runtime-store-sqlite`: durable state and event storage
- `crates/runtime-provider-codex`: Codex provider adapter
- `crates/runtime-provider-claude`: Claude provider adapter
- `crates/runtime-tools`: shared tool/runtime process support
- `sidecars/claude-bridge`: Claude bridge process
- `sidecars/gg-mcp-server`: MCP sidecar process

The sidecars are intentional. They keep provider-specific and MCP-specific behavior behind stable process boundaries instead of contaminating the main server.

## Quick Start

### Install release bundle

```bash
./scripts/install-runtime.sh latest
```

Or directly from GitHub:

```bash
curl -fsSL https://raw.githubusercontent.com/amxv/gg-agent-runtime/main/scripts/install-runtime.sh | \
  bash -s -- latest
```

Only set `GG_RUNTIME_REPO` if you intentionally want to install from a fork:

```bash
GG_RUNTIME_REPO=owner/repo ./scripts/install-runtime.sh latest
```

### Login providers on the machine

```bash
codex login
claude login
```

### Start the runtime

```bash
export PATH="$HOME/.local/bin:$PATH"
cp "$HOME/.local/runtime-server.toml.example" ./runtime-server.toml
gg-runtime-server --config ./runtime-server.toml
```

That is the default path. You do not need to customize config unless you want different bind addresses, auth settings, or data locations.

## What A Frontend Talks To

The runtime exposes:

- JSON HTTP endpoints for lifecycle actions
- SSE streams for live event delivery
- OpenAPI output for the public route surface

Important routes:

- `GET /health`
- `GET /openapi.yaml`
- `GET /v1/openapi.yaml`
- `/v1/providers/*`
- `/v1/sessions/*`
- `/v1/events/stream`
- `/v1/teams/*`
- `/v1/processes/*`
- `/v1/worktrees/*`
- `/v1/mcp/*`

This is the core design bet of the repo: your application should talk to a runtime service, not directly to provider CLIs.

## Auth Model

- Codex: machine login via `codex login`, with staged runtime auth support from `~/.gg/codex/auth.json`
- Claude default: `host_machine`, which uses machine login material
- Claude optional: `runtime_managed`, which lets the runtime own imported auth/config files

## OpenAPI

- public route: `GET /openapi.yaml`
- auth route: `GET /v1/openapi.yaml`
- repo artifact: [`openapi/runtime-server-openapi.yaml`](openapi/runtime-server-openapi.yaml)

Regenerate it with:

```bash
cargo run -p runtime-server --bin gg-runtime-server -- --write-openapi
```

The OpenAPI file is generated from maintained source parsing in
[`crates/runtime-server/src/openapi.rs`](crates/runtime-server/src/openapi.rs),
not from runtime route introspection. That is a deliberate tradeoff and is documented honestly.

## Source Install

If you want to build from source instead of downloading a release bundle:

```bash
./scripts/install-from-source.sh
```

## Release Pipeline

The repo includes a GitHub Actions release workflow:

- [`.github/workflows/release.yml`](.github/workflows/release.yml)

It builds release bundles for:

- Linux x86_64
- macOS arm64
- macOS x86_64

and publishes `gg-runtime-<platform>-<arch>.tar.gz` assets on `v*` tags.

## Docs

- Install: [docs/INSTALL.md](docs/INSTALL.md)
- Deployment: [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)
- API: [docs/API.md](docs/API.md)
- Architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Config template: [examples/runtime-server.toml](examples/runtime-server.toml)

## Status

This is a real deployable runtime, not a sketch repo. The branch that landed this extracted:

- provider-backed sessions
- durable event replay
- team/comms flows
- worktree lifecycle support
- MCP sidecar support
- real Codex and Claude validation
- release packaging and docs

The natural next layer on top of this repo is better clients, not more backend reinvention.
