use std::fmt::Write;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use pillar_shared::proto::NodeStatus;

use crate::node_registry::NodeRegistry;

/// Metric definitions: (name, help text).
const METRIC_HEADERS: &[(&str, &str)] = &[
    ("pillar_node_healthy", "Whether the node is healthy (1) or not (0)"),
    ("pillar_node_slots_behind", "Number of slots behind the reference"),
    ("pillar_node_local_slot", "Current local slot of the node"),
    ("pillar_node_reference_slot", "Reference slot from the network"),
    ("pillar_node_restarts_total", "Total number of validator restarts"),
    ("pillar_node_crash_looping", "Whether the node is in a crash loop (1) or not (0)"),
    ("pillar_system_cpu_usage_percent", "System CPU usage percentage"),
    ("pillar_system_memory_used_bytes", "System memory used in bytes"),
    ("pillar_system_memory_total_bytes", "System total memory in bytes"),
    ("pillar_system_disk_used_bytes", "Disk used in bytes"),
    ("pillar_system_disk_total_bytes", "Disk total in bytes"),
    ("pillar_system_network_rx_bytes", "Network bytes received"),
    ("pillar_system_network_tx_bytes", "Network bytes transmitted"),
    ("pillar_process_cpu_percent", "Per-process CPU usage percentage"),
    ("pillar_process_memory_bytes", "Per-process memory usage in bytes"),
    // Operator self-health
    ("pillar_operator_reconcile_count", "Total operator reconciliation ticks"),
    ("pillar_operator_health_check_errors", "Cumulative health check failures"),
    ("pillar_operator_consecutive_off_count", "Current consecutive Off debounce counter"),
    ("pillar_operator_recovery_count", "Snapshot recoveries attempted"),
    ("pillar_operator_state_write_errors", "State file write failures (deprecated, always 0 in agent)"),
    ("pillar_operator_pending_cmd_errors", "Command errors (deprecated, always 0 in agent)"),
    ("pillar_operator_uptime_secs", "Seconds since operator started"),
    ("pillar_operator_version_mismatch", "Validator/cluster version mismatch (1/0)"),
    // Link self-health
    ("pillar_link_controller_connected", "Active gRPC connection to controller (1/0)"),
    ("pillar_link_controller_latency_ms", "Last ReportStatus round-trip in ms"),
    ("pillar_link_status_reports_sent", "Successful status report count"),
    ("pillar_link_status_reports_failed", "Failed status report count"),
    ("pillar_link_state_file_age_secs", "State file age in seconds (deprecated, always 0 in agent)"),
    ("pillar_link_state_read_errors", "State file read errors (deprecated, always 0 in agent)"),
    ("pillar_link_log_batches_dropped", "Log batches dropped on controller unreachable"),
    ("pillar_link_uptime_secs", "Seconds since link started"),
    ("pillar_link_commands_received", "Commands received via CommandStream"),
    // Process start times (unix epoch, stable per process lifetime)
    ("pillar_operator_started_at_unix_secs", "Operator process start time (unix epoch)"),
    ("pillar_link_started_at_unix_secs", "Link process start time (unix epoch)"),
];

fn emit_node_metrics(out: &mut String, node_id: &str, status: &NodeStatus) {
    let base = format!(
        "node_id=\"{}\",role=\"{}\",cluster=\"{}\"",
        node_id, status.role, status.cluster
    );

    // Node health metrics
    let node_metrics: &[(&str, f64)] = &[
        ("pillar_node_healthy", if status.healthy { 1.0 } else { 0.0 }),
        ("pillar_node_slots_behind", status.slots_behind as f64),
        ("pillar_node_local_slot", status.local_slot as f64),
        ("pillar_node_reference_slot", status.reference_slot as f64),
        ("pillar_node_restarts_total", status.restart_count as f64),
        ("pillar_node_crash_looping", if status.crash_looping { 1.0 } else { 0.0 }),
        ("pillar_system_cpu_usage_percent", status.cpu_usage_percent),
        ("pillar_system_memory_used_bytes", status.memory_used_bytes as f64),
        ("pillar_system_memory_total_bytes", status.memory_total_bytes as f64),
        ("pillar_system_disk_used_bytes", status.disk_used_bytes as f64),
        ("pillar_system_disk_total_bytes", status.disk_total_bytes as f64),
        ("pillar_system_network_rx_bytes", status.network_rx_bytes as f64),
        ("pillar_system_network_tx_bytes", status.network_tx_bytes as f64),
    ];

    // Operator self-health
    let operator_metrics: &[(&str, f64)] = &[
        ("pillar_operator_reconcile_count", status.operator_reconcile_count as f64),
        ("pillar_operator_health_check_errors", status.operator_health_check_errors as f64),
        ("pillar_operator_consecutive_off_count", status.operator_consecutive_off_count as f64),
        ("pillar_operator_recovery_count", status.operator_recovery_count as f64),
        ("pillar_operator_state_write_errors", status.operator_state_write_errors as f64),
        ("pillar_operator_pending_cmd_errors", status.operator_pending_cmd_errors as f64),
        ("pillar_operator_uptime_secs", status.operator_uptime_secs as f64),
        ("pillar_operator_version_mismatch", if status.operator_version_mismatch { 1.0 } else { 0.0 }),
    ];

    // Link self-health
    let link_metrics: &[(&str, f64)] = &[
        ("pillar_link_controller_connected", if status.link_controller_connected { 1.0 } else { 0.0 }),
        ("pillar_link_controller_latency_ms", status.link_controller_latency_ms as f64),
        ("pillar_link_status_reports_sent", status.link_status_reports_sent as f64),
        ("pillar_link_status_reports_failed", status.link_status_reports_failed as f64),
        ("pillar_link_state_file_age_secs", status.link_state_file_age_secs as f64),
        ("pillar_link_state_read_errors", status.link_state_read_errors as f64),
        ("pillar_link_log_batches_dropped", status.link_log_batches_dropped as f64),
        ("pillar_link_uptime_secs", status.link_uptime_secs as f64),
        ("pillar_link_commands_received", status.link_commands_received as f64),
    ];

    // Process start times
    let start_time_metrics: &[(&str, f64)] = &[
        ("pillar_operator_started_at_unix_secs", status.operator_started_at_unix_secs as f64),
        ("pillar_link_started_at_unix_secs", status.link_started_at_unix_secs as f64),
    ];

    for (name, value) in node_metrics
        .iter()
        .chain(operator_metrics.iter())
        .chain(link_metrics.iter())
        .chain(start_time_metrics.iter())
    {
        let _ = writeln!(out, "{name}{{{base}}} {value}");
    }

    // Per-process metrics
    let processes: &[(&str, f64, u64)] = &[
        ("validator", status.validator_cpu_percent, status.validator_memory_bytes),
        ("operator", status.operator_cpu_percent, status.operator_memory_bytes),
        ("link", status.link_cpu_percent, status.link_memory_bytes),
    ];

    for (process, cpu, mem) in processes {
        let labels = format!("{base},process=\"{process}\"");
        let _ = writeln!(out, "pillar_process_cpu_percent{{{labels}}} {cpu}");
        let _ = writeln!(out, "pillar_process_memory_bytes{{{labels}}} {mem}");
    }
}

/// Build Prometheus text format from all node statuses in the registry.
pub async fn gather_metrics(registry: &NodeRegistry) -> String {
    let statuses = registry.get_all_statuses().await;
    let mut out = String::new();

    if statuses.is_empty() {
        return out;
    }

    for (name, help) in METRIC_HEADERS {
        let _ = writeln!(out, "# HELP {name} {help}");
        let _ = writeln!(out, "# TYPE {name} gauge");
    }
    out.push('\n');

    for (node_id, status) in &statuses {
        emit_node_metrics(&mut out, node_id, status);
    }

    out
}

/// Axum handler that serves Prometheus metrics.
pub async fn metrics_handler(
    State(state): State<crate::api::ApiState>,
) -> impl IntoResponse {
    let body = gather_metrics(&state.registry).await;
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_status() -> NodeStatus {
        NodeStatus {
            state: "healthy".to_string(),
            healthy: true,
            local_slot: 1000,
            reference_slot: 1005,
            slots_behind: 5,
            restart_count: 2,
            crash_looping: false,
            cpu_usage_percent: 45.5,
            memory_used_bytes: 1024 * 1024 * 100,
            memory_total_bytes: 1024 * 1024 * 256,
            disk_used_bytes: 500_000_000,
            disk_total_bytes: 1_000_000_000,
            network_rx_bytes: 12345,
            network_tx_bytes: 67890,
            validator_cpu_percent: 30.0,
            validator_memory_bytes: 50_000_000,
            operator_cpu_percent: 1.5,
            operator_memory_bytes: 20_000_000,
            link_cpu_percent: 0.5,
            link_memory_bytes: 10_000_000,
            role: "rpc".to_string(),
            cluster: "mainnet".to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn empty_registry_produces_empty_output() {
        let reg = NodeRegistry::new();
        let output = gather_metrics(&reg).await;
        assert!(output.is_empty());
    }

    #[tokio::test]
    async fn metrics_contain_node_labels() {
        let reg = NodeRegistry::new();
        reg.register_node("node-1").await;
        reg.update_status("node-1", sample_status()).await;

        let output = gather_metrics(&reg).await;
        assert!(output.contains("node_id=\"node-1\""));
        assert!(output.contains("role=\"rpc\""));
        assert!(output.contains("cluster=\"mainnet\""));
    }

    #[tokio::test]
    async fn metrics_contain_expected_gauges() {
        let reg = NodeRegistry::new();
        reg.register_node("n1").await;
        reg.update_status("n1", sample_status()).await;

        let output = gather_metrics(&reg).await;
        assert!(output.contains("pillar_node_healthy{"));
        assert!(output.contains("pillar_node_slots_behind{"));
        assert!(output.contains("pillar_node_local_slot{"));
        assert!(output.contains("pillar_system_cpu_usage_percent{"));
        assert!(output.contains("pillar_process_cpu_percent{"));
        assert!(output.contains("process=\"validator\""));
        assert!(output.contains("process=\"operator\""));
        assert!(output.contains("process=\"link\""));
    }

    #[tokio::test]
    async fn metrics_contain_help_and_type() {
        let reg = NodeRegistry::new();
        reg.register_node("n1").await;
        reg.update_status("n1", sample_status()).await;

        let output = gather_metrics(&reg).await;
        assert!(output.contains("# HELP pillar_node_healthy"));
        assert!(output.contains("# TYPE pillar_node_healthy gauge"));
    }

    #[tokio::test]
    async fn healthy_node_emits_1() {
        let reg = NodeRegistry::new();
        reg.register_node("n1").await;
        reg.update_status("n1", sample_status()).await;

        let output = gather_metrics(&reg).await;
        assert!(output.contains("pillar_node_healthy{node_id=\"n1\",role=\"rpc\",cluster=\"mainnet\"} 1"));
    }

    #[tokio::test]
    async fn unhealthy_node_emits_0() {
        let reg = NodeRegistry::new();
        reg.register_node("n1").await;
        let mut status = sample_status();
        status.healthy = false;
        reg.update_status("n1", status).await;

        let output = gather_metrics(&reg).await;
        assert!(output.contains("pillar_node_healthy{node_id=\"n1\",role=\"rpc\",cluster=\"mainnet\"} 0"));
    }
}
