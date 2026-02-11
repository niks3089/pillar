mod client;
mod config;
mod error;
mod event;
mod health;
mod lifecycle;
mod operator;
mod provisioner;
mod role;
mod snapshot;
mod state;

use std::path::PathBuf;

use anyhow::Context;
use config::OperatorConfig;
use figment::{providers::Format, Figment};
use tokio_util::sync::CancellationToken;

const SERVICE_NAME: &str = "operator";
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

fn load_config() -> anyhow::Result<OperatorConfig> {
    let config_path = std::env::var("PILLAR_CONFIG").unwrap_or_else(|_| "config.yaml".to_string());

    let config = Figment::new()
        .merge(figment::providers::Yaml::file(&config_path))
        .extract::<OperatorConfig>()
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
        role = %config.role,
        client = %config.client,
        cluster = %config.network.cluster,
        state_path = %config.state_path,
        "config loaded"
    );

    // Ensure state directory exists
    if let Some(parent) = PathBuf::from(&config.state_path).parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context(format!("creating state dir {}", parent.display()))?;
    }

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

    let cancel = CancellationToken::new();

    // Handle SIGINT/SIGTERM
    let cancel_signal = cancel.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("received shutdown signal");
        cancel_signal.cancel();
    });

    let validator_process = validator_client
        .binary_path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    // Run operator loop
    let mut op = operator::Operator::new(
        config.clone(),
        health_checker,
        service_manager,
        snapshot_manager,
        PathBuf::from(&config.paths.ledger_path),
        PathBuf::from(&config.state_path),
        validator_process,
        validator_client.binary_path().clone(),
    );

    op.run(cancel).await;

    tracing::info!("{SERVICE_NAME} stopped");
    Ok(())
}
