//! Periodically refreshes sysinfo and enriches the shared NodeStatus with
//! system/process metrics and agent health counters.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use pillar_shared::proto::NodeStatus;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::agent_health::AgentHealth;
use crate::metrics::Metrics;
use crate::system_info::{SysPid, SystemInfo};

pub type SharedStatus = Arc<RwLock<Option<NodeStatus>>>;

/// Enrich a NodeStatus with system/process metrics and agent health.
fn enrich_status(
    status: &mut NodeStatus,
    sys: &SystemInfo,
    agent_pid: SysPid,
    agent_health: &AgentHealth,
) {
    // System metrics
    status.cpu_usage_percent = sys.cpu_usage_percent() as f64;
    status.memory_used_bytes = sys.memory_used_bytes();
    status.memory_total_bytes = sys.memory_total_bytes();
    status.disk_used_bytes = sys.disk_used_bytes();
    status.disk_total_bytes = sys.disk_total_bytes();
    status.network_rx_bytes = sys.network_rx_bytes();
    status.network_tx_bytes = sys.network_tx_bytes();

    // Process metrics: validator
    if !status.validator_process.is_empty() {
        if let Some(stats) = sys.find_process_by_name(&status.validator_process) {
            status.validator_cpu_percent = stats.cpu_usage_percent as f64;
            status.validator_memory_bytes = stats.memory_rss_bytes;
        }
    }

    // Process metrics: agent
    if let Some(stats) = sys.process_stats(agent_pid) {
        status.agent_cpu_percent = stats.cpu_usage_percent as f64;
        status.agent_memory_bytes = stats.memory_rss_bytes;
    }

    // Controller connectivity health
    status.controller_connected = agent_health.controller_connected.load(Ordering::Relaxed);
    status.controller_latency_ms = agent_health.controller_latency_ms.load(Ordering::Relaxed);
    status.status_reports_sent = agent_health.status_reports_sent.load(Ordering::Relaxed);
    status.status_reports_failed = agent_health.status_reports_failed.load(Ordering::Relaxed);
    status.log_batches_dropped = agent_health.log_batches_dropped.load(Ordering::Relaxed);
    status.commands_received = agent_health.commands_received.load(Ordering::Relaxed);
}

/// Run the metrics updater loop: refresh sysinfo, enrich shared status, update Prometheus.
pub async fn run(
    shared: SharedStatus,
    metrics: Arc<Metrics>,
    agent_health: Arc<AgentHealth>,
    interval: Duration,
    cancel: CancellationToken,
) {
    let mut sys = SystemInfo::new();
    let agent_pid = SysPid::from_u32(std::process::id());
    tracing::info!(interval_secs = interval.as_secs(), "metrics updater starting");

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("metrics updater shutting down");
                return;
            }
            _ = tokio::time::sleep(interval) => {
                // 1. Refresh sysinfo
                sys.refresh();
                sys.refresh_all_processes();

                // 2. Take write lock, enrich, update prometheus
                let mut guard = shared.write().await;
                if let Some(ref mut status) = *guard {
                    enrich_status(status, &sys, agent_pid, &agent_health);
                    metrics.update_from_state(status);
                }
            }
        }
    }
}
