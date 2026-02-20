use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// S3 release manifest shape.
#[derive(Debug, Deserialize)]
pub struct ReleaseManifest {
    pub controller: Option<ComponentRelease>,
    pub agent: Option<ComponentRelease>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ComponentRelease {
    pub version: String,
    pub download_url: String,
    pub sha256: String,
    #[serde(default)]
    pub release_notes: String,
}

/// Shared state exposed to API handlers.
#[derive(Debug, Clone, Serialize)]
pub struct AvailableUpdate {
    pub version: String,
    pub download_url: String,
    pub sha256: String,
    pub release_notes: String,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct UpdateInfo {
    pub controller_update: Option<AvailableUpdate>,
    pub agent_update: Option<AvailableUpdate>,
    pub checked_at: Option<i64>,
}

pub type SharedUpdateInfo = Arc<RwLock<UpdateInfo>>;

const CHECK_INTERVAL_SECS: u64 = 300; // 5 minutes

/// Background task that polls the manifest URL and updates shared state.
pub async fn run_update_checker(
    url: String,
    current_version: String,
    update_info: SharedUpdateInfo,
    cancel: CancellationToken,
) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(CHECK_INTERVAL_SECS));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                match fetch_and_compare(&client, &url, &current_version).await {
                    Ok(info) => {
                        *update_info.write().await = info;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "update check failed");
                    }
                }
            }
            _ = cancel.cancelled() => break,
        }
    }
}

async fn fetch_and_compare(
    client: &reqwest::Client,
    url: &str,
    current_version: &str,
) -> Result<UpdateInfo, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("fetch error: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let manifest: ReleaseManifest = resp
        .json()
        .await
        .map_err(|e| format!("parse error: {e}"))?;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let controller_update = manifest
        .controller
        .filter(|c| is_newer_version(&c.version, current_version))
        .map(|c| AvailableUpdate {
            version: c.version,
            download_url: c.download_url,
            sha256: c.sha256,
            release_notes: c.release_notes,
        });

    let agent_update = manifest
        .agent
        .filter(|a| is_newer_version(&a.version, current_version))
        .map(|a| AvailableUpdate {
            version: a.version,
            download_url: a.download_url,
            sha256: a.sha256,
            release_notes: a.release_notes,
        });

    Ok(UpdateInfo {
        controller_update,
        agent_update,
        checked_at: Some(now_ms),
    })
}

/// Returns true if `candidate` is strictly newer than `current`.
/// Both must be semver-style "major.minor.patch" strings.
fn is_newer_version(candidate: &str, current: &str) -> bool {
    let parse = |s: &str| -> Option<(u32, u32, u32)> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        Some((
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        ))
    };

    match (parse(candidate), parse(current)) {
        (Some(c), Some(cur)) => c > cur,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_version() {
        assert!(is_newer_version("0.2.0", "0.1.0"));
        assert!(is_newer_version("1.0.0", "0.9.9"));
        assert!(is_newer_version("0.1.1", "0.1.0"));
    }

    #[test]
    fn equal_version() {
        assert!(!is_newer_version("0.1.0", "0.1.0"));
        assert!(!is_newer_version("1.0.0", "1.0.0"));
    }

    #[test]
    fn older_version() {
        assert!(!is_newer_version("0.1.0", "0.2.0"));
        assert!(!is_newer_version("0.9.9", "1.0.0"));
    }

    #[test]
    fn malformed_version() {
        assert!(!is_newer_version("abc", "0.1.0"));
        assert!(!is_newer_version("0.1.0", "xyz"));
        assert!(!is_newer_version("1.0", "0.1.0"));
        assert!(!is_newer_version("", ""));
    }
}
