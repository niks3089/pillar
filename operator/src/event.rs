use crate::health::NodeState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorEvent {
    pub timestamp: DateTime<Utc>,
    pub kind: EventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    StateTransition {
        from: NodeState,
        to: NodeState,
    },
    ServiceRestarted {
        reason: String,
    },
    SnapshotDownloadStarted,
    SnapshotDownloadCompleted {
        duration_secs: f64,
    },
    CrashLoopDetected {
        restarts_in_window: usize,
    },
}
