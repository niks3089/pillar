mod agent_health;
mod client;
mod command;
mod config;
mod error;
mod event;
mod grpc;
mod health;
mod http;
mod lifecycle;
mod log_collector;
mod metrics;
mod metrics_updater;
mod reconcile;
mod role;
mod rpc_client;
mod script_executor;
mod snapshot;
mod system_info;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use config::AgentConfig;
use figment::{providers::Format, Figment};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

const SERVICE_NAME: &str = "agent";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn init_logger() {
    let env_filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_thread_ids(true)
        .compact()
        .try_init()
        .ok();
}

fn load_config() -> anyhow::Result<AgentConfig> {
    let config_path =
        std::env::var("PILLAR_AGENT_CONFIG").unwrap_or_else(|_| "agent.yaml".to_string());

    let config = Figment::new()
        .merge(figment::providers::Yaml::file(&config_path))
        .extract::<AgentConfig>()
        .context(format!("loading config from {config_path}"))?;

    Ok(config)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    init_logger();
    tracing::info!("{SERVICE_NAME} v{VERSION} starting");

    let mut config = load_config()?;
    config.validate().map_err(|e| anyhow::anyhow!(e))?;

    // Default node_id to system hostname if not explicitly set
    if config.controller.node_id.is_empty() {
        config.controller.node_id = sysinfo::System::host_name()
            .unwrap_or_else(|| "unknown".to_string());
    }
    tracing::info!(
        role = %config.role,
        client = %config.client,
        cluster = %config.network.cluster,
        http_listen = %config.http_listen,
        controller_endpoint = %config.controller.endpoint,
        controller_node_id = %config.controller.node_id,
        "config loaded"
    );

    let cancel = CancellationToken::new();

    // Handle SIGINT/SIGTERM
    tokio::spawn({
        let cancel = cancel.clone();
        async move {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("received shutdown signal");
            cancel.cancel();
        }
    });

    // Shared state between reconciler, metrics updater, gRPC, and HTTP
    let shared_status: metrics_updater::SharedStatus = Arc::new(RwLock::new(None));

    // Command channel: gRPC → reconciler
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(32);

    // Script result channel: reconciler → gRPC
    let (result_tx, result_rx) = tokio::sync::mpsc::channel(32);

    let prom_metrics = Arc::new(metrics::Metrics::new());
    let agent_health = Arc::new(agent_health::AgentHealth::new());

    // Build services
    let health_checker =
        health::create_health_checker(config.role, &config.health, &config.network);

    let validator_client = client::ValidatorClient::from_kind(config.client);

    let service_manager = lifecycle::SystemdManager::new(
        validator_client.service_name().to_string(),
    );

    let snapshot_manager = snapshot::TcpSnapshotManager::new(
        config.snapshot.server_hostname.clone(),
        PathBuf::from(&config.paths.snapshot_path),
    );

    let validator_process = validator_client
        .binary_path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    // 1. Spawn reconcile loop
    let reconcile_config = config.clone();
    let ledger_dir = PathBuf::from(&config.paths.ledger_path);
    let accounts_dir = PathBuf::from(&config.paths.accounts_path);
    let snapshot_dir = PathBuf::from(&config.paths.snapshot_path);
    let reconcile_shared = shared_status.clone();
    let reconcile_cancel = cancel.clone();
    let reconcile_handle = tokio::spawn(async move {
        let mut reconciler = reconcile::Reconciler::new(
            reconcile_config,
            health_checker,
            service_manager,
            snapshot_manager,
            ledger_dir,
            accounts_dir,
            snapshot_dir,
            validator_process,
            reconcile_shared,
            cmd_rx,
            result_tx,
        );
        reconciler.run(reconcile_cancel).await;
    });

    // 2. Spawn metrics updater
    let sysinfo_interval = Duration::from_secs(config.sysinfo_refresh_interval_secs);
    tokio::spawn(metrics_updater::run(
        shared_status.clone(),
        prom_metrics.clone(),
        agent_health.clone(),
        sysinfo_interval,
        cancel.clone(),
    ));

    // 3. Spawn log collector (if enabled)
    if config.log_collector.enabled {
        let lc_config = config.log_collector.clone();
        let lc_controller = config.controller.clone();
        let lc_health = agent_health.clone();
        let lc_shared = shared_status.clone();
        let lc_cancel = cancel.clone();
        tokio::spawn(async move {
            log_collector::run(lc_config, lc_controller, lc_health, lc_shared, lc_cancel).await;
        });
    }

    // 4. Spawn controller link (gRPC)
    let link = grpc::ControllerLink::new(
        config.controller.clone(),
        shared_status.clone(),
        agent_health.clone(),
        cmd_tx,
        result_rx,
    );
    let grpc_cancel = cancel.clone();
    tokio::spawn(async move { link.run(grpc_cancel).await });

    // 5. Start HTTP server
    let app_state = http::AppState {
        shared_status,
        metrics: prom_metrics,
    };
    let app = http::router(app_state);
    let listener = tokio::net::TcpListener::bind(&config.http_listen)
        .await
        .context(format!("binding to {}", config.http_listen))?;
    tracing::info!(listen = %config.http_listen, "HTTP server starting");

    let http_cancel = cancel.clone();
    let http = axum::serve(listener, app)
        .with_graceful_shutdown(async move { http_cancel.cancelled().await });

    // If the reconciler dies while we're not shutting down, exit instead of leaving the
    // validator unmanaged behind a still-healthy /health endpoint.
    tokio::select! {
        res = http => res.context("HTTP server error")?,
        res = reconcile_handle => {
            if !cancel.is_cancelled() {
                cancel.cancel();
                anyhow::bail!("reconciler task exited unexpectedly: {res:?}");
            }
        }
    }

    tracing::info!("{SERVICE_NAME} stopped");
    Ok(())
}
