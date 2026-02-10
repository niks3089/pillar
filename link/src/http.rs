use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use crate::metrics::Metrics;
use crate::state_reader::SharedState;

#[derive(Clone)]
pub struct AppState {
    pub shared_state: SharedState,
    pub metrics: Arc<Metrics>,
}

pub fn router(app_state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/version", get(version))
        .route("/metrics", get(metrics))
        .with_state(app_state)
}

/// 200 if the node is healthy, 503 otherwise.
async fn health(State(app): State<AppState>) -> impl IntoResponse {
    let state = app.shared_state.read().await;
    match state.as_ref() {
        Some(s) if s.healthy => (StatusCode::OK, "healthy"),
        Some(_) => (StatusCode::SERVICE_UNAVAILABLE, "unhealthy"),
        None => (StatusCode::SERVICE_UNAVAILABLE, "no state"),
    }
}

/// Full node status as JSON (proto types have serde derives).
async fn status(State(app): State<AppState>) -> impl IntoResponse {
    let state = app.shared_state.read().await;
    match state.as_ref() {
        Some(s) => match serde_json::to_value(s) {
            Ok(v) => (StatusCode::OK, axum::Json(v)).into_response(),
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize status");
                (StatusCode::INTERNAL_SERVER_ERROR, "serialization error").into_response()
            }
        },
        None => (StatusCode::SERVICE_UNAVAILABLE, "no operator state available").into_response(),
    }
}

/// Service version info.
async fn version() -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "service": "link",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Prometheus text format metrics.
async fn metrics(State(app): State<AppState>) -> impl IntoResponse {
    let body = app.metrics.gather();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use shared::proto::NodeStatus;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    fn test_app(state: Option<NodeStatus>) -> Router {
        let app_state = AppState {
            shared_state: Arc::new(RwLock::new(state)),
            metrics: Arc::new(crate::metrics::Metrics::new()),
        };
        router(app_state)
    }

    fn healthy_status() -> NodeStatus {
        NodeStatus {
            state: "healthy".to_string(),
            local_slot: 1000,
            reference_slot: 1005,
            slots_behind: 5,
            healthy: true,
            restart_count: 0,
            crash_looping: false,
            health_check_duration_secs: 0.1,
            version: "0.1.0".to_string(),
            role: "rpc".to_string(),
            client: "agave".to_string(),
            cluster: "mainnet".to_string(),
            updated_at_unix_secs: chrono::Utc::now().timestamp(),
            state_duration_secs: 60,
            validator_process: "agave-validator".to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn health_returns_200_when_healthy() {
        let app = test_app(Some(healthy_status()));
        let resp = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn health_returns_503_when_unhealthy() {
        let mut status = healthy_status();
        status.healthy = false;
        status.state = "behind".to_string();
        let app = test_app(Some(status));
        let resp = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn health_returns_503_when_no_state() {
        let app = test_app(None);
        let resp = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn status_returns_json() {
        let app = test_app(Some(healthy_status()));
        let resp = app
            .oneshot(Request::get("/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["role"], "rpc");
        assert_eq!(json["client"], "agave");
    }

    #[tokio::test]
    async fn status_returns_503_when_no_state() {
        let app = test_app(None);
        let resp = app
            .oneshot(Request::get("/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn version_returns_json() {
        let app = test_app(None);
        let resp = app
            .oneshot(Request::get("/version").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["service"], "link");
    }

    #[tokio::test]
    async fn metrics_returns_prometheus_format() {
        let app = test_app(None);
        let resp = app
            .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("pillar_node_healthy"));
    }
}
