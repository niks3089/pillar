use std::collections::VecDeque;
use std::path::PathBuf;
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
}

impl Operator {
    pub fn new(
        config: OperatorConfig,
        health_checker: Box<dyn HealthChecker>,
        service_manager: SystemdManager,
        snapshot_manager: TcpSnapshotManager,
        ledger_dir: PathBuf,
        state_path: PathBuf,
        validator_process: String,
    ) -> Self {
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
                    shared::PendingCommand::Provision {
                        staged_binary_path,
                        ref provision,
                    } => {
                        self.upgrading = true;
                        let staged = PathBuf::from(&staged_binary_path);
                        let result = crate::provisioner::provision(provision, &staged).await;
                        self.upgrading = false;
                        match result {
                            Ok(()) => {
                                tracing::info!(command_type, "provision completed successfully");
                                self.on_state_transition(
                                    self.current_state,
                                    NodeState::StartingUp,
                                );
                            }
                            Err(e) => {
                                tracing::error!(command_type, error = %e, "provision failed");
                            }
                        }
                    }
                    shared::PendingCommand::Upgrade {
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
                    shared::PendingCommand::Restart { reason } => {
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
                    shared::PendingCommand::Recover { reason } => {
                        tracing::info!(reason = %reason, "recover command from controller");
                        self.force_recovery().await;
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
