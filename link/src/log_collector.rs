//! Tails journald for configured systemd units and streams log batches
//! to the controller via PushLogs gRPC.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::config::{ControllerConfig, LogCollectorConfig};

pub mod proto {
    tonic::include_proto!("pillar");
}

use pillar_shared::proto::{LogBatch, LogEntry};
use proto::pillar_controller_client::PillarControllerClient;

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
///
/// tracing compact format: `2026-02-08T19:10:04.044506Z ERROR ThreadId(01) ...`
/// The level keyword appears right after the ISO timestamp as the second
/// whitespace-delimited token. We look for known level strings there first,
/// then fall back to scanning the full message for ` ERROR `, ` WARN `, etc.
fn detect_level_from_message(message: &str) -> Option<&'static str> {
    // Fast path: check second token (tracing compact format)
    let mut tokens = message.split_whitespace();
    if let Some(first) = tokens.next() {
        // First token should look like an ISO timestamp (starts with digit, contains 'T')
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

    // Slow path: scan message body for level keywords (handles multi-line or non-tracing formats)
    if message.contains(" ERROR ") || message.starts_with("ERROR ") {
        return Some("error");
    }
    if message.contains(" WARN ") || message.starts_with("WARN ") {
        return Some("warn");
    }

    None
}

/// Derive a service name from a systemd unit name.
fn unit_to_service(unit: &str) -> &str {
    if unit.contains("validator") {
        "validator"
    } else if unit.contains("operator") {
        "operator"
    } else if unit.contains("link") {
        "link"
    } else if unit.contains("controller") {
        "controller"
    } else {
        unit.strip_suffix(".service").unwrap_or(unit)
    }
}

/// Extract MESSAGE from journald JSON. The field may be a string or a byte array
/// (when the message contains non-UTF8 data like ANSI escape codes).
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
            // Strip ANSI escape sequences for cleaner logs
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
            // Skip ESC [ ... (letter) sequences
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
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

    // Prefer the level embedded in tracing output over journald PRIORITY,
    // because journald sets PRIORITY=6 (info) for all stdout regardless of
    // what the application actually logged.
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

/// Spawn a journalctl tail for a single unit, sending parsed entries to the channel.
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
    cancel: CancellationToken,
) {
    tracing::info!(
        units = ?config.units,
        buffer_size = config.buffer_size,
        flush_interval_ms = config.flush_interval_ms,
        "log collector starting"
    );

    let (tx, mut rx) = mpsc::channel::<LogEntry>(config.buffer_size * 2);

    // Spawn a journalctl tailer per unit.
    for unit in &config.units {
        let tx = tx.clone();
        let unit = unit.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tail_unit(unit, tx, cancel).await;
        });
    }
    drop(tx); // Drop our copy so rx closes when all tailers exit.

    let node_id = controller.node_id.clone();
    let flush_interval = Duration::from_millis(config.flush_interval_ms);
    let buffer_size = config.buffer_size;

    let mut buffer: Vec<LogEntry> = Vec::with_capacity(buffer_size);
    let mut flush_timer = tokio::time::interval(flush_interval);
    flush_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut client: Option<PillarControllerClient<tonic::transport::Channel>> = None;
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                // Flush remaining on shutdown.
                if !buffer.is_empty() {
                    flush_batch(&mut client, &controller.endpoint, &node_id, &mut buffer, &mut backoff, max_backoff).await;
                }
                break;
            }
            entry = rx.recv() => {
                match entry {
                    Some(e) => {
                        buffer.push(e);
                        if buffer.len() >= buffer_size {
                            flush_batch(&mut client, &controller.endpoint, &node_id, &mut buffer, &mut backoff, max_backoff).await;
                            flush_timer.reset();
                        }
                    }
                    None => {
                        // All tailers exited.
                        if !buffer.is_empty() {
                            flush_batch(&mut client, &controller.endpoint, &node_id, &mut buffer, &mut backoff, max_backoff).await;
                        }
                        break;
                    }
                }
            }
            _ = flush_timer.tick() => {
                if !buffer.is_empty() {
                    flush_batch(&mut client, &controller.endpoint, &node_id, &mut buffer, &mut backoff, max_backoff).await;
                }
            }
        }
    }

    tracing::info!("log collector stopped");
}

/// Flush buffered log entries to the controller via PushLogs gRPC.
/// On failure, drops the batch (best-effort) and resets the client.
async fn flush_batch(
    client: &mut Option<PillarControllerClient<tonic::transport::Channel>>,
    endpoint: &str,
    node_id: &str,
    buffer: &mut Vec<LogEntry>,
    backoff: &mut Duration,
    max_backoff: Duration,
) {
    let entries = std::mem::take(buffer);
    let count = entries.len();

    // Ensure we have a connected client.
    if client.is_none() {
        match PillarControllerClient::connect(endpoint.to_string()).await {
            Ok(c) => {
                *client = Some(c);
                *backoff = Duration::from_secs(1);
            }
            Err(e) => {
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

    // PushLogs is client-streaming, but we send one batch per call for simplicity.
    let stream = tokio_stream::once(batch);
    let c = client.as_mut().unwrap();
    match c.push_logs(tonic::Request::new(stream)).await {
        Ok(resp) => {
            let ack = resp.into_inner();
            tracing::debug!(sent = count, acked = ack.received_count, "log batch flushed");
        }
        Err(e) => {
            tracing::debug!(error = %e, "log collector: push_logs failed, dropping {count} entries");
            *client = None; // Force reconnect next time.
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
        let line = r#"{"MESSAGE":"crash","PRIORITY":"3","__REALTIME_TIMESTAMP":"1700000000000000","_SYSTEMD_UNIT":"operator.service"}"#;
        let entry = parse_journal_line(line).unwrap();
        assert_eq!(entry.level, "error");
        assert_eq!(entry.service, "operator");
    }

    #[test]
    fn parse_warn_priority() {
        let line = r#"{"MESSAGE":"slow","PRIORITY":"4","__REALTIME_TIMESTAMP":"1700000000000000","_SYSTEMD_UNIT":"link.service"}"#;
        let entry = parse_journal_line(line).unwrap();
        assert_eq!(entry.level, "warn");
        assert_eq!(entry.service, "link");
    }

    #[test]
    fn parse_tracing_error_overrides_priority() {
        // journald PRIORITY=6 (info), but message contains tracing ERROR
        let line = r#"{"MESSAGE":"2026-02-08T19:10:04.044506Z ERROR ThreadId(01) pillar_operator::operator: recovery failed","PRIORITY":"6","__REALTIME_TIMESTAMP":"1700000000000000","_SYSTEMD_UNIT":"operator.service"}"#;
        let entry = parse_journal_line(line).unwrap();
        assert_eq!(entry.level, "error");
    }

    #[test]
    fn parse_tracing_warn_overrides_priority() {
        let line = r#"{"MESSAGE":"2026-02-08T19:10:04.044506Z  WARN ThreadId(01) pillar_operator::snapshot: node is stale","PRIORITY":"6","__REALTIME_TIMESTAMP":"1700000000000000","_SYSTEMD_UNIT":"operator.service"}"#;
        let entry = parse_journal_line(line).unwrap();
        assert_eq!(entry.level, "warn");
    }

    #[test]
    fn parse_tracing_info_from_message() {
        let line = r#"{"MESSAGE":"2026-02-08T19:10:04.044506Z  INFO ThreadId(01) pillar_link: starting","PRIORITY":"6","__REALTIME_TIMESTAMP":"1700000000000000","_SYSTEMD_UNIT":"link.service"}"#;
        let entry = parse_journal_line(line).unwrap();
        assert_eq!(entry.level, "info");
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
        // journald encodes non-UTF8 messages (e.g. with ANSI codes) as byte arrays
        let line = r#"{"MESSAGE":[104,101,108,108,111],"PRIORITY":"6","__REALTIME_TIMESTAMP":"1700000000000000","_SYSTEMD_UNIT":"operator.service"}"#;
        let entry = parse_journal_line(line).unwrap();
        assert_eq!(entry.message, "hello");
    }

    #[test]
    fn parse_byte_array_message_with_ansi() {
        // ANSI: ESC[2m ... ESC[0m wrapping text
        let line = r#"{"MESSAGE":[27,91,50,109,104,105,27,91,48,109],"PRIORITY":"6","__REALTIME_TIMESTAMP":"1700000000000000","_SYSTEMD_UNIT":"test.service"}"#;
        let entry = parse_journal_line(line).unwrap();
        assert_eq!(entry.message, "hi");
    }

    #[test]
    fn strip_ansi_removes_sequences() {
        assert_eq!(strip_ansi("\x1b[2mhello\x1b[0m"), "hello");
        assert_eq!(strip_ansi("no escapes"), "no escapes");
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn priority_mapping() {
        assert_eq!(priority_to_level("0"), "error");
        assert_eq!(priority_to_level("3"), "error");
        assert_eq!(priority_to_level("4"), "warn");
        assert_eq!(priority_to_level("5"), "info");
        assert_eq!(priority_to_level("6"), "info");
        assert_eq!(priority_to_level("7"), "debug");
        assert_eq!(priority_to_level("99"), "info");
    }

    #[test]
    fn detect_level_tracing_format() {
        assert_eq!(
            detect_level_from_message("2026-02-08T19:10:04.044506Z ERROR ThreadId(01) foo"),
            Some("error")
        );
        assert_eq!(
            detect_level_from_message("2026-02-08T19:10:04.044506Z  WARN ThreadId(01) foo"),
            Some("warn")
        );
        assert_eq!(
            detect_level_from_message("2026-02-08T19:10:04.044506Z  INFO ThreadId(01) foo"),
            Some("info")
        );
        assert_eq!(
            detect_level_from_message("2026-02-08T19:10:04.044506Z DEBUG ThreadId(01) foo"),
            Some("debug")
        );
        // Non-tracing message: no timestamp prefix
        assert_eq!(detect_level_from_message("hello world"), None);
        // Fallback scan: ERROR in body with surrounding spaces
        assert_eq!(
            detect_level_from_message("See ERROR details"),
            Some("error")
        );
        assert_eq!(
            detect_level_from_message("something ERROR happened"),
            Some("error")
        );
        // No match: ERROR not surrounded by spaces
        assert_eq!(detect_level_from_message("ERRORS found"), None);
    }

    #[test]
    fn unit_to_service_mapping() {
        assert_eq!(unit_to_service("solana-validator.service"), "validator");
        assert_eq!(unit_to_service("operator.service"), "operator");
        assert_eq!(unit_to_service("link.service"), "link");
        assert_eq!(unit_to_service("controller.service"), "controller");
        assert_eq!(unit_to_service("custom.service"), "custom");
    }
}
