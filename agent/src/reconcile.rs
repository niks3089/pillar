use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::command::AgentCommand;
use crate::config::AgentConfig;
use crate::event::{EventKind, OperatorEvent};
use crate::health::{HealthChecker, NodeHealth, NodeState};
use crate::lifecycle::SystemdManager;
use crate::metrics_updater::SharedStatus;
use crate::snapshot::recovery::SnapshotRecovery;
use crate::snapshot::TcpSnapshotManager;

use pillar_shared::proto::NodeStatus;

pub struct Reconciler {
    config: AgentConfig,
    health_checker: Box<dyn HealthChecker>,
    service_manager: SystemdManager,
    snapshot_manager: TcpSnapshotManager,
    ledger_dir: PathBuf,
    validator_process: String,
    shared_status: SharedStatus,
    cmd_rx: mpsc::Receiver<AgentCommand>,

    // Internal state
    current_state: NodeState,
    state_entered_at: Instant,
    started_at: Instant,
    restart_timestamps: VecDeque<Instant>,
    last_health: NodeHealth,
    last_check_duration_secs: f64,
    consecutive_off_count: usize,
    upgrading: bool,

    // Self-health counters
    started_at_unix_secs: i64,
    reconcile_count: u64,
    health_check_error_count: u64,
    recovery_count: u64,

    // Version mismatch detection
    local_validator_version: Option<String>,
    cluster_version: Option<String>,
    version_mismatch: bool,
}

impl Reconciler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: AgentConfig,
        health_checker: Box<dyn HealthChecker>,
        service_manager: SystemdManager,
        snapshot_manager: TcpSnapshotManager,
        ledger_dir: PathBuf,
        validator_process: String,
        binary_path: PathBuf,
        shared_status: SharedStatus,
        cmd_rx: mpsc::Receiver<AgentCommand>,
    ) -> Self {
        let local_validator_version = detect_validator_version(&binary_path);
        if let Some(ref v) = local_validator_version {
            tracing::info!(version = %v, binary = %binary_path.display(), "detected local validator version");
        } else {
            tracing::warn!(binary = %binary_path.display(), "could not detect local validator version");
        }

        Self {
            config,
            health_checker,
            service_manager,
            snapshot_manager,
            ledger_dir,
            validator_process,
            shared_status,
            cmd_rx,
            current_state: NodeState::Off,
            state_entered_at: Instant::now(),
            started_at: Instant::now(),
            restart_timestamps: VecDeque::new(),
            last_health: NodeHealth::default(),
            last_check_duration_secs: 0.0,
            consecutive_off_count: 0,
            upgrading: false,
            started_at_unix_secs: chrono::Utc::now().timestamp(),
            reconcile_count: 0,
            health_check_error_count: 0,
            recovery_count: 0,
            local_validator_version,
            cluster_version: None,
            version_mismatch: false,
        }
    }

    fn evict_old_restarts(&mut self) {
        let window = Duration::from_secs(self.config.lifecycle.crash_window_secs);
        let cutoff = Instant::now() - window;
        while self.restart_timestamps.front().is_some_and(|t| *t < cutoff) {
            self.restart_timestamps.pop_front();
        }
    }

    fn restarts_in_window(&self) -> usize {
        let window = Duration::from_secs(self.config.lifecycle.crash_window_secs);
        let cutoff = Instant::now() - window;
        self.restart_timestamps.iter().filter(|t| **t >= cutoff).count()
    }

    fn record_restart(&mut self) {
        self.restart_timestamps.push_back(Instant::now());
    }

    /// Run the reconciliation loop until cancelled.
    /// Wakes immediately when a command arrives via the channel.
    pub async fn run(&mut self, cancel: CancellationToken) {
        let interval = Duration::from_secs(self.config.health.check_interval_secs);
        tracing::info!(interval_secs = interval.as_secs(), "reconcile loop starting");

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("reconciler cancelled, shutting down");
                    return;
                }
                _ = tokio::time::sleep(interval) => {
                    self.reconcile().await;
                }
                Some(cmd) = self.cmd_rx.recv() => {
                    // Immediate wake on command
                    self.handle_command(cmd).await;
                    // Drain any extra commands that arrived simultaneously
                    while let Ok(cmd) = self.cmd_rx.try_recv() {
                        self.handle_command(cmd).await;
                    }
                    self.reconcile().await;
                }
            }
        }
    }

    /// Handle a command received from the gRPC layer via the channel.
    async fn handle_command(&mut self, cmd: AgentCommand) {
        let command_type = cmd.command_type();
        match cmd {
            AgentCommand::Provision {
                staged_binary_path,
                ref config,
            } => {
                self.upgrading = true;
                let result = crate::provisioner::provision(config, &staged_binary_path).await;
                self.upgrading = false;
                match result {
                    Ok(()) => {
                        tracing::info!(command_type, "provision completed — exiting for config reload");
                        self.on_state_transition(self.current_state, NodeState::StartingUp);
                        self.publish_status().await;
                        std::process::exit(0);
                    }
                    Err(e) => {
                        tracing::error!(command_type, error = %e, "provision failed");
                    }
                }
            }
            AgentCommand::Upgrade {
                staged_binary_path,
                ref upgrade,
            } => {
                self.upgrading = true;
                let result = crate::provisioner::upgrade(upgrade, &staged_binary_path).await;
                self.upgrading = false;
                match result {
                    Ok(()) => {
                        tracing::info!(command_type, "upgrade completed successfully");
                        self.on_state_transition(self.current_state, NodeState::StartingUp);
                    }
                    Err(e) => {
                        tracing::error!(command_type, error = %e, "upgrade failed");
                    }
                }
            }
            AgentCommand::Restart { reason } => {
                tracing::info!(reason = %reason, "restart command from controller");
                match self.service_manager.restart().await {
                    Ok(()) => {
                        self.record_restart();
                        self.emit_event(EventKind::ServiceRestarted {
                            reason: format!("controller: {reason}"),
                        });
                        self.on_state_transition(self.current_state, NodeState::StartingUp);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "restart command failed");
                    }
                }
            }
            AgentCommand::Recover { reason } => {
                tracing::info!(reason = %reason, "recover command from controller");
                self.force_recovery().await;
            }
            AgentCommand::Stop { reason } => {
                tracing::info!(reason = %reason, "stop command from controller");
                match self.service_manager.stop().await {
                    Ok(()) => {
                        self.emit_event(EventKind::ServiceStopped {
                            reason: format!("controller: {reason}"),
                        });
                        self.on_state_transition(self.current_state, NodeState::Off);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "stop command failed");
                    }
                }
            }
        }
    }

    async fn reconcile(&mut self) {
        self.reconcile_count += 1;

        let start = Instant::now();

        // 1. Check health
        let health = match self.health_checker.check().await {
            Ok(h) => {
                if h.state == NodeState::Off {
                    self.consecutive_off_count += 1;
                } else {
                    self.consecutive_off_count = 0;
                }
                h
            }
            Err(e) => {
                self.consecutive_off_count += 1;
                self.health_check_error_count += 1;
                tracing::warn!(
                    error = %e,
                    consecutive_off = self.consecutive_off_count,
                    "health check error"
                );
                NodeHealth {
                    state: NodeState::Off,
                    ..Default::default()
                }
            }
        };

        self.last_check_duration_secs = start.elapsed().as_secs_f64();

        // 2. Handle state transition — don't transition to Off until consecutive threshold met
        let effective_state = if health.state == NodeState::Off
            && self.consecutive_off_count < self.config.health.consecutive_off_threshold
        {
            self.current_state
        } else {
            health.state
        };

        if effective_state != self.current_state {
            self.on_state_transition(self.current_state, effective_state);
        }

        self.last_health = health;

        // 2b. Version mismatch detection
        if let Some(ref cv) = self.last_health.cluster_version {
            self.cluster_version = Some(cv.clone());
        }
        if let (Some(local), Some(cluster)) =
            (&self.local_validator_version, &self.cluster_version)
        {
            let local_major = parse_major_version(local);
            let cluster_major = parse_major_version(cluster);
            if local_major != cluster_major {
                if !self.version_mismatch {
                    self.version_mismatch = true;
                    tracing::error!(
                        local_version = %local,
                        cluster_version = %cluster,
                        "version mismatch detected — validator binary must be upgraded"
                    );
                    self.emit_event(EventKind::VersionMismatchDetected {
                        local_version: local.clone(),
                        cluster_version: cluster.clone(),
                    });
                }
            } else if self.version_mismatch {
                self.version_mismatch = false;
                tracing::info!(
                    local_version = %local,
                    cluster_version = %cluster,
                    "version mismatch resolved"
                );
            }
        }

        // 3. Publish status to shared Arc (instead of writing to file)
        self.publish_status().await;

        // 4. Check timeouts
        self.check_timeouts().await;

        // 5. If Off, attempt recovery
        if self.current_state == NodeState::Off {
            self.attempt_recovery().await;
        }
    }

    fn on_state_transition(&mut self, from: NodeState, to: NodeState) {
        tracing::info!(from = ?from, to = ?to, "state transition");
        self.current_state = to;
        self.state_entered_at = Instant::now();
        self.emit_event(EventKind::StateTransition { from, to });
    }

    async fn check_timeouts(&mut self) {
        let elapsed = self.state_entered_at.elapsed();

        match self.current_state {
            NodeState::StartingUp => {
                let max_wait = Duration::from_secs(self.config.lifecycle.max_startup_wait_secs);
                if elapsed > max_wait {
                    tracing::warn!(
                        elapsed_secs = elapsed.as_secs(),
                        max_secs = max_wait.as_secs(),
                        "startup timeout exceeded, triggering recovery"
                    );
                    self.attempt_recovery().await;
                }
            }
            NodeState::Behind => {
                let max_wait = Duration::from_secs(self.config.lifecycle.max_catchup_wait_secs);
                if elapsed > max_wait {
                    tracing::warn!(
                        elapsed_secs = elapsed.as_secs(),
                        max_secs = max_wait.as_secs(),
                        "catchup timeout exceeded, triggering recovery"
                    );
                    self.attempt_recovery().await;
                }
            }
            _ => {}
        }
    }

    async fn attempt_recovery(&mut self) {
        if self.upgrading {
            tracing::info!("upgrade/provision in progress, skipping recovery");
            return;
        }

        if self.version_mismatch {
            tracing::warn!("version mismatch — recovery skipped, binary upgrade required");
            return;
        }

        if !self.service_manager.service_exists().await {
            tracing::debug!("validator service not installed, waiting for provisioning");
            return;
        }

        self.evict_old_restarts();

        if self.restarts_in_window() >= self.config.lifecycle.crash_threshold {
            tracing::error!(
                restarts_in_window = self.restarts_in_window(),
                window_secs = self.config.lifecycle.crash_window_secs,
                threshold = self.config.lifecycle.crash_threshold,
                "crash loop detected, backing off"
            );
            self.emit_event(EventKind::CrashLoopDetected {
                restarts_in_window: self.restarts_in_window(),
            });
            return;
        }

        self.recovery_count += 1;

        let reference_slot = self.last_health.slot_info.reference_slot.unwrap_or(0);

        let recovery = SnapshotRecovery::new(
            &self.service_manager,
            &self.snapshot_manager,
            &self.ledger_dir,
            self.config.snapshot.staleness_threshold_slots,
        );

        let result = recovery.recover_if_stale(reference_slot).await;

        match result {
            Ok(_) => {
                self.record_restart();
                self.emit_event(EventKind::ServiceRestarted {
                    reason: "recovery".to_string(),
                });
                self.on_state_transition(self.current_state, NodeState::StartingUp);
            }
            Err(e) => {
                tracing::error!(error = %e, "recovery failed");
                if let Err(restart_err) = self.service_manager.restart().await {
                    tracing::error!(error = %restart_err, "restart also failed");
                } else {
                    self.record_restart();
                    self.emit_event(EventKind::ServiceRestarted {
                        reason: "restart_fallback".to_string(),
                    });
                    self.on_state_transition(self.current_state, NodeState::StartingUp);
                }
            }
        }
    }

    async fn force_recovery(&mut self) {
        if self.upgrading {
            tracing::info!("upgrade/provision in progress, skipping forced recovery");
            return;
        }

        let recovery = SnapshotRecovery::new(
            &self.service_manager,
            &self.snapshot_manager,
            &self.ledger_dir,
            self.config.snapshot.staleness_threshold_slots,
        );

        match recovery.force_recovery().await {
            Ok(()) => {
                self.record_restart();
                self.emit_event(EventKind::ServiceRestarted {
                    reason: "forced_recovery".to_string(),
                });
                self.on_state_transition(self.current_state, NodeState::StartingUp);
            }
            Err(e) => {
                tracing::error!(error = %e, "forced recovery failed, attempting simple restart");
                if let Err(restart_err) = self.service_manager.restart().await {
                    tracing::error!(error = %restart_err, "restart also failed");
                } else {
                    self.record_restart();
                    self.emit_event(EventKind::ServiceRestarted {
                        reason: "forced_recovery_restart_fallback".to_string(),
                    });
                    self.on_state_transition(self.current_state, NodeState::StartingUp);
                }
            }
        }
    }

    /// Write health fields to shared status Arc.
    /// Preserves sysinfo fields written by metrics_updater.
    async fn publish_status(&mut self) {
        let mut guard = self.shared_status.write().await;
        let status = guard.get_or_insert_with(NodeStatus::default);

        // Overwrite health fields only
        status.state = self.current_state.as_str().to_string();
        status.local_slot = self.last_health.slot_info.local_slot.unwrap_or(0) as i64;
        status.reference_slot = self.last_health.slot_info.reference_slot.unwrap_or(0) as i64;
        status.slots_behind = self.last_health.slots_behind.unwrap_or(0);
        status.healthy = self.current_state == NodeState::Healthy;
        status.restart_count = self.restarts_in_window() as u64;
        status.crash_looping = self.restarts_in_window() >= self.config.lifecycle.crash_threshold;
        status.health_check_duration_secs = self.last_check_duration_secs;
        status.version = env!("CARGO_PKG_VERSION").to_string();
        status.role = self.config.role.to_string();
        status.client = self.config.client.to_string();
        status.cluster = self.config.network.cluster.clone();
        status.updated_at_unix_secs = chrono::Utc::now().timestamp();
        status.state_duration_secs = self.state_entered_at.elapsed().as_secs();
        status.validator_process = self.validator_process.clone();
        status.pending_upgrade = if self.upgrading {
            "in-progress".to_string()
        } else if self.version_mismatch {
            format!(
                "version_mismatch: local={}, cluster={}",
                self.local_validator_version.as_deref().unwrap_or("unknown"),
                self.cluster_version.as_deref().unwrap_or("unknown")
            )
        } else {
            String::new()
        };

        // Agent self-health
        status.reconcile_count = self.reconcile_count;
        status.health_check_errors = self.health_check_error_count;
        status.consecutive_off_count = self.consecutive_off_count as u32;
        status.recovery_count = self.recovery_count;
        status.agent_uptime_secs = self.started_at.elapsed().as_secs();
        status.version_mismatch = self.version_mismatch;
        status.agent_started_at_unix_secs = self.started_at_unix_secs;
        status.agent_version = env!("CARGO_PKG_VERSION").to_string();

        // DO NOT touch: cpu_usage_percent, memory_*, disk_*, network_*, validator_*,
        // controller_*, status_reports_*, etc. (written by metrics_updater)

        // Optionally write debug state file
        if !self.config.debug_state_file.is_empty() {
            let path = std::path::PathBuf::from(&self.config.debug_state_file);
            if let Err(e) = pillar_shared::write_state(status, &path) {
                tracing::warn!(error = %e, "failed to write debug state file");
            }
        }
    }

    fn emit_event(&self, kind: EventKind) {
        let event = OperatorEvent {
            timestamp: chrono::Utc::now(),
            kind,
        };
        tracing::info!(event = ?event, "operator event");
    }
}

/// Run `<binary_path> --version` and extract the version string.
fn detect_validator_version(binary_path: &Path) -> Option<String> {
    let output = std::process::Command::new(binary_path)
        .arg("--version")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_version_output(&stdout)
}

fn parse_version_output(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|token| {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() >= 2 && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
            Some(token.to_string())
        } else {
            None
        }
    })
}

fn parse_major_version(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_major_version_basic() {
        assert_eq!(parse_major_version("3.1.8"), Some(3));
        assert_eq!(parse_major_version("2.1.21"), Some(2));
        assert_eq!(parse_major_version("1.18.26"), Some(1));
    }

    #[test]
    fn parse_major_version_edge_cases() {
        assert_eq!(parse_major_version(""), None);
        assert_eq!(parse_major_version("abc"), None);
        assert_eq!(parse_major_version("0.1.0"), Some(0));
    }

    #[test]
    fn parse_version_output_agave() {
        let output = "agave-validator 2.1.21 (src:8a085eeb; feat:1234)";
        assert_eq!(parse_version_output(output), Some("2.1.21".to_string()));
    }

    #[test]
    fn parse_version_output_solana_core() {
        let output = "solana-validator 1.18.26 (src:abc123; feat:5678)";
        assert_eq!(parse_version_output(output), Some("1.18.26".to_string()));
    }

    #[test]
    fn parse_version_output_two_part() {
        let output = "firedancer 0.1";
        assert_eq!(parse_version_output(output), Some("0.1".to_string()));
    }

    #[test]
    fn parse_version_output_no_version() {
        assert_eq!(parse_version_output("no version here"), None);
        assert_eq!(parse_version_output(""), None);
    }
}
