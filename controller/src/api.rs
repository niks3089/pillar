use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::{any, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

use pillar_shared::proto::{
    controller_command, ControllerCommand, LogEntry, NodeStatus, ProvisionCommand, RecoverCommand,
    RestartCommand, StopCommand, UpgradeCommand,
};

use crate::config::ControllerConfig;
use crate::db::{self, Db, NodeRow};
use crate::node_registry::NodeRegistry;

#[derive(Clone)]
pub struct ApiState {
    pub db: Db,
    pub registry: NodeRegistry,
    pub config: ControllerConfig,
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/overview", get(overview))
        .route("/api/nodes", get(list_nodes))
        .route("/api/nodes/{id}", get(get_node).delete(delete_node))
        .route("/api/nodes/{id}/history", get(node_history))
        .route("/api/nodes/{id}/logs", get(node_logs))
        .route("/api/nodes/{id}/logs/stream", get(node_logs_stream))
        .route("/api/nodes/{id}/restart", post(restart_node))
        .route("/api/nodes/{id}/recover", post(recover_node))
        .route("/api/nodes/{id}/provision", post(provision_node))
        .route("/api/nodes/{id}/upgrade", post(upgrade_node))
        .route("/api/nodes/{id}/stop", post(stop_node))
        .route("/api/nodes/{id}/cancel", post(cancel_deployment))
        .route("/api/cluster-defaults/{cluster}", get(cluster_defaults))
        .route("/api/onboard-command", get(onboard_command))
        .route(
            "/api/settings/grafana",
            get(get_grafana_settings).put(set_grafana_settings),
        )
        .route(
            "/api/dashboards/fleet-overview",
            get(dashboard_fleet_overview),
        )
        .route("/api/dashboards/node-detail", get(dashboard_node_detail))
        .route("/metrics", get(crate::metrics_endpoint::metrics_handler))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Query parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    #[serde(default = "default_limit")]
    limit: u32,
}

#[derive(Debug, Deserialize)]
struct LogsQuery {
    service: Option<String>,
    level: Option<String>,
    since: Option<i64>,
    #[serde(default = "default_limit")]
    limit: u32,
}

fn default_limit() -> u32 {
    100
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OnboardCommandResponse {
    command: String,
}

#[derive(Serialize)]
struct CommandResponse {
    ok: bool,
    message: String,
}

/// Wraps a NodeRow with an optional live status from the in-memory registry.
#[derive(Serialize)]
struct NodeWithStatus {
    #[serde(flatten)]
    node: NodeRow,
    #[serde(skip_serializing_if = "Option::is_none")]
    live_status: Option<NodeStatus>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Emit a controller-originated log entry into the node's broadcast channel and persist to DB.
async fn emit_controller_log(
    registry: &NodeRegistry,
    db: &Db,
    node_id: &str,
    level: &str,
    message: &str,
) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let entry = LogEntry {
        service: "controller".to_string(),
        level: level.to_string(),
        message: message.to_string(),
        unit: String::new(),
        timestamp_unix_ms: now_ms,
    };
    registry.publish_logs(node_id, std::slice::from_ref(&entry)).await;
    if let Err(e) = db::insert_logs(db, node_id, std::slice::from_ref(&entry)).await {
        tracing::warn!(error = %e, "failed to persist controller log");
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn overview(State(state): State<ApiState>) -> impl IntoResponse {
    match db::get_fleet_overview(&state.db).await {
        Ok(mut overview) => {
            let live = state.registry.get_all_statuses().await;
            overview.connected_nodes = live.len() as u32;
            Json(overview).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn list_nodes(State(state): State<ApiState>) -> impl IntoResponse {
    match db::list_nodes(&state.db).await {
        Ok(nodes) => {
            let mut result = Vec::with_capacity(nodes.len());
            for node in nodes {
                let live_status = state.registry.get_status(&node.node_id).await;
                result.push(NodeWithStatus { node, live_status });
            }
            Json(result).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_node(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match db::get_node(&state.db, &id).await {
        Ok(Some(node)) => {
            let live_status = state.registry.get_status(&id).await;
            Json(NodeWithStatus { node, live_status }).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "node not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn delete_node(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match db::delete_node(&state.db, &id).await {
        Ok(true) => {
            state.registry.remove_node(&id).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "node not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn node_history(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> impl IntoResponse {
    match db::get_status_history(&state.db, &id, query.limit).await {
        Ok(history) => Json(history).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn node_logs(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> impl IntoResponse {
    match db::get_logs(
        &state.db,
        &id,
        query.service.as_deref(),
        query.level.as_deref(),
        query.since,
        query.limit,
    )
    .await
    {
        Ok(logs) => Json(logs).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn node_logs_stream(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let subscriber = state.registry.get_log_subscriber(&id).await;

    match subscriber {
        Some(rx) => {
            let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|result| {
                match result {
                    Ok(entry) => {
                        let json = serde_json::to_string(&entry).unwrap_or_default();
                        Some(Ok::<_, std::convert::Infallible>(Event::default().data(json)))
                    }
                    // Skip lagged messages.
                    Err(_) => None,
                }
            });
            Sse::new(stream).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "node not found or not connected"})),
        )
            .into_response(),
    }
}

async fn restart_node(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let cmd = ControllerCommand {
        command: Some(controller_command::Command::Restart(RestartCommand {
            reason: "restart requested via API".to_string(),
        })),
    };

    match state.registry.send_command(&id, cmd).await {
        Ok(()) => {
            emit_controller_log(&state.registry, &state.db, &id, "info", "Restart command sent")
                .await;
            Json(CommandResponse {
                ok: true,
                message: "restart command sent".to_string(),
            })
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(CommandResponse {
                ok: false,
                message: e,
            }),
        )
            .into_response(),
    }
}

async fn recover_node(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let cmd = ControllerCommand {
        command: Some(controller_command::Command::Recover(RecoverCommand {
            reason: "recovery requested via API".to_string(),
        })),
    };

    match state.registry.send_command(&id, cmd).await {
        Ok(()) => {
            emit_controller_log(&state.registry, &state.db, &id, "info", "Recovery command sent")
                .await;
            Json(CommandResponse {
                ok: true,
                message: "recover command sent".to_string(),
            })
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(CommandResponse {
                ok: false,
                message: e,
            }),
        )
            .into_response(),
    }
}

async fn stop_node(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let cmd = ControllerCommand {
        command: Some(controller_command::Command::Stop(StopCommand {
            reason: "stop requested via API".to_string(),
        })),
    };

    match state.registry.send_command(&id, cmd).await {
        Ok(()) => {
            if let Err(e) = db::set_lifecycle_state(&state.db, &id, "stopped").await {
                tracing::warn!(error = %e, "failed to set lifecycle_state to stopped");
            }
            emit_controller_log(&state.registry, &state.db, &id, "info", "Stop command sent")
                .await;
            Json(CommandResponse {
                ok: true,
                message: "stop command sent".to_string(),
            })
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(CommandResponse {
                ok: false,
                message: e,
            }),
        )
            .into_response(),
    }
}

async fn cancel_deployment(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Only allow cancel when node is actively deploying
    match db::get_lifecycle_state(&state.db, &id).await {
        Ok(Some(s)) if s == "provisioning" || s == "starting_up" => {}
        Ok(Some(s)) => {
            return (
                StatusCode::CONFLICT,
                Json(CommandResponse {
                    ok: false,
                    message: format!("cannot cancel: node is in '{s}' state, not provisioning/starting_up"),
                }),
            )
                .into_response();
        }
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(CommandResponse {
                    ok: false,
                    message: "node not found".to_string(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(CommandResponse {
                    ok: false,
                    message: format!("db error: {e}"),
                }),
            )
                .into_response();
        }
    }

    let cmd = ControllerCommand {
        command: Some(controller_command::Command::Stop(StopCommand {
            reason: "deployment cancelled via API".to_string(),
        })),
    };

    match state.registry.send_command(&id, cmd).await {
        Ok(()) => {
            if let Err(e) = db::set_lifecycle_state(&state.db, &id, "registered").await {
                tracing::warn!(error = %e, "failed to reset lifecycle_state to registered");
            }
            emit_controller_log(
                &state.registry,
                &state.db,
                &id,
                "info",
                "Deployment cancelled",
            )
            .await;
            Json(CommandResponse {
                ok: true,
                message: "deployment cancelled".to_string(),
            })
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(CommandResponse {
                ok: false,
                message: e,
            }),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ProvisionRequest {
    client: String,
    version: String,
    cluster: String,
    #[serde(default)]
    identity_keypair_path: String,
    #[serde(default)]
    vote_account_keypair_path: String,
    #[serde(default)]
    ledger_path: String,
    #[serde(default)]
    snapshot_path: String,
    #[serde(default)]
    accounts_path: String,
    #[serde(default)]
    entrypoints: Vec<String>,
    #[serde(default)]
    known_validators: Vec<String>,
    #[serde(default)]
    download_url: String,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    jito_mev: bool,
    #[serde(default)]
    jito_block_engine_url: String,
    #[serde(default)]
    yellowstone_grpc: bool,
    #[serde(default)]
    rpc_port: u32,
    #[serde(default)]
    dynamic_port_range: String,
    #[serde(default)]
    node_type: String,
    #[serde(default)]
    gossip_port: u32,
    /// Client-specific CLI flags: "flag-name" -> "value" (empty for bare flags).
    #[serde(default)]
    validator_flags: std::collections::HashMap<String, String>,
    #[serde(default)]
    geyser_plugin_configs: Vec<String>,
    #[serde(default)]
    environment_vars: std::collections::HashMap<String, String>,
    #[serde(default)]
    extra_args: Vec<String>,
    #[serde(default)]
    restart_sec: u32,
    #[serde(default)]
    log_rate_limit_disable: bool,
    #[serde(default)]
    start_limit_disable: bool,
}

async fn provision_node(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<ProvisionRequest>,
) -> impl IntoResponse {
    // Reject if node is already provisioning or actively running a validator
    match db::get_lifecycle_state(&state.db, &id).await {
        Ok(Some(s)) if s == "provisioning" || s == "starting_up" => {
            return (
                StatusCode::CONFLICT,
                Json(CommandResponse {
                    ok: false,
                    message: format!("node is already in '{s}' state"),
                }),
            )
                .into_response();
        }
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(CommandResponse {
                    ok: false,
                    message: "node not found".to_string(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(CommandResponse {
                    ok: false,
                    message: format!("db error: {e}"),
                }),
            )
                .into_response();
        }
        _ => {} // registered, healthy, offline, etc. — allow provisioning
    }

    let log_msg = format!(
        "Provision command sent: {} {} ({})",
        req.client, req.version, req.cluster
    );

    // Save provision config JSON for the node record
    let provision_json = serde_json::to_string(&req).unwrap_or_default();

    let cmd = ControllerCommand {
        command: Some(controller_command::Command::Provision(ProvisionCommand {
            client: req.client,
            version: req.version,
            cluster: req.cluster,
            identity_keypair_path: req.identity_keypair_path,
            vote_account_keypair_path: req.vote_account_keypair_path,
            ledger_path: req.ledger_path,
            snapshot_path: req.snapshot_path,
            accounts_path: req.accounts_path,
            entrypoints: req.entrypoints,
            known_validators: req.known_validators,
            download_url: req.download_url,
            sha256: req.sha256,
            jito_mev: req.jito_mev,
            jito_block_engine_url: req.jito_block_engine_url,
            yellowstone_grpc: req.yellowstone_grpc,
            rpc_port: req.rpc_port,
            dynamic_port_range: req.dynamic_port_range,
            node_type: req.node_type,
            gossip_port: req.gossip_port,
            validator_flags: req.validator_flags,
            geyser_plugin_configs: req.geyser_plugin_configs,
            environment_vars: req.environment_vars,
            extra_args: req.extra_args,
            restart_sec: req.restart_sec,
            log_rate_limit_disable: req.log_rate_limit_disable,
            start_limit_disable: req.start_limit_disable,
        })),
    };

    match state.registry.send_command(&id, cmd).await {
        Ok(()) => {
            // Mark node as provisioning in the database
            if let Err(e) = db::set_lifecycle_state(&state.db, &id, "provisioning").await {
                tracing::warn!(error = %e, "failed to set lifecycle_state to provisioning");
            }
            // Store provision config
            if let Err(e) = db::set_provision_config(&state.db, &id, &provision_json).await {
                tracing::warn!(error = %e, "failed to store provision config");
            }
            emit_controller_log(&state.registry, &state.db, &id, "info", &log_msg).await;
            Json(CommandResponse {
                ok: true,
                message: "provision command sent".to_string(),
            })
            .into_response()
        }
        Err(e) => {
            emit_controller_log(
                &state.registry,
                &state.db,
                &id,
                "error",
                &format!("Provision failed: {e}"),
            )
            .await;
            (
                StatusCode::BAD_REQUEST,
                Json(CommandResponse {
                    ok: false,
                    message: e,
                }),
            )
                .into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
struct UpgradeRequest {
    binary_name: String,
    version: String,
    download_url: String,
    sha256: String,
    #[serde(default)]
    reason: String,
}

async fn upgrade_node(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpgradeRequest>,
) -> impl IntoResponse {
    let log_msg = format!(
        "Upgrade command sent: {} v{} ({})",
        req.binary_name, req.version, req.reason
    );

    let cmd = ControllerCommand {
        command: Some(controller_command::Command::Upgrade(UpgradeCommand {
            binary_name: req.binary_name,
            version: req.version,
            download_url: req.download_url,
            sha256: req.sha256,
            reason: req.reason,
        })),
    };

    match state.registry.send_command(&id, cmd).await {
        Ok(()) => {
            emit_controller_log(&state.registry, &state.db, &id, "info", &log_msg).await;
            Json(CommandResponse {
                ok: true,
                message: "upgrade command sent".to_string(),
            })
            .into_response()
        }
        Err(e) => {
            emit_controller_log(
                &state.registry,
                &state.db,
                &id,
                "error",
                &format!("Upgrade failed: {e}"),
            )
            .await;
            (
                StatusCode::BAD_REQUEST,
                Json(CommandResponse {
                    ok: false,
                    message: e,
                }),
            )
                .into_response()
        }
    }
}

#[derive(Serialize)]
struct ClusterDefaultsResponse {
    entrypoints: Vec<String>,
    known_validators: Vec<String>,
    reference_rpc: String,
    expected_genesis_hash: String,
}

async fn cluster_defaults(Path(cluster): Path<String>) -> impl IntoResponse {
    let (entrypoints, known_validators, reference_rpc, expected_genesis_hash) =
        match cluster.as_str() {
            "devnet" => (
                vec![
                    "entrypoint.devnet.solana.com:8001".to_string(),
                    "entrypoint2.devnet.solana.com:8001".to_string(),
                    "entrypoint3.devnet.solana.com:8001".to_string(),
                    "entrypoint4.devnet.solana.com:8001".to_string(),
                    "entrypoint5.devnet.solana.com:8001".to_string(),
                ],
                vec![],
                "https://api.devnet.solana.com".to_string(),
                "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG".to_string(),
            ),
            "testnet" => (
                vec![
                    "entrypoint.testnet.solana.com:8001".to_string(),
                    "entrypoint2.testnet.solana.com:8001".to_string(),
                    "entrypoint3.testnet.solana.com:8001".to_string(),
                    "entrypoint4.testnet.solana.com:8001".to_string(),
                    "entrypoint5.testnet.solana.com:8001".to_string(),
                ],
                vec![
                    "5D1fNXzvv5NjV1ysLjirC4WY92RNsVH18vjmcszZd8on".to_string(),
                    "dDzy5SR3AXdYWVqbDEkVFdvSPCtS9ihF5kJkHCtXoFs".to_string(),
                    "FS9MmFpFd1iMSSwzDYnqLPhWkoXKhJGBRCq1SFRsqFB".to_string(),
                    "eoKpUABi59aT4with2BRcnKHr6MAxfY53VNa1yoV3Cy".to_string(),
                ],
                "https://api.testnet.solana.com".to_string(),
                "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY".to_string(),
            ),
            _ => (
                vec![
                    "entrypoint.mainnet-beta.solana.com:8001".to_string(),
                    "entrypoint2.mainnet-beta.solana.com:8001".to_string(),
                    "entrypoint3.mainnet-beta.solana.com:8001".to_string(),
                    "entrypoint4.mainnet-beta.solana.com:8001".to_string(),
                    "entrypoint5.mainnet-beta.solana.com:8001".to_string(),
                ],
                vec![
                    "7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2".to_string(),
                    "GdnSyH3YtwcxFvQrVVJMm1JhTS4QVX7MFsX56uJLUfiZ".to_string(),
                    "DE1bawNcRJB9rVm3buyMVfr8mBEoyyu73NBovf2oXJsJ".to_string(),
                    "CakcnaRDHka2gXyfbEd2d3xsvkJkqsLw2akB3zsN1D2S".to_string(),
                ],
                "https://api.mainnet-beta.solana.com".to_string(),
                "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d".to_string(),
            ),
        };

    Json(ClusterDefaultsResponse {
        entrypoints,
        known_validators,
        reference_rpc,
        expected_genesis_hash,
    })
}

async fn onboard_command(State(state): State<ApiState>) -> impl IntoResponse {
    let endpoint = if state.config.external_url.is_empty() {
        state.config.grpc_listen.clone()
    } else {
        state.config.external_url.clone()
    };

    Json(OnboardCommandResponse {
        command: format!(
            "curl -sSL https://get.pillar.sh | bash -s -- --controller {endpoint}"
        ),
    })
}

// ---------------------------------------------------------------------------
// Grafana settings
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct GrafanaSettings {
    grafana_url: String,
}

async fn get_grafana_settings(State(state): State<ApiState>) -> impl IntoResponse {
    match db::get_setting(&state.db, "grafana_url").await {
        Ok(val) => Json(GrafanaSettings {
            grafana_url: val.unwrap_or_default(),
        })
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn set_grafana_settings(
    State(state): State<ApiState>,
    Json(body): Json<GrafanaSettings>,
) -> impl IntoResponse {
    let url = body.grafana_url.trim().to_string();
    if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "grafana_url must start with http:// or https://"})),
        )
            .into_response();
    }

    match db::set_setting(&state.db, "grafana_url", &url).await {
        Ok(()) => Json(GrafanaSettings { grafana_url: url }).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Dashboard JSON endpoints
// ---------------------------------------------------------------------------

async fn dashboard_fleet_overview() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        include_str!("../dashboards/grafana/fleet-overview.json"),
    )
}

async fn dashboard_node_detail() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        include_str!("../dashboards/grafana/node-detail.json"),
    )
}

// ---------------------------------------------------------------------------
// Grafana reverse proxy
// ---------------------------------------------------------------------------

pub fn grafana_router(state: ApiState) -> Router {
    Router::new()
        .route("/{*path}", any(grafana_proxy))
        .with_state(state)
}

async fn grafana_proxy(
    State(state): State<ApiState>,
    Path(path): Path<String>,
    req: axum::extract::Request,
) -> Response {
    let grafana_url = match db::get_setting(&state.db, "grafana_url").await {
        Ok(Some(url)) if !url.is_empty() => url,
        _ => {
            return (
                StatusCode::BAD_GATEWAY,
                "Grafana URL not configured",
            )
                .into_response();
        }
    };

    let upstream = format!(
        "{}/grafana/{}{}",
        grafana_url.trim_end_matches('/'),
        path,
        req.uri()
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default()
    );

    let client = reqwest::Client::new();
    let method = req.method().clone();
    let mut builder = client.request(method, &upstream);

    // Forward relevant headers
    for (name, value) in req.headers() {
        if name == axum::http::header::HOST {
            continue;
        }
        if let Ok(v) = value.to_str() {
            builder = builder.header(name.as_str(), v);
        }
    }

    // Forward body
    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::BAD_REQUEST, "body too large").into_response(),
    };
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes.to_vec());
    }

    let upstream_resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "grafana proxy error");
            return (StatusCode::BAD_GATEWAY, "Grafana unreachable").into_response();
        }
    };

    let status = StatusCode::from_u16(upstream_resp.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let mut resp_builder = Response::builder().status(status);

    for (name, value) in upstream_resp.headers() {
        resp_builder = resp_builder.header(name, value);
    }

    let resp_bytes = upstream_resp.bytes().await.unwrap_or_default();
    resp_builder
        .body(Body::from(resp_bytes))
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "proxy error").into_response())
}
