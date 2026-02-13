use crate::client::ClientKind;
use crate::role::NodeRole;
use crate::snapshot::DownloadMethod;
use serde::{Deserialize, Serialize};

// Default constants
const DEFAULT_SERVICE_NAME: &str = "solana-validator";
const DEFAULT_MAX_STARTUP_WAIT_SECS: u64 = 600;
const DEFAULT_MAX_CATCHUP_WAIT_SECS: u64 = 1800;
const DEFAULT_CRASH_WINDOW_SECS: u64 = 3600;
const DEFAULT_CRASH_THRESHOLD: usize = 3;
const DEFAULT_STALENESS_THRESHOLD_SLOTS: u64 = 1000;
const DEFAULT_DOWNLOAD_TIMEOUT_SECS: u64 = 3600;
const DEFAULT_CONSECUTIVE_OFF_THRESHOLD: usize = 3;
const DEFAULT_CHECK_INTERVAL_SECS: u64 = 20;
const DEFAULT_SLOTS_BEHIND_THRESHOLD: u64 = 100;
const DEFAULT_RPC_TIMEOUT_SECS: u64 = 10;
const DEFAULT_LOCAL_RPC_URL: &str = "http://127.0.0.1:8899";
const DEFAULT_LEDGER_PATH: &str = "/mnt/ledger";
const DEFAULT_SNAPSHOT_PATH: &str = "/mnt/snapshots";
const DEFAULT_HTTP_LISTEN: &str = "0.0.0.0:9090";
const DEFAULT_REPORT_INTERVAL_SECS: u64 = 10;
const DEFAULT_SYSINFO_REFRESH_INTERVAL_SECS: u64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub role: NodeRole,
    #[serde(default)]
    pub client: ClientKind,
    pub network: NetworkConfig,
    #[serde(default)]
    pub lifecycle: LifecycleConfig,
    #[serde(default)]
    pub snapshot: SnapshotConfig,
    #[serde(default)]
    pub health: HealthConfig,
    #[serde(default)]
    pub paths: PathConfig,
    #[serde(default = "default_http_listen")]
    pub http_listen: String,
    pub controller: ControllerConfig,
    #[serde(default = "default_sysinfo_refresh_interval_secs")]
    pub sysinfo_refresh_interval_secs: u64,
    #[serde(default)]
    pub log_collector: LogCollectorConfig,
    /// Optional: write state to a binary file for debugging.
    #[serde(default)]
    pub debug_state_file: String,
}

impl AgentConfig {
    /// Validate config values to catch dangerous misconfigurations at startup.
    pub fn validate(&self) -> Result<(), String> {
        let mut errors = Vec::new();

        if self.health.check_interval_secs == 0 {
            errors.push("health.check_interval_secs must be > 0 (would cause CPU spin loop)".to_string());
        }
        if self.lifecycle.max_startup_wait_secs == 0 {
            errors.push("lifecycle.max_startup_wait_secs must be > 0 (would cause immediate timeout)".to_string());
        }
        if self.lifecycle.max_catchup_wait_secs == 0 {
            errors.push("lifecycle.max_catchup_wait_secs must be > 0 (would cause immediate timeout)".to_string());
        }
        if self.lifecycle.crash_threshold == 0 {
            errors.push("lifecycle.crash_threshold must be > 0 (would cause immediate crash loop)".to_string());
        }
        if self.lifecycle.crash_window_secs == 0 {
            errors.push("lifecycle.crash_window_secs must be > 0".to_string());
        }
        if self.network.reference_rpc_urls.is_empty() {
            errors.push("network.reference_rpc_urls must not be empty (health checks would always fail)".to_string());
        }
        if self.health.rpc_timeout_secs == 0 {
            errors.push("health.rpc_timeout_secs must be > 0".to_string());
        }
        if self.snapshot.staleness_threshold_slots == 0 {
            errors.push("snapshot.staleness_threshold_slots must be > 0".to_string());
        }
        if self.snapshot.download_timeout_secs == 0 {
            errors.push("snapshot.download_timeout_secs must be > 0".to_string());
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
        if self.sysinfo_refresh_interval_secs == 0 {
            errors.push("sysinfo_refresh_interval_secs must be > 0".to_string());
        }

        // Path validation
        let ledger = std::path::Path::new(&self.paths.ledger_path);
        if !ledger.is_absolute() {
            errors.push(format!("paths.ledger_path must be absolute: {}", self.paths.ledger_path));
        } else if ledger.components().count() < 3 {
            errors.push(format!("paths.ledger_path too shallow: {}", self.paths.ledger_path));
        }

        let snapshot = std::path::Path::new(&self.paths.snapshot_path);
        if !snapshot.is_absolute() {
            errors.push(format!("paths.snapshot_path must be absolute: {}", self.paths.snapshot_path));
        } else if snapshot.components().count() < 3 {
            errors.push(format!("paths.snapshot_path too shallow: {}", self.paths.snapshot_path));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!("config validation failed:\n  - {}", errors.join("\n  - ")))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub cluster: String,
    pub reference_rpc_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleConfig {
    #[serde(default = "default_service_name")]
    pub service_name: String,
    #[serde(default = "default_max_startup_wait_secs")]
    pub max_startup_wait_secs: u64,
    #[serde(default = "default_max_catchup_wait_secs")]
    pub max_catchup_wait_secs: u64,
    #[serde(default = "default_crash_window_secs")]
    pub crash_window_secs: u64,
    #[serde(default = "default_crash_threshold")]
    pub crash_threshold: usize,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            service_name: DEFAULT_SERVICE_NAME.to_string(),
            max_startup_wait_secs: DEFAULT_MAX_STARTUP_WAIT_SECS,
            max_catchup_wait_secs: DEFAULT_MAX_CATCHUP_WAIT_SECS,
            crash_window_secs: DEFAULT_CRASH_WINDOW_SECS,
            crash_threshold: DEFAULT_CRASH_THRESHOLD,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotConfig {
    #[serde(default)]
    pub download_method: DownloadMethod,
    #[serde(default)]
    pub server_hostname: String,
    #[serde(default = "default_staleness_threshold_slots")]
    pub staleness_threshold_slots: u64,
    #[serde(default = "default_download_timeout_secs")]
    pub download_timeout_secs: u64,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            download_method: DownloadMethod::default(),
            server_hostname: String::new(),
            staleness_threshold_slots: DEFAULT_STALENESS_THRESHOLD_SLOTS,
            download_timeout_secs: DEFAULT_DOWNLOAD_TIMEOUT_SECS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthConfig {
    #[serde(default = "default_check_interval_secs")]
    pub check_interval_secs: u64,
    #[serde(default = "default_slots_behind_threshold")]
    pub slots_behind_threshold: u64,
    #[serde(default = "default_rpc_timeout_secs")]
    pub rpc_timeout_secs: u64,
    #[serde(default = "default_local_rpc_url")]
    pub local_rpc_url: String,
    #[serde(default = "default_consecutive_off_threshold")]
    pub consecutive_off_threshold: usize,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: DEFAULT_CHECK_INTERVAL_SECS,
            slots_behind_threshold: DEFAULT_SLOTS_BEHIND_THRESHOLD,
            rpc_timeout_secs: DEFAULT_RPC_TIMEOUT_SECS,
            local_rpc_url: DEFAULT_LOCAL_RPC_URL.to_string(),
            consecutive_off_threshold: DEFAULT_CONSECUTIVE_OFF_THRESHOLD,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathConfig {
    #[serde(default = "default_ledger_path")]
    pub ledger_path: String,
    #[serde(default = "default_snapshot_path")]
    pub snapshot_path: String,
}

impl Default for PathConfig {
    fn default() -> Self {
        Self {
            ledger_path: DEFAULT_LEDGER_PATH.to_string(),
            snapshot_path: DEFAULT_SNAPSHOT_PATH.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerConfig {
    pub endpoint: String,
    pub node_id: String,
    #[serde(default = "default_report_interval_secs")]
    pub report_interval_secs: u64,
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

fn default_log_collector_enabled() -> bool { true }
fn default_log_collector_units() -> Vec<String> {
    vec![
        "solana-validator.service".to_string(),
        "pillar-agent.service".to_string(),
    ]
}
fn default_log_collector_buffer_size() -> usize { 100 }
fn default_log_collector_flush_interval_ms() -> u64 { 1000 }

// Serde default functions
fn default_service_name() -> String { DEFAULT_SERVICE_NAME.to_string() }
fn default_max_startup_wait_secs() -> u64 { DEFAULT_MAX_STARTUP_WAIT_SECS }
fn default_max_catchup_wait_secs() -> u64 { DEFAULT_MAX_CATCHUP_WAIT_SECS }
fn default_crash_window_secs() -> u64 { DEFAULT_CRASH_WINDOW_SECS }
fn default_crash_threshold() -> usize { DEFAULT_CRASH_THRESHOLD }
fn default_staleness_threshold_slots() -> u64 { DEFAULT_STALENESS_THRESHOLD_SLOTS }
fn default_download_timeout_secs() -> u64 { DEFAULT_DOWNLOAD_TIMEOUT_SECS }
fn default_consecutive_off_threshold() -> usize { DEFAULT_CONSECUTIVE_OFF_THRESHOLD }
fn default_check_interval_secs() -> u64 { DEFAULT_CHECK_INTERVAL_SECS }
fn default_slots_behind_threshold() -> u64 { DEFAULT_SLOTS_BEHIND_THRESHOLD }
fn default_rpc_timeout_secs() -> u64 { DEFAULT_RPC_TIMEOUT_SECS }
fn default_local_rpc_url() -> String { DEFAULT_LOCAL_RPC_URL.to_string() }
fn default_ledger_path() -> String { DEFAULT_LEDGER_PATH.to_string() }
fn default_snapshot_path() -> String { DEFAULT_SNAPSHOT_PATH.to_string() }
fn default_http_listen() -> String { DEFAULT_HTTP_LISTEN.to_string() }
fn default_report_interval_secs() -> u64 { DEFAULT_REPORT_INTERVAL_SECS }
fn default_sysinfo_refresh_interval_secs() -> u64 { DEFAULT_SYSINFO_REFRESH_INTERVAL_SECS }
