use dashmap::DashMap;
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc};

use pillar_shared::proto::{ControllerCommand, LogEntry, NodeStatus};

struct NodeEntry {
    status: Option<NodeStatus>,
    command_tx: Option<mpsc::Sender<ControllerCommand>>,
    log_tx: broadcast::Sender<LogEntry>,
}

#[derive(Clone)]
pub struct NodeRegistry {
    nodes: Arc<DashMap<String, NodeEntry>>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(DashMap::new()),
        }
    }

    /// Insert a node entry if it doesn't already exist.
    pub async fn register_node(&self, node_id: &str) {
        self.nodes.entry(node_id.to_string()).or_insert_with(|| {
            let (log_tx, _) = broadcast::channel(1024);
            NodeEntry {
                status: None,
                command_tx: None,
                log_tx,
            }
        });
    }

    /// Update the stored status for a node.
    pub async fn update_status(&self, node_id: &str, status: NodeStatus) {
        if let Some(mut entry) = self.nodes.get_mut(node_id) {
            entry.status = Some(status);
        }
    }

    /// Clone and return the current status for a node.
    pub async fn get_status(&self, node_id: &str) -> Option<NodeStatus> {
        self.nodes.get(node_id).and_then(|e| e.status.clone())
    }

    /// Return all (node_id, status) pairs where status is present.
    pub async fn get_all_statuses(&self) -> Vec<(String, NodeStatus)> {
        self.nodes
            .iter()
            .filter_map(|entry| {
                entry
                    .value()
                    .status
                    .as_ref()
                    .map(|s| (entry.key().clone(), s.clone()))
            })
            .collect()
    }

    /// Remove a node from the registry.
    pub async fn remove_node(&self, node_id: &str) {
        self.nodes.remove(node_id);
    }

    /// Create an mpsc command channel for the node. If the node doesn't exist,
    /// it is registered first. Returns the receiver end.
    pub async fn create_command_channel(
        &self,
        node_id: &str,
    ) -> mpsc::Receiver<ControllerCommand> {
        self.register_node(node_id).await;

        let (tx, rx) = mpsc::channel(32);
        if let Some(mut entry) = self.nodes.get_mut(node_id) {
            entry.command_tx = Some(tx);
        }
        rx
    }

    /// Send a command to a node via the stored mpsc sender.
    /// Clones the sender so we don't hold the DashMap ref across the await.
    pub async fn send_command(
        &self,
        node_id: &str,
        cmd: ControllerCommand,
    ) -> Result<(), String> {
        let tx = {
            let entry = self
                .nodes
                .get(node_id)
                .ok_or_else(|| format!("node not found: {node_id}"))?;
            entry
                .command_tx
                .as_ref()
                .ok_or_else(|| format!("node not connected: {node_id}"))?
                .clone()
        };
        tx.send(cmd)
            .await
            .map_err(|e| format!("send failed: {e}"))
    }

    /// Subscribe to the log broadcast channel for a node.
    pub async fn get_log_subscriber(
        &self,
        node_id: &str,
    ) -> Option<broadcast::Receiver<LogEntry>> {
        self.nodes.get(node_id).map(|e| e.log_tx.subscribe())
    }

    /// Publish log entries to the node's broadcast channel.
    /// Slow subscribers will miss messages (errors are ignored).
    pub async fn publish_logs(&self, node_id: &str, entries: &[LogEntry]) {
        if let Some(entry) = self.nodes.get(node_id) {
            for log in entries {
                let _ = entry.log_tx.send(log.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pillar_shared::proto::controller_command;

    fn sample_status(state: &str) -> NodeStatus {
        NodeStatus {
            state: state.to_string(),
            healthy: state == "healthy",
            ..Default::default()
        }
    }

    fn restart_command() -> ControllerCommand {
        ControllerCommand {
            command: Some(controller_command::Command::Restart(
                pillar_shared::proto::RestartCommand {
                    reason: "test".to_string(),
                },
            )),
        }
    }

    #[tokio::test]
    async fn register_and_get_status() {
        let reg = NodeRegistry::new();
        reg.register_node("node-1").await;

        assert!(reg.get_status("node-1").await.is_none());

        reg.update_status("node-1", sample_status("healthy")).await;
        let s = reg.get_status("node-1").await.unwrap();
        assert_eq!(s.state, "healthy");
        assert!(s.healthy);
    }

    #[tokio::test]
    async fn register_is_idempotent() {
        let reg = NodeRegistry::new();
        reg.register_node("node-1").await;
        reg.update_status("node-1", sample_status("healthy")).await;

        // Re-registering should not overwrite the existing entry.
        reg.register_node("node-1").await;
        let s = reg.get_status("node-1").await.unwrap();
        assert_eq!(s.state, "healthy");
    }

    #[tokio::test]
    async fn get_all_statuses_filters_none() {
        let reg = NodeRegistry::new();
        reg.register_node("a").await;
        reg.register_node("b").await;
        reg.update_status("a", sample_status("healthy")).await;

        let all = reg.get_all_statuses().await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "a");
    }

    #[tokio::test]
    async fn remove_node_cleans_up() {
        let reg = NodeRegistry::new();
        reg.register_node("node-1").await;
        reg.update_status("node-1", sample_status("healthy")).await;
        reg.remove_node("node-1").await;

        assert!(reg.get_status("node-1").await.is_none());
        assert!(reg.get_all_statuses().await.is_empty());
    }

    #[tokio::test]
    async fn command_channel_roundtrip() {
        let reg = NodeRegistry::new();
        let mut rx = reg.create_command_channel("node-1").await;

        let cmd = restart_command();
        reg.send_command("node-1", cmd).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert!(matches!(
            received.command,
            Some(controller_command::Command::Restart(_))
        ));
    }

    #[tokio::test]
    async fn send_command_missing_node() {
        let reg = NodeRegistry::new();
        let result = reg.send_command("nonexistent", restart_command()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn send_command_no_channel() {
        let reg = NodeRegistry::new();
        reg.register_node("node-1").await;
        let result = reg.send_command("node-1", restart_command()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not connected"));
    }

    #[tokio::test]
    async fn log_pubsub() {
        let reg = NodeRegistry::new();
        reg.register_node("node-1").await;

        let mut sub = reg.get_log_subscriber("node-1").await.unwrap();

        let entries = vec![
            LogEntry {
                service: "validator".to_string(),
                level: "info".to_string(),
                message: "hello".to_string(),
                ..Default::default()
            },
            LogEntry {
                service: "operator".to_string(),
                level: "warn".to_string(),
                message: "world".to_string(),
                ..Default::default()
            },
        ];

        reg.publish_logs("node-1", &entries).await;

        let log1 = sub.recv().await.unwrap();
        assert_eq!(log1.service, "validator");
        assert_eq!(log1.message, "hello");

        let log2 = sub.recv().await.unwrap();
        assert_eq!(log2.service, "operator");
        assert_eq!(log2.message, "world");
    }

    #[tokio::test]
    async fn log_subscriber_missing_node() {
        let reg = NodeRegistry::new();
        assert!(reg.get_log_subscriber("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn create_command_channel_auto_registers() {
        let reg = NodeRegistry::new();
        // Node doesn't exist yet — create_command_channel should register it.
        let _rx = reg.create_command_channel("auto-node").await;
        // Node should now exist in registry.
        reg.update_status("auto-node", sample_status("behind"))
            .await;
        let s = reg.get_status("auto-node").await.unwrap();
        assert_eq!(s.state, "behind");
    }
}
