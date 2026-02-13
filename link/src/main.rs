mod config;
mod grpc;
mod http;
mod link_health;
mod log_collector;
mod metrics;
mod provisioner;
mod state_reader;
mod system_info;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use config::LinkConfig;
use figment::providers::Format;
use figment::Figment;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

const SERVICE_NAME: &str = "link";
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

fn load_config() -> anyhow::Result<LinkConfig> {
    let config_path =
        std::env::var("PILLAR_LINK_CONFIG").unwrap_or_else(|_| "link-config.yaml".to_string());

    let config = Figment::new()
        .merge(figment::providers::Yaml::file(&config_path))
        .extract::<LinkConfig>()
        .context(format!("loading config from {config_path}"))?;

    Ok(config)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logger();
    tracing::info!("{SERVICE_NAME} v{VERSION} starting");

    let config = load_config()?;
    config.validate().map_err(|e| anyhow::anyhow!(e))?;
    tracing::info!(
        state_path = %config.state_path,
        http_listen = %config.http_listen,
        poll_interval_secs = config.poll_interval_secs,
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

    // Shared state between reader and HTTP handlers
    let shared_state: state_reader::SharedState = Arc::new(RwLock::new(None));
    let prom_metrics = Arc::new(metrics::Metrics::new());
    let poll_interval = Duration::from_secs(config.poll_interval_secs);
    let link_health = Arc::new(link_health::LinkHealth::new());

    // 1. Spawn state reader (reads state file, enriches with sysinfo, updates Prometheus)
    tokio::spawn(state_reader::run_state_reader(
        PathBuf::from(&config.state_path),
        shared_state.clone(),
        prom_metrics.clone(),
        link_health.clone(),
        poll_interval,
        cancel.clone(),
    ));

    // 2b. Spawn log collector (if enabled)
    if config.log_collector.enabled {
        let lc_config = config.log_collector.clone();
        let lc_controller = config.controller.clone();
        let lc_cancel = cancel.clone();
        let lc_health = link_health.clone();
        tokio::spawn(async move {
            log_collector::run(lc_config, lc_controller, lc_health, lc_cancel).await;
        });
    }

    // 3. Spawn controller link (always required)
    let link = grpc::ControllerLink::new(
        config.controller,
        shared_state.clone(),
        link_health.clone(),
    );
    let grpc_cancel = cancel.clone();
    tokio::spawn(async move { link.run(grpc_cancel).await });

    // 4. Start HTTP server
    let app_state = http::AppState {
        shared_state,
        metrics: prom_metrics,
    };
    let app = http::router(app_state);
    let listener = tokio::net::TcpListener::bind(&config.http_listen)
        .await
        .context(format!("binding to {}", config.http_listen))?;
    tracing::info!(listen = %config.http_listen, "HTTP server starting");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { cancel.cancelled().await })
        .await
        .context("HTTP server error")?;

    tracing::info!("{SERVICE_NAME} stopped");
    Ok(())
}
