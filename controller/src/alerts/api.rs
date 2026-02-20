use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;

use crate::api::ApiState;
use super::{db, notify};

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/rules", get(list_rules).post(create_rule))
        .route(
            "/rules/{id}",
            get(get_rule).put(update_rule).delete(delete_rule),
        )
        .route("/history", get(list_history))
        .route("/active", get(list_active))
        .route("/settings", get(get_settings).put(save_settings))
        .route("/settings/test", post(test_notification))
}

// ── Rules ───────────────────────────────────────────────────────────

async fn list_rules(State(state): State<ApiState>) -> impl IntoResponse {
    match db::list_rules(&state.db).await {
        Ok(rules) => Json(rules).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_rule(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match db::get_rule(&state.db, &id).await {
        Ok(Some(rule)) => Json(rule).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "rule not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct CreateRuleRequest {
    name: String,
    #[serde(default)]
    description: String,
    field: String,
    operator: String,
    threshold: String,
    #[serde(default)]
    node_id_filter: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_severity")]
    severity: String,
    #[serde(default)]
    cooldown_secs: i64,
}

fn default_true() -> bool {
    true
}
fn default_severity() -> String {
    "warning".to_string()
}

fn rand_u16() -> u16 {
    let mut buf = [0u8; 2];
    getrandom::getrandom(&mut buf).unwrap_or_default();
    u16::from_le_bytes(buf)
}

async fn create_rule(
    State(state): State<ApiState>,
    Json(req): Json<CreateRuleRequest>,
) -> impl IntoResponse {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let id = format!("custom-{}-{:04x}", now, rand_u16());

    let rule = db::AlertRuleRow {
        id,
        name: req.name,
        description: req.description,
        field: req.field,
        operator: req.operator,
        threshold: req.threshold,
        node_id_filter: req.node_id_filter,
        enabled: req.enabled,
        severity: req.severity,
        cooldown_secs: req.cooldown_secs,
        is_default: false,
        created_at: now,
        updated_at: now,
    };

    match db::insert_rule(&state.db, &rule).await {
        Ok(()) => {
            let _ = state.alert_engine.reload().await;
            (StatusCode::CREATED, Json(rule)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn update_rule(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<db::UpdateRuleRequest>,
) -> impl IntoResponse {
    match db::update_rule(&state.db, &id, &req).await {
        Ok(true) => {
            let _ = state.alert_engine.reload().await;
            match db::get_rule(&state.db, &id).await {
                Ok(Some(rule)) => Json(rule).into_response(),
                _ => Json(serde_json::json!({"ok": true})).into_response(),
            }
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "rule not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn delete_rule(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match db::delete_rule(&state.db, &id).await {
        Ok(true) => {
            let _ = state.alert_engine.reload().await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "cannot delete default rule or rule not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ── History ─────────────────────────────────────────────────────────

async fn list_history(
    State(state): State<ApiState>,
    Query(query): Query<db::HistoryQuery>,
) -> impl IntoResponse {
    match db::list_history(&state.db, &query).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn list_active(State(state): State<ApiState>) -> impl IntoResponse {
    match db::list_active(&state.db).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ── SendGrid settings ───────────────────────────────────────────────

async fn get_settings(State(state): State<ApiState>) -> impl IntoResponse {
    let config = notify::load_config(&state.db).await;
    Json(config)
}

async fn save_settings(
    State(state): State<ApiState>,
    Json(config): Json<notify::SendGridConfig>,
) -> impl IntoResponse {
    match notify::save_config(&state.db, &config).await {
        Ok(()) => {
            let _ = state.alert_engine.reload().await;
            Json(config).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn test_notification(State(state): State<ApiState>) -> impl IntoResponse {
    let config = notify::load_config(&state.db).await;
    match notify::send_test(&config).await {
        Ok(()) => {
            Json(serde_json::json!({"ok": true, "message": "test email sent"})).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
