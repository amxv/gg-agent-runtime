use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use runtime_core::RuntimeApp;

#[derive(Clone)]
pub struct AppState {
    pub app: Arc<RuntimeApp>,
    pub bearer_token: String,
    pub public_base_url: String,
}

pub fn build_router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/health", get(protected_health))
        .route("/providers", get(list_providers))
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

#[derive(Debug, Serialize)]
struct ProviderListResponse {
    providers: Vec<runtime_core::ProviderMetadata>,
}

async fn list_providers(State(state): State<AppState>) -> Json<ProviderListResponse> {
    Json(ProviderListResponse {
        providers: state.app.provider_registry.metadata(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use tower::ServiceExt;

    use crate::bootstrap::bootstrap_runtime;
    use crate::config::RuntimeServerConfig;

    #[tokio::test]
    async fn health_route_is_public() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut config = RuntimeServerConfig::default();
        config.data.root_dir = temp_dir.path().to_path_buf();
        let bootstrapped = bootstrap_runtime(config).await.expect("bootstrap");

        let router = build_router(AppState {
            app: bootstrapped.app,
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
}
