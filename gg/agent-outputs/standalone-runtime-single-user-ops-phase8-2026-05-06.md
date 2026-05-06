# Standalone Runtime Ops Guide (Single-User Hosted)

## Scope
This document describes deployment and operations expectations for the standalone runtime in single-user hosted mode (one operator, one machine/VPS, one runtime instance).

## Runtime assumptions
- Exactly one operator controls the runtime instance.
- API access is guarded by one bearer token (`auth.token` or generated token file).
- Provider credentials are machine-local files; there is no hosted OAuth callback layer.
- The runtime host is trusted and has direct filesystem/process access.

## Required host prerequisites
- Linux/macOS host with `git` and shell utilities available.
- Writable runtime data root (`~/.gg-runtime` by default).
- If Claude is enabled, the `claude-bridge` sidecar and `gg-mcp-server` launcher paths must be executable.
- If Codex is enabled, Codex CLI/app-server dependencies required by `runtime-provider-codex` must be installed.

## Credential placement (canonical)

### Codex
- Source-of-truth login file on host: `~/.gg/codex/auth.json`
- Runtime stages a copy into provider runtime home under data root:
  - `<runtime_data_root>/providers/codex/home/auth.json`
- Do not mutate source auth during runtime operations.

### Claude
Canonical auth/config paths used by bridge runtime validation:
- credentials: `~/.claude/.credentials.json`
- config preference order:
  1. `~/.gg/claude/.claude.json`
  2. `~/.claude.json` (fallback)

Runtime-managed import endpoints may also stage auth/config into runtime provider directories for isolated operation.

## Startup and recovery behavior
On startup, the runtime performs recovery/reconciliation before serving traffic:
- Session runtime recovery:
  - Rehydrates sessions/turns/approvals.
  - Attempts provider-side resume/inspection for non-terminal sessions.
  - Clears stale active-turn references.
  - Resolves orphaned pending approvals.
  - Resubscribes waiters for in-progress turns when recoverable.
- Process recovery:
  - Rehydrates processes.
  - Converts stale `running`/`queued` records from previous crashes to terminal `failed` and persists them.
- Worktree recovery:
  - Normalizes/tombstones malformed or duplicate managed worktree identities.
  - Reconciles conflicting claims and stale references.
- Team delivery recovery:
  - Deferred delivery resume runs when recipients become ready.

Recovery summary and ongoing anomaly signals are exposed through diagnostics endpoints.

## Diagnostics endpoints to monitor
All are authenticated under `/v1`:
- `GET /v1/diagnostics`
- `GET /v1/diagnostics/providers`
- `GET /v1/diagnostics/comms`
- `GET /v1/diagnostics/processes`
- `GET /v1/diagnostics/worktrees`
- `GET /v1/diagnostics/recovery`
- `GET /v1/diagnostics/team-operations`

## SSE and client reconnect expectations
- SSE streams support replay via `after_seq` query and `Last-Event-ID` header.
- `Last-Event-ID` must be a non-negative integer; invalid values return `400`.
- Clients should treat SSE as authoritative event continuity channel and always reconnect with latest received cursor.

## Auth and API hardening expectations
- All `/v1/*` routes require `Authorization: Bearer <token>`.
- Unauthorized requests return `401` JSON error payload.
- Runtime errors map to structured JSON error responses (`400`, `404`, or `500`).

## Common failure modes and operator actions
- Provider auth missing/expired:
  - Symptom: provider diagnostics unhealthy/authenticated=false.
  - Action: refresh source credentials or use runtime auth endpoint/import path.
- Provider sidecar/bridge crash:
  - Symptom: session/provider errors, recovery anomaly entries.
  - Action: inspect provider diagnostics and logs; restart runtime; verify startup recovery completed.
- Worktree cleanup failure:
  - Symptom: team operation diagnostics include worktree cleanup failure codes.
  - Action: inspect repo permissions, git lock state, and manual cleanup policy.
- Process crash/timeout:
  - Symptom: process status transitions to `failed`/`timed_out`/`killed`.
  - Action: inspect process logs (`/v1/processes/{id}/logs`) and rerun with corrected command/cwd.

## VPS deployment expectations
- Run as a dedicated Unix user with controlled HOME directory.
- Restrict network ingress to trusted clients only.
- Keep runtime token out of shell history and logs.
- Back up runtime SQLite and logs if historical diagnostics are required.
- Use external supervision (systemd/launchd) for restart-on-crash behavior.

## Minimum operational validation after deploy
1. `GET /health` returns `ok`.
2. Authenticated `GET /v1/health` succeeds.
3. `GET /v1/diagnostics/recovery` shows startup summary and no critical anomalies.
4. Provider diagnostics report expected auth state for enabled providers.
5. A smoke session + turn roundtrip succeeds for each enabled provider.
