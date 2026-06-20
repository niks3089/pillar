use std::path::Path;

use crate::error::PillarResult;
use crate::lifecycle::SystemdManager;

use super::download_tcp::TcpSnapshotManager;
use super::staleness::is_stale;
use super::wipe_directory;

/// Orchestrates the full recovery flow:
///   stop → wipe ledger + accounts → download snapshot → restart.
///
/// When no TCP snapshot server is configured, the download step is skipped
/// and the validator is restarted to bootstrap via gossip (native peer
/// discovery + snapshot download from the network).
pub struct SnapshotRecovery<'a> {
    service: &'a SystemdManager,
    snapshots: &'a TcpSnapshotManager,
    ledger_dir: &'a Path,
    accounts_dir: &'a Path,
    snapshot_dir: &'a Path,
    staleness_threshold: u64,
}

impl<'a> SnapshotRecovery<'a> {
    pub fn new(
        service: &'a SystemdManager,
        snapshots: &'a TcpSnapshotManager,
        ledger_dir: &'a Path,
        accounts_dir: &'a Path,
        snapshot_dir: &'a Path,
        staleness_threshold: u64,
    ) -> Self {
        Self {
            service,
            snapshots,
            ledger_dir,
            accounts_dir,
            snapshot_dir,
            staleness_threshold,
        }
    }

    /// Run recovery if the node is stale relative to the reference slot.
    /// Returns `true` if recovery was performed, `false` if not needed.
    pub async fn recover_if_stale(&self, reference_slot: u64) -> PillarResult<bool> {
        let local_slot = self.snapshots.highest_local_slot().await?;

        if !is_stale(local_slot, reference_slot, self.staleness_threshold) {
            tracing::info!(
                local_slot = ?local_slot,
                reference_slot,
                threshold = self.staleness_threshold,
                "node is not stale, skipping recovery"
            );
            return Ok(false);
        }

        tracing::warn!(
            local_slot = ?local_slot,
            reference_slot,
            threshold = self.staleness_threshold,
            "node is stale, starting recovery"
        );

        self.do_recovery().await?;
        Ok(true)
    }

    /// Force a full recovery regardless of staleness.
    #[allow(dead_code)]
    pub async fn force_recovery(&self) -> PillarResult<()> {
        tracing::warn!("forced recovery requested");
        self.do_recovery().await
    }

    async fn do_recovery(&self) -> PillarResult<()> {
        // 1. Stop the validator
        tracing::info!("stopping validator for recovery");
        self.service.stop().await?;

        // 2. Wipe ledger and accounts (like rpc-operator's WIPE_ACCOUNTS_AND_LEDGER)
        tracing::info!(dir = %self.ledger_dir.display(), "wiping ledger directory");
        wipe_directory(self.ledger_dir).await?;

        tracing::info!(dir = %self.accounts_dir.display(), "wiping accounts directory");
        wipe_directory(self.accounts_dir).await?;

        // 3. Wipe stale snapshots so validator downloads fresh ones
        tracing::info!(dir = %self.snapshot_dir.display(), "wiping snapshot directory");
        wipe_directory(self.snapshot_dir).await?;

        // 4. Try TCP snapshot download if a server is configured.
        //    If no server configured, skip — the validator will bootstrap via
        //    native gossip-based peer discovery and download from the network.
        match self.snapshots.download_snapshot().await {
            Ok(()) => {
                tracing::info!("TCP snapshot download complete");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "snapshot download failed — validator will bootstrap via gossip"
                );
            }
        }

        // 5. Restart the validator
        tracing::info!("restarting validator after recovery");
        self.service.start().await?;

        tracing::info!("recovery complete");
        Ok(())
    }
}
