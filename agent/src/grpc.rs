use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agent_health::AgentHealth;
use crate::command::AgentCommand;
use crate::config::ControllerConfig;
use crate::metrics_updater::SharedStatus;

pub mod proto {
    tonic::include_proto!("pillar");
}

use proto::pillar_controller_client::PillarControllerClient;

use pillar_shared::proto::{CommandStreamRequest, ControllerCommand, ReportStatusRequest};

/// Connection to the centralized controller.
///
/// Runs two concurrent loops:
///   1. report_status — pushes NodeStatus to controller every N seconds
///   2. command_stream — listens for commands from controller
pub struct ControllerLink {
    config: ControllerConfig,
    shared_status: SharedStatus,
    agent_health: Arc<AgentHealth>,
    cmd_tx: mpsc::Sender<AgentCommand>,
}

impl ControllerLink {
    pub fn new(
        config: ControllerConfig,
        shared_status: SharedStatus,
        agent_health: Arc<AgentHealth>,
        cmd_tx: mpsc::Sender<AgentCommand>,
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
        }
    }

    /// Run the controller connection. Retries on failure with backoff.
    pub async fn run(&self, cancel: CancellationToken) {
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(60);

        loop {
            if cancel.is_cancelled() {
                break;
            }

            tracing::info!(endpoint = %self.config.endpoint, "connecting to controller");

            match PillarControllerClient::connect(self.config.endpoint.clone()).await {
                Ok(client) => {
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
        &self,
        client: PillarControllerClient<tonic::transport::Channel>,
        cancel: CancellationToken,
    ) {
        // Register with controller on every (re-)connect
        let mut reg_client = client.clone();
        let reg_req = tonic::Request::new(pillar_shared::proto::RegisterNodeRequest {
            node_id: self.config.node_id.clone(),
            hostname: gethostname(),
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
        let mut cmd_client = client;
        let cmd_node_id = self.config.node_id.clone();
        let cmd_health = self.agent_health.clone();
        let cmd_tx = self.cmd_tx.clone();

        let cmd_handle = tokio::spawn(async move {
            run_command_stream(&mut cmd_client, &cmd_node_id, cmd_health, cmd_tx, cmd_cancel).await
        });

        tokio::select! {
            _ = report_handle => {
                tracing::warn!("report_status loop exited, will reconnect");
            }
            _ = cmd_handle => {
                tracing::warn!("command_stream exited, will reconnect");
            }
            _ = cancel.cancelled() => {}
        }
    }
}

/// Push enriched NodeStatus to controller on a timer.
async fn run_report_loop(
    client: &mut PillarControllerClient<tonic::transport::Channel>,
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

        let state = shared_status.read().await;
        let Some(ref status) = *state else {
            tracing::debug!("no status yet, skipping report");
            continue;
        };

        let request = tonic::Request::new(ReportStatusRequest {
            node_id: node_id.to_string(),
            status: Some(status.clone()),
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
    client: &mut PillarControllerClient<tonic::transport::Channel>,
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

/// Handle a controller command — send directly to reconcile loop via channel,
/// or spawn a download task for provision/upgrade.
async fn handle_command(cmd: ControllerCommand, cmd_tx: &mpsc::Sender<AgentCommand>) {
    use pillar_shared::proto::controller_command::Command;
    match cmd.command {
        Some(Command::Restart(r)) => {
            tracing::info!(reason = %r.reason, "received restart command");
            let _ = cmd_tx.send(AgentCommand::Restart { reason: r.reason }).await;
        }
        Some(Command::Recover(r)) => {
            tracing::info!(reason = %r.reason, "received recover command");
            let _ = cmd_tx.send(AgentCommand::Recover { reason: r.reason }).await;
        }
        Some(Command::UpdateConfig(c)) => {
            tracing::info!(config_size = c.config_yaml.len(), "received config update command");
            // TODO: write config and signal reload
        }
        Some(Command::Upgrade(u)) => {
            tracing::info!(
                binary = %u.binary_name,
                version = %u.version,
                reason = %u.reason,
                "received upgrade command"
            );
            let download_url = u.download_url.clone();
            let sha256 = u.sha256.clone();
            let tx = cmd_tx.clone();
            tokio::spawn(async move {
                match crate::provisioner::download_and_stage(&download_url, &sha256).await {
                    Ok(staged) => {
                        let _ = tx.send(AgentCommand::Upgrade {
                            staged_binary_path: staged,
                            upgrade: u,
                        }).await;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "upgrade download failed");
                    }
                }
            });
        }
        Some(Command::Provision(p)) => {
            tracing::info!(
                client = %p.client,
                version = %p.version,
                cluster = %p.cluster,
                "received provision command"
            );
            let download_url = p.download_url.clone();
            let sha256 = p.sha256.clone();
            let tx = cmd_tx.clone();
            tokio::spawn(async move {
                match crate::provisioner::download_and_stage(&download_url, &sha256).await {
                    Ok(staged) => {
                        let _ = tx.send(AgentCommand::Provision {
                            staged_binary_path: staged,
                            config: Box::new(p),
                        }).await;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "provision download failed");
                    }
                }
            });
        }
        Some(Command::Stop(s)) => {
            tracing::info!(reason = %s.reason, "received stop command");
            // Clean up staging dir if a download is in progress
            let staging_dir = std::path::Path::new("/tmp/pillar-staging");
            if staging_dir.exists() {
                if let Err(e) = tokio::fs::remove_dir_all(staging_dir).await {
                    tracing::warn!(error = %e, "failed to clean up staging dir");
                }
            }
            let _ = cmd_tx.send(AgentCommand::Stop { reason: s.reason }).await;
        }
        None => {
            tracing::warn!("received empty controller command");
        }
    }
}

fn gethostname() -> String {
    sysinfo::System::host_name().unwrap_or_default()
}
