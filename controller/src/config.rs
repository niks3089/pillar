use serde::{Deserialize, Serialize};

const DEFAULT_GRPC_LISTEN: &str = "0.0.0.0:50051";
const DEFAULT_HTTP_LISTEN: &str = "0.0.0.0:8080";
const DEFAULT_DB_PATH: &str = "/var/lib/pillar/controller.db";
const DEFAULT_RETENTION_DAYS: u32 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerConfig {
    #[serde(default = "default_grpc_listen")]
    pub grpc_listen: String,
    #[serde(default = "default_http_listen")]
    pub http_listen: String,
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default)]
    pub external_url: String,
}

impl ControllerConfig {
    pub fn validate(&self) -> Result<(), String> {
        let mut errors = Vec::new();

        if self.grpc_listen.is_empty() {
            errors.push("grpc_listen must not be empty".to_string());
        }
        if self.http_listen.is_empty() {
            errors.push("http_listen must not be empty".to_string());
        }
        if self.db_path.is_empty() {
            errors.push("db_path must not be empty".to_string());
        }
        if self.retention_days == 0 {
            errors.push("retention_days must be > 0".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "config validation failed:\n  - {}",
                errors.join("\n  - ")
            ))
        }
    }
}

fn default_grpc_listen() -> String {
    DEFAULT_GRPC_LISTEN.to_string()
}
fn default_http_listen() -> String {
    DEFAULT_HTTP_LISTEN.to_string()
}
fn default_db_path() -> String {
    DEFAULT_DB_PATH.to_string()
}
fn default_retention_days() -> u32 {
    DEFAULT_RETENTION_DAYS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let config: ControllerConfig = serde_json::from_str("{}").unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(config.grpc_listen, DEFAULT_GRPC_LISTEN);
        assert_eq!(config.http_listen, DEFAULT_HTTP_LISTEN);
        assert_eq!(config.retention_days, DEFAULT_RETENTION_DAYS);
    }

    #[test]
    fn empty_grpc_listen_fails_validation() {
        let config = ControllerConfig {
            grpc_listen: String::new(),
            http_listen: DEFAULT_HTTP_LISTEN.to_string(),
            db_path: "/tmp/test.db".to_string(),
            retention_days: DEFAULT_RETENTION_DAYS,
            external_url: String::new(),
        };
        let err = config.validate().unwrap_err();
        assert!(err.contains("grpc_listen"));
    }

    #[test]
    fn zero_retention_fails_validation() {
        let config = ControllerConfig {
            grpc_listen: DEFAULT_GRPC_LISTEN.to_string(),
            http_listen: DEFAULT_HTTP_LISTEN.to_string(),
            db_path: "/tmp/test.db".to_string(),
            retention_days: 0,
            external_url: String::new(),
        };
        let err = config.validate().unwrap_err();
        assert!(err.contains("retention_days"));
    }
}
