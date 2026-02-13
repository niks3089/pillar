mod api;
mod config;
mod db;
mod grpc_server;
mod metrics_endpoint;
mod node_registry;
mod web;

use std::time::Duration;

use anyhow::Context;
use figment::providers::Format;
use figment::Figment;
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;

use config::ControllerConfig;
use grpc_server::GrpcServer;
use grpc_server::PillarControllerServer;
use node_registry::NodeRegistry;

const SERVICE_NAME: &str = "controller";
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

fn load_config() -> anyhow::Result<ControllerConfig> {
    let config_path = std::env::var("PILLAR_CONTROLLER_CONFIG")
        .unwrap_or_else(|_| "controller-config.yaml".to_string());

    let config = Figment::new()
        .merge(figment::providers::Yaml::file(&config_path))
        .extract::<ControllerConfig>()
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
        grpc_listen = %config.grpc_listen,
        http_listen = %config.http_listen,
        db_path = %config.db_path,
        retention_days = config.retention_days,
        "config loaded"
    );

    // Ensure the DB directory exists.
    if let Some(parent) = std::path::Path::new(&config.db_path).parent() {
        std::fs::create_dir_all(parent)
            .context(format!("creating db directory {}", parent.display()))?;
    }

    let database = db::open_db(&config.db_path)?;

    // Seed grafana_url from config into DB if not already set
    if !config.grafana_url.is_empty()
        && db::get_setting(&database, "grafana_url").await?.is_none()
    {
        db::set_setting(&database, "grafana_url", &config.grafana_url).await?;
        tracing::info!("seeded grafana_url from config");
    }

    let registry = NodeRegistry::new();
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

    // Spawn gRPC server
    let grpc_addr = config
        .grpc_listen
        .parse()
        .context("parsing grpc_listen address")?;
    let grpc = GrpcServer::new(database.clone(), registry.clone(), &config.external_url);
    let grpc_cancel = cancel.clone();
    tokio::spawn(async move {
        tracing::info!(addr = %grpc_addr, "gRPC server starting");
        if let Err(e) = tonic::transport::Server::builder()
            .add_service(PillarControllerServer::new(grpc))
            .serve_with_shutdown(grpc_addr, async move { grpc_cancel.cancelled().await })
            .await
        {
            tracing::error!(error = %e, "gRPC server error");
        }
    });

    // Spawn retention pruner (runs every hour)
    let prune_db = database.clone();
    let retention_days = config.retention_days;
    let prune_cancel = cancel.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match db::prune_old_data(&prune_db, retention_days).await {
                        Ok((status_deleted, logs_deleted)) => {
                            if status_deleted > 0 || logs_deleted > 0 {
                                tracing::info!(
                                    status_deleted,
                                    logs_deleted,
                                    "pruned old data"
                                );
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "prune error"),
                    }
                }
                _ = prune_cancel.cancelled() => break,
            }
        }
    });

    // Build HTTP router
    let api_state = api::ApiState {
        db: database.clone(),
        registry: registry.clone(),
        config: config.clone(),
    };

    let app = api::router(api_state)
        .merge(web::router())
        .layer(CorsLayer::permissive());

    // Start HTTP server
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
