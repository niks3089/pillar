use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use pillar_shared::proto::NodeStatus;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::link_health::LinkHealth;
use crate::metrics::Metrics;
use crate::system_info::{SysPid, SystemInfo};

pub type SharedState = Arc<RwLock<Option<NodeStatus>>>;

/// Enrich a NodeStatus with system/process metrics and link health.
fn enrich_status(
    status: &mut NodeStatus,
    sys: &SystemInfo,
    link_pid: SysPid,
    link_health: &LinkHealth,
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

    // Process metrics: operator
    if let Some(stats) = sys.find_process_by_name("operator") {
        status.operator_cpu_percent = stats.cpu_usage_percent as f64;
        status.operator_memory_bytes = stats.memory_rss_bytes;
    }

    // Process metrics: link (own PID)
    if let Some(stats) = sys.process_stats(link_pid) {
        status.link_cpu_percent = stats.cpu_usage_percent as f64;
        status.link_memory_bytes = stats.memory_rss_bytes;
    }

    // Link self-health
    status.link_controller_connected = link_health.controller_connected.load(Ordering::Relaxed);
    status.link_controller_latency_ms = link_health.controller_latency_ms.load(Ordering::Relaxed);
    status.link_status_reports_sent = link_health.status_reports_sent.load(Ordering::Relaxed);
    status.link_status_reports_failed = link_health.status_reports_failed.load(Ordering::Relaxed);
    status.link_state_read_errors = link_health.state_read_errors.load(Ordering::Relaxed);
    status.link_log_batches_dropped = link_health.log_batches_dropped.load(Ordering::Relaxed);
    status.link_uptime_secs = link_health.started_at.elapsed().as_secs();
    status.link_commands_received = link_health.commands_received.load(Ordering::Relaxed);
    status.link_started_at_unix_secs = link_health.started_at_unix_secs;

    // State file age
    if status.updated_at_unix_secs > 0 {
        let age = (chrono::Utc::now().timestamp() - status.updated_at_unix_secs).max(0) as u64;
        status.link_state_file_age_secs = age;
        link_health.set_state_file_age_secs(age);
    }
}

/// Poll the operator state file, enrich with system metrics, update Prometheus, and store in shared state.
pub async fn run_state_reader(
    path: PathBuf,
    shared: SharedState,
    metrics: Arc<Metrics>,
    link_health: Arc<LinkHealth>,
    interval: Duration,
    cancel: CancellationToken,
) {
    let mut sys = SystemInfo::new();
    let link_pid = SysPid::from_u32(std::process::id());
    tracing::info!(path = %path.display(), interval_secs = interval.as_secs(), "state reader starting");

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("state reader shutting down");
                return;
            }
            _ = tokio::time::sleep(interval) => {
                // 1. Read state file
                match pillar_shared::read_state(&path) {
                    Ok(Some(mut status)) => {
                        // 2. Refresh sysinfo
                        sys.refresh();
                        sys.refresh_all_processes();

                        // 3. Enrich with system/process metrics + link health
                        enrich_status(&mut status, &sys, link_pid, &link_health);

                        // 4. Update Prometheus
                        metrics.update_from_state(&status);

                        // 5. Store in shared state
                        *shared.write().await = Some(status);
                    }
                    Ok(None) => {
                        tracing::debug!(path = %path.display(), "state file not found yet");
                        *shared.write().await = None;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, path = %path.display(), "failed to read state file");
                        link_health.inc_state_read_errors();
                        metrics.inc_state_read_errors();
                    }
                }
            }
        }
    }
}
