use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::Connection;

use super::db::{insert_rule_if_absent, AlertRuleRow};

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[allow(clippy::too_many_arguments)]
fn rule(id: &str, name: &str, desc: &str, field: &str, op: &str,
        threshold: &str, severity: &str, enabled: bool, cooldown: i64, now: i64) -> AlertRuleRow {
    AlertRuleRow {
        id: id.into(), name: name.into(), description: desc.into(),
        field: field.into(), operator: op.into(), threshold: threshold.into(),
        node_id_filter: None, enabled, severity: severity.into(),
        cooldown_secs: cooldown, is_default: true, created_at: now, updated_at: now,
    }
}

pub fn seed(conn: &Connection) -> Result<()> {
    let now = now_secs();
    let rules = [
        rule("node_offline", "Node Offline", "Node state is off",
             "state", "eq", "off", "critical", true, 0, now),
        rule("node_crash_looping", "Crash Loop Detected", "crash_looping flag is true",
             "crash_looping", "eq", "true", "critical", true, 0, now),
        rule("node_behind", "Node Falling Behind", "slots_behind exceeds threshold",
             "slots_behind", "gt", "100", "warning", true, 0, now),
        rule("node_recovering", "Node Recovering", "Node is in recovering state",
             "state", "eq", "recovering", "warning", true, 0, now),
        rule("cpu_high", "High CPU Usage", "CPU usage exceeds threshold",
             "cpu_usage_percent", "gt", "90", "warning", false, 0, now),
        rule("memory_high", "High Memory Usage", "Memory usage exceeds threshold",
             "memory_percent", "gt", "90", "warning", false, 0, now),
        rule("disk_high", "High Disk Usage", "Disk usage exceeds threshold",
             "disk_percent", "gt", "85", "warning", false, 0, now),
        rule("version_mismatch", "Version Mismatch", "Agent detects version mismatch",
             "version_mismatch", "eq", "true", "info", false, 0, now),
        rule("agent_restart", "Agent Restarted", "Agent uptime < 60s",
             "agent_uptime_secs", "lt", "60", "info", true, 120, now),
    ];
    for r in &rules {
        insert_rule_if_absent(conn, r)?;
    }
    tracing::info!(count = rules.len(), "seeded default alert rules");
    Ok(())
}
