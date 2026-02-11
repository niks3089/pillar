use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::config::OperatorConfig;
use crate::event::{EventKind, OperatorEvent};
use crate::health::{HealthChecker, NodeHealth, NodeState};
use crate::lifecycle::SystemdManager;
use crate::snapshot::recovery::SnapshotRecovery;
use crate::snapshot::TcpSnapshotManager;
use crate::state;

pub struct Operator {
    config: OperatorConfig,
    health_checker: Box<dyn HealthChecker>,
    service_manager: SystemdManager,
    snapshot_manager: TcpSnapshotManager,
    ledger_dir: PathBuf,
    state_path: PathBuf,
    validator_process: String,

    // Internal state
    current_state: NodeState,
    state_entered_at: Instant,
    restart_timestamps: VecDeque<Instant>,
    last_health: NodeHealth,
    last_check_duration_secs: f64,
    consecutive_off_count: usize,
    /// Set while a provision/upgrade is in progress. Prevents recovery from triggering.
    upgrading: bool,

    // Version mismatch detection
    local_validator_version: Option<String>,
    cluster_version: Option<String>,
    version_mismatch: bool,
}

impl Operator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: OperatorConfig,
        health_checker: Box<dyn HealthChecker>,
        service_manager: SystemdManager,
        snapshot_manager: TcpSnapshotManager,
        ledger_dir: PathBuf,
        state_path: PathBuf,
        validator_process: String,
        binary_path: PathBuf,
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
            state_path,
            validator_process,
            current_state: NodeState::Off,
            state_entered_at: Instant::now(),
            restart_timestamps: VecDeque::new(),
            last_health: NodeHealth::default(),
            last_check_duration_secs: 0.0,
            consecutive_off_count: 0,
            upgrading: false,
            local_validator_version,
            cluster_version: None,
            version_mismatch: false,
        }
    }

    /// Remove restart timestamps outside the crash window.
    fn evict_old_restarts(&mut self) {
        let window = Duration::from_secs(self.config.lifecycle.crash_window_secs);
        let cutoff = Instant::now() - window;
        while self.restart_timestamps.front().is_some_and(|t| *t < cutoff) {
            self.restart_timestamps.pop_front();
        }
    }

    /// Count of restarts within the current crash window.
    fn restarts_in_window(&self) -> usize {
        let window = Duration::from_secs(self.config.lifecycle.crash_window_secs);
        let cutoff = Instant::now() - window;
        self.restart_timestamps.iter().filter(|t| **t >= cutoff).count()
    }

    /// Record a restart event.
    fn record_restart(&mut self) {
        self.restart_timestamps.push_back(Instant::now());
    }

    /// Run the reconciliation loop until cancelled.
    pub async fn run(&mut self, cancel: CancellationToken) {
        let interval = Duration::from_secs(self.config.health.check_interval_secs);
        tracing::info!(interval_secs = interval.as_secs(), "operator loop starting");

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("operator cancelled, shutting down");
                    return;
                }
                _ = tokio::time::sleep(interval) => {
                    self.reconcile().await;
                }
            }
        }
    }

    async fn reconcile(&mut self) {
        // 0. Check for pending command from Link
        match crate::provisioner::process_pending_command() {
            Ok(Some(cmd)) => {
                let command_type = cmd.command_type();
                match cmd {
                    pillar_shared::PendingCommand::Provision {
                        staged_binary_path,
                        ref provision,
                    } => {
                        self.upgrading = true;
                        let staged = PathBuf::from(&staged_binary_path);
                        let result = crate::provisioner::provision(provision, &staged).await;
                        self.upgrading = false;
                        match result {
                            Ok(()) => {
                                tracing::info!(command_type, "provision completed — exiting for config reload");
                                // Write final state before exiting
                                self.on_state_transition(
                                    self.current_state,
                                    NodeState::StartingUp,
                                );
                                self.publish_state().await;
                                // Exit so systemd restarts us with updated operator config
                                // (client, cluster, service_name may have changed)
                                std::process::exit(0);
                            }
                            Err(e) => {
                                tracing::error!(command_type, error = %e, "provision failed");
                            }
                        }
                    }
                    pillar_shared::PendingCommand::Upgrade {
                        staged_binary_path,
                        ref upgrade,
                    } => {
                        self.upgrading = true;
                        let staged = PathBuf::from(&staged_binary_path);
                        let result = crate::provisioner::upgrade(upgrade, &staged).await;
                        self.upgrading = false;
                        match result {
                            Ok(()) => {
                                tracing::info!(command_type, "upgrade completed successfully");
                                self.on_state_transition(
                                    self.current_state,
                                    NodeState::StartingUp,
                                );
                            }
                            Err(e) => {
                                tracing::error!(command_type, error = %e, "upgrade failed");
                            }
                        }
                    }
                    pillar_shared::PendingCommand::Restart { reason } => {
                        tracing::info!(reason = %reason, "restart command from controller");
                        match self.service_manager.restart().await {
                            Ok(()) => {
                                self.record_restart();
                                self.emit_event(EventKind::ServiceRestarted {
                                    reason: format!("controller: {reason}"),
                                });
                                self.on_state_transition(
                                    self.current_state,
                                    NodeState::StartingUp,
                                );
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "restart command failed");
                            }
                        }
                    }
                    pillar_shared::PendingCommand::Recover { reason } => {
                        tracing::info!(reason = %reason, "recover command from controller");
                        self.force_recovery().await;
                    }
                    pillar_shared::PendingCommand::Stop { reason } => {
                        tracing::info!(reason = %reason, "stop command from controller");
                        match self.service_manager.stop().await {
                            Ok(()) => {
                                self.emit_event(EventKind::ServiceStopped {
                                    reason: format!("controller: {reason}"),
                                });
                                self.on_state_transition(
                                    self.current_state,
                                    NodeState::Off,
                                );
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "stop command failed");
                            }
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "failed to read pending command");
            }
        }

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

        // 3. Write state for Link to read
        self.publish_state().await;

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

        // Don't try to recover a service that was never installed
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

    /// Force recovery without crash-loop detection. Used for explicit controller requests.
    async fn force_recovery(&mut self) {
        if self.upgrading {
            tracing::info!("upgrade/provision in progress, skipping forced recovery");
            return;
        }

        let reference_slot = self.last_health.slot_info.reference_slot.unwrap_or(0);

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

        let _ = reference_slot; // suppress unused binding warning
    }

    async fn publish_state(&self) {
        let status = state::NodeStatus {
            state: self.current_state.as_str().to_string(),
            local_slot: self.last_health.slot_info.local_slot.unwrap_or(0) as i64,
            reference_slot: self.last_health.slot_info.reference_slot.unwrap_or(0) as i64,
            slots_behind: self.last_health.slots_behind.unwrap_or(0),
            healthy: self.current_state == NodeState::Healthy,
            restart_count: self.restarts_in_window() as u64,
            crash_looping: self.restarts_in_window() >= self.config.lifecycle.crash_threshold,
            health_check_duration_secs: self.last_check_duration_secs,
            version: env!("CARGO_PKG_VERSION").to_string(),
            role: self.config.role.to_string(),
            client: self.config.client.to_string(),
            cluster: self.config.network.cluster.clone(),
            updated_at_unix_secs: chrono::Utc::now().timestamp(),
            state_duration_secs: self.state_entered_at.elapsed().as_secs(),
            validator_process: self.validator_process.clone(),
            pending_upgrade: if self.upgrading {
                "in-progress".to_string()
            } else if self.version_mismatch {
                format!(
                    "version_mismatch: local={}, cluster={}",
                    self.local_validator_version.as_deref().unwrap_or("unknown"),
                    self.cluster_version.as_deref().unwrap_or("unknown")
                )
            } else {
                String::new()
            },
            // System metrics left as 0 — enriched by Link
            ..Default::default()
        };

        if let Err(e) = state::write_state(&status, &self.state_path) {
            tracing::warn!(error = %e, "failed to write operator state");
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
/// E.g. `"agave-validator 2.1.21 (src:8a085eeb; ...)"` → `"2.1.21"`.
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

/// Extract a semver-like version from `--version` output.
/// Looks for the first token matching `X.Y.Z` (digits separated by dots).
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

/// Extract the major version number from a version string.
/// `"3.1.8"` → `Some(3)`, `"2.1.21"` → `Some(2)`, `""` → `None`.
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
