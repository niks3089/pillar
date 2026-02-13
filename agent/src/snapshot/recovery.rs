use std::path::Path;

use crate::error::{PillarError, PillarResult};
use crate::lifecycle::SystemdManager;

use super::download_tcp::TcpSnapshotManager;
use super::staleness::is_stale;

/// Orchestrates the full recovery flow: stop → wipe ledger → download snapshot → restart.
pub struct SnapshotRecovery<'a> {
    service: &'a SystemdManager,
    snapshots: &'a TcpSnapshotManager,
    ledger_dir: &'a Path,
    staleness_threshold: u64,
}

impl<'a> SnapshotRecovery<'a> {
    pub fn new(
        service: &'a SystemdManager,
        snapshots: &'a TcpSnapshotManager,
        ledger_dir: &'a Path,
        staleness_threshold: u64,
    ) -> Self {
        Self {
            service,
            snapshots,
            ledger_dir,
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

        // 2. Wipe ledger
        tracing::info!(dir = %self.ledger_dir.display(), "wiping ledger directory");
        wipe_directory(self.ledger_dir).await?;

        // 3. Download fresh snapshot
        tracing::info!("downloading fresh snapshot");
        self.snapshots.download_snapshot().await?;

        // 4. Restart the validator
        tracing::info!("restarting validator after recovery");
        self.service.start().await?;

        tracing::info!("recovery complete");
        Ok(())
    }
}

/// Remove all contents of a directory without removing the directory itself.
async fn wipe_directory(dir: &Path) -> PillarResult<()> {
    if !dir.exists() {
        return Ok(());
    }

    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| PillarError::Snapshot(format!("failed to read {}: {e}", dir.display())))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| PillarError::Snapshot(format!("failed to read dir entry: {e}")))?
    {
        let path = entry.path();
        if path.is_dir() {
            tokio::fs::remove_dir_all(&path).await.map_err(|e| {
                PillarError::Snapshot(format!("failed to remove {}: {e}", path.display()))
            })?;
        } else {
            tokio::fs::remove_file(&path).await.map_err(|e| {
                PillarError::Snapshot(format!("failed to remove {}: {e}", path.display()))
            })?;
        }
    }

    Ok(())
}
