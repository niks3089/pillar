use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum NodeState {
    #[default]
    Off,
    StartingUp,
    Behind,
    Healthy,
    Recovering,
}

impl NodeState {
    pub fn as_str(&self) -> &str {
        match self {
            NodeState::Off => "off",
            NodeState::StartingUp => "starting_up",
            NodeState::Behind => "behind",
            NodeState::Healthy => "healthy",
            NodeState::Recovering => "recovering",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SlotInfo {
    pub local_slot: Option<u64>,
    pub reference_slot: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeHealth {
    pub state: NodeState,
    pub slot_info: SlotInfo,
    pub slots_behind: Option<i64>,
    pub cluster_version: Option<String>,
}
