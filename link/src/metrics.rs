use pillar_shared::proto::NodeStatus;
use prometheus::{
    Encoder, Gauge, GaugeVec, IntCounter, IntGauge, Opts, Registry, TextEncoder,
};

/// All possible node states, used to set the GaugeVec.
const NODE_STATES: &[&str] = &["off", "starting_up", "behind", "healthy", "recovering"];

fn register_gauge(registry: &Registry, name: &str, help: &str) -> Gauge {
    let g = Gauge::new(name, help).expect("metric definition");
    registry.register(Box::new(g.clone())).expect("register metric");
    g
}

fn register_int_gauge(registry: &Registry, name: &str, help: &str) -> IntGauge {
    let g = IntGauge::new(name, help).expect("metric definition");
    registry.register(Box::new(g.clone())).expect("register metric");
    g
}

fn register_int_counter(registry: &Registry, name: &str, help: &str) -> IntCounter {
    let c = IntCounter::new(name, help).expect("metric definition");
    registry.register(Box::new(c.clone())).expect("register metric");
    c
}

fn register_gauge_vec(registry: &Registry, name: &str, help: &str, labels: &[&str]) -> GaugeVec {
    let g = GaugeVec::new(Opts::new(name, help), labels).expect("metric definition");
    registry.register(Box::new(g.clone())).expect("register metric");
    g
}

/// All Prometheus metrics exposed by Link.
pub struct Metrics {
    registry: Registry,

    // Operator state metrics
    node_state: GaugeVec,
    node_slots_behind: IntGauge,
    node_local_slot: IntGauge,
    node_reference_slot: IntGauge,
    node_healthy: IntGauge,
    node_restarts_total: IntGauge,
    node_crash_looping: IntGauge,
    health_check_duration_seconds: Gauge,
    node_info: GaugeVec,

    // System metrics (sysinfo)
    system_cpu_usage_percent: Gauge,
    system_memory_used_bytes: IntGauge,
    system_memory_total_bytes: IntGauge,
    system_network_rx_bytes_total: IntCounter,
    system_network_tx_bytes_total: IntCounter,
    system_disk_used_bytes: IntGauge,
    system_disk_total_bytes: IntGauge,

    // Process metrics — labeled by process: "validator", "operator", "link"
    process_cpu_percent: GaugeVec,
    process_memory_bytes: GaugeVec,

    // Link health metrics
    state_file_age_seconds: Gauge,
    state_read_errors_total: IntCounter,

    // Operator self-health (pass-through from proto)
    operator_reconcile_count: IntGauge,
    operator_health_check_errors: IntGauge,
    operator_consecutive_off_count: IntGauge,
    operator_recovery_count: IntGauge,
    operator_state_write_errors: IntGauge,
    operator_pending_cmd_errors: IntGauge,
    operator_uptime_secs: IntGauge,
    operator_version_mismatch: IntGauge,

    // Link self-health (from enriched proto)
    link_controller_connected: IntGauge,
    link_controller_latency_ms: IntGauge,
    link_status_reports_sent: IntGauge,
    link_status_reports_failed: IntGauge,
    link_state_file_age_secs: IntGauge,
    link_state_read_errors_gauge: IntGauge,
    link_log_batches_dropped: IntGauge,
    link_uptime_secs: IntGauge,
    link_commands_received: IntGauge,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();

        Self {
            node_state: register_gauge_vec(&registry, "pillar_node_state", "Current node state (1.0 for active state)", &["state"]),
            node_slots_behind: register_int_gauge(&registry, "pillar_node_slots_behind", "Slots behind reference"),
            node_local_slot: register_int_gauge(&registry, "pillar_node_local_slot", "Local slot height"),
            node_reference_slot: register_int_gauge(&registry, "pillar_node_reference_slot", "Reference slot height"),
            node_healthy: register_int_gauge(&registry, "pillar_node_healthy", "Whether the node is healthy (1/0)"),
            node_restarts_total: register_int_gauge(&registry, "pillar_node_restarts_total", "Total restart count"),
            node_crash_looping: register_int_gauge(&registry, "pillar_node_crash_looping", "Whether crash loop is detected (1/0)"),
            health_check_duration_seconds: register_gauge(&registry, "pillar_health_check_duration_seconds", "Last health check duration"),
            node_info: register_gauge_vec(&registry, "pillar_node_info", "Node metadata", &["role", "client", "cluster", "version"]),
            system_cpu_usage_percent: register_gauge(&registry, "pillar_system_cpu_usage_percent", "CPU usage percentage"),
            system_memory_used_bytes: register_int_gauge(&registry, "pillar_system_memory_used_bytes", "Used memory in bytes"),
            system_memory_total_bytes: register_int_gauge(&registry, "pillar_system_memory_total_bytes", "Total memory in bytes"),
            system_network_rx_bytes_total: register_int_counter(&registry, "pillar_system_network_rx_bytes_total", "Total network bytes received"),
            system_network_tx_bytes_total: register_int_counter(&registry, "pillar_system_network_tx_bytes_total", "Total network bytes transmitted"),
            system_disk_used_bytes: register_int_gauge(&registry, "pillar_system_disk_used_bytes", "Used disk space in bytes"),
            system_disk_total_bytes: register_int_gauge(&registry, "pillar_system_disk_total_bytes", "Total disk space in bytes"),
            process_cpu_percent: register_gauge_vec(&registry, "pillar_process_cpu_percent", "Process CPU usage percentage", &["process"]),
            process_memory_bytes: register_gauge_vec(&registry, "pillar_process_memory_bytes", "Process RSS memory in bytes", &["process"]),
            state_file_age_seconds: register_gauge(&registry, "pillar_link_state_file_age_seconds", "Age of the operator state file in seconds"),
            state_read_errors_total: register_int_counter(&registry, "pillar_link_state_read_errors_total", "Total state file read errors"),

            // Operator self-health
            operator_reconcile_count: register_int_gauge(&registry, "pillar_operator_reconcile_count", "Total operator reconciliation ticks"),
            operator_health_check_errors: register_int_gauge(&registry, "pillar_operator_health_check_errors", "Cumulative health check failures"),
            operator_consecutive_off_count: register_int_gauge(&registry, "pillar_operator_consecutive_off_count", "Current consecutive Off debounce counter"),
            operator_recovery_count: register_int_gauge(&registry, "pillar_operator_recovery_count", "Snapshot recoveries attempted"),
            operator_state_write_errors: register_int_gauge(&registry, "pillar_operator_state_write_errors", "State file write failures"),
            operator_pending_cmd_errors: register_int_gauge(&registry, "pillar_operator_pending_cmd_errors", "Pending command read/parse failures"),
            operator_uptime_secs: register_int_gauge(&registry, "pillar_operator_uptime_secs", "Seconds since operator started"),
            operator_version_mismatch: register_int_gauge(&registry, "pillar_operator_version_mismatch", "Validator/cluster version mismatch (1/0)"),

            // Link self-health
            link_controller_connected: register_int_gauge(&registry, "pillar_link_controller_connected", "Active gRPC connection to controller (1/0)"),
            link_controller_latency_ms: register_int_gauge(&registry, "pillar_link_controller_latency_ms", "Last ReportStatus round-trip in ms"),
            link_status_reports_sent: register_int_gauge(&registry, "pillar_link_status_reports_sent", "Successful status report count"),
            link_status_reports_failed: register_int_gauge(&registry, "pillar_link_status_reports_failed", "Failed status report count"),
            link_state_file_age_secs: register_int_gauge(&registry, "pillar_link_state_file_age_secs", "Operator state file age in seconds"),
            link_state_read_errors_gauge: register_int_gauge(&registry, "pillar_link_state_read_errors", "State file read errors (fleet-wide)"),
            link_log_batches_dropped: register_int_gauge(&registry, "pillar_link_log_batches_dropped", "Log batches dropped on controller unreachable"),
            link_uptime_secs: register_int_gauge(&registry, "pillar_link_uptime_secs", "Seconds since link started"),
            link_commands_received: register_int_gauge(&registry, "pillar_link_commands_received", "Commands received via CommandStream"),

            registry,
        }
    }

    /// Update Prometheus metrics from the enriched NodeStatus.
    pub fn update_from_state(&self, status: &NodeStatus) {
        // Node state gauge
        for &s in NODE_STATES {
            let val = if s == status.state { 1.0 } else { 0.0 };
            self.node_state.with_label_values(&[s]).set(val);
        }

        self.node_slots_behind.set(status.slots_behind);
        self.node_local_slot.set(status.local_slot);
        self.node_reference_slot.set(status.reference_slot);
        self.node_healthy.set(if status.healthy { 1 } else { 0 });
        self.node_restarts_total.set(status.restart_count as i64);
        self.node_crash_looping
            .set(if status.crash_looping { 1 } else { 0 });
        self.health_check_duration_seconds
            .set(status.health_check_duration_secs);

        self.node_info.reset();
        self.node_info
            .with_label_values(&[&status.role, &status.client, &status.cluster, &status.version])
            .set(1.0);

        // State file age from updated_at_unix_secs
        if status.updated_at_unix_secs > 0 {
            let age = chrono::Utc::now().timestamp() - status.updated_at_unix_secs;
            self.state_file_age_seconds.set(age.max(0) as f64);
        }

        // System metrics from enriched NodeStatus
        self.system_cpu_usage_percent
            .set(status.cpu_usage_percent);
        self.system_memory_used_bytes
            .set(status.memory_used_bytes as i64);
        self.system_memory_total_bytes
            .set(status.memory_total_bytes as i64);
        self.system_disk_used_bytes
            .set(status.disk_used_bytes as i64);
        self.system_disk_total_bytes
            .set(status.disk_total_bytes as i64);

        // Network counters: IntCounter only supports inc_by, so we track the delta.
        let rx = status.network_rx_bytes;
        let current_rx = self.system_network_rx_bytes_total.get();
        if rx > current_rx {
            self.system_network_rx_bytes_total.inc_by(rx - current_rx);
        }

        let tx = status.network_tx_bytes;
        let current_tx = self.system_network_tx_bytes_total.get();
        if tx > current_tx {
            self.system_network_tx_bytes_total.inc_by(tx - current_tx);
        }

        // Process metrics from enriched NodeStatus
        self.process_cpu_percent
            .with_label_values(&["validator"])
            .set(status.validator_cpu_percent);
        self.process_memory_bytes
            .with_label_values(&["validator"])
            .set(status.validator_memory_bytes as f64);

        self.process_cpu_percent
            .with_label_values(&["operator"])
            .set(status.operator_cpu_percent);
        self.process_memory_bytes
            .with_label_values(&["operator"])
            .set(status.operator_memory_bytes as f64);

        self.process_cpu_percent
            .with_label_values(&["link"])
            .set(status.link_cpu_percent);
        self.process_memory_bytes
            .with_label_values(&["link"])
            .set(status.link_memory_bytes as f64);

        // Operator self-health
        self.operator_reconcile_count
            .set(status.operator_reconcile_count as i64);
        self.operator_health_check_errors
            .set(status.operator_health_check_errors as i64);
        self.operator_consecutive_off_count
            .set(status.operator_consecutive_off_count as i64);
        self.operator_recovery_count
            .set(status.operator_recovery_count as i64);
        self.operator_state_write_errors
            .set(status.operator_state_write_errors as i64);
        self.operator_pending_cmd_errors
            .set(status.operator_pending_cmd_errors as i64);
        self.operator_uptime_secs
            .set(status.operator_uptime_secs as i64);
        self.operator_version_mismatch
            .set(if status.operator_version_mismatch { 1 } else { 0 });

        // Link self-health
        self.link_controller_connected
            .set(if status.link_controller_connected { 1 } else { 0 });
        self.link_controller_latency_ms
            .set(status.link_controller_latency_ms as i64);
        self.link_status_reports_sent
            .set(status.link_status_reports_sent as i64);
        self.link_status_reports_failed
            .set(status.link_status_reports_failed as i64);
        self.link_state_file_age_secs
            .set(status.link_state_file_age_secs as i64);
        self.link_state_read_errors_gauge
            .set(status.link_state_read_errors as i64);
        self.link_log_batches_dropped
            .set(status.link_log_batches_dropped as i64);
        self.link_uptime_secs.set(status.link_uptime_secs as i64);
        self.link_commands_received
            .set(status.link_commands_received as i64);
    }

    /// Increment state read error counter.
    pub fn inc_state_read_errors(&self) {
        self.state_read_errors_total.inc();
    }

    /// Gather all metrics and encode as Prometheus text format.
    pub fn gather(&self) -> String {
        let encoder = TextEncoder::new();
        let families = self.registry.gather();
        let mut buf = Vec::new();
        encoder.encode(&families, &mut buf).expect("encode metrics");
        String::from_utf8(buf).expect("utf8 metrics")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy_status() -> NodeStatus {
        NodeStatus {
            state: "healthy".to_string(),
            local_slot: 5000,
            reference_slot: 5010,
            slots_behind: 10,
            healthy: true,
            restart_count: 3,
            crash_looping: false,
            health_check_duration_secs: 0.25,
            version: "0.1.0".to_string(),
            role: "rpc".to_string(),
            client: "agave".to_string(),
            cluster: "mainnet".to_string(),
            updated_at_unix_secs: chrono::Utc::now().timestamp(),
            state_duration_secs: 60,
            validator_process: "agave-validator".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn update_from_state_sets_healthy() {
        let m = Metrics::new();
        m.update_from_state(&healthy_status());

        let output = m.gather();
        assert!(output.contains("pillar_node_healthy 1"));
        assert!(output.contains(r#"pillar_node_state{state="healthy"} 1"#));
        assert!(output.contains(r#"pillar_node_state{state="off"} 0"#));
    }

    #[test]
    fn update_from_state_sets_slots() {
        let m = Metrics::new();
        m.update_from_state(&healthy_status());

        let output = m.gather();
        assert!(output.contains("pillar_node_local_slot 5000"));
        assert!(output.contains("pillar_node_reference_slot 5010"));
        assert!(output.contains("pillar_node_slots_behind 10"));
    }

    #[test]
    fn update_from_state_sets_restarts() {
        let m = Metrics::new();
        m.update_from_state(&healthy_status());

        let output = m.gather();
        assert!(output.contains("pillar_node_restarts_total 3"));
        assert!(output.contains("pillar_node_crash_looping 0"));
    }

    #[test]
    fn update_from_state_crash_looping() {
        let m = Metrics::new();
        let mut status = healthy_status();
        status.crash_looping = true;
        status.healthy = false;
        status.state = "off".to_string();
        m.update_from_state(&status);

        let output = m.gather();
        assert!(output.contains("pillar_node_crash_looping 1"));
        assert!(output.contains("pillar_node_healthy 0"));
        assert!(output.contains(r#"pillar_node_state{state="off"} 1"#));
    }

    #[test]
    fn update_from_state_sets_node_info() {
        let m = Metrics::new();
        m.update_from_state(&healthy_status());

        let output = m.gather();
        assert!(output.contains(
            r#"pillar_node_info{client="agave",cluster="mainnet",role="rpc",version="0.1.0"} 1"#
        ));
    }

    #[test]
    fn update_from_state_sets_process_metrics() {
        let m = Metrics::new();
        let mut status = healthy_status();
        status.validator_cpu_percent = 75.5;
        status.validator_memory_bytes = 8_000_000_000;
        m.update_from_state(&status);

        let output = m.gather();
        assert!(output.contains(r#"pillar_process_cpu_percent{process="validator"}"#));
        assert!(output.contains(r#"pillar_process_memory_bytes{process="validator"}"#));
    }

    #[test]
    fn inc_state_read_errors() {
        let m = Metrics::new();
        m.inc_state_read_errors();
        m.inc_state_read_errors();

        let output = m.gather();
        assert!(output.contains("pillar_link_state_read_errors_total 2"));
    }
}
