pub mod rpc_client;

use async_trait::async_trait;

use crate::config::{HealthConfig, NetworkConfig};
use crate::error::PillarResult;
use crate::role::NodeRole;

pub use pillar_shared::types::{NodeHealth, NodeState, SlotInfo};

use rpc_client::RpcClient;

#[async_trait]
pub trait HealthChecker: Send + Sync {
    async fn check(&self) -> PillarResult<NodeHealth>;
}

/// Build the appropriate HealthChecker for the node's role.
pub fn create_health_checker(
    role: NodeRole,
    health_config: &HealthConfig,
    network_config: &NetworkConfig,
) -> Box<dyn HealthChecker> {
    let client = RpcClient::new(
        health_config.local_rpc_url.clone(),
        network_config.reference_rpc_urls.clone(),
        health_config.rpc_timeout_secs,
    );
    let check_voting = matches!(role, NodeRole::Validator);

    Box::new(SlotHealthChecker::new(
        client,
        health_config.slots_behind_threshold,
        check_voting,
    ))
}

/// Unified health checker for all node roles.
///
/// Compares local slot against reference RPCs to determine how far behind
/// the node is. For validators (`check_voting = true`), also queries
/// `getVoteAccounts` to confirm the node is actively voting before
/// declaring it healthy. RPC/gRPC nodes skip the voting check.
struct SlotHealthChecker {
    client: RpcClient,
    slots_behind_threshold: u64,
    /// True for Validator role — requires active voting to be Healthy.
    /// False for RPC/gRPC — caught-up slots are sufficient.
    check_voting: bool,
}

impl SlotHealthChecker {
    fn new(client: RpcClient, slots_behind_threshold: u64, check_voting: bool) -> Self {
        Self {
            client,
            slots_behind_threshold,
            check_voting,
        }
    }

    /// Returns true if the node is actively voting, or true unconditionally
    /// for non-validator roles (voting check is skipped).
    async fn is_voting(&self) -> bool {
        if !self.check_voting {
            return true;
        }
        match self.client.get_vote_accounts().await {
            Ok((current, _delinquent)) => current > 0,
            Err(e) => {
                tracing::warn!(error = %e, "failed to check vote accounts");
                false
            }
        }
    }
}

#[async_trait]
impl HealthChecker for SlotHealthChecker {
    async fn check(&self) -> PillarResult<NodeHealth> {
        let cmp = self.client.compare_slots().await;
        let is_voting = self.is_voting().await;
        let cluster_version = self.client.get_reference_version().await;

        let state = determine_state(
            cmp.local_slot,
            cmp.slots_behind,
            is_voting,
            self.slots_behind_threshold,
        );

        Ok(NodeHealth {
            state,
            slot_info: SlotInfo {
                local_slot: cmp.local_slot,
                reference_slot: cmp.reference_slot,
            },
            slots_behind: cmp.slots_behind,
            cluster_version,
        })
    }
}

fn determine_state(
    local_slot: Option<u64>,
    slots_behind: Option<i64>,
    is_voting: bool,
    threshold: u64,
) -> NodeState {
    match local_slot {
        None => NodeState::Off,
        Some(_) => match slots_behind {
            Some(behind) if behind > threshold as i64 => NodeState::Behind,
            Some(_) if is_voting => NodeState::Healthy,
            Some(_) => NodeState::Behind, // caught up but not voting yet
            None => NodeState::StartingUp,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_local_slot_is_off() {
        assert_eq!(determine_state(None, None, false, 100), NodeState::Off);
    }

    #[test]
    fn far_behind_is_behind() {
        assert_eq!(
            determine_state(Some(1000), Some(200), false, 100),
            NodeState::Behind
        );
    }

    #[test]
    fn within_threshold_and_voting_is_healthy() {
        assert_eq!(
            determine_state(Some(1000), Some(50), true, 100),
            NodeState::Healthy
        );
    }

    #[test]
    fn within_threshold_not_voting_is_behind() {
        assert_eq!(
            determine_state(Some(1000), Some(50), false, 100),
            NodeState::Behind
        );
    }

    #[test]
    fn no_reference_is_starting_up() {
        assert_eq!(
            determine_state(Some(1000), None, false, 100),
            NodeState::StartingUp
        );
    }

    #[test]
    fn ahead_and_voting_is_healthy() {
        assert_eq!(
            determine_state(Some(1000), Some(-5), true, 100),
            NodeState::Healthy
        );
    }

    // RPC/gRPC nodes pass is_voting=true (check skipped), so caught-up = healthy
    #[test]
    fn rpc_node_within_threshold_is_healthy() {
        assert_eq!(
            determine_state(Some(1000), Some(50), true, 100),
            NodeState::Healthy
        );
    }

    // RPC/gRPC nodes still report Behind when far behind
    #[test]
    fn rpc_node_far_behind_is_behind() {
        assert_eq!(
            determine_state(Some(1000), Some(200), true, 100),
            NodeState::Behind
        );
    }
}
