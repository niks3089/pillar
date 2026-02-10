pub mod rpc_client;
pub mod rpc_health;
pub mod types;
pub mod validator_health;

use async_trait::async_trait;

use crate::config::{HealthConfig, NetworkConfig};
use crate::error::PillarResult;
use crate::role::NodeRole;

pub use types::{NodeHealth, NodeState};

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
    let client = rpc_client::RpcClient::new(
        health_config.local_rpc_url.clone(),
        network_config.reference_rpc_urls.clone(),
        health_config.rpc_timeout_secs,
    );

    match role {
        NodeRole::Rpc | NodeRole::Grpc => Box::new(rpc_health::RpcHealthChecker::new(
            client,
            health_config.slots_behind_threshold,
        )),
        NodeRole::Validator => Box::new(validator_health::ValidatorHealthChecker::new(
            client,
            health_config.slots_behind_threshold,
        )),
    }
}
