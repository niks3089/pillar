use pillar_shared::proto::ExecuteScript;

/// Sent from gRPC handler to the reconcile loop via mpsc channel.
pub enum AgentCommand {
    ExecuteScript(ExecuteScript),
}
