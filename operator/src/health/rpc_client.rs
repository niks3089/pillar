use std::time::Duration;

use serde_json::{json, Value};

use crate::error::{PillarError, PillarResult};

/// Result of comparing local and reference slots.
pub struct SlotComparison {
    pub local_slot: Option<u64>,
    pub reference_slot: Option<u64>,
    pub slots_behind: Option<i64>,
}

/// Thin wrapper for raw Solana JSON-RPC calls via reqwest.
/// No Solana SDK dependency — just HTTP POST with JSON payloads.
pub struct RpcClient {
    local_url: String,
    reference_urls: Vec<String>,
    client: reqwest::Client,
}

impl RpcClient {
    pub fn new(local_url: String, reference_urls: Vec<String>, timeout_secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .expect("failed to build reqwest client");

        Self {
            local_url,
            reference_urls,
            client,
        }
    }

    /// Call `getSlot` on the local node.
    pub async fn get_local_slot(&self) -> PillarResult<u64> {
        self.get_slot(&self.local_url).await
    }

    /// Call `getSlot` on the first responsive reference RPC.
    pub async fn get_reference_slot(&self) -> PillarResult<u64> {
        let mut last_err = None;
        for url in &self.reference_urls {
            match self.get_slot(url).await {
                Ok(slot) => return Ok(slot),
                Err(e) => {
                    tracing::warn!(url, error = %e, "reference RPC failed, trying next");
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            PillarError::Rpc {
                method: "getSlot".to_string(),
                reason: "no reference RPCs configured".to_string(),
            }
        }))
    }

    /// Call `getHealth` on the local node. Returns `true` if healthy.
    #[allow(dead_code)]
    pub async fn get_local_health(&self) -> PillarResult<bool> {
        let result = self.call(&self.local_url, "getHealth", json!([])).await;
        match result {
            Ok(val) => Ok(val.get("result").and_then(|v| v.as_str()) == Some("ok")),
            Err(_) => Ok(false),
        }
    }

    /// Call `getVoteAccounts` on the local node.
    /// Returns (current_voters, delinquent_voters) counts.
    pub async fn get_vote_accounts(&self) -> PillarResult<(usize, usize)> {
        let resp = self
            .call(&self.local_url, "getVoteAccounts", json!([]))
            .await?;

        let result = resp.get("result").ok_or_else(|| PillarError::Rpc {
            method: "getVoteAccounts".to_string(),
            reason: "missing result field".to_string(),
        })?;

        let current = result
            .get("current")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        let delinquent = result
            .get("delinquent")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        Ok((current, delinquent))
    }

    /// Fetch local and reference slots and compute the difference.
    pub async fn compare_slots(&self) -> SlotComparison {
        let local_slot = self.get_local_slot().await.ok();
        let reference_slot = self.get_reference_slot().await.ok();
        let slots_behind = match (local_slot, reference_slot) {
            (Some(local), Some(reference)) => Some(reference as i64 - local as i64),
            _ => None,
        };
        SlotComparison {
            local_slot,
            reference_slot,
            slots_behind,
        }
    }

    async fn get_slot(&self, url: &str) -> PillarResult<u64> {
        let resp = self.call(url, "getSlot", json!([])).await?;
        resp.get("result")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| PillarError::Rpc {
                method: "getSlot".to_string(),
                reason: format!("unexpected response from {url}"),
            })
    }

    async fn call(&self, url: &str, method: &str, params: Value) -> PillarResult<Value> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let resp = self
            .client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|e| PillarError::Rpc {
                method: method.to_string(),
                reason: format!("{url}: {e}"),
            })?;

        resp.json::<Value>()
            .await
            .map_err(|e| PillarError::Rpc {
                method: method.to_string(),
                reason: format!("invalid JSON from {url}: {e}"),
            })
    }
}
