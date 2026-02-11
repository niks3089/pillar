use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::config::ControllerConfig;
use crate::state_reader::SharedState;

pub mod proto {
    tonic::include_proto!("pillar");
}

use proto::pillar_controller_client::PillarControllerClient;

use pillar_shared::proto::{CommandStreamRequest, ControllerCommand, ReportStatusRequest};

/// Connection to the centralized controller.
///
/// Runs two concurrent loops:
///   1. report_status — pushes NodeStatus to controller every N seconds
///   2. command_stream — listens for commands from controller (restart, recover, update_config)
pub struct ControllerLink {
    config: ControllerConfig,
    operator_state: SharedState,
}

impl ControllerLink {
    pub fn new(config: ControllerConfig, operator_state: SharedState) -> Self {
        tracing::info!(
            endpoint = %config.endpoint,
            node_id = %config.node_id,
            report_interval_secs = config.report_interval_secs,
            "controller link created"
        );
        Self {
            config,
            operator_state,
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
                    self.run_connected(client, cancel.clone()).await;
                }
                Err(e) => {
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

    /// Run report_status and command_stream concurrently on an established connection.
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
        let report_shared = self.operator_state.clone();
        let report_interval = Duration::from_secs(self.config.report_interval_secs);

        let report_handle = tokio::spawn(async move {
            run_report_loop(
                &mut report_client,
                &report_node_id,
                report_shared,
                report_interval,
                report_cancel,
            )
            .await
        });

        let cmd_cancel = cancel.clone();
        let mut cmd_client = client;
        let cmd_node_id = self.config.node_id.clone();

        let cmd_handle = tokio::spawn(async move {
            run_command_stream(&mut cmd_client, &cmd_node_id, cmd_cancel).await
        });

        // If either task exits, cancel the other and reconnect
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
    operator_state: SharedState,
    interval: Duration,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(interval) => {}
        }

        let state = operator_state.read().await;
        let Some(ref status) = *state else {
            tracing::debug!("no operator state yet, skipping report");
            continue;
        };

        let request = tonic::Request::new(ReportStatusRequest {
            node_id: node_id.to_string(),
            status: Some(status.clone()),
        });

        match client.report_status(request).await {
            Ok(_) => tracing::debug!("reported status to controller"),
            Err(e) => {
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
                    Ok(Some(cmd)) => handle_command(cmd),
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

fn handle_command(cmd: ControllerCommand) {
    use pillar_shared::proto::controller_command::Command;
    match cmd.command {
        Some(Command::Restart(r)) => {
            tracing::info!(reason = %r.reason, "received restart command");
            tokio::spawn(async move {
                if let Err(e) = crate::provisioner::write_pending_command(
                    &pillar_shared::PendingCommand::Restart {
                        reason: r.reason,
                    },
                )
                .await
                {
                    tracing::error!(error = %e, "restart command failed");
                }
            });
        }
        Some(Command::Recover(r)) => {
            tracing::info!(reason = %r.reason, "received recover command");
            tokio::spawn(async move {
                if let Err(e) = crate::provisioner::write_pending_command(
                    &pillar_shared::PendingCommand::Recover {
                        reason: r.reason,
                    },
                )
                .await
                {
                    tracing::error!(error = %e, "recover command failed");
                }
            });
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
            tokio::spawn(async move {
                if let Err(e) = handle_upgrade(u, &download_url, &sha256).await {
                    tracing::error!(error = %e, "upgrade command failed");
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
            tokio::spawn(async move {
                if let Err(e) = handle_provision(p, &download_url, &sha256).await {
                    tracing::error!(error = %e, "provision command failed");
                }
            });
        }
        Some(Command::Stop(s)) => {
            tracing::info!(reason = %s.reason, "received stop command");
            tokio::spawn(async move {
                // Clean up staging dir if a download is in progress
                let staging_dir = std::path::Path::new("/tmp/pillar-staging");
                if staging_dir.exists() {
                    if let Err(e) = tokio::fs::remove_dir_all(staging_dir).await {
                        tracing::warn!(error = %e, "failed to clean up staging dir");
                    }
                }
                if let Err(e) = crate::provisioner::write_pending_command(
                    &pillar_shared::PendingCommand::Stop {
                        reason: s.reason,
                    },
                )
                .await
                {
                    tracing::error!(error = %e, "stop command failed");
                }
            });
        }
        None => {
            tracing::warn!("received empty controller command");
        }
    }
}

async fn handle_provision(
    p: pillar_shared::proto::ProvisionCommand,
    download_url: &str,
    sha256: &str,
) -> Result<(), String> {
    use crate::provisioner;

    let staging_dir = std::path::Path::new("/tmp/pillar-staging");
    tokio::fs::create_dir_all(staging_dir)
        .await
        .map_err(|e| format!("create staging dir: {e}"))?;
    let staged = staging_dir.join("binary");

    let timeout = std::time::Duration::from_secs(3600);
    provisioner::download_and_verify(download_url, &staged, sha256, timeout).await?;

    provisioner::write_pending_command(&pillar_shared::PendingCommand::Provision {
        staged_binary_path: staged.display().to_string(),
        provision: Box::new(p),
    })
    .await?;

    tracing::info!("provision command file written for operator");
    Ok(())
}

async fn handle_upgrade(
    u: pillar_shared::proto::UpgradeCommand,
    download_url: &str,
    sha256: &str,
) -> Result<(), String> {
    use crate::provisioner;

    let staging_dir = std::path::Path::new("/tmp/pillar-staging");
    tokio::fs::create_dir_all(staging_dir)
        .await
        .map_err(|e| format!("create staging dir: {e}"))?;
    let staged = staging_dir.join("binary");

    let timeout = std::time::Duration::from_secs(3600);
    provisioner::download_and_verify(download_url, &staged, sha256, timeout).await?;

    provisioner::write_pending_command(&pillar_shared::PendingCommand::Upgrade {
        staged_binary_path: staged.display().to_string(),
        upgrade: u,
    })
    .await?;

    tracing::info!("upgrade command file written for operator");
    Ok(())
}

fn gethostname() -> String {
    sysinfo::System::host_name().unwrap_or_default()
}
