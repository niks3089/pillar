use std::collections::HashMap;

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::{any, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicI64, Ordering};
use tokio_stream::StreamExt;

use pillar_shared::proto::{
    controller_command, ControllerCommand, ExecuteScript, LogEntry, NodeStatus,
};

use crate::auth::SessionStore;
use crate::config::ControllerConfig;
use crate::db::{self, Db, NodeRow};
use crate::node_registry::NodeRegistry;
use crate::templates;
use crate::update_checker::SharedUpdateInfo;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct ApiState {
    pub db: Db,
    pub registry: NodeRegistry,
    pub config: ControllerConfig,
    pub auth_token: String,
    pub update_info: SharedUpdateInfo,
    pub sessions: SessionStore,
}

pub fn router(state: ApiState) -> Router {
    // Public routes (no auth required)
    let public = Router::new()
        .route("/api/login", post(crate::auth::login))
        .route("/api/logout", post(crate::auth::logout))
        .route("/api/auth/check", get(crate::auth::auth_check))
        .route("/api/certs/client-bundle", get(client_cert_bundle))
        .route("/metrics", get(crate::metrics_endpoint::metrics_handler))
        .with_state(state.clone());

    // Protected routes (auth required)
    let protected = Router::new()
        .route("/api/overview", get(overview))
        .route("/api/nodes", get(list_nodes))
        .route("/api/nodes/{id}", get(get_node).delete(delete_node))
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
        .route("/api/version", get(version_info))
        .route("/api/upgrade-controller", post(upgrade_controller))
        .route("/api/nodes/{id}/upgrade-agent", post(upgrade_agent))
        .route(
            "/api/settings/grafana",
            get(get_grafana_settings).put(set_grafana_settings),
        )
        .route(
            "/api/auth/credentials",
            put(crate::auth::change_credentials),
        )
        .route(
            "/api/dashboards/fleet-overview",
            get(dashboard_fleet_overview),
        )
        .route("/api/dashboards/node-detail", get(dashboard_node_detail))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_auth,
        ))
        .with_state(state);

    public.merge(protected)
}

// ---------------------------------------------------------------------------
// Query parameter structs
// ---------------------------------------------------------------------------

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
// SSE log entry wrapper (maps proto LogEntry → frontend-expected JSON shape)
// ---------------------------------------------------------------------------

static SSE_ID_COUNTER: AtomicI64 = AtomicI64::new(0);

#[derive(Serialize)]
struct SseLogEntry {
    id: i64,
    service: String,
    level: String,
    message: String,
    unit: Option<String>,
    timestamp_ms: i64,
}

impl From<LogEntry> for SseLogEntry {
    fn from(entry: LogEntry) -> Self {
        SseLogEntry {
            id: SSE_ID_COUNTER.fetch_add(1, Ordering::Relaxed),
            service: entry.service,
            level: entry.level,
            message: entry.message,
            unit: if entry.unit.is_empty() {
                None
            } else {
                Some(entry.unit)
            },
            timestamp_ms: entry.timestamp_unix_ms,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generate a unique script ID.
fn generate_script_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("script-{ts}-{:04x}", rand_u16())
}

fn rand_u16() -> u16 {
    let mut buf = [0u8; 2];
    getrandom::getrandom(&mut buf).unwrap_or_default();
    u16::from_le_bytes(buf)
}

/// Wrap an ExecuteScript in a ControllerCommand.
fn wrap_script(script: ExecuteScript) -> ControllerCommand {
    ControllerCommand {
        command: Some(controller_command::Command::Execute(script)),
    }
}

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
    registry
        .publish_logs(node_id, std::slice::from_ref(&entry))
        .await;
    if let Err(e) = db::insert_logs(db, node_id, std::slice::from_ref(&entry)).await {
        tracing::warn!(error = %e, "failed to persist controller log");
    }
}

/// Look up the service name for a node from its provision_config_json in DB.
async fn get_service_name_for_node(db: &Db, node_id: &str) -> Option<String> {
    let config_json = db::get_provision_config(db, node_id).await.ok()??;
    let parsed: serde_json::Value = serde_json::from_str(&config_json).ok()?;
    let client = parsed.get("client")?.as_str()?;
    Some(templates::service_name_for_client(client).to_string())
}

/// Look up the ledger path for a node from its provision_config_json in DB.
async fn get_ledger_path_for_node(db: &Db, node_id: &str) -> Option<String> {
    let config_json = db::get_provision_config(db, node_id).await.ok()??;
    let parsed: serde_json::Value = serde_json::from_str(&config_json).ok()?;
    parsed
        .get("ledger_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
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
                        let sse_entry = SseLogEntry::from(entry);
                        let json = serde_json::to_string(&sse_entry).unwrap_or_default();
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
    let service_name = get_service_name_for_node(&state.db, &id)
        .await
        .unwrap_or_else(|| "solana-validator".to_string());

    let mut vars = HashMap::new();
    vars.insert("service_name".to_string(), service_name);

    let script = templates::render(templates::scripts::RESTART, &vars);
    let cmd = wrap_script(ExecuteScript {
        script_id: generate_script_id(),
        script,
        description: "Restart validator".to_string(),
        timeout_secs: 60,
    });

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
    let service_name = get_service_name_for_node(&state.db, &id)
        .await
        .unwrap_or_else(|| "solana-validator".to_string());
    let ledger_path = get_ledger_path_for_node(&state.db, &id)
        .await
        .unwrap_or_else(|| "/mnt/ledger".to_string());

    let mut vars = HashMap::new();
    vars.insert("service_name".to_string(), service_name);
    vars.insert("ledger_path".to_string(), ledger_path);

    let script = templates::render(templates::scripts::RECOVER, &vars);
    let cmd = wrap_script(ExecuteScript {
        script_id: generate_script_id(),
        script,
        description: "Recovery: stop, wipe ledger, restart".to_string(),
        timeout_secs: 300,
    });

    match state.registry.send_command(&id, cmd).await {
        Ok(()) => {
            emit_controller_log(
                &state.registry,
                &state.db,
                &id,
                "info",
                "Recovery command sent",
            )
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
    let service_name = get_service_name_for_node(&state.db, &id)
        .await
        .unwrap_or_else(|| "solana-validator".to_string());

    let mut vars = HashMap::new();
    vars.insert("service_name".to_string(), service_name);

    let script = templates::render(templates::scripts::STOP, &vars);
    let cmd = wrap_script(ExecuteScript {
        script_id: generate_script_id(),
        script,
        description: "Stop validator".to_string(),
        timeout_secs: 60,
    });

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
                    message: format!(
                        "cannot cancel: node is in '{s}' state, not provisioning/starting_up"
                    ),
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

    let service_name = get_service_name_for_node(&state.db, &id)
        .await
        .unwrap_or_else(|| "solana-validator".to_string());

    let mut vars = HashMap::new();
    vars.insert("service_name".to_string(), service_name);

    let script = templates::render(templates::scripts::STOP, &vars);
    let cmd = wrap_script(ExecuteScript {
        script_id: generate_script_id(),
        script,
        description: "Cancel deployment: stop validator".to_string(),
        timeout_secs: 60,
    });

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
    jito_relayer_url: String,
    #[serde(default)]
    jito_shred_receiver_addr: String,
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
    validator_flags: HashMap<String, String>,
    #[serde(default)]
    geyser_plugin_configs: Vec<String>,
    #[serde(default)]
    environment_vars: HashMap<String, String>,
    #[serde(default)]
    extra_args: Vec<String>,
    #[serde(default)]
    restart_sec: u32,
    #[serde(default)]
    log_rate_limit_disable: bool,
    #[serde(default)]
    start_limit_disable: bool,
    /// Skip the validator's inbound UDP port reachability check. Needed on hosts behind
    /// NAT or an upstream firewall that blocks inbound UDP (the validator otherwise hangs
    /// retrying ip_echo before it will bootstrap).
    #[serde(default)]
    no_port_check: bool,
}

/// Build template variables from a ProvisionRequest.
fn build_provision_vars(req: &ProvisionRequest) -> HashMap<String, String> {
    let service_name = templates::service_name_for_client(&req.client).to_string();
    let binary_path = templates::binary_path_for_client(&req.client).to_string();
    let rpc_port = if req.rpc_port == 0 { 8899 } else { req.rpc_port };
    let gossip_port = if req.gossip_port == 0 {
        8001
    } else {
        req.gossip_port
    };
    let dynamic_port_range = if req.dynamic_port_range.is_empty() {
        "8000-8030".to_string()
    } else {
        req.dynamic_port_range.clone()
    };
    let restart_sec = if req.restart_sec == 0 {
        1
    } else {
        req.restart_sec
    };

    // Build ExecStart for Agave/Jito or fdctl command for Firedancer
    let exec_start = if req.client == "firedancer" || req.client == "frankendancer" {
        format!("{binary_path} run --config /etc/pillar/validator.toml")
    } else {
        templates::build_exec_start(
            &binary_path,
            &req.identity_keypair_path,
            &req.vote_account_keypair_path,
            &req.ledger_path,
            &req.snapshot_path,
            &req.accounts_path,
            rpc_port,
            gossip_port,
            &dynamic_port_range,
            &req.entrypoints,
            &req.known_validators,
            &req.cluster,
            &templates::JitoConfig {
                enabled: req.jito_mev,
                block_engine_url: req.jito_block_engine_url.clone(),
                relayer_url: req.jito_relayer_url.clone(),
                shred_receiver_addr: req.jito_shred_receiver_addr.clone(),
            },
            req.yellowstone_grpc,
            &req.geyser_plugin_configs,
            &req.validator_flags,
            &req.extra_args,
            req.no_port_check,
        )
    };

    // Build Yellowstone section
    let yellowstone_section = if req.yellowstone_grpc {
        r#"# Write Yellowstone gRPC config
sudo mkdir -p /etc/pillar
sudo tee /etc/pillar/yellowstone-grpc.json > /dev/null <<'YSJSON'
{"libpath":"/usr/local/lib/libyellowstone_grpc_geyser.so","log":{"level":"info"},"grpc":{"address":"0.0.0.0:10000","max_decoding_message_size":"4_194_304"}}
YSJSON
echo "Wrote /etc/pillar/yellowstone-grpc.json""#
            .to_string()
    } else {
        String::new()
    };

    // Build Firedancer TOML for firedancer/frankendancer
    let firedancer_toml = if req.client == "firedancer" || req.client == "frankendancer" {
        let entrypoints_toml = req
            .entrypoints
            .iter()
            .map(|e| format!("\"{e}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "[layout]\naffinity = \"auto\"\n\n\
             [consensus]\nidentity_path = \"{identity}\"\n\
             vote_account_path = \"{vote}\"\n\
             expected_genesis_hash = \"auto\"\n\n\
             [ledger]\npath = \"{ledger}\"\n\
             accounts_path = \"{accounts}\"\n\
             limit_size = true\n\n\
             [gossip]\nentrypoints = [{ep}]\n",
            identity = req.identity_keypair_path,
            vote = req.vote_account_keypair_path,
            ledger = req.ledger_path,
            accounts = req.accounts_path,
            ep = entrypoints_toml,
        )
    } else {
        String::new()
    };

    // Build start_limit and log_rate_limit lines
    let start_limit_line = if req.start_limit_disable {
        "StartLimitIntervalSec=0".to_string()
    } else {
        String::new()
    };

    let log_rate_limit_line = if req.log_rate_limit_disable {
        "LogRateLimitIntervalSec=0".to_string()
    } else {
        String::new()
    };

    // Build Environment= lines
    let mut env_keys: Vec<&String> = req.environment_vars.keys().collect();
    env_keys.sort();
    let environment_lines = env_keys
        .iter()
        .map(|k| format!("Environment={}={}", k, req.environment_vars[*k]))
        .collect::<Vec<_>>()
        .join("\n");

    // Build sed commands for agent config update
    let reference_rpc = templates::reference_rpc_for_cluster(&req.cluster);
    let agent_config_sed_commands = format!(
        r#"if [ -f "$CONFIG" ]; then
  sudo sed -i 's/^client:.*/client: {client}/' "$CONFIG"
  sudo sed -i 's/^  cluster:.*/  cluster: {cluster}/' "$CONFIG"
  sudo sed -i 's|^  service_name:.*|  service_name: {service_name}|' "$CONFIG"
  sudo sed -i '/reference_rpc_urls:/,/^[^ ]/ {{ /- http/d }}' "$CONFIG"
  sudo sed -i '/reference_rpc_urls:/a\\    - {reference_rpc}' "$CONFIG"
  echo "Updated agent config: client={client}, cluster={cluster}, service={service_name}"
fi"#,
        client = req.client,
        cluster = req.cluster,
        service_name = service_name,
        reference_rpc = reference_rpc,
    );

    // Look up the old service name (might differ if switching clients).
    // We use the new service name as fallback since we don't have DB access here.
    let old_service_name = service_name.clone();

    let mut vars = HashMap::new();
    vars.insert("version".to_string(), req.version.clone());
    vars.insert("cluster".to_string(), req.cluster.clone());
    vars.insert("download_url".to_string(), req.download_url.clone());
    vars.insert("sha256".to_string(), req.sha256.clone());
    vars.insert("binary_path".to_string(), binary_path);
    vars.insert("service_name".to_string(), service_name);
    vars.insert("old_service_name".to_string(), old_service_name);
    vars.insert("exec_start".to_string(), exec_start);
    vars.insert("restart_sec".to_string(), restart_sec.to_string());
    vars.insert("yellowstone_section".to_string(), yellowstone_section);
    vars.insert("firedancer_toml".to_string(), firedancer_toml);
    vars.insert("start_limit_line".to_string(), start_limit_line);
    vars.insert("log_rate_limit_line".to_string(), log_rate_limit_line);
    vars.insert("environment_lines".to_string(), environment_lines);
    vars.insert(
        "agent_config_sed_commands".to_string(),
        agent_config_sed_commands,
    );
    vars
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

    // Select template by client
    let template = match templates::provision_template(&req.client) {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(CommandResponse {
                    ok: false,
                    message: e,
                }),
            )
                .into_response();
        }
    };

    let log_msg = format!(
        "Provision command sent: {} {} ({})",
        req.client, req.version, req.cluster
    );
    let description = format!("Provision {} v{} on {}", req.client, req.version, req.cluster);

    // Save provision config JSON for the node record
    let provision_json = serde_json::to_string(&req).unwrap_or_default();

    // Build template variables and render script
    let vars = build_provision_vars(&req);
    let script = templates::render(template, &vars);
    let script_id = generate_script_id();

    let cmd = wrap_script(ExecuteScript {
        script_id: script_id.clone(),
        script,
        description,
        timeout_secs: 3600,
    });

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
            // Record script execution
            if let Err(e) =
                db::insert_script_execution(&state.db, &script_id, &id, "provision").await
            {
                tracing::warn!(error = %e, "failed to record script execution");
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

    let service_name = templates::service_name_for_binary(&req.binary_name).to_string();
    let binary_dest = format!("/usr/local/bin/{}", req.binary_name);

    let mut vars = HashMap::new();
    vars.insert("binary_name".to_string(), req.binary_name.clone());
    vars.insert("version".to_string(), req.version.clone());
    vars.insert("download_url".to_string(), req.download_url);
    vars.insert("sha256".to_string(), req.sha256);
    vars.insert("service_name".to_string(), service_name);
    vars.insert("binary_dest".to_string(), binary_dest);

    let script = templates::render(templates::scripts::UPGRADE_VALIDATOR, &vars);
    let description = format!("Upgrade {} to v{}", req.binary_name, req.version);

    let cmd = wrap_script(ExecuteScript {
        script_id: generate_script_id(),
        script,
        description,
        timeout_secs: 3600,
    });

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
    /// Jito MEV defaults for this cluster (block engine URL + tip programs), so the UI
    /// can pre-fill the Jito fields with cluster-correct values.
    jito_block_engine_url: String,
    jito_tip_payment_program: String,
    jito_tip_distribution_program: String,
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

    let jito = templates::jito_defaults_for_cluster(&cluster);

    Json(ClusterDefaultsResponse {
        entrypoints,
        known_validators,
        reference_rpc,
        expected_genesis_hash,
        jito_block_engine_url: jito.block_engine_url.to_string(),
        jito_tip_payment_program: jito.tip_payment_program.to_string(),
        jito_tip_distribution_program: jito.tip_distribution_program.to_string(),
    })
}

async fn onboard_command(State(state): State<ApiState>) -> impl IntoResponse {
    let endpoint = if state.config.external_url.is_empty() {
        state.config.grpc_listen.clone()
    } else {
        state.config.external_url.clone()
    };

    let mut cmd = format!(
        "curl -sSL https://janus-meter.s3.eu-north-1.amazonaws.com/pillar/latest/install-node.sh | sudo bash -s -- --controller {endpoint}"
    );

    if !state.auth_token.is_empty() {
        cmd.push_str(&format!(" \\\n  --token {}", state.auth_token));
    }

    // When TLS is enabled, include the HTTP base URL so install script can fetch ca.pem
    if !state.config.certs_dir.is_empty() {
        let http_base = if state.config.external_url.is_empty() {
            format!(
                "http://localhost:{}",
                state
                    .config
                    .http_listen
                    .rsplit_once(':')
                    .map(|(_, p)| p)
                    .unwrap_or("8080")
            )
        } else {
            let host = state
                .config
                .external_url
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .rsplit_once(':')
                .map(|(h, _)| h)
                .unwrap_or(&state.config.external_url);
            let http_port = state
                .config
                .http_listen
                .rsplit_once(':')
                .map(|(_, p)| p)
                .unwrap_or("8080");
            format!("http://{host}:{http_port}")
        };
        cmd.push_str(&format!(" \\\n  --http-url {http_base}"));
    }

    Json(OnboardCommandResponse { command: cmd })
}

// ---------------------------------------------------------------------------
// Version / upgrade endpoints
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct VersionInfoResponse {
    current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    controller_update: Option<crate::update_checker::AvailableUpdate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_update: Option<crate::update_checker::AvailableUpdate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checked_at: Option<i64>,
}

async fn version_info(State(state): State<ApiState>) -> impl IntoResponse {
    let info =
        crate::update_checker::get_or_refresh(VERSION, &state.update_info).await;
    Json(VersionInfoResponse {
        current_version: VERSION.to_string(),
        controller_update: info.controller_update,
        agent_update: info.agent_update,
        checked_at: info.checked_at,
    })
}

async fn upgrade_controller(State(state): State<ApiState>) -> impl IntoResponse {
    let info = state.update_info.read().await;
    let update = match &info.controller_update {
        Some(u) => u.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(CommandResponse {
                    ok: false,
                    message: "no controller update available".to_string(),
                }),
            )
                .into_response();
        }
    };
    drop(info);

    let download_url = update.download_url.clone();
    let sha256 = update.sha256.clone();
    let version = update.version.clone();

    tracing::info!(version = %version, "controller self-upgrade initiated");

    // Spawn background task — sleep briefly so the HTTP response flushes,
    // then download, verify, install, and restart.
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let script = format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
STAGING="/tmp/pillar-controller-upgrade"
rm -rf "$STAGING" && mkdir -p "$STAGING"
curl -sSL -o "$STAGING/binary" "{download_url}"
echo "{sha256}  $STAGING/binary" | sha256sum -c
sudo install -m 755 "$STAGING/binary" /usr/local/bin/controller
rm -rf "$STAGING"
sudo systemctl restart pillar-controller
"#,
            download_url = download_url,
            sha256 = sha256,
        );

        let output = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(&script)
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => {
                tracing::info!("controller upgrade script completed (process will restart)");
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                tracing::error!(stderr = %stderr, "controller upgrade script failed");
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to run controller upgrade script");
            }
        }
    });

    Json(CommandResponse {
        ok: true,
        message: format!("controller upgrade to v{version} initiated, restarting shortly"),
    })
    .into_response()
}

async fn upgrade_agent(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let info = state.update_info.read().await;
    let update = match &info.agent_update {
        Some(u) => u.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(CommandResponse {
                    ok: false,
                    message: "no agent update available".to_string(),
                }),
            )
                .into_response();
        }
    };
    drop(info);

    let mut vars = HashMap::new();
    vars.insert("version".to_string(), update.version.clone());
    vars.insert("download_url".to_string(), update.download_url.clone());
    vars.insert("sha256".to_string(), update.sha256.clone());

    let script = templates::render(templates::scripts::UPGRADE_AGENT, &vars);
    let description = format!("Upgrade agent to v{}", update.version);

    let cmd = wrap_script(ExecuteScript {
        script_id: generate_script_id(),
        script,
        description: description.clone(),
        timeout_secs: 300,
    });

    match state.registry.send_command(&id, cmd).await {
        Ok(()) => {
            emit_controller_log(
                &state.registry,
                &state.db,
                &id,
                "info",
                &format!("Agent upgrade to v{} initiated", update.version),
            )
            .await;
            Json(CommandResponse {
                ok: true,
                message: format!("agent upgrade to v{} command sent", update.version),
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

#[derive(Serialize)]
struct CertBundleResponse {
    ca_cert: String,
}

async fn client_cert_bundle(State(state): State<ApiState>) -> impl IntoResponse {
    if state.config.certs_dir.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "TLS not enabled (certs_dir not configured)"})),
        )
            .into_response();
    }

    let ca_path = std::path::Path::new(&state.config.certs_dir).join("ca.pem");
    match std::fs::read_to_string(&ca_path) {
        Ok(ca) => Json(CertBundleResponse { ca_cert: ca }).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "failed to read ca.pem"})),
        )
            .into_response(),
    }
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
            return (StatusCode::BAD_GATEWAY, "Grafana URL not configured").into_response();
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
        // Let axum set these based on the actual body we send
        if name == "transfer-encoding" || name == "content-length" {
            continue;
        }
        resp_builder = resp_builder.header(name, value);
    }

    let resp_bytes = upstream_resp.bytes().await.unwrap_or_default();
    resp_builder
        .body(Body::from(resp_bytes))
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "proxy error").into_response())
}
