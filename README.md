# GG Standalone Agent Runtime

`gg-runtime-server` is shipped as one runtime bundle:
- `bin/gg-runtime-server`
- `sidecars/claude-bridge/claude-bridge`
- `sidecars/gg-mcp-server/gg-mcp-server`

Treat it as one deployable product even though the internals are split into crates.

## Quick Start (VPS/Mac)

1. Install runtime bundle:

```bash
GG_RUNTIME_REPO=owner/repo ./scripts/install-runtime.sh latest
```

or:

```bash
curl -fsSL https://raw.githubusercontent.com/owner/repo/main/scripts/install-runtime.sh | \
  GG_RUNTIME_REPO=owner/repo bash -s -- latest
```

2. Login providers on the machine:

```bash
codex login
claude login
```

3. Start:

```bash
export PATH="$HOME/.local/bin:$PATH"
cp "$HOME/.local/runtime-server.toml.example" ./runtime-server.toml
gg-runtime-server --config ./runtime-server.toml
```

That is the default setup path. Only change config if you need non-default behavior.

## One-Command Source Install

If running from repo source:

```bash
./scripts/install-from-source.sh
```

## OpenAPI (Generated)

- Source of truth route: `GET /openapi.yaml`
- Auth route: `GET /v1/openapi.yaml`
- Artifact: `openapi/runtime-server-openapi.yaml`

Generation is driven by a maintained source extractor in
`crates/runtime-server/src/openapi.rs` that parses route declarations in
`crates/runtime-server/src/http.rs` (it is not runtime route introspection).

Regenerate:

```bash
cargo run -p runtime-server --bin gg-runtime-server -- --write-openapi
```

## Release Pipeline

GitHub Actions release workflow:
- [`.github/workflows/release.yml`](.github/workflows/release.yml)

It builds runtime bundles for:
- Linux x86_64
- macOS arm64
- macOS x86_64

and publishes `gg-runtime-<platform>-<arch>.tar.gz` assets on `v*` tags.

## Docs

- Install: [docs/INSTALL.md](docs/INSTALL.md)
- Deployment: [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)
- API/OpenAPI: [docs/API.md](docs/API.md)
- Architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Config template: [examples/runtime-server.toml](examples/runtime-server.toml)

## Auth Model

- Codex: machine login (`codex login`) with staged runtime auth support.
- Claude default: `providers.claude_auth_mode = "host_machine"` (machine login via `claude login`).
- Claude optional: `providers.claude_auth_mode = "runtime_managed"` for runtime-owned auth material.
