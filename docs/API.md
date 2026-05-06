# API Guide

## OpenAPI

- Public route: `GET /openapi.yaml`
- Auth route: `GET /v1/openapi.yaml`
- Generated artifact in repo: [`openapi/runtime-server-openapi.yaml`](../openapi/runtime-server-openapi.yaml)

Regenerate artifact:

```bash
cargo run -p runtime-server --bin gg-runtime-server -- --write-openapi
```

The served OpenAPI is generated from a maintained source extractor in
`crates/runtime-server/src/openapi.rs` that parses route declarations in
`crates/runtime-server/src/http.rs` (not runtime route introspection).

## Core Endpoint Groups

- Health/version: `/health`, `/v1/health`, `/v1/version`
- Provider/auth: `/v1/providers/*`
- Sessions/turns/events/SSE: `/v1/sessions/*`, `/v1/events*`
- Teams/comms: `/v1/teams/*`
- Processes: `/v1/processes/*`
- Worktrees: `/v1/worktrees/*`
- Runtime MCP: `/v1/mcp/*`

## SSE Usage

Global:

```bash
curl -N -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:8080/v1/events/stream"
```

Per-session:

```bash
curl -N -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:8080/v1/sessions/$SESSION_ID/events/stream"
```

## Contract Notes

- Route/method coverage in OpenAPI is generated from maintained source parsing of route declarations in server code.
- Most request/response bodies are intentionally represented as `JsonObject` in the current generated schema.
- Runtime behavior remains source of truth if there is any temporary shape drift.
