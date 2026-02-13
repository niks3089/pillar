use thiserror::Error;

pub type PillarResult<T> = Result<T, PillarError>;

#[derive(Debug, Error)]
pub enum PillarError {
    #[error("systemd error: {operation} on {service}: {reason}")]
    Systemd {
        operation: String,
        service: String,
        reason: String,
    },

    #[allow(dead_code)]
    #[error("health check failed: {0}")]
    HealthCheck(String),

    #[error("snapshot error: {0}")]
    Snapshot(String),

    #[allow(dead_code)]
    #[error("config error: {0}")]
    Config(String),

    #[error("rpc error: {method}: {reason}")]
    Rpc { method: String, reason: String },

    #[allow(dead_code)]
    #[error("timeout: {operation} after {duration_secs}s")]
    Timeout {
        operation: String,
        duration_secs: u64,
    },

    #[error("{0:#}")]
    Internal(#[from] anyhow::Error),
}
