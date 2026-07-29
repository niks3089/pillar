//! Recent release versions per validator client, from the GitHub Releases API.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

const CACHE_TTL: Duration = Duration::from_secs(3600);

fn repo_for_client(client: &str) -> Option<&'static str> {
    match client {
        "agave" => Some("anza-xyz/agave"),
        "jito" => Some("jito-foundation/jito-solana"),
        "firedancer" | "frankendancer" => Some("firedancer-io/firedancer"),
        "surfpool" => Some("solana-foundation/surfpool"),
        _ => None,
    }
}

type CacheEntry = (Instant, Vec<String>);

/// 1h in-memory cache — unauthenticated GitHub API allows 60 req/h per IP.
#[derive(Clone, Default)]
pub struct ReleaseCache(Arc<RwLock<HashMap<String, CacheEntry>>>);

impl ReleaseCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn versions_for(&self, client: &str) -> Result<Vec<String>, String> {
        let repo =
            repo_for_client(client).ok_or_else(|| format!("unknown client '{client}'"))?;
        if let Some((at, versions)) = self.0.read().await.get(client) {
            if at.elapsed() < CACHE_TTL {
                return Ok(versions.clone());
            }
        }
        let versions = fetch_versions(repo).await?;
        self.0
            .write()
            .await
            .insert(client.to_string(), (Instant::now(), versions.clone()));
        Ok(versions)
    }
}

async fn fetch_versions(repo: &str) -> Result<Vec<String>, String> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("pillar-controller")
        .build()
        .map_err(|e| format!("client error: {e}"))?;
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=15");
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("fetch error: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API HTTP {}", resp.status()));
    }
    let releases: Vec<serde_json::Value> =
        resp.json().await.map_err(|e| format!("parse error: {e}"))?;
    // Prereleases are included: agave marks betas/rcs as prerelease, and
    // testnet nodes typically run exactly those.
    Ok(releases
        .iter()
        .filter(|r| !r["draft"].as_bool().unwrap_or(false))
        .filter_map(|r| r["tag_name"].as_str())
        .map(|t| t.trim_start_matches('v').to_string())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_mapping() {
        assert_eq!(repo_for_client("agave"), Some("anza-xyz/agave"));
        assert_eq!(
            repo_for_client("frankendancer"),
            Some("firedancer-io/firedancer")
        );
        assert_eq!(repo_for_client("nope"), None);
    }
}
