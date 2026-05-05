use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use runtime_core::{
    ApprovalResponseInput, CreateSessionInput, ProviderKind, ResumeSessionInput, RuntimeApp,
    RuntimeError, RuntimeEventRecord, RuntimeSessionManager, SendTurnAccepted, SendTurnInput,
};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[derive(Clone)]
pub struct AppState {
    pub app: Arc<RuntimeApp>,
    pub runtime: Arc<RuntimeSessionManager>,
    pub bearer_token: String,
    pub public_base_url: String,
}

pub fn build_router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/health", get(protected_health))
        .route("/providers", get(list_providers))
        .route("/providers/{provider}/models", get(list_provider_models))
        .route("/providers/codex/auth/status", get(codex_auth_status))
        .route("/version", get(version))
        .route("/sessions", post(create_session).get(list_sessions))
        .route("/sessions/{session_id}", get(get_session))
        .route("/sessions/{session_id}/resume", post(resume_session))
        .route("/sessions/{session_id}/close", post(close_session))
        .route("/sessions/{session_id}/turns", post(send_turn))
        .route(
            "/sessions/{session_id}/turns/{turn_id}/interrupt",
            post(interrupt_turn),
        )
        .route(
            "/sessions/{session_id}/approvals/{approval_id}",
            post(respond_approval),
        )
        .route("/sessions/{session_id}/events", get(replay_session_events))
        .route(
            "/sessions/{session_id}/events/stream",
            get(stream_session_events),
        )
        .route_layer(middleware::from_fn_with_state(
            state.bearer_token.clone(),
            bearer_auth,
        ));

    Router::new()
        .route("/health", get(health))
        .nest("/v1", protected)
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    providers: usize,
    public_base_url: String,
}

#[derive(Debug, Serialize)]
struct VersionResponse {
    version: &'static str,
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        providers: state.app.provider_registry.len(),
        public_base_url: state.public_base_url,
    })
}

async fn protected_health(State(state): State<AppState>) -> Json<HealthResponse> {
    health(State(state)).await
}

async fn version() -> Json<VersionResponse> {
    Json(VersionResponse {
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Debug, Serialize)]
struct ProviderListResponse {
    providers: Vec<runtime_core::ProviderMetadata>,
}

async fn list_providers(State(state): State<AppState>) -> Json<ProviderListResponse> {
    Json(ProviderListResponse {
        providers: state.app.provider_registry.metadata(),
    })
}

#[derive(Debug, Serialize)]
struct ProviderModelsResponse {
    provider: String,
    models: Vec<runtime_core::ProviderModel>,
}

async fn list_provider_models(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Result<Json<ProviderModelsResponse>, ApiError> {
    let provider = parse_provider_kind(provider.as_str())?;
    let adapter = state
        .app
        .provider_registry
        .get(provider)
        .ok_or_else(|| ApiError::not_found(format!("provider {}", provider.as_str())))?;
    let models = adapter.list_models().await.map_err(ApiError::from)?;
    Ok(Json(ProviderModelsResponse {
        provider: provider.as_str().to_string(),
        models,
    }))
}

async fn codex_auth_status(
    State(state): State<AppState>,
) -> Result<Json<runtime_core::ProviderAuthStatus>, ApiError> {
    let status = state
        .runtime
        .provider_auth_status(ProviderKind::Codex)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(status))
}

async fn create_session(
    State(state): State<AppState>,
    Json(input): Json<CreateSessionInput>,
) -> Result<Json<runtime_core::SessionRecord>, ApiError> {
    let session = state
        .runtime
        .create_session(input)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(session))
}

async fn list_sessions(State(state): State<AppState>) -> Json<Vec<runtime_core::SessionRecord>> {
    Json(state.runtime.list_sessions().await)
}

async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<runtime_core::SessionRecord>, ApiError> {
    let session = state
        .runtime
        .get_session(session_id.as_str())
        .await
        .map_err(ApiError::from)?;
    Ok(Json(session))
}

async fn resume_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    input: Option<Json<ResumeSessionInput>>,
) -> Result<Json<runtime_core::SessionRecord>, ApiError> {
    let input = input
        .map(|Json(value)| value)
        .unwrap_or(ResumeSessionInput {
            provider_session_ref: None,
            canonical_provider_session_ref: None,
        });
    let session = state
        .runtime
        .resume_session(session_id.as_str(), input)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(session))
}

#[derive(Debug, Deserialize)]
struct CloseSessionInput {
    reason: Option<String>,
}

async fn close_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    input: Option<Json<CloseSessionInput>>,
) -> Result<Json<runtime_core::SessionRecord>, ApiError> {
    let reason = input.and_then(|Json(value)| value.reason);
    let session = state
        .runtime
        .close_session(session_id.as_str(), reason)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(session))
}

async fn send_turn(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(input): Json<SendTurnInput>,
) -> Result<Json<SendTurnAccepted>, ApiError> {
    let accepted = state
        .runtime
        .send_turn(session_id.as_str(), input)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(accepted))
}

async fn interrupt_turn(
    State(state): State<AppState>,
    Path((session_id, turn_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    state
        .runtime
        .interrupt_turn(session_id.as_str(), turn_id.as_str())
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::ACCEPTED)
}

async fn respond_approval(
    State(state): State<AppState>,
    Path((session_id, approval_id)): Path<(String, String)>,
    Json(input): Json<ApprovalResponseInput>,
) -> Result<Json<runtime_core::ApprovalRecord>, ApiError> {
    let approval = state
        .runtime
        .respond_approval(session_id.as_str(), approval_id.as_str(), input)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(approval))
}

#[derive(Debug, Deserialize)]
struct EventReplayQuery {
    after_seq: Option<i64>,
    limit: Option<usize>,
}

async fn replay_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<EventReplayQuery>,
) -> Result<Json<Vec<RuntimeEventRecord>>, ApiError> {
    let events = state
        .runtime
        .replay_session_events(
            session_id.as_str(),
            query.after_seq,
            query.limit.unwrap_or(500).min(10_000),
        )
        .map_err(ApiError::from)?;
    Ok(Json(events))
}

async fn stream_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<EventReplayQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError>
{
    let _ = state
        .runtime
        .get_session(session_id.as_str())
        .await
        .map_err(ApiError::from)?;

    // Subscribe before replay to avoid missing events appended during replay/live handoff.
    let receiver = state.runtime.subscribe_events();
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok());
    let cursor = query.after_seq.or(last_event_id);
    let replay_page_limit = query.limit.unwrap_or(2_000).clamp(1, 10_000);
    let mut replay_events = Vec::new();
    let mut replay_cursor = cursor;

    loop {
        let page = state
            .runtime
            .replay_session_events(session_id.as_str(), replay_cursor, replay_page_limit)
            .map_err(ApiError::from)?;
        if page.is_empty() {
            break;
        }
        replay_cursor = page.last().map(|event| event.seq);
        let page_len = page.len();
        replay_events.extend(page);
        if page_len < replay_page_limit {
            break;
        }
    }

    #[cfg(test)]
    if let Some(delay_ms) = headers
        .get("x-gg-test-handoff-delay-ms")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
    {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }

    let replay_high_watermark_seq = replay_cursor.or(cursor).unwrap_or(0);

    let replay_stream = tokio_stream::iter(replay_events.into_iter().filter_map(|event| {
        let payload = serde_json::to_string(&event).ok()?;
        Some(Ok(Event::default()
            .id(event.seq.to_string())
            .event(event.kind)
            .data(payload)))
    }));

    let live_stream = BroadcastStream::new(receiver).filter_map(move |next| match next {
        Ok(event) if event.session_id.as_deref() == Some(session_id.as_str()) => {
            if event.seq <= replay_high_watermark_seq {
                return None;
            }
            let payload = match serde_json::to_string(&event) {
                Ok(payload) => payload,
                Err(_) => return None,
            };
            Some(Ok(Event::default()
                .id(event.seq.to_string())
                .event(event.kind)
                .data(payload)))
        }
        Ok(_) => None,
        Err(_) => None,
    });
    let stream = replay_stream.chain(live_stream);

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(10))))
}

fn parse_provider_kind(value: &str) -> Result<ProviderKind, ApiError> {
    ProviderKind::from_str(value)
        .ok_or_else(|| ApiError::bad_request(format!("unknown provider {}", value)))
}

async fn bearer_auth(
    State(expected_token): State<String>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());

    let expected = format!("Bearer {expected_token}");
    if auth_header == Some(expected.as_str()) {
        return next.run(request).await;
    }

    (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response()
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    fn not_found(message: String) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message,
        }
    }
}

impl From<RuntimeError> for ApiError {
    fn from(value: RuntimeError) -> Self {
        match value {
            RuntimeError::NotFound(message) | RuntimeError::ProviderNotRegistered(message) => {
                Self::not_found(message)
            }
            RuntimeError::Configuration(message)
            | RuntimeError::InvalidState(message)
            | RuntimeError::ProtocolViolation(message)
            | RuntimeError::Unsupported(message) => Self::bad_request(message),
            RuntimeError::ProviderAlreadyRegistered(message)
            | RuntimeError::Bootstrap(message)
            | RuntimeError::Io(message) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message,
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use runtime_core::{
        ApprovalDecision, ProviderApprovalResponseRequest, ProviderAuthStatus,
        ProviderCreateSessionRequest, ProviderInterruptTurnRequest, ProviderMetadata,
        ProviderModel, ProviderResumeSessionRequest, ProviderSendTurnRequest, ProviderSession,
        ProviderTurnAck, ProviderTurnResult, ProviderTurnStatus, ProviderWaitTurnRequest,
        RuntimeProvider,
    };
    use runtime_store_sqlite::{SqliteRuntimeStore, SqliteStoreConfig};
    use runtime_tools::{
        ProcessManagerConfig, StubProcessManager, StubTeamCommsService, StubToolGateway,
        StubWorktreeService, TeamCommsConfig, WorktreeServiceConfig,
    };
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};
    use tower::ServiceExt;

    use crate::bootstrap::bootstrap_runtime;
    use crate::config::RuntimeServerConfig;

    #[derive(Default)]
    struct TestProviderState {
        sessions: HashMap<String, TestProviderSession>,
    }

    #[derive(Default)]
    struct TestProviderSession {
        provider_session_ref: String,
        history: Vec<String>,
        completed: HashMap<String, ProviderTurnResult>,
        pending: HashMap<String, ProviderSendTurnRequest>,
    }

    #[derive(Default)]
    struct TestProvider {
        state: Mutex<TestProviderState>,
    }

    impl TestProvider {
        fn extract_text(input: &[serde_json::Value]) -> String {
            for item in input {
                if let Some(text) = item.get("text").and_then(serde_json::Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }
            }
            "empty".to_string()
        }
    }

    #[async_trait::async_trait]
    impl RuntimeProvider for TestProvider {
        fn kind(&self) -> ProviderKind {
            ProviderKind::Codex
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                kind: ProviderKind::Codex,
                display_name: "Test Codex".to_string(),
                enabled: true,
            }
        }

        async fn healthcheck(&self) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, RuntimeError> {
            Ok(vec![ProviderModel {
                id: "test-model".to_string(),
                display_name: "Test Model".to_string(),
            }])
        }

        async fn auth_status(&self) -> Result<ProviderAuthStatus, RuntimeError> {
            Ok(ProviderAuthStatus {
                authenticated: true,
                mode: Some("test".to_string()),
                detail: None,
            })
        }

        async fn create_session(
            &self,
            req: ProviderCreateSessionRequest,
        ) -> Result<ProviderSession, RuntimeError> {
            let mut state = self.state.lock().await;
            state.sessions.insert(
                req.runtime_session_id.clone(),
                TestProviderSession {
                    provider_session_ref: format!("test-thread-{}", req.runtime_session_id),
                    ..Default::default()
                },
            );
            Ok(ProviderSession {
                runtime_session_id: req.runtime_session_id.clone(),
                provider_session_ref: format!("test-thread-{}", req.runtime_session_id),
                canonical_provider_session_ref: None,
            })
        }

        async fn resume_session(
            &self,
            req: ProviderResumeSessionRequest,
        ) -> Result<ProviderSession, RuntimeError> {
            let mut state = self.state.lock().await;
            let session = state
                .sessions
                .entry(req.runtime_session_id.clone())
                .or_default();
            session.provider_session_ref = req.provider_session_ref.clone();
            Ok(ProviderSession {
                runtime_session_id: req.runtime_session_id,
                provider_session_ref: req.provider_session_ref,
                canonical_provider_session_ref: req.canonical_provider_session_ref,
            })
        }

        async fn send_turn(
            &self,
            req: ProviderSendTurnRequest,
        ) -> Result<ProviderTurnAck, RuntimeError> {
            let mut state = self.state.lock().await;
            let session = state
                .sessions
                .get_mut(req.runtime_session_id.as_str())
                .ok_or_else(|| {
                    RuntimeError::NotFound(format!("test session {}", req.runtime_session_id))
                })?;

            if let Some(approval_id) = req.approval_id.clone() {
                session.pending.insert(approval_id, req.clone());
                return Ok(ProviderTurnAck {
                    runtime_session_id: req.runtime_session_id,
                    turn_id: req.turn_id,
                });
            }

            let user_text = Self::extract_text(req.input.as_slice());
            let first_prompt = session
                .history
                .first()
                .cloned()
                .unwrap_or_else(|| "none".to_string());
            let reply = if user_text.contains("first prompt") {
                first_prompt
            } else {
                format!("ack:{user_text}")
            };
            session.history.push(user_text);
            session.completed.insert(
                req.turn_id.clone(),
                ProviderTurnResult {
                    runtime_session_id: req.runtime_session_id.clone(),
                    turn_id: req.turn_id.clone(),
                    status: ProviderTurnStatus::Completed,
                    usage: Some(serde_json::json!({ "last_message": reply })),
                    error: None,
                },
            );

            Ok(ProviderTurnAck {
                runtime_session_id: req.runtime_session_id,
                turn_id: req.turn_id,
            })
        }

        async fn interrupt_turn(
            &self,
            _req: ProviderInterruptTurnRequest,
        ) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn respond_approval(
            &self,
            req: ProviderApprovalResponseRequest,
        ) -> Result<(), RuntimeError> {
            let decision = ApprovalDecision::parse(req.decision.as_str())?;
            let mut state = self.state.lock().await;
            let session = state
                .sessions
                .get_mut(req.runtime_session_id.as_str())
                .ok_or_else(|| {
                    RuntimeError::NotFound(format!("test session {}", req.runtime_session_id))
                })?;

            let pending = session
                .pending
                .remove(req.approval_id.as_str())
                .ok_or_else(|| RuntimeError::NotFound(format!("approval {}", req.approval_id)))?;
            if decision == ApprovalDecision::Decline {
                session.completed.insert(
                    req.turn_id.clone(),
                    ProviderTurnResult {
                        runtime_session_id: req.runtime_session_id,
                        turn_id: req.turn_id,
                        status: ProviderTurnStatus::Interrupted,
                        usage: None,
                        error: Some(serde_json::json!({ "message": "declined" })),
                    },
                );
            } else {
                let user_text = Self::extract_text(pending.input.as_slice());
                session.completed.insert(
                    req.turn_id.clone(),
                    ProviderTurnResult {
                        runtime_session_id: pending.runtime_session_id,
                        turn_id: pending.turn_id,
                        status: ProviderTurnStatus::Completed,
                        usage: Some(serde_json::json!({ "last_message": user_text })),
                        error: None,
                    },
                );
            }
            Ok(())
        }

        async fn wait_for_turn(
            &self,
            req: ProviderWaitTurnRequest,
        ) -> Result<ProviderTurnResult, RuntimeError> {
            let state = self.state.lock().await;
            let session = state
                .sessions
                .get(req.runtime_session_id.as_str())
                .ok_or_else(|| {
                    RuntimeError::NotFound(format!("test session {}", req.runtime_session_id))
                })?;
            session
                .completed
                .get(req.turn_id.as_str())
                .cloned()
                .ok_or_else(|| RuntimeError::NotFound(format!("test turn {}", req.turn_id)))
        }
    }

    async fn build_test_router() -> (Router, String, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(SqliteRuntimeStore::new(SqliteStoreConfig {
            database_path: temp_dir.path().join("runtime.sqlite3"),
        }));

        let mut registry = runtime_core::ProviderRegistry::new();
        registry
            .register(Arc::new(TestProvider::default()))
            .expect("register test provider");
        let provider_registry = Arc::new(registry);

        let app = runtime_core::RuntimeApp::new(
            provider_registry.clone(),
            runtime_core::RuntimeServices {
                store: store.clone(),
                tool_gateway: Arc::new(StubToolGateway),
                process_manager: Arc::new(StubProcessManager::new(ProcessManagerConfig {
                    enabled: false,
                    max_concurrent: 1,
                    default_timeout_ms: 60_000,
                    max_output_bytes_per_process: 100_000,
                    allow_shell: false,
                })),
                team_comms: Arc::new(StubTeamCommsService::new(TeamCommsConfig {
                    enabled: true,
                    max_pending_deliveries: 1_000,
                })),
                worktrees: Arc::new(StubWorktreeService::new(WorktreeServiceConfig {
                    enabled: false,
                    root_dir: temp_dir.path().display().to_string(),
                    init_script_path: "none".to_string(),
                    deletion_policy_default: "retain".to_string(),
                })),
            },
            runtime_core::EventQueueLimits {
                live_queue_capacity: 512,
                critical_queue_capacity: 512,
                team_queue_capacity: 512,
            },
            runtime_core::ProcessLimits {
                max_concurrent: 1,
                default_timeout_ms: 60_000,
                max_output_bytes_per_process: 100_000,
            },
            runtime_core::WorktreeSettings {
                enabled: false,
                root_dir: temp_dir.path().display().to_string(),
                init_script_path: "none".to_string(),
                deletion_policy_default: "retain".to_string(),
            },
        )
        .expect("build app");
        app.initialize().await.expect("initialize app");

        let runtime = Arc::new(
            RuntimeSessionManager::new(store, provider_registry, 512).expect("build runtime"),
        );
        let bearer_token = "test-token".to_string();

        let router = build_router(AppState {
            app: Arc::new(app),
            runtime,
            bearer_token: bearer_token.clone(),
            public_base_url: "http://localhost:8080".to_string(),
        });

        (router, bearer_token, temp_dir)
    }

    #[tokio::test]
    async fn version_route_is_available() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut config = RuntimeServerConfig::default();
        config.data.root_dir = temp_dir.path().to_path_buf();
        let bootstrapped = bootstrap_runtime(config).await.expect("bootstrap");

        let token = bootstrapped.auth.bearer_token.clone();
        let router = build_router(AppState {
            app: bootstrapped.app,
            runtime: bootstrapped.runtime,
            bearer_token: token.clone(),
            public_base_url: bootstrapped.public_base_url,
        });

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/v1/version")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("version response");
        assert_eq!(response.status(), StatusCode::OK);
        let payload = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("version body");
        let json: serde_json::Value = serde_json::from_slice(&payload).expect("version json");
        assert_eq!(
            json.get("version").and_then(serde_json::Value::as_str),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    #[tokio::test]
    async fn session_stream_replays_from_cursor_before_live_events() {
        let (router, token, _temp_dir) = build_test_router().await;

        let create_body = serde_json::json!({
            "provider": "codex",
            "model": "test-model",
            "cwd": null,
            "permission_mode": null,
            "metadata": {}
        });
        let create_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(create_body.to_string()))
                    .unwrap(),
            )
            .await
            .expect("create response");
        assert_eq!(create_response.status(), StatusCode::OK);
        let create_payload = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .expect("create payload");
        let created: serde_json::Value =
            serde_json::from_slice(&create_payload).expect("create json");
        let session_id = created
            .get("id")
            .and_then(serde_json::Value::as_str)
            .expect("session id")
            .to_string();

        for text in ["first prompt", "what was my first prompt"] {
            let send_body = serde_json::json!({
                "input": [{ "type": "text", "text": text }],
                "expected_turn_id": null,
                "permission_mode": null
            });
            let send_response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/v1/sessions/{session_id}/turns"))
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::from(send_body.to_string()))
                        .unwrap(),
                )
                .await
                .expect("send response");
            assert_eq!(send_response.status(), StatusCode::OK);

            let mut idle = false;
            for _ in 0..50 {
                let session_response = router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(format!("/v1/sessions/{session_id}"))
                            .header(header::AUTHORIZATION, format!("Bearer {token}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .expect("session response");
                let body = to_bytes(session_response.into_body(), usize::MAX)
                    .await
                    .expect("session body");
                let session: serde_json::Value =
                    serde_json::from_slice(&body).expect("session json");
                if session
                    .get("active_turn_id")
                    .is_some_and(serde_json::Value::is_null)
                {
                    idle = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(idle, "turn did not finish in time for replay test");
        }

        let replay_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/sessions/{session_id}/events"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("events response");
        let replay_payload = to_bytes(replay_response.into_body(), usize::MAX)
            .await
            .expect("events payload");
        let events: Vec<runtime_core::RuntimeEventRecord> =
            serde_json::from_slice(&replay_payload).expect("events json");
        assert!(
            events.len() >= 5,
            "expected at least session.created + 2 turn start/terminal pairs"
        );
        let cursor = events
            .iter()
            .find(|event| event.kind == "turn.completed")
            .map(|event| event.seq)
            .expect("turn.completed seq");
        let expected_ids = events
            .iter()
            .filter(|event| event.seq > cursor)
            .map(|event| event.seq.to_string())
            .collect::<Vec<_>>();
        assert!(
            !expected_ids.is_empty(),
            "expected replay window after cursor"
        );
        let recalled_message = events
            .iter()
            .filter_map(|event| event.payload.get("usage"))
            .filter_map(|usage| usage.get("last_message"))
            .filter_map(serde_json::Value::as_str)
            .find(|message| *message == "first prompt");
        assert_eq!(
            recalled_message,
            Some("first prompt"),
            "second turn should preserve context from the first turn"
        );

        let stream_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/sessions/{session_id}/events/stream?after_seq={cursor}"
                    ))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("stream response");
        assert_eq!(stream_response.status(), StatusCode::OK);

        let mut data_stream = stream_response.into_body().into_data_stream();
        let mut sse_payload = String::new();
        for _ in 0..8 {
            let next = timeout(Duration::from_secs(1), data_stream.next()).await;
            match next {
                Ok(Some(Ok(chunk))) => {
                    sse_payload.push_str(String::from_utf8_lossy(chunk.as_ref()).as_ref());
                    let all_present = expected_ids
                        .iter()
                        .all(|seq| sse_payload.contains(format!("id: {seq}").as_str()));
                    if all_present {
                        break;
                    }
                }
                _ => break,
            }
        }
        for seq in expected_ids {
            assert!(
                sse_payload.contains(format!("id: {seq}").as_str()),
                "missing replayed seq {seq} in SSE payload: {sse_payload}"
            );
        }
    }

    #[tokio::test]
    async fn session_stream_replays_exhaustive_backlog_across_pages() {
        let (router, token, _temp_dir) = build_test_router().await;

        let create_body = serde_json::json!({
            "provider": "codex",
            "model": "test-model",
            "cwd": null,
            "permission_mode": null,
            "metadata": {}
        });
        let create_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(create_body.to_string()))
                    .unwrap(),
            )
            .await
            .expect("create response");
        assert_eq!(create_response.status(), StatusCode::OK);
        let create_payload = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .expect("create payload");
        let created: serde_json::Value =
            serde_json::from_slice(&create_payload).expect("create json");
        let session_id = created
            .get("id")
            .and_then(serde_json::Value::as_str)
            .expect("session id")
            .to_string();

        for index in 0..8 {
            let send_body = serde_json::json!({
                "input": [{ "type": "text", "text": format!("replay page turn {index}") }],
                "expected_turn_id": null,
                "permission_mode": null
            });
            let send_response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/v1/sessions/{session_id}/turns"))
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::from(send_body.to_string()))
                        .unwrap(),
                )
                .await
                .expect("send response");
            assert_eq!(send_response.status(), StatusCode::OK);

            let mut idle = false;
            for _ in 0..80 {
                let session_response = router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(format!("/v1/sessions/{session_id}"))
                            .header(header::AUTHORIZATION, format!("Bearer {token}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .expect("session response");
                let body = to_bytes(session_response.into_body(), usize::MAX)
                    .await
                    .expect("session body");
                let session: serde_json::Value =
                    serde_json::from_slice(&body).expect("session json");
                if session
                    .get("active_turn_id")
                    .is_some_and(serde_json::Value::is_null)
                {
                    idle = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(idle, "turn {index} did not finish in time");
        }

        let events_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/sessions/{session_id}/events"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("events response");
        assert_eq!(events_response.status(), StatusCode::OK);
        let events_payload = to_bytes(events_response.into_body(), usize::MAX)
            .await
            .expect("events payload");
        let events: Vec<runtime_core::RuntimeEventRecord> =
            serde_json::from_slice(&events_payload).expect("events json");
        assert!(
            events.len() > 10,
            "expected sizable backlog for pagination regression"
        );
        let cursor = 1_i64;
        let expected_ids = events
            .iter()
            .filter(|event| event.seq > cursor)
            .map(|event| event.seq)
            .collect::<Vec<_>>();
        assert!(
            expected_ids.len() > 8,
            "expected more than one replay page of missed events"
        );

        let stream_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/sessions/{session_id}/events/stream?after_seq={cursor}&limit=3"
                    ))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("stream response");
        assert_eq!(stream_response.status(), StatusCode::OK);

        let mut data_stream = stream_response.into_body().into_data_stream();
        let mut sse_payload = String::new();
        for _ in 0..80 {
            let next = timeout(Duration::from_millis(300), data_stream.next()).await;
            match next {
                Ok(Some(Ok(chunk))) => {
                    sse_payload.push_str(String::from_utf8_lossy(chunk.as_ref()).as_ref());
                    let all_present = expected_ids
                        .iter()
                        .all(|seq| sse_payload.contains(format!("id: {seq}\n").as_str()));
                    if all_present {
                        break;
                    }
                }
                _ => break,
            }
        }

        for seq in expected_ids {
            assert!(
                sse_payload.contains(format!("id: {seq}\n").as_str()),
                "missing replay backlog seq {seq} in paged stream payload"
            );
        }
    }

    #[tokio::test]
    async fn session_stream_handoff_window_event_is_not_lost() {
        let (router, token, _temp_dir) = build_test_router().await;

        let create_body = serde_json::json!({
            "provider": "codex",
            "model": "test-model",
            "cwd": null,
            "permission_mode": null,
            "metadata": {}
        });
        let create_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(create_body.to_string()))
                    .unwrap(),
            )
            .await
            .expect("create response");
        assert_eq!(create_response.status(), StatusCode::OK);
        let create_payload = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .expect("create payload");
        let created: serde_json::Value =
            serde_json::from_slice(&create_payload).expect("create json");
        let session_id = created
            .get("id")
            .and_then(serde_json::Value::as_str)
            .expect("session id")
            .to_string();

        let events_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/sessions/{session_id}/events"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("events response");
        assert_eq!(events_response.status(), StatusCode::OK);
        let events_payload = to_bytes(events_response.into_body(), usize::MAX)
            .await
            .expect("events payload");
        let events: Vec<runtime_core::RuntimeEventRecord> =
            serde_json::from_slice(&events_payload).expect("events json");
        let cursor = events.last().map(|event| event.seq).unwrap_or(0);

        let stream_router = router.clone();
        let stream_token = token.clone();
        let stream_session_id = session_id.clone();
        let stream_handle = tokio::spawn(async move {
            stream_router
                .oneshot(
                    Request::builder()
                        .uri(format!(
                            "/v1/sessions/{stream_session_id}/events/stream?after_seq={cursor}"
                        ))
                        .header(header::AUTHORIZATION, format!("Bearer {stream_token}"))
                        .header("x-gg-test-handoff-delay-ms", "300")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(80)).await;
        let send_body = serde_json::json!({
            "input": [{ "type": "text", "text": "handoff window message" }],
            "expected_turn_id": null,
            "permission_mode": null
        });
        let send_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/sessions/{session_id}/turns"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(send_body.to_string()))
                    .unwrap(),
            )
            .await
            .expect("send response");
        assert_eq!(send_response.status(), StatusCode::OK);

        let stream_response = stream_handle
            .await
            .expect("stream task join")
            .expect("stream response");
        assert_eq!(stream_response.status(), StatusCode::OK);

        let mut data_stream = stream_response.into_body().into_data_stream();
        let mut sse_payload = String::new();
        for _ in 0..8 {
            let next = timeout(Duration::from_secs(1), data_stream.next()).await;
            match next {
                Ok(Some(Ok(chunk))) => {
                    sse_payload.push_str(String::from_utf8_lossy(chunk.as_ref()).as_ref());
                    if sse_payload.contains("event: turn.started")
                        || sse_payload.contains("event: turn.completed")
                    {
                        break;
                    }
                }
                _ => break,
            }
        }

        assert!(
            sse_payload.contains("event: turn.started")
                || sse_payload.contains("event: turn.completed"),
            "expected handoff-window event to be delivered in stream payload: {sse_payload}"
        );
    }

    #[tokio::test]
    async fn health_route_is_public() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut config = RuntimeServerConfig::default();
        config.data.root_dir = temp_dir.path().to_path_buf();
        let bootstrapped = bootstrap_runtime(config).await.expect("bootstrap");

        let router = build_router(AppState {
            app: bootstrapped.app,
            runtime: bootstrapped.runtime,
            bearer_token: bootstrapped.auth.bearer_token,
            public_base_url: bootstrapped.public_base_url,
        });

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn protected_route_requires_token() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut config = RuntimeServerConfig::default();
        config.data.root_dir = temp_dir.path().to_path_buf();
        let bootstrapped = bootstrap_runtime(config).await.expect("bootstrap");

        let token = bootstrapped.auth.bearer_token.clone();
        let router = build_router(AppState {
            app: bootstrapped.app,
            runtime: bootstrapped.runtime,
            bearer_token: token.clone(),
            public_base_url: bootstrapped.public_base_url,
        });

        let unauthorized = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("response");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = router
            .oneshot(
                Request::builder()
                    .uri("/v1/health")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("response");
        assert_eq!(authorized.status(), StatusCode::OK);
    }

    #[tokio::test]
    #[ignore = "real Codex smoke test: requires local ~/.gg/codex/auth.json"]
    async fn smoke_real_codex_runtime_slice_with_staged_auth_copy() {
        let home_dir = std::env::var("HOME").expect("HOME must be set");
        let source_auth = std::path::PathBuf::from(home_dir)
            .join(".gg")
            .join("codex")
            .join("auth.json");
        assert!(
            source_auth.exists(),
            "expected real auth file at {}",
            source_auth.display()
        );

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut config = RuntimeServerConfig::default();
        config.data.root_dir = temp_dir.path().to_path_buf();
        config.providers.claude.enabled = false;
        config.providers.codex.enabled = true;

        let bootstrapped = bootstrap_runtime(config.clone()).await.expect("bootstrap");

        let staged_auth = config
            .resolve_provider_dir("codex")
            .join("home")
            .join("auth.json");
        assert!(staged_auth.exists(), "expected staged auth copy");

        let token = bootstrapped.auth.bearer_token.clone();
        let router = build_router(AppState {
            app: bootstrapped.app,
            runtime: bootstrapped.runtime,
            bearer_token: token.clone(),
            public_base_url: bootstrapped.public_base_url,
        });

        let auth_status_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/providers/codex/auth/status")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("auth status response");
        assert_eq!(auth_status_response.status(), StatusCode::OK);
        let auth_status_bytes = to_bytes(auth_status_response.into_body(), usize::MAX)
            .await
            .expect("auth status body");
        let auth_status_json: serde_json::Value =
            serde_json::from_slice(&auth_status_bytes).expect("auth status json");
        assert_eq!(
            auth_status_json["authenticated"].as_bool(),
            Some(true),
            "expected codex auth to be authenticated"
        );

        let create_body = serde_json::json!({
            "provider": "codex",
            "model": "gpt-5.2-codex",
            "cwd": temp_dir.path().display().to_string(),
            "permission_mode": null,
            "metadata": {
                "smoke": "real_codex_phase3"
            }
        });
        let create_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(create_body.to_string()))
                    .unwrap(),
            )
            .await
            .expect("create response");
        assert_eq!(create_response.status(), StatusCode::OK);
        let create_bytes = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .expect("create body");
        let created_session: serde_json::Value =
            serde_json::from_slice(&create_bytes).expect("create json");
        let session_id = created_session["id"]
            .as_str()
            .expect("session id")
            .to_string();

        let turn_body = serde_json::json!({
            "input": [
                {
                    "type": "text",
                    "text": "Reply with exactly this token and nothing else: phase3token_94731"
                }
            ],
            "expected_turn_id": null,
            "permission_mode": null
        });
        let send_turn_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/sessions/{session_id}/turns"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(turn_body.to_string()))
                    .unwrap(),
            )
            .await
            .expect("send turn response");
        assert_eq!(send_turn_response.status(), StatusCode::OK);
        let send_turn_bytes = to_bytes(send_turn_response.into_body(), usize::MAX)
            .await
            .expect("send turn body");
        let accepted_turn: serde_json::Value =
            serde_json::from_slice(&send_turn_bytes).expect("send turn json");
        let turn_id = accepted_turn["turn_id"]
            .as_str()
            .expect("turn id")
            .to_string();

        let mut finished = false;
        for _attempt in 0..80 {
            let get_session_response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/v1/sessions/{session_id}"))
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .expect("get session response");
            assert_eq!(get_session_response.status(), StatusCode::OK);
            let session_bytes = to_bytes(get_session_response.into_body(), usize::MAX)
                .await
                .expect("get session body");
            let session_json: serde_json::Value =
                serde_json::from_slice(&session_bytes).expect("get session json");
            if session_json["active_turn_id"].is_null() {
                finished = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        assert!(finished, "turn did not reach terminal state in time");

        let second_turn_body = serde_json::json!({
            "input": [
                {
                    "type": "text",
                    "text": "What exact token did you reply with previously? Reply with only that token."
                }
            ],
            "expected_turn_id": null,
            "permission_mode": null
        });
        let second_send_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/sessions/{session_id}/turns"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(second_turn_body.to_string()))
                    .unwrap(),
            )
            .await
            .expect("second send response");
        assert_eq!(second_send_response.status(), StatusCode::OK);

        let mut second_finished = false;
        for _attempt in 0..80 {
            let get_session_response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/v1/sessions/{session_id}"))
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .expect("get session response");
            assert_eq!(get_session_response.status(), StatusCode::OK);
            let session_bytes = to_bytes(get_session_response.into_body(), usize::MAX)
                .await
                .expect("get session body");
            let session_json: serde_json::Value =
                serde_json::from_slice(&session_bytes).expect("get session json");
            if session_json["active_turn_id"].is_null() {
                second_finished = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        assert!(
            second_finished,
            "second turn did not reach terminal state in time"
        );

        let events_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/sessions/{session_id}/events"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("events response");
        assert_eq!(events_response.status(), StatusCode::OK);
        let events_bytes = to_bytes(events_response.into_body(), usize::MAX)
            .await
            .expect("events body");
        let events: serde_json::Value = serde_json::from_slice(&events_bytes).expect("events json");
        let kinds = events
            .as_array()
            .expect("events array")
            .iter()
            .filter_map(|event| event.get("kind").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert!(
            kinds.contains(&"turn.started"),
            "missing turn.started event"
        );
        assert!(
            kinds.contains(&"turn.completed")
                || kinds.contains(&"turn.failed")
                || kinds.contains(&"turn.interrupted"),
            "missing terminal turn event for {}",
            turn_id
        );
        let terminal_count_before_restart = kinds
            .iter()
            .filter(|kind| {
                **kind == "turn.completed"
                    || **kind == "turn.failed"
                    || **kind == "turn.interrupted"
            })
            .count();
        assert!(
            terminal_count_before_restart >= 2,
            "expected at least two terminal turns before restart"
        );

        // Simulate restart and verify persisted session can be resumed and used.
        let restarted = bootstrap_runtime(config.clone())
            .await
            .expect("restart bootstrap");
        let restarted_token = restarted.auth.bearer_token.clone();
        let restarted_router = build_router(AppState {
            app: restarted.app,
            runtime: restarted.runtime,
            bearer_token: restarted_token.clone(),
            public_base_url: restarted.public_base_url,
        });

        let resume_response = restarted_router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/sessions/{session_id}/resume"))
                    .header(header::AUTHORIZATION, format!("Bearer {restarted_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("resume response");
        assert_eq!(resume_response.status(), StatusCode::OK);

        let third_turn_body = serde_json::json!({
            "input": [
                {
                    "type": "text",
                    "text": "After resume, what exact token did you output earlier? Reply with only the token."
                }
            ],
            "expected_turn_id": null,
            "permission_mode": null
        });
        let third_send_response = restarted_router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/sessions/{session_id}/turns"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {restarted_token}"))
                    .body(Body::from(third_turn_body.to_string()))
                    .unwrap(),
            )
            .await
            .expect("third send response");
        assert_eq!(third_send_response.status(), StatusCode::OK);

        let mut third_finished = false;
        for _attempt in 0..80 {
            let get_session_response = restarted_router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/v1/sessions/{session_id}"))
                        .header(header::AUTHORIZATION, format!("Bearer {restarted_token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .expect("get resumed session response");
            assert_eq!(get_session_response.status(), StatusCode::OK);
            let session_bytes = to_bytes(get_session_response.into_body(), usize::MAX)
                .await
                .expect("get resumed session body");
            let session_json: serde_json::Value =
                serde_json::from_slice(&session_bytes).expect("get resumed session json");
            if session_json["active_turn_id"].is_null() {
                third_finished = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        assert!(
            third_finished,
            "third turn after resume did not reach terminal state in time"
        );

        let resumed_events_response = restarted_router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/sessions/{session_id}/events"))
                    .header(header::AUTHORIZATION, format!("Bearer {restarted_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("resumed events response");
        assert_eq!(resumed_events_response.status(), StatusCode::OK);
        let resumed_events_bytes = to_bytes(resumed_events_response.into_body(), usize::MAX)
            .await
            .expect("resumed events body");
        let resumed_events: serde_json::Value =
            serde_json::from_slice(&resumed_events_bytes).expect("resumed events json");
        let resumed_kinds = resumed_events
            .as_array()
            .expect("resumed events array")
            .iter()
            .filter_map(|event| event.get("kind"))
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(
            resumed_kinds.iter().any(|kind| kind == "session.resumed"),
            "expected session.resumed event after explicit resume"
        );
        let terminal_count_after_resume = resumed_kinds
            .iter()
            .filter(|kind| {
                kind.as_str() == "turn.completed"
                    || kind.as_str() == "turn.failed"
                    || kind.as_str() == "turn.interrupted"
            })
            .count();
        assert!(
            terminal_count_after_resume >= 3,
            "expected another terminal turn after resume"
        );

        let close_response = restarted_router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/sessions/{session_id}/close"))
                    .header(header::AUTHORIZATION, format!("Bearer {restarted_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("close response");
        assert_eq!(close_response.status(), StatusCode::OK);
    }
}
