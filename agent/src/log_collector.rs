//! Tails journald for configured systemd units and streams log batches
//! to the controller via PushLogs gRPC.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agent_health::AgentHealth;
use crate::config::{ControllerConfig, LogCollectorConfig};
use crate::metrics_updater::SharedStatus;

pub mod proto {
    tonic::include_proto!("pillar");
}

use pillar_shared::proto::{LogBatch, LogEntry};

/// Parsed snapshot download progress from validator logs.
#[derive(Debug, Clone)]
struct SnapshotProgress {
    downloaded_bytes: u64,
    total_bytes: u64,
    speed_bps: f64,
}

/// Parse snapshot download progress from a validator log message.
///
/// Matches lines like:
///   "Downloading 52428800000 bytes from ..."  → sets total, resets downloaded to 0
///   "downloaded 548684968 bytes 10.4% 13474726.0 bytes/s" → incremental progress
///   "Downloaded 52428800000 bytes in 3845s"  → download complete
fn detect_snapshot_progress(message: &str) -> Option<SnapshotProgress> {
    // "Downloading <total> bytes from ..." — start of a new download
    if let Some(rest) = message.strip_prefix("Downloading ") {
        let total: u64 = rest.split_whitespace().next()?.parse().ok()?;
        return Some(SnapshotProgress {
            downloaded_bytes: 0,
            total_bytes: total,
            speed_bps: 0.0,
        });
    }

    // "downloaded <bytes> bytes <percent>% <speed> bytes/s" — incremental progress
    if message.starts_with("downloaded ") {
        let parts: Vec<&str> = message.split_whitespace().collect();
        // downloaded <bytes> bytes <pct>% <speed> bytes/s
        if parts.len() >= 6 && parts[2] == "bytes" && parts[5] == "bytes/s" {
            let downloaded: u64 = parts[1].parse().ok()?;
            let pct: f64 = parts[3].trim_end_matches('%').parse().ok()?;
            let speed: f64 = parts[4].parse().ok()?;
            // Reconstruct total from percentage
            let total = if pct > 0.0 {
                (downloaded as f64 / (pct / 100.0)) as u64
            } else {
                0
            };
            return Some(SnapshotProgress {
                downloaded_bytes: downloaded,
                total_bytes: total,
                speed_bps: speed,
            });
        }
    }

    // "Downloaded <total> bytes in <duration>" — download finished
    if message.starts_with("Downloaded ") && message.contains(" bytes in ") {
        let parts: Vec<&str> = message.split_whitespace().collect();
        if parts.len() >= 2 {
            let total: u64 = parts[1].parse().ok()?;
            return Some(SnapshotProgress {
                downloaded_bytes: total,
                total_bytes: total,
                speed_bps: 0.0,
            });
        }
    }

    None
}

/// Returns true if the message is a bootstrap/download related line that should
/// bypass the min_level filter for validator logs.
fn is_bootstrap_message(message: &str) -> bool {
    // Check for snapshot download progress
    if message.starts_with("Downloading ")
        || message.starts_with("downloaded ")
        || (message.starts_with("Downloaded ") && message.contains(" bytes"))
    {
        return true;
    }
    // Bootstrap peer discovery, RPC search
    let lower = message.to_lowercase();
    if lower.contains("bootstrap") {
        return true;
    }
    false
}

/// Update shared status with snapshot download progress. Zeros out fields
/// when download is complete (downloaded == total and both > 0).
async fn update_snapshot_progress(shared: &SharedStatus, progress: &SnapshotProgress) {
    let mut guard = shared.write().await;
    if let Some(ref mut status) = *guard {
        if progress.downloaded_bytes == progress.total_bytes && progress.total_bytes > 0 {
            // Download finished — clear fields
            status.snapshot_download_bytes = 0;
            status.snapshot_download_total_bytes = 0;
            status.snapshot_download_speed_bps = 0.0;
        } else {
            status.snapshot_download_bytes = progress.downloaded_bytes;
            status.snapshot_download_total_bytes = progress.total_bytes;
            status.snapshot_download_speed_bps = progress.speed_bps;
        }
    }
}

/// Map journald priority (0-7) to a level string.
fn priority_to_level(priority: &str) -> &'static str {
    match priority {
        "0" | "1" | "2" | "3" => "error",
        "4" => "warn",
        "5" | "6" => "info",
        "7" => "debug",
        _ => "info",
    }
}

/// Extract the log level from a tracing-formatted message.
fn detect_level_from_message(message: &str) -> Option<&'static str> {
    let mut tokens = message.split_whitespace();
    if let Some(first) = tokens.next() {
        if first.len() > 10 && first.as_bytes()[0].is_ascii_digit() && first.contains('T') {
            if let Some(second) = tokens.next() {
                match second {
                    "ERROR" => return Some("error"),
                    "WARN" => return Some("warn"),
                    "INFO" => return Some("info"),
                    "DEBUG" | "TRACE" => return Some("debug"),
                    _ => {}
                }
            }
        }
    }

    if message.contains(" ERROR ") || message.starts_with("ERROR ") {
        return Some("error");
    }
    if message.contains(" WARN ") || message.starts_with("WARN ") {
        return Some("warn");
    }

    None
}

/// Map a level string to a numeric rank for filtering (higher = more severe).
fn level_rank(level: &str) -> u8 {
    match level {
        "error" => 4,
        "warn" => 3,
        "info" => 2,
        "debug" => 1,
        _ => 0,
    }
}

/// Returns true if `entry_level` meets the `min_level` threshold.
pub fn level_passes(entry_level: &str, min_level: &str) -> bool {
    level_rank(entry_level) >= level_rank(min_level)
}

/// Derive a service name from a systemd unit name.
fn unit_to_service(unit: &str) -> &str {
    if unit.contains("validator") {
        "validator"
    } else if unit.contains("agent") || unit.contains("operator") || unit.contains("link") {
        "agent"
    } else if unit.contains("controller") {
        "controller"
    } else {
        unit.strip_suffix(".service").unwrap_or(unit)
    }
}

/// Extract MESSAGE from journald JSON.
fn extract_message(obj: &serde_json::Value) -> Option<String> {
    let msg_val = obj.get("MESSAGE")?;
    match msg_val {
        serde_json::Value::String(s) => {
            if s.is_empty() {
                None
            } else {
                Some(s.clone())
            }
        }
        serde_json::Value::Array(arr) => {
            let bytes: Vec<u8> = arr.iter().filter_map(|v| v.as_u64().map(|n| n as u8)).collect();
            let s = String::from_utf8_lossy(&bytes).to_string();
            let stripped = strip_ansi(&s);
            if stripped.is_empty() {
                None
            } else {
                Some(stripped)
            }
        }
        _ => None,
    }
}

/// Strip ANSI escape sequences from a string.
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Parse a journald JSON line into a LogEntry.
fn parse_journal_line(line: &str) -> Option<LogEntry> {
    let obj: serde_json::Value = serde_json::from_str(line).ok()?;

    let message = extract_message(&obj)?;
    if message.is_empty() {
        return None;
    }

    let level = detect_level_from_message(&message)
        .unwrap_or_else(|| {
            let priority = obj
                .get("PRIORITY")
                .and_then(|v| v.as_str())
                .unwrap_or("6");
            priority_to_level(priority)
        })
        .to_string();

    let timestamp_us: i64 = obj
        .get("__REALTIME_TIMESTAMP")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let timestamp_unix_ms = timestamp_us / 1000;

    let unit = obj
        .get("_SYSTEMD_UNIT")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let service = unit_to_service(&unit).to_string();

    Some(LogEntry {
        service,
        timestamp_unix_ms,
        level,
        message,
        unit,
    })
}

/// Spawn a journalctl tail for a single unit.
async fn tail_unit(unit: String, tx: mpsc::Sender<LogEntry>, cancel: CancellationToken) {
    let mut child = match Command::new("journalctl")
        .args(["-f", "-u", &unit, "--output=json", "-n", "0"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(unit = %unit, error = %e, "failed to spawn journalctl");
            return;
        }
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return,
    };

    let mut reader = BufReader::new(stdout).lines();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            line = reader.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if let Some(entry) = parse_journal_line(&line) {
                            if tx.send(entry).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(None) => {
                        tracing::info!(unit = %unit, "journalctl stream ended");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(unit = %unit, error = %e, "journalctl read error");
                        break;
                    }
                }
            }
        }
    }

    let _ = child.kill().await;
}

/// Run the log collector: tail configured units, buffer entries, flush via gRPC.
pub async fn run(
    config: LogCollectorConfig,
    controller: ControllerConfig,
    agent_health: Arc<AgentHealth>,
    shared_status: SharedStatus,
    cancel: CancellationToken,
) {
    tracing::info!(
        units = ?config.units,
        buffer_size = config.buffer_size,
        flush_interval_ms = config.flush_interval_ms,
        "log collector starting"
    );

    let (tx, mut rx) = mpsc::channel::<LogEntry>(config.buffer_size * 2);

    for unit in &config.units {
        let tx = tx.clone();
        let unit = unit.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tail_unit(unit, tx, cancel).await;
        });
    }
    drop(tx);

    let node_id = controller.node_id.clone();
    let flush_interval = Duration::from_millis(config.flush_interval_ms);
    let buffer_size = config.buffer_size;
    let validator_min_level = config.validator_min_level.clone();
    let default_min_level = config.default_min_level.clone();

    let mut buffer: Vec<LogEntry> = Vec::with_capacity(buffer_size);
    let mut flush_timer = tokio::time::interval(flush_interval);
    flush_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut client: Option<crate::grpc::LogClient> = None;
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                if !buffer.is_empty() {
                    flush_batch(&mut client, &controller, &node_id, &mut buffer, &mut backoff, max_backoff, &agent_health).await;
                }
                break;
            }
            entry = rx.recv() => {
                match entry {
                    Some(e) => {
                        // Check for snapshot download progress before filtering.
                        let is_bootstrap = e.service == "validator" && is_bootstrap_message(&e.message);
                        if e.service == "validator" {
                            if let Some(progress) = detect_snapshot_progress(&e.message) {
                                update_snapshot_progress(&shared_status, &progress).await;
                            }
                        }

                        // Filter by min level: validator defaults to warn, others to debug.
                        // Bootstrap/download messages bypass the filter.
                        if !is_bootstrap {
                            let min = if e.service == "validator" {
                                &validator_min_level
                            } else {
                                &default_min_level
                            };
                            if !level_passes(&e.level, min) {
                                continue;
                            }
                        }
                        buffer.push(e);
                        if buffer.len() >= buffer_size {
                            flush_batch(&mut client, &controller, &node_id, &mut buffer, &mut backoff, max_backoff, &agent_health).await;
                            flush_timer.reset();
                        }
                    }
                    None => {
                        if !buffer.is_empty() {
                            flush_batch(&mut client, &controller, &node_id, &mut buffer, &mut backoff, max_backoff, &agent_health).await;
                        }
                        break;
                    }
                }
            }
            _ = flush_timer.tick() => {
                if !buffer.is_empty() {
                    flush_batch(&mut client, &controller, &node_id, &mut buffer, &mut backoff, max_backoff, &agent_health).await;
                }
            }
        }
    }

    tracing::info!("log collector stopped");
}

async fn flush_batch(
    client: &mut Option<crate::grpc::LogClient>,
    controller_config: &ControllerConfig,
    node_id: &str,
    buffer: &mut Vec<LogEntry>,
    backoff: &mut Duration,
    max_backoff: Duration,
    agent_health: &AgentHealth,
) {
    let entries = std::mem::take(buffer);
    let count = entries.len();

    if client.is_none() {
        match crate::grpc::build_channel(controller_config).await {
            Ok(channel) => {
                *client = Some(crate::grpc::make_log_client(channel, &controller_config.auth_token));
                *backoff = Duration::from_secs(1);
            }
            Err(e) => {
                agent_health.inc_log_batches_dropped();
                tracing::debug!(error = %e, "log collector: failed to connect, dropping {count} entries");
                *backoff = (*backoff * 2).min(max_backoff);
                return;
            }
        }
    }

    let batch = LogBatch {
        node_id: node_id.to_string(),
        entries,
    };

    let stream = tokio_stream::once(batch);
    let c = client.as_mut().unwrap();
    match c.push_logs(tonic::Request::new(stream)).await {
        Ok(resp) => {
            let ack = resp.into_inner();
            tracing::debug!(sent = count, acked = ack.received_count, "log batch flushed");
        }
        Err(e) => {
            agent_health.inc_log_batches_dropped();
            tracing::debug!(error = %e, "log collector: push_logs failed, dropping {count} entries");
            *client = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_journal_line() {
        let line = r#"{"MESSAGE":"hello world","PRIORITY":"6","__REALTIME_TIMESTAMP":"1700000000000000","_SYSTEMD_UNIT":"solana-validator.service"}"#;
        let entry = parse_journal_line(line).unwrap();
        assert_eq!(entry.service, "validator");
        assert_eq!(entry.level, "info");
        assert_eq!(entry.message, "hello world");
        assert_eq!(entry.timestamp_unix_ms, 1700000000000);
        assert_eq!(entry.unit, "solana-validator.service");
    }

    #[test]
    fn parse_error_priority() {
        let line = r#"{"MESSAGE":"crash","PRIORITY":"3","__REALTIME_TIMESTAMP":"1700000000000000","_SYSTEMD_UNIT":"pillar-agent.service"}"#;
        let entry = parse_journal_line(line).unwrap();
        assert_eq!(entry.level, "error");
        assert_eq!(entry.service, "agent");
    }

    #[test]
    fn parse_tracing_error_overrides_priority() {
        let line = r#"{"MESSAGE":"2026-02-08T19:10:04.044506Z ERROR ThreadId(01) pillar_agent::reconcile: recovery failed","PRIORITY":"6","__REALTIME_TIMESTAMP":"1700000000000000","_SYSTEMD_UNIT":"pillar-agent.service"}"#;
        let entry = parse_journal_line(line).unwrap();
        assert_eq!(entry.level, "error");
    }

    #[test]
    fn parse_empty_message_returns_none() {
        let line = r#"{"MESSAGE":"","PRIORITY":"6","__REALTIME_TIMESTAMP":"1700000000000000","_SYSTEMD_UNIT":"test.service"}"#;
        assert!(parse_journal_line(line).is_none());
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(parse_journal_line("not json").is_none());
    }

    #[test]
    fn parse_byte_array_message() {
        let line = r#"{"MESSAGE":[104,101,108,108,111],"PRIORITY":"6","__REALTIME_TIMESTAMP":"1700000000000000","_SYSTEMD_UNIT":"pillar-agent.service"}"#;
        let entry = parse_journal_line(line).unwrap();
        assert_eq!(entry.message, "hello");
    }

    #[test]
    fn strip_ansi_removes_sequences() {
        assert_eq!(strip_ansi("\x1b[2mhello\x1b[0m"), "hello");
        assert_eq!(strip_ansi("no escapes"), "no escapes");
    }

    #[test]
    fn priority_mapping() {
        assert_eq!(priority_to_level("0"), "error");
        assert_eq!(priority_to_level("3"), "error");
        assert_eq!(priority_to_level("4"), "warn");
        assert_eq!(priority_to_level("5"), "info");
        assert_eq!(priority_to_level("6"), "info");
        assert_eq!(priority_to_level("7"), "debug");
    }

    #[test]
    fn unit_to_service_mapping() {
        assert_eq!(unit_to_service("solana-validator.service"), "validator");
        assert_eq!(unit_to_service("pillar-agent.service"), "agent");
        assert_eq!(unit_to_service("controller.service"), "controller");
        assert_eq!(unit_to_service("custom.service"), "custom");
    }

    #[test]
    fn detect_download_start() {
        let msg = "Downloading 52428800000 bytes from 10.0.0.5:8899";
        let progress = detect_snapshot_progress(msg).unwrap();
        assert_eq!(progress.total_bytes, 52428800000);
        assert_eq!(progress.downloaded_bytes, 0);
        assert_eq!(progress.speed_bps, 0.0);
    }

    #[test]
    fn detect_download_progress() {
        let msg = "downloaded 548684968 bytes 10.4% 13474726.0 bytes/s";
        let progress = detect_snapshot_progress(msg).unwrap();
        assert_eq!(progress.downloaded_bytes, 548684968);
        assert!(progress.speed_bps > 13_000_000.0);
        assert!(progress.total_bytes > 5_000_000_000);
    }

    #[test]
    fn detect_download_complete() {
        let msg = "Downloaded 52428800000 bytes in 3845s";
        let progress = detect_snapshot_progress(msg).unwrap();
        assert_eq!(progress.downloaded_bytes, 52428800000);
        assert_eq!(progress.total_bytes, 52428800000);
        assert_eq!(progress.speed_bps, 0.0);
    }

    #[test]
    fn detect_no_progress_for_unrelated() {
        assert!(detect_snapshot_progress("validator started").is_none());
        assert!(detect_snapshot_progress("ERROR crash happened").is_none());
    }

    #[test]
    fn bootstrap_message_detection() {
        assert!(is_bootstrap_message("Downloading 52428800000 bytes from 10.0.0.5:8899"));
        assert!(is_bootstrap_message("downloaded 548684968 bytes 10.4% 13474726.0 bytes/s"));
        assert!(is_bootstrap_message("Downloaded 52428800000 bytes in 3845s"));
        assert!(is_bootstrap_message("Searching for an RPC service with bootstrap config"));
        assert!(!is_bootstrap_message("validator started successfully"));
    }

    #[test]
    fn level_filtering() {
        // validator defaults to warn — info/debug should be dropped
        assert!(level_passes("error", "warn"));
        assert!(level_passes("warn", "warn"));
        assert!(!level_passes("info", "warn"));
        assert!(!level_passes("debug", "warn"));

        // agent defaults to debug — everything passes
        assert!(level_passes("error", "debug"));
        assert!(level_passes("warn", "debug"));
        assert!(level_passes("info", "debug"));
        assert!(level_passes("debug", "debug"));
    }
}
