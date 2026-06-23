mod api;
mod auth;
mod certs;
mod config;
mod db;
mod grpc_server;
mod metrics_endpoint;
mod node_registry;
mod templates;
mod update_checker;
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
        let mut tls = tonic::transport::ServerTlsConfig::new().identity(identity);
        if config.require_client_certs {
            let ca = std::fs::read_to_string(&cert_paths.ca_cert).context("reading CA cert")?;
            tls = tls.client_ca_root(tonic::transport::Certificate::from_pem(ca));
            tracing::info!("mTLS enabled: client certificates required");
        }
        tracing::info!(certs_dir = %config.certs_dir, "TLS enabled on gRPC server");
        Some(tls)
    } else {
        tracing::info!("TLS disabled (certs_dir not set), gRPC server running plaintext");
        None
    };

    // Seed admin credentials if not present. We generate a random password instead of a
    // guessable default (`admin/admin`) and print it ONCE; the operator changes it via the UI.
    if db::get_setting(&database, "admin_username").await?.is_none() {
        db::set_setting(&database, "admin_username", "admin").await?;
        let password = certs::generate_token();
        let hash = auth::hash_password(&password)
            .map_err(|e| anyhow::anyhow!("failed to hash admin password: {e}"))?;
        db::set_setting(&database, "admin_password_hash", &hash).await?;
        tracing::warn!(
            username = "admin",
            password = %password,
            "generated initial admin credentials — log in and change the password; shown only once"
        );
    }

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

    // Admin API token — a SEPARATE secret from the agent enrollment token above.
    // Used as `Authorization: Bearer <token>` for programmatic API access.
    let api_token = if let Some(stored) = db::get_setting(&database, "api_token").await? {
        stored
    } else {
        let token = certs::generate_token();
        db::set_setting(&database, "api_token", &token).await?;
        tracing::warn!(
            api_token = %token,
            "generated admin API token — use as 'Authorization: Bearer <token>'; shown only once"
        );
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
        config.require_client_certs,
    );
    let grpc_cancel = cancel.clone();
    let grpc_token = auth_token.clone();
    let grpc_handle = tokio::spawn(async move {
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
    let prune_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match db::prune_old_data(&prune_db, retention_days).await {
                        Ok(logs_deleted) => {
                            if logs_deleted > 0 {
                                tracing::info!(
                                    logs_deleted,
                                    "pruned old logs"
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

    // Check for updates on startup (lazy-refresh: re-checks when stale via /api/version)
    let update_info: update_checker::SharedUpdateInfo = Default::default();
    update_checker::spawn_initial_check(VERSION.to_string(), update_info.clone());

    // Build HTTP router
    let sessions = auth::SessionStore::new();
    let api_state = api::ApiState {
        db: database.clone(),
        registry: registry.clone(),
        config: config.clone(),
        auth_token: auth_token.clone(),
        api_token: api_token.clone(),
        update_info: update_info.clone(),
        sessions,
    };

    let grafana_state = api_state.clone();
    let auth_state = api_state.clone();
    let app = api::router(api_state)
        .nest(
            "/grafana",
            api::grafana_router(grafana_state).layer(
                axum::middleware::from_fn_with_state(auth_state, auth::require_auth),
            ),
        )
        .merge(web::router())
        .layer(CorsLayer::permissive());

    // Start HTTP server
    let listener = tokio::net::TcpListener::bind(&config.http_listen)
        .await
        .context(format!("binding to {}", config.http_listen))?;
    tracing::info!(listen = %config.http_listen, "HTTP server starting");

    let http_cancel = cancel.clone();
    let http = axum::serve(listener, app)
        .with_graceful_shutdown(async move { http_cancel.cancelled().await });

    // Supervise: if a background task dies while we're not shutting down, treat it as fatal
    // rather than serving HTTP over a dead gRPC server / pruner.
    tokio::select! {
        res = http => res.context("HTTP server error")?,
        res = grpc_handle => {
            if !cancel.is_cancelled() {
                cancel.cancel();
                anyhow::bail!("gRPC server task exited unexpectedly: {res:?}");
            }
        }
        res = prune_handle => {
            if !cancel.is_cancelled() {
                cancel.cancel();
                anyhow::bail!("retention pruner task exited unexpectedly: {res:?}");
            }
        }
    }

    tracing::info!("{SERVICE_NAME} stopped");
    Ok(())
}
