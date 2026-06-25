use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};
use tokio_util::sync::CancellationToken;

use crate::agent_health::AgentHealth;
use crate::command::AgentCommand;
use crate::config::ControllerConfig;
use crate::metrics_updater::SharedStatus;

use pillar_shared::proto::{
    CommandStreamRequest, ControllerCommand, ReportStatusRequest, ScriptResult,
};

pub mod proto {
    tonic::include_proto!("pillar");
}

use proto::pillar_controller_client::PillarControllerClient;

/// Type alias for the client with an optional auth interceptor.
type AuthClient = PillarControllerClient<tonic::service::interceptor::InterceptedService<Channel, AuthInterceptor>>;

/// Public alias for use by the log collector.
pub type LogClient = AuthClient;

/// Create an authenticated client from a channel (for use by the log collector).
pub fn make_log_client(channel: Channel, token: &str) -> LogClient {
    let interceptor = make_interceptor(token);
    PillarControllerClient::with_interceptor(channel, interceptor)
}

/// Interceptor that injects a bearer token into every gRPC request.
#[derive(Clone)]
pub struct AuthInterceptor {
    token: Option<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>,
}

impl tonic::service::Interceptor for AuthInterceptor {
    fn call(&mut self, mut req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        if let Some(ref token) = self.token {
            req.metadata_mut().insert("authorization", token.clone());
        }
        Ok(req)
    }
}

fn make_interceptor(token: &str) -> AuthInterceptor {
    let token = if token.is_empty() {
        None
    } else {
        let val = format!("Bearer {token}");
        Some(val.parse().expect("valid metadata value"))
    };
    AuthInterceptor { token }
}

/// Connection to the centralized controller.
///
/// Runs three concurrent loops:
///   1. report_status — pushes NodeStatus to controller every N seconds
///   2. command_stream — listens for commands from controller
///   3. result_reporter — sends ScriptResult back to controller
pub struct ControllerLink {
    config: ControllerConfig,
    shared_status: SharedStatus,
    agent_health: Arc<AgentHealth>,
    cmd_tx: mpsc::Sender<AgentCommand>,
    result_rx: mpsc::Receiver<ScriptResult>,
}

impl ControllerLink {
    pub fn new(
        config: ControllerConfig,
        shared_status: SharedStatus,
        agent_health: Arc<AgentHealth>,
        cmd_tx: mpsc::Sender<AgentCommand>,
        result_rx: mpsc::Receiver<ScriptResult>,
    ) -> Self {
        tracing::info!(
            endpoint = %config.endpoint,
            node_id = %config.node_id,
            report_interval_secs = config.report_interval_secs,
            "controller link created"
        );
        Self {
            config,
            shared_status,
            agent_health,
            cmd_tx,
            result_rx,
        }
    }

    /// Run the controller connection. Retries on failure with backoff.
    pub async fn run(mut self, cancel: CancellationToken) {
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(60);

        loop {
            if cancel.is_cancelled() {
                break;
            }

            tracing::info!(endpoint = %self.config.endpoint, "connecting to controller");

            match build_channel(&self.config).await {
                Ok(channel) => {
                    let interceptor = make_interceptor(&self.config.auth_token);
                    let client = PillarControllerClient::with_interceptor(channel, interceptor);
                    backoff = Duration::from_secs(1);
                    self.agent_health.set_controller_connected(true);
                    self.run_connected(client, cancel.clone()).await;
                    self.agent_health.set_controller_connected(false);
                }
                Err(e) => {
                    self.agent_health.set_controller_connected(false);
                    tracing::warn!(
                        error = %e,
                        backoff_secs = backoff.as_secs(),
                        "failed to connect to controller, retrying"
                    );
                }
            }

            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(max_backoff);
        }

        tracing::info!("controller link shutting down");
    }

    async fn run_connected(
        &mut self,
        client: AuthClient,
        cancel: CancellationToken,
    ) {
        // Register with controller on every (re-)connect
        let mut reg_client = client.clone();
        let reg_req = tonic::Request::new(pillar_shared::proto::RegisterNodeRequest {
            node_id: self.config.node_id.clone(),
            hostname: gethostname(),
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            ..Default::default()
        });
        match reg_client.register_node(reg_req).await {
            Ok(resp) => {
                let r = resp.into_inner();
                tracing::info!(accepted = r.accepted, message = %r.message, "registered with controller");
            }
            Err(e) => {
                tracing::warn!(error = %e, "RegisterNode failed (will still report status)");
            }
        }

        let report_cancel = cancel.clone();
        let mut report_client = client.clone();
        let report_node_id = self.config.node_id.clone();
        let report_shared = self.shared_status.clone();
        let report_interval = Duration::from_secs(self.config.report_interval_secs);
        let report_health = self.agent_health.clone();

        let report_handle = tokio::spawn(async move {
            run_report_loop(
                &mut report_client,
                &report_node_id,
                report_shared,
                report_interval,
                report_health,
                report_cancel,
            )
            .await
        });

        let cmd_cancel = cancel.clone();
        let mut cmd_client = client.clone();
        let cmd_node_id = self.config.node_id.clone();
        let cmd_health = self.agent_health.clone();
        let cmd_tx = self.cmd_tx.clone();

        let cmd_handle = tokio::spawn(async move {
            run_command_stream(&mut cmd_client, &cmd_node_id, cmd_health, cmd_tx, cmd_cancel).await
        });

        // Spawn result reporter — drains result_rx and sends to controller
        let result_cancel = cancel.clone();
        let mut result_client = client;
        // Take the receiver temporarily using a swap with an empty channel
        let (dummy_tx, dummy_rx) = mpsc::channel(1);
        let mut rx = std::mem::replace(&mut self.result_rx, dummy_rx);
        drop(dummy_tx);
        let result_handle = tokio::spawn(async move {
            run_result_reporter(&mut result_client, &mut rx, result_cancel).await;
            rx
        });

        tokio::select! {
            _ = report_handle => {
                tracing::warn!("report_status loop exited, will reconnect");
            }
            _ = cmd_handle => {
                tracing::warn!("command_stream exited, will reconnect");
            }
            returned_rx = result_handle => {
                tracing::warn!("result_reporter exited, will reconnect");
                // Restore the receiver for the next connection
                if let Ok(rx) = returned_rx {
                    self.result_rx = rx;
                }
            }
            _ = cancel.cancelled() => {}
        }
    }
}

/// Push enriched NodeStatus to controller on a timer.
async fn run_report_loop(
    client: &mut AuthClient,
    node_id: &str,
    shared_status: SharedStatus,
    interval: Duration,
    agent_health: Arc<AgentHealth>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(interval) => {}
        }

        // Clone out and drop the guard before the RPC: holding the read lock across
        // the await would block publish_status / metrics writers on a slow controller.
        let status = {
            let guard = shared_status.read().await;
            match guard.as_ref() {
                Some(s) => s.clone(),
                None => {
                    tracing::debug!("no status yet, skipping report");
                    continue;
                }
            }
        };

        let request = tonic::Request::new(ReportStatusRequest {
            node_id: node_id.to_string(),
            status: Some(status),
        });

        let start = Instant::now();
        match client.report_status(request).await {
            Ok(_) => {
                agent_health.set_controller_latency_ms(start.elapsed().as_millis() as u64);
                agent_health.inc_status_reports_sent();
                tracing::debug!("reported status to controller");
            }
            Err(e) => {
                agent_health.inc_status_reports_failed();
                tracing::warn!(error = %e, "report_status failed");
                return;
            }
        }
    }
}

/// Listen for commands from controller via server-streaming RPC.
async fn run_command_stream(
    client: &mut AuthClient,
    node_id: &str,
    agent_health: Arc<AgentHealth>,
    cmd_tx: mpsc::Sender<AgentCommand>,
    cancel: CancellationToken,
) {
    let request = tonic::Request::new(CommandStreamRequest {
        node_id: node_id.to_string(),
    });

    let stream = match client.command_stream(request).await {
        Ok(resp) => resp.into_inner(),
        Err(e) => {
            tracing::warn!(error = %e, "command_stream failed to open");
            return;
        }
    };

    let mut stream = stream;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            msg = stream.message() => {
                match msg {
                    Ok(Some(cmd)) => {
                        agent_health.inc_commands_received();
                        handle_command(cmd, &cmd_tx).await;
                    }
                    Ok(None) => {
                        tracing::info!("command stream closed by controller");
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "command stream error");
                        return;
                    }
                }
            }
        }
    }
}

/// Handle a controller command — send to reconcile loop via channel.
async fn handle_command(cmd: ControllerCommand, cmd_tx: &mpsc::Sender<AgentCommand>) {
    use pillar_shared::proto::controller_command::Command;
    match cmd.command {
        Some(Command::Execute(script)) => {
            tracing::info!(
                script_id = %script.script_id,
                desc = %script.description,
                "received script command"
            );
            let _ = cmd_tx.send(AgentCommand::ExecuteScript(script)).await;
        }
        None => {
            tracing::warn!("received empty controller command");
        }
    }
}

/// Send ScriptResult messages back to the controller.
async fn run_result_reporter(
    client: &mut AuthClient,
    result_rx: &mut mpsc::Receiver<ScriptResult>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            Some(result) = result_rx.recv() => {
                tracing::info!(
                    script_id = %result.script_id,
                    exit_code = result.exit_code,
                    timed_out = result.timed_out,
                    "reporting script result to controller"
                );
                let request = tonic::Request::new(result);
                match client.report_script_result(request).await {
                    Ok(_) => {
                        tracing::debug!("script result reported successfully");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to report script result");
                        return;
                    }
                }
            }
        }
    }
}

/// Build a tonic Channel with optional server TLS from the controller config.
pub async fn build_channel(config: &ControllerConfig) -> Result<Channel, tonic::transport::Error> {
    let endpoint = Channel::from_shared(config.endpoint.clone()).expect("valid endpoint URI");

    let endpoint = if !config.ca_cert_path.is_empty() {
        let ca =
            std::fs::read_to_string(&config.ca_cert_path).expect("reading CA cert");
        let mut tls = ClientTlsConfig::new().ca_certificate(Certificate::from_pem(&ca));
        if !config.client_cert_path.is_empty() && !config.client_key_path.is_empty() {
            let cert = std::fs::read_to_string(&config.client_cert_path).expect("reading client cert");
            let key = std::fs::read_to_string(&config.client_key_path).expect("reading client key");
            tls = tls.identity(Identity::from_pem(cert, key));
            tracing::info!("mTLS client certificate loaded");
        }
        tracing::info!("TLS enabled for controller connection");
        endpoint.tls_config(tls)?
    } else {
        endpoint
    };

    endpoint.connect().await
}

fn gethostname() -> String {
    sysinfo::System::host_name().unwrap_or_default()
}
