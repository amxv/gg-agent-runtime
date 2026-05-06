# Standalone Runtime Acceptance Review

Date: 2026-05-06

Verdict: changes required

## Findings

### High: Codex turns are not deployable end-to-end on the real standalone server

The real localhost server booted and accepted a Codex session plus `POST /v1/sessions/{id}/turns`, but the external SSE stream emitted `turn.failed` immediately. The captured stderr from the live run was:

- `error: unexpected argument '-o' found`
- `Usage: codex [OPTIONS] [PROMPT]`
- `Usage: codex [OPTIONS] <COMMAND> [ARGS]`

The provider is currently building the command as top-level `codex -o <file> exec ...` in [crates/runtime-provider-codex/src/lib.rs](/Users/ashray/.gg/worktrees/Users__ashray__code__amxv__gg-agent-runtime/gg--standalone-agent-runtime/crates/runtime-provider-codex/src/lib.rs:152). On the installed CLI, `-o/--output-last-message` is an `exec` subcommand option, not a top-level `codex` option. During review, `codex exec --help` showed the valid shape as `codex exec -o <file> ...`.

This is not just a bad acceptance config. After switching to an absolute `CODEX_HOME`, plain `codex exec ...` and `codex exec -o ...` both started, while `codex -o ... exec ...` still failed exactly as above. So the current blocker is the invocation contract in the provider, not only the home path.

The real Codex smoke on this branch did not catch this because it treats any terminal turn as acceptable. The test explicitly accepts `turn.completed`, `turn.failed`, or `turn.interrupted` as success criteria in [crates/runtime-server/src/http.rs](/Users/ashray/.gg/worktrees/Users__ashray__code__amxv__gg-agent-runtime/gg--standalone-agent-runtime/crates/runtime-server/src/http.rs:5216). That means the smoke can pass even when the Codex vertical slice is functionally broken for real users.

### High: Claude auth is still coupled to the operator's canonical host files instead of being runtime-isolated

The design doc requires runtime-managed Claude auth/config so the standalone service is self-contained. In the live review run, `GET /v1/providers/claude/auth/status` returned `authenticated=true` while also reporting:

- `runtime_credentials_present=false`
- `runtime_config_present=false`
- `bridge_credentials_present=true`
- `bridge_config_present=true`

In other words, the runtime was considered authenticated because the bridge could see the operator's existing `~/.claude/.credentials.json` and `~/.gg/claude/.claude.json`, even though the runtime-managed files were absent.

This behavior is encoded directly in [crates/runtime-provider-claude/src/lib.rs](/Users/ashray/.gg/worktrees/Users__ashray__code__amxv__gg-agent-runtime/gg--standalone-agent-runtime/crates/runtime-provider-claude/src/lib.rs:623), where auth status is computed from either runtime-managed files or canonical bridge-visible files. Bridge startup validation also instructs the operator to rely on canonical host files in [crates/runtime-provider-claude/src/lib.rs](/Users/ashray/.gg/worktrees/Users__ashray__code__amxv__gg-agent-runtime/gg--standalone-agent-runtime/crates/runtime-provider-claude/src/lib.rs:409).

That is a release blocker for the "deployable standalone runtime" claim because a fresh host without pre-existing user-level Claude state would not behave the same way as this branch review environment.

### Medium: The public HTTP contract is still incomplete and not yet a stable OSS-style release surface

The branch does expose a working Axum HTTP server, but the release contract is still partial:

- There is no generated OpenAPI artifact, no OpenAPI route, and no codepath for spec generation in `runtime-server`.
- There is no repo README, no release flow doc, and no top-level usage/deployment document beyond the phase-specific ops/demo notes under `gg/agent-outputs/`.
- The live route table only exposes `GET /v1/providers/codex/auth/status`; the Codex auth start/api-key/cancel/logout endpoints called for by the design doc are absent in [crates/runtime-server/src/http.rs](/Users/ashray/.gg/worktrees/Users__ashray__code__amxv__gg-agent-runtime/gg--standalone-agent-runtime/crates/runtime-server/src/http.rs:47).
- The process API currently expects `command: string` in [crates/runtime-core/src/services.rs](/Users/ashray/.gg/worktrees/Users__ashray__code__amxv__gg-agent-runtime/gg--standalone-agent-runtime/crates/runtime-core/src/services.rs:96) and [crates/runtime-server/src/http.rs](/Users/ashray/.gg/worktrees/Users__ashray__code__amxv__gg-agent-runtime/gg--standalone-agent-runtime/crates/runtime-server/src/http.rs:1595), while the design doc's OpenAPI draft specifies an argv-style command array.
- Session creation returns a raw `SessionRecord` with `id` in [crates/runtime-server/src/http.rs](/Users/ashray/.gg/worktrees/Users__ashray__code__amxv__gg-agent-runtime/gg--standalone-agent-runtime/crates/runtime-server/src/http.rs:326) and [crates/runtime-core/src/state.rs](/Users/ashray/.gg/worktrees/Users__ashray__code__amxv__gg-agent-runtime/gg--standalone-agent-runtime/crates/runtime-core/src/state.rs:61), rather than the design draft's `session_id` schema and 201 semantics. Team creation has the same kind of drift in [crates/runtime-server/src/http.rs](/Users/ashray/.gg/worktrees/Users__ashray__code__amxv__gg-agent-runtime/gg--standalone-agent-runtime/crates/runtime-server/src/http.rs:630).

The runtime is therefore "API-usable" but not yet "API-frozen and documented" in the sense required for an eventual public OSS release.

### Medium: Relative runtime data roots can break Codex because `CODEX_HOME` is exported as a relative path

My live localhost validation used a temporary relative `data.root_dir`, which flowed into `codex_home` via [crates/runtime-server/src/bootstrap.rs](/Users/ashray/.gg/worktrees/Users__ashray__code__amxv__gg-agent-runtime/gg--standalone-agent-runtime/crates/runtime-server/src/bootstrap.rs:45). The Codex provider then exports that value directly as `CODEX_HOME` and also changes the child process cwd to the session repo in [crates/runtime-provider-codex/src/lib.rs](/Users/ashray/.gg/worktrees/Users__ashray__code__amxv__gg-agent-runtime/gg--standalone-agent-runtime/crates/runtime-provider-codex/src/lib.rs:152).

That produced the live error:

- `CODEX_HOME points to "tmp/standalone-acceptance-data/providers/codex/home", but that path does not exist`

This is a real bug for non-absolute configs. It is not the primary acceptance blocker because the `-o` ordering bug still breaks Codex after fixing the path, but it is still a deployability footgun that should be corrected.

## Validation Performed

- Read the authoritative design doc plus required plan, research, ops, and phase-8 demo documents.
- Reviewed branch state relative to `main`: 8 commits ahead, working tree clean.
- Reviewed the new standalone workspace, server routes, provider adapters, runtime core, SQLite store, tool gateway, Claude sidecar, and gg-mcp sidecar.
- Ran the non-ignored server suite: `cargo test -p runtime-server -- --nocapture`
  - Result: 31 passed, 0 failed, 5 ignored.
- Ran the real Codex smoke set: `cargo test -p runtime-server smoke_real_codex_ -- --ignored --nocapture`
  - Result: 4 passed.
  - Important caveat: the phase-3 Codex smoke currently allows `turn.failed` as a passing terminal outcome, so it is not a sufficient acceptance guard for actual Codex turn success.
- Ran the real Claude HTTP smoke with canonical credential/config sources:
  - `GG_CLAUDE_SMOKE_CREDENTIALS_SOURCE="$HOME/.claude/.credentials.json" GG_CLAUDE_SMOKE_CONFIG_SOURCE="$HOME/.gg/claude/.claude.json" cargo test -p runtime-server ignored_real_claude_http_smoke_exercises_mcp_with_gg_mcp_enabled -- --ignored --nocapture`
  - Result: 1 passed.
- Booted the real `gg-runtime-server` binary on localhost and exercised real HTTP/SSE behavior:
  - `/health`, `/v1/health`, `/v1/version`, `/v1/providers`
  - provider auth status for Codex and Claude
  - live global SSE stream
  - live Codex session create + turn send
  - live team create
  - live Claude teammate spawn with managed worktree
  - live public process execution + log retrieval
  - live team direct message delivery/defer behavior
  - live teammate removal

## Release Readiness

- Standalone HTTP/SSE microservice shape: partial
  - The server boots and the external SSE stream works.
  - Team/worktree/process flows are real and externally reachable.
  - Codex turn execution is not currently reliable enough to accept the runtime as deployable.

- API surface clarity: partial
  - There is a substantial route surface, but response shapes/status codes drift from the design draft and there is no generated OpenAPI contract.

- Docs quality: partial
  - The ops guide and hosted demo notes are useful.
  - They are not a substitute for a repo README, operator setup guide, and release-facing API documentation.

- README: missing

- OpenAPI/spec: missing

- Release flow / OSS packaging guidance: missing

## Residual Risks After Fixing The Blockers

- The Codex and Claude integrations are both sensitive to upstream CLI/SDK changes.
- Team delivery behavior and worktree lifecycle look materially closer to ready than the provider auth/release surface.
- The server has strong feature coverage, but the acceptance bar should remain "changes required" until the real Codex turn path succeeds and the runtime no longer depends on operator-global Claude state.
