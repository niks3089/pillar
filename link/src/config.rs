use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkConfig {
    #[serde(default = "default_state_path")]
    pub state_path: String,
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_http_listen")]
    pub http_listen: String,
    pub controller: ControllerConfig,
    #[serde(default)]
    pub log_collector: LogCollectorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogCollectorConfig {
    #[serde(default = "default_log_collector_enabled")]
    pub enabled: bool,
    #[serde(default = "default_log_collector_units")]
    pub units: Vec<String>,
    #[serde(default = "default_log_collector_buffer_size")]
    pub buffer_size: usize,
    #[serde(default = "default_log_collector_flush_interval_ms")]
    pub flush_interval_ms: u64,
}

impl Default for LogCollectorConfig {
    fn default() -> Self {
        Self {
            enabled: default_log_collector_enabled(),
            units: default_log_collector_units(),
            buffer_size: default_log_collector_buffer_size(),
            flush_interval_ms: default_log_collector_flush_interval_ms(),
        }
    }
}

fn default_log_collector_enabled() -> bool {
    true
}

fn default_log_collector_units() -> Vec<String> {
    vec![
        "solana-validator.service".to_string(),
        "operator.service".to_string(),
        "link.service".to_string(),
        "controller.service".to_string(),
    ]
}

fn default_log_collector_buffer_size() -> usize {
    100
}

fn default_log_collector_flush_interval_ms() -> u64 {
    1000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerConfig {
    pub endpoint: String,
    pub node_id: String,
    #[serde(default = "default_report_interval_secs")]
    pub report_interval_secs: u64,
}

fn default_report_interval_secs() -> u64 {
    10
}

impl LinkConfig {
    /// Validate config values at startup.
    pub fn validate(&self) -> Result<(), String> {
        let mut errors = Vec::new();

        if self.poll_interval_secs == 0 {
            errors.push("poll_interval_secs must be > 0 (would cause CPU spin loop)".to_string());
        }
        if self.state_path.is_empty() {
            errors.push("state_path must not be empty".to_string());
        }
        if self.http_listen.is_empty() {
            errors.push("http_listen must not be empty".to_string());
        }
        if self.controller.endpoint.is_empty() {
            errors.push("controller.endpoint must not be empty".to_string());
        }
        if self.controller.node_id.is_empty() {
            errors.push("controller.node_id must not be empty".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!("config validation failed:\n  - {}", errors.join("\n  - ")))
        }
    }
}

fn default_state_path() -> String {
    "/var/run/pillar/operator-state.bin".to_string()
}

fn default_poll_interval_secs() -> u64 {
    5
}

fn default_http_listen() -> String {
    "0.0.0.0:9090".to_string()
}
