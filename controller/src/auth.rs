use std::sync::Arc;
use std::time::Instant;

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use dashmap::DashMap;
use serde::Deserialize;

use crate::api::ApiState;
use crate::db;

const SESSION_TTL_SECS: u64 = 86400; // 24 hours
const SESSION_ID_BYTES: usize = 32; // 64 hex chars

#[derive(Clone)]
pub struct SessionStore {
    sessions: Arc<DashMap<String, Instant>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
        }
    }

    fn insert(&self, session_id: String) {
        self.sessions.insert(session_id, Instant::now());
    }

    fn validate(&self, session_id: &str) -> bool {
        if let Some(created) = self.sessions.get(session_id) {
            if created.elapsed().as_secs() < SESSION_TTL_SECS {
                return true;
            }
            drop(created);
            self.sessions.remove(session_id);
        }
        false
    }

    fn remove(&self, session_id: &str) {
        self.sessions.remove(session_id);
    }
}

fn generate_session_id() -> String {
    let mut buf = [0u8; SESSION_ID_BYTES];
    getrandom::getrandom(&mut buf).unwrap_or_default();
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hash a password with argon2.
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let mut salt_bytes = [0u8; 16];
    getrandom::getrandom(&mut salt_bytes).expect("getrandom failed");
    let salt = SaltString::encode_b64(&salt_bytes)?;
    let argon2 = Argon2::default();
    let hash = argon2.hash_password(password.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// Verify a password against a stored argon2 hash.
fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
}

pub async fn login(State(state): State<ApiState>, Json(req): Json<LoginRequest>) -> Response {
    let stored_username = db::get_setting(&state.db, "admin_username").await.ok().flatten();
    let stored_hash = db::get_setting(&state.db, "admin_password_hash").await.ok().flatten();

    let (Some(expected_username), Some(expected_hash)) = (stored_username, stored_hash) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "admin credentials not configured"})),
        )
            .into_response();
    };

    if req.username != expected_username || !verify_password(&req.password, &expected_hash) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid username or password"})),
        )
            .into_response();
    }

    let session_id = generate_session_id();
    state.sessions.insert(session_id.clone());

    let cookie = format!(
        "pillar_session={session_id}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_TTL_SECS}"
    );

    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({"ok": true})),
    )
        .into_response()
}

pub async fn auth_check(State(state): State<ApiState>, req: Request) -> Response {
    let authenticated = is_authenticated(&state, &req);
    let username = if authenticated {
        db::get_setting(&state.db, "admin_username")
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    } else {
        String::new()
    };
    Json(serde_json::json!({"authenticated": authenticated, "username": username})).into_response()
}

pub async fn logout(State(state): State<ApiState>, req: Request) -> Response {
    if let Some(session_id) = extract_session_cookie(&req) {
        state.sessions.remove(&session_id);
    }

    let clear_cookie =
        "pillar_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0".to_string();

    (
        StatusCode::OK,
        [(header::SET_COOKIE, clear_cookie)],
        Json(serde_json::json!({"ok": true})),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Change credentials
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ChangeCredentialsRequest {
    current_password: String,
    #[serde(default)]
    new_username: String,
    #[serde(default)]
    new_password: String,
}

pub async fn change_credentials(
    State(state): State<ApiState>,
    Json(req): Json<ChangeCredentialsRequest>,
) -> Response {
    let stored_hash = db::get_setting(&state.db, "admin_password_hash")
        .await
        .ok()
        .flatten();

    let Some(expected_hash) = stored_hash else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "admin credentials not configured"})),
        )
            .into_response();
    };

    if !verify_password(&req.current_password, &expected_hash) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "current password is incorrect"})),
        )
            .into_response();
    }

    let new_username = req.new_username.trim();
    let new_password = req.new_password.trim();

    if new_username.is_empty() && new_password.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "provide new_username and/or new_password"})),
        )
            .into_response();
    }

    if !new_username.is_empty() {
        if let Err(e) = db::set_setting(&state.db, "admin_username", new_username).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    }

    if !new_password.is_empty() {
        let hashed = match hash_password(new_password) {
            Ok(h) => h,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("hashing error: {e}")})),
                )
                    .into_response();
            }
        };
        if let Err(e) = db::set_setting(&state.db, "admin_password_hash", &hashed).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    }

    Json(serde_json::json!({"ok": true})).into_response()
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

pub async fn require_auth(
    State(state): State<ApiState>,
    req: Request,
    next: Next,
) -> Response {
    if is_authenticated(&state, &req) {
        return next.run(req).await;
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "authentication required"})),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_authenticated(state: &ApiState, req: &Request) -> bool {
    // 1. Check session cookie
    if let Some(session_id) = extract_session_cookie(req) {
        if state.sessions.validate(&session_id) {
            return true;
        }
    }

    // 2. Bearer <token> against the admin API token — a separate secret from the
    //    agent enrollment token. Empty api_token fails closed.
    if !state.api_token.is_empty() {
        if let Some(auth_header) = req.headers().get(header::AUTHORIZATION) {
            if let Ok(value) = auth_header.to_str() {
                if let Some(token) = value.strip_prefix("Bearer ") {
                    if constant_time_eq(token, &state.api_token) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Constant-time comparison so token checks don't leak via timing.
pub(crate) fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn extract_session_cookie(req: &Request) -> Option<String> {
    let cookie_header = req.headers().get(header::COOKIE)?;
    let cookie_str = cookie_header.to_str().ok()?;
    for part in cookie_str.split(';') {
        let trimmed = part.trim();
        if let Some(value) = trimmed.strip_prefix("pillar_session=") {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}
