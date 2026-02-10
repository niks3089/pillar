use tokio::process::Command;

use crate::error::{PillarError, PillarResult};

pub struct SystemdManager {
    service_name: String,
    max_retries: u32,
    base_delay_ms: u64,
}

impl SystemdManager {
    pub fn new(service_name: String) -> Self {
        Self {
            service_name,
            max_retries: 3,
            base_delay_ms: 500,
        }
    }

    pub async fn start(&self) -> PillarResult<()> {
        tracing::info!(service = %self.service_name, "starting service");
        self.run_systemctl("start").await
    }

    pub async fn stop(&self) -> PillarResult<()> {
        tracing::info!(service = %self.service_name, "stopping service");
        self.run_systemctl("stop").await
    }

    pub async fn restart(&self) -> PillarResult<()> {
        tracing::info!(service = %self.service_name, "restarting service");
        self.run_systemctl("restart").await
    }

    #[allow(dead_code)]
    pub async fn is_active(&self) -> PillarResult<bool> {
        let output = Command::new("sudo")
            .args(["systemctl", "is-active", &self.service_name])
            .output()
            .await
            .map_err(|e| PillarError::Systemd {
                operation: "is-active".to_string(),
                service: self.service_name.clone(),
                reason: format!("failed to execute sudo systemctl: {e}"),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(stdout == "active")
    }

    async fn run_systemctl(&self, operation: &str) -> PillarResult<()> {
        let mut last_err = None;

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                let delay = self.base_delay_ms * 2u64.pow(attempt - 1);
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }

            // Use sudo so the sol user can manage system services via sudoers policy.
            let output = Command::new("sudo")
                .args(["systemctl", operation, &self.service_name])
                .output()
                .await
                .map_err(|e| PillarError::Systemd {
                    operation: operation.to_string(),
                    service: self.service_name.clone(),
                    reason: format!("failed to execute sudo systemctl: {e}"),
                })?;

            if output.status.success() {
                return Ok(());
            }

            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            last_err = Some(stderr);
        }

        Err(PillarError::Systemd {
            operation: operation.to_string(),
            service: self.service_name.clone(),
            reason: last_err.unwrap_or_else(|| "unknown error".to_string()),
        })
    }
}
