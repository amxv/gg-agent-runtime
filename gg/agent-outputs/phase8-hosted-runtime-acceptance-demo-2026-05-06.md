# Phase 8 Hosted Runtime Acceptance Demo (Single Flow)

## Purpose
Prove the hosted-runtime thesis end-to-end in one operator flow:
- provider auth (Codex + Claude)
- lead session creation
- team creation
- teammate spawn with managed worktree
- SSE event streaming
- process execution
- team message exchange
- cleanup

## Prerequisites
- runtime server running and reachable at `http://127.0.0.1:8080`
- bearer token exported in `GG_RUNTIME_TOKEN`
- host has:
  - `~/.gg/codex/auth.json`
  - `~/.claude/.credentials.json`
  - `~/.gg/claude/.claude.json` (or fallback `~/.claude.json`)
- `jq` installed

## Demo commands (single acceptance flow)

```bash
set -euo pipefail

BASE_URL="http://127.0.0.1:8080"
AUTH_HEADER="Authorization: Bearer ${GG_RUNTIME_TOKEN}"

# 1) Provider auth visibility (Codex + Claude)
curl -sS -H "$AUTH_HEADER" "$BASE_URL/v1/providers/codex/auth/status" | jq .
curl -sS -H "$AUTH_HEADER" "$BASE_URL/v1/providers/claude/auth/status" | jq .

# 2) Create a local git repo used for managed worktree spawn
DEMO_ROOT="$(mktemp -d)"
REPO_DIR="$DEMO_ROOT/repo"
mkdir -p "$REPO_DIR"
git -C "$REPO_DIR" init
echo "phase8 acceptance" > "$REPO_DIR/README.md"
git -C "$REPO_DIR" add README.md
git -C "$REPO_DIR" -c user.name="phase8" -c user.email="phase8@example.com" commit -m "init"

# 3) Create lead session (Codex)
LEAD_SESSION_ID="$(curl -sS -H "$AUTH_HEADER" -H 'content-type: application/json' \
  -d "{\"provider\":\"codex\",\"cwd\":\"$REPO_DIR\",\"model\":\"gpt-5\"}" \
  "$BASE_URL/v1/sessions" | jq -r '.id')"
echo "lead_session_id=$LEAD_SESSION_ID"

# 4) Create team
TEAM_ID="$(curl -sS -H "$AUTH_HEADER" -H 'content-type: application/json' \
  -d "{\"name\":\"Phase 8 Acceptance Team\",\"lead_agent_id\":\"$LEAD_SESSION_ID\"}" \
  "$BASE_URL/v1/teams" | jq -r '.team.id')"
echo "team_id=$TEAM_ID"

# 5) Start SSE stream (global stream shown; team stream also valid)
# Keep this in a separate shell when running manually.
# curl -N -H "$AUTH_HEADER" "$BASE_URL/v1/events/stream"

# 6) Spawn teammate with managed worktree (Claude)
SPAWN_JSON="$(curl -sS -H "$AUTH_HEADER" -H 'content-type: application/json' \
  -d "{\"source_session_id\":\"$LEAD_SESSION_ID\",\"provider\":\"claude\",\"prompt\":\"You are the spawned worker.\",\"worktree\":{\"mode\":\"create\",\"name\":\"phase8-acceptance-worker\",\"branch_prefix\":\"gg\",\"run_init_script\":false}}" \
  "$BASE_URL/v1/teams/$TEAM_ID/members/spawn")"
SPAWNED_SESSION_ID="$(echo "$SPAWN_JSON" | jq -r '.spawned_session.id')"
WORKTREE_ID="$(echo "$SPAWN_JSON" | jq -r '.worktree.id')"
echo "spawned_session_id=$SPAWNED_SESSION_ID"
echo "worktree_id=$WORKTREE_ID"

# 7) Run a process (public process API)
PROCESS_ID="$(curl -sS -H "$AUTH_HEADER" -H 'content-type: application/json' \
  -d '{"command":"echo phase8_acceptance_process_ok"}' \
  "$BASE_URL/v1/processes" | jq -r '.process.process_id')"
echo "process_id=$PROCESS_ID"

# 8) Send a direct team message from lead -> spawned member
curl -sS -H "$AUTH_HEADER" -H 'content-type: application/json' \
  -d "{\"sender_agent_id\":\"$LEAD_SESSION_ID\",\"recipient_agent_id\":\"$SPAWNED_SESSION_ID\",\"input\":[{\"type\":\"text\",\"text\":\"Acceptance ping\"}],\"priority\":\"normal\",\"policy\":\"non_interrupting\"}" \
  "$BASE_URL/v1/teams/$TEAM_ID/messages" | jq .

# 9) Cleanup: remove member, delete team, close sessions
curl -sS -X DELETE -H "$AUTH_HEADER" "$BASE_URL/v1/teams/$TEAM_ID/members/$SPAWNED_SESSION_ID" | jq .
curl -sS -X DELETE -H "$AUTH_HEADER" "$BASE_URL/v1/teams/$TEAM_ID" | jq .
curl -sS -H "$AUTH_HEADER" -H 'content-type: application/json' \
  -d '{"reason":"phase8_acceptance_cleanup"}' "$BASE_URL/v1/sessions/$LEAD_SESSION_ID/close" | jq .
curl -sS -H "$AUTH_HEADER" -H 'content-type: application/json' \
  -d '{"reason":"phase8_acceptance_cleanup"}' "$BASE_URL/v1/sessions/$SPAWNED_SESSION_ID/close" | jq .
```

## Evidence references from this branch run
These real-provider smokes were run and passed in this worktree on 2026-05-06:
- `cargo test -p runtime-server smoke_real_codex_mcp_process_run_with_staged_auth_copy -- --ignored --nocapture`
- `cargo test -p runtime-server smoke_real_codex_phase5_team_comms_slice -- --ignored --nocapture`
- `cargo test -p runtime-server smoke_real_codex_phase6_spawn_worktree_and_cleanup -- --ignored --nocapture`
- `GG_CLAUDE_SMOKE_CREDENTIALS_SOURCE="$HOME/.claude/.credentials.json" GG_CLAUDE_SMOKE_CONFIG_SOURCE="$HOME/.gg/claude/.claude.json" cargo test -p runtime-server ignored_real_claude_http_smoke_exercises_mcp_with_gg_mcp_enabled -- --ignored --nocapture`
