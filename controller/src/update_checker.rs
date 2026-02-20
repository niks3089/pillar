use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

const MANIFEST_URL: &str =
    "https://janus-meter.s3.eu-north-1.amazonaws.com/pillar/latest/manifest.json";
const STALE_AFTER_MS: i64 = 3_600_000; // 1 hour

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

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Fire-and-forget: fetch manifest once on startup.
pub fn spawn_initial_check(current_version: String, update_info: SharedUpdateInfo) {
    tokio::spawn(async move {
        do_check(&current_version, &update_info).await;
    });
}

/// Called by the `/api/version` handler. Returns cached data immediately,
/// spawns a background refresh if the cache is stale (>1 hour).
pub async fn get_or_refresh(
    current_version: &str,
    update_info: &SharedUpdateInfo,
) -> UpdateInfo {
    let info = update_info.read().await.clone();
    let stale = match info.checked_at {
        Some(ts) => now_ms() - ts > STALE_AFTER_MS,
        None => true,
    };
    if stale {
        let ver = current_version.to_string();
        let ui = update_info.clone();
        tokio::spawn(async move {
            do_check(&ver, &ui).await;
        });
    }
    info
}

async fn do_check(current_version: &str, update_info: &SharedUpdateInfo) {
    match fetch_manifest(current_version).await {
        Ok(info) => {
            *update_info.write().await = info;
        }
        Err(e) => {
            tracing::warn!(error = %e, "update check failed");
        }
    }
}

async fn fetch_manifest(current_version: &str) -> Result<UpdateInfo, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("client error: {e}"))?;

    let resp = client
        .get(MANIFEST_URL)
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
        checked_at: Some(now_ms()),
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
