use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::command::AgentCommand;
use crate::config::AgentConfig;
use crate::event::{EventKind, OperatorEvent};
use crate::health::{HealthChecker, NodeHealth, NodeState};
use crate::lifecycle::SystemdManager;
use crate::metrics_updater::SharedStatus;
use crate::script_executor::ScriptExecutor;
use crate::snapshot::recovery::SnapshotRecovery;
use crate::snapshot::TcpSnapshotManager;

use pillar_shared::proto::{NodeStatus, ScriptResult};

pub struct Reconciler {
    config: AgentConfig,
    health_checker: Box<dyn HealthChecker>,
    service_manager: SystemdManager,
    snapshot_manager: TcpSnapshotManager,
    ledger_dir: PathBuf,
    validator_process: String,
    shared_status: SharedStatus,
    cmd_rx: mpsc::Receiver<AgentCommand>,

    // Script execution
    script_executor: ScriptExecutor,
    result_tx: mpsc::Sender<ScriptResult>,

    // Internal state
    current_state: NodeState,
    state_entered_at: Instant,
    started_at: Instant,
    restart_timestamps: VecDeque<Instant>,
    last_health: NodeHealth,
    last_check_duration_secs: f64,
    consecutive_off_count: usize,
    executing_script: bool,

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
        shared_status: SharedStatus,
        cmd_rx: mpsc::Receiver<AgentCommand>,
        result_tx: mpsc::Sender<ScriptResult>,
    ) -> Self {
        Self {
            config,
            health_checker,
            service_manager,
            snapshot_manager,
            ledger_dir,
            validator_process,
            shared_status,
            cmd_rx,
            script_executor: ScriptExecutor::new(),
            result_tx,
            current_state: NodeState::Off,
            state_entered_at: Instant::now(),
            started_at: Instant::now(),
            restart_timestamps: VecDeque::new(),
            last_health: NodeHealth::default(),
            last_check_duration_secs: 0.0,
            consecutive_off_count: 0,
            executing_script: false,
            started_at_unix_secs: chrono::Utc::now().timestamp(),
            reconcile_count: 0,
            health_check_error_count: 0,
            recovery_count: 0,
            local_validator_version: None,
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
        match cmd {
            AgentCommand::ExecuteScript(script) => {
                self.executing_script = true;
                let script_id = script.script_id.clone();
                let desc = script.description.clone();

                let result = self
                    .script_executor
                    .execute(script, &self.config.controller.node_id)
                    .await;
                self.executing_script = false;

                if result.exit_code == 0 {
                    tracing::info!(script_id, desc, "script succeeded");
                } else {
                    tracing::error!(
                        script_id,
                        desc,
                        exit_code = result.exit_code,
                        timed_out = result.timed_out,
                        "script failed"
                    );
                }

                let _ = self.result_tx.send(result).await;
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
        if self.executing_script {
            tracing::info!("script execution in progress, skipping recovery");
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
        status.pending_upgrade = if self.executing_script {
            "script-executing".to_string()
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
}
