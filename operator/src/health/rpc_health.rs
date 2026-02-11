use async_trait::async_trait;

use crate::error::PillarResult;

use super::rpc_client::RpcClient;
use super::types::{NodeHealth, NodeState, SlotInfo};
use super::HealthChecker;

/// Health checker for RPC and gRPC nodes.
///
/// Calls `getSlot` on both local and reference RPCs, compares them,
/// and derives the node state from how far behind it is.
pub struct RpcHealthChecker {
    client: RpcClient,
    slots_behind_threshold: u64,
}

impl RpcHealthChecker {
    pub fn new(client: RpcClient, slots_behind_threshold: u64) -> Self {
        Self {
            client,
            slots_behind_threshold,
        }
    }
}

#[async_trait]
impl HealthChecker for RpcHealthChecker {
    async fn check(&self) -> PillarResult<NodeHealth> {
        let cmp = self.client.compare_slots().await;
        let cluster_version = self.client.get_reference_version().await;

        let state = determine_state(cmp.local_slot, cmp.slots_behind, self.slots_behind_threshold);

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
    threshold: u64,
) -> NodeState {
    match local_slot {
        None => NodeState::Off,
        Some(_) => match slots_behind {
            Some(behind) if behind > threshold as i64 => NodeState::Behind,
            Some(_) => NodeState::Healthy,
            None => NodeState::StartingUp,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_local_slot_is_off() {
        assert_eq!(determine_state(None, None, 100), NodeState::Off);
    }

    #[test]
    fn far_behind_is_behind() {
        assert_eq!(determine_state(Some(1000), Some(200), 100), NodeState::Behind);
    }

    #[test]
    fn within_threshold_is_healthy() {
        assert_eq!(determine_state(Some(1000), Some(50), 100), NodeState::Healthy);
    }

    #[test]
    fn no_reference_is_starting_up() {
        assert_eq!(determine_state(Some(1000), None, 100), NodeState::StartingUp);
    }

    #[test]
    fn ahead_is_healthy() {
        assert_eq!(determine_state(Some(1000), Some(-5), 100), NodeState::Healthy);
    }
}
