use std::path::PathBuf;

use pillar_shared::proto::{ProvisionCommand, UpgradeCommand};

/// Sent from gRPC handler to the reconcile loop via mpsc channel.
pub enum AgentCommand {
    Restart {
        reason: String,
    },
    Recover {
        reason: String,
    },
    Stop {
        reason: String,
    },
    Provision {
        staged_binary_path: PathBuf,
        config: Box<ProvisionCommand>,
    },
    Upgrade {
        staged_binary_path: PathBuf,
        upgrade: UpgradeCommand,
    },
}

impl AgentCommand {
    pub fn command_type(&self) -> &'static str {
        match self {
            AgentCommand::Restart { .. } => "restart",
            AgentCommand::Recover { .. } => "recover",
            AgentCommand::Stop { .. } => "stop",
            AgentCommand::Provision { .. } => "provision",
            AgentCommand::Upgrade { .. } => "upgrade",
        }
    }
}
