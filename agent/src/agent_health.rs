use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Atomic counters for agent self-health metrics.
/// Shared across async tasks via `Arc<AgentHealth>`, updated with `Relaxed` ordering.
pub struct AgentHealth {
    pub controller_connected: AtomicBool,
    pub controller_latency_ms: AtomicU64,
    pub status_reports_sent: AtomicU64,
    pub status_reports_failed: AtomicU64,
    pub log_batches_dropped: AtomicU64,
    pub commands_received: AtomicU64,
}

impl AgentHealth {
    pub fn new() -> Self {
        Self {
            controller_connected: AtomicBool::new(false),
            controller_latency_ms: AtomicU64::new(0),
            status_reports_sent: AtomicU64::new(0),
            status_reports_failed: AtomicU64::new(0),
            log_batches_dropped: AtomicU64::new(0),
            commands_received: AtomicU64::new(0),
        }
    }

    pub fn inc_status_reports_sent(&self) {
        self.status_reports_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_status_reports_failed(&self) {
        self.status_reports_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_log_batches_dropped(&self) {
        self.log_batches_dropped.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_commands_received(&self) {
        self.commands_received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_controller_connected(&self, connected: bool) {
        self.controller_connected.store(connected, Ordering::Relaxed);
    }

    pub fn set_controller_latency_ms(&self, ms: u64) {
        self.controller_latency_ms.store(ms, Ordering::Relaxed);
    }
}
