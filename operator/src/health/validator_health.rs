use async_trait::async_trait;

use crate::error::PillarResult;

use super::rpc_client::RpcClient;
use super::types::{NodeHealth, NodeState, SlotInfo};
use super::HealthChecker;

/// Health checker for validator nodes.
///
/// Same slot comparison as RPC, plus checks `getVoteAccounts` to
/// determine if the validator is actively voting.
pub struct ValidatorHealthChecker {
    client: RpcClient,
    slots_behind_threshold: u64,
}

impl ValidatorHealthChecker {
    pub fn new(client: RpcClient, slots_behind_threshold: u64) -> Self {
        Self {
            client,
            slots_behind_threshold,
        }
    }
}

#[async_trait]
impl HealthChecker for ValidatorHealthChecker {
    async fn check(&self) -> PillarResult<NodeHealth> {
        let cmp = self.client.compare_slots().await;
        let is_voting = self.check_voting().await;
        let cluster_version = self.client.get_reference_version().await;

        let state = determine_validator_state(
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

impl ValidatorHealthChecker {
    async fn check_voting(&self) -> bool {
        match self.client.get_vote_accounts().await {
            Ok((current, _delinquent)) => current > 0,
            Err(e) => {
                tracing::warn!(error = %e, "failed to check vote accounts");
                false
            }
        }
    }
}

fn determine_validator_state(
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
            Some(_) => NodeState::Behind, // caught up on slots but not voting yet
            None => NodeState::StartingUp,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_local_slot_is_off() {
        assert_eq!(determine_validator_state(None, None, false, 100), NodeState::Off);
    }

    #[test]
    fn far_behind_is_behind() {
        assert_eq!(
            determine_validator_state(Some(1000), Some(200), false, 100),
            NodeState::Behind
        );
    }

    #[test]
    fn caught_up_and_voting_is_healthy() {
        assert_eq!(
            determine_validator_state(Some(1000), Some(10), true, 100),
            NodeState::Healthy
        );
    }

    #[test]
    fn caught_up_but_not_voting_is_behind() {
        assert_eq!(
            determine_validator_state(Some(1000), Some(10), false, 100),
            NodeState::Behind
        );
    }

    #[test]
    fn no_reference_is_starting_up() {
        assert_eq!(
            determine_validator_state(Some(1000), None, false, 100),
            NodeState::StartingUp
        );
    }
}
