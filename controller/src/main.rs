mod api;
mod certs;
mod config;
mod db;
mod grpc_server;
mod metrics_endpoint;
mod node_registry;
mod templates;
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
    // Install the ring crypto provider for rustls (required when both ring and aws-lc features exist)
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

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

    // Optionally generate TLS certs (server-only TLS, no client certs)
    let tls_config = if !config.certs_dir.is_empty() {
        let cert_paths = certs::ensure_certs(&config.certs_dir, &config.external_url)?;
        let server_cert = std::fs::read_to_string(&cert_paths.server_cert)
            .context("reading server cert")?;
        let server_key = std::fs::read_to_string(&cert_paths.server_key)
            .context("reading server key")?;

        let identity = tonic::transport::Identity::from_pem(&server_cert, &server_key);
        let tls = tonic::transport::ServerTlsConfig::new().identity(identity);
        tracing::info!(certs_dir = %config.certs_dir, "TLS enabled on gRPC server");
        Some(tls)
    } else {
        tracing::info!("TLS disabled (certs_dir not set), gRPC server running plaintext");
        None
    };

    // Ensure auth token exists: use config value, else load from DB, else generate + persist.
    let auth_token = if !config.auth_token.is_empty() {
        config.auth_token.clone()
    } else if let Some(stored) = db::get_setting(&database, "auth_token").await? {
        tracing::info!("loaded auth_token from database");
        stored
    } else {
        let token = certs::generate_token();
        db::set_setting(&database, "auth_token", &token).await?;
        tracing::info!("generated new auth_token and persisted to database");
        token
    };

    // Spawn gRPC server
    let grpc_addr = config
        .grpc_listen
        .parse()
        .context("parsing grpc_listen address")?;
    let grpc = GrpcServer::new(
        database.clone(),
        registry.clone(),
        &config.external_url,
    );
    let grpc_cancel = cancel.clone();
    let grpc_token = auth_token.clone();
    tokio::spawn(async move {
        tracing::info!(addr = %grpc_addr, "gRPC server starting");
        let mut builder = tonic::transport::Server::builder();
        if let Some(tls) = tls_config {
            builder = builder.tls_config(tls).expect("invalid TLS config");
        }
        let svc = PillarControllerServer::with_interceptor(grpc, move |req: tonic::Request<()>| {
            grpc_server::check_auth_token(&grpc_token, req)
        });
        if let Err(e) = builder
            .add_service(svc)
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
        auth_token: auth_token.clone(),
    };

    let grafana_state = api_state.clone();
    let app = api::router(api_state)
        .nest("/grafana", api::grafana_router(grafana_state))
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
