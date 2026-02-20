pub mod download_tcp;
pub mod recovery;
pub mod staleness;

use std::path::Path;

use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

use crate::error::{PillarError, PillarResult};

pub use download_tcp::TcpSnapshotManager;

/// Snapshot download transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, Default)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum DownloadMethod {
    #[default]
    Tcp,
}

/// Parse slot number from a full snapshot filename.
/// Solana full snapshots: `snapshot-<slot>-<hash>.tar.zst`
pub fn parse_slot_from_filename(filename: &str) -> Option<u64> {
    let name = filename.strip_prefix("snapshot-")?;
    let slot_str = name.split('-').next()?;
    slot_str.parse().ok()
}

/// Parse base and ending slot from an incremental snapshot filename.
/// Solana incremental snapshots: `incremental-snapshot-<base_slot>-<end_slot>-<hash>.tar.zst`
pub fn parse_incremental_slots(filename: &str) -> Option<(u64, u64)> {
    let name = filename.strip_prefix("incremental-snapshot-")?;
    let mut parts = name.split('-');
    let base_slot: u64 = parts.next()?.parse().ok()?;
    let end_slot: u64 = parts.next()?.parse().ok()?;
    Some((base_slot, end_slot))
}

/// Scan a directory for snapshot files and return the highest slot found,
/// considering both full and incremental snapshots (like rpc-operator).
pub async fn scan_snapshot_dir(dir: &Path) -> PillarResult<Option<u64>> {
    if !dir.exists() {
        return Ok(None);
    }

    let mut highest: Option<u64> = None;
    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| PillarError::Snapshot(format!("failed to read {}: {e}", dir.display())))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| PillarError::Snapshot(format!("failed to read dir entry: {e}")))?
    {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        // Check full snapshot: snapshot-<slot>-<hash>.tar.zst
        if let Some(slot) = parse_slot_from_filename(&name) {
            highest = Some(highest.map_or(slot, |h: u64| h.max(slot)));
        }

        // Check incremental snapshot: incremental-snapshot-<base>-<end>-<hash>.tar.zst
        // Use the ending slot (which is always >= base slot)
        if let Some((_base, end_slot)) = parse_incremental_slots(&name) {
            highest = Some(highest.map_or(end_slot, |h: u64| h.max(end_slot)));
        }
    }

    Ok(highest)
}

/// Scan a directory and return (highest_full_slot, highest_incremental_end_slot) separately.
/// Used for compatibility checking after download.
pub async fn scan_snapshot_slots(dir: &Path) -> PillarResult<(Option<u64>, Option<(u64, u64)>)> {
    if !dir.exists() {
        return Ok((None, None));
    }

    let mut highest_full: Option<u64> = None;
    let mut highest_incr: Option<(u64, u64)> = None;

    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| PillarError::Snapshot(format!("failed to read {}: {e}", dir.display())))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| PillarError::Snapshot(format!("failed to read dir entry: {e}")))?
    {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if let Some(slot) = parse_slot_from_filename(&name) {
            highest_full = Some(highest_full.map_or(slot, |h: u64| h.max(slot)));
        }

        if let Some((base, end)) = parse_incremental_slots(&name) {
            let better = highest_incr.map_or(true, |(_, prev_end)| end > prev_end);
            if better {
                highest_incr = Some((base, end));
            }
        }
    }

    Ok((highest_full, highest_incr))
}

/// Remove all contents of a directory without removing the directory itself.
pub async fn wipe_directory(dir: &Path) -> PillarResult<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_snapshot() {
        assert_eq!(
            parse_slot_from_filename("snapshot-123456789-abcdef.tar.zst"),
            Some(123456789)
        );
    }

    #[test]
    fn parse_non_snapshot() {
        assert_eq!(parse_slot_from_filename("random-file.txt"), None);
    }

    #[test]
    fn parse_incremental_ignored_by_full_parser() {
        assert_eq!(
            parse_slot_from_filename("incremental-snapshot-100-200-abcdef.tar.zst"),
            None
        );
    }

    #[test]
    fn parse_incremental_basic() {
        assert_eq!(
            parse_incremental_slots("incremental-snapshot-100-200-abcdef.tar.zst"),
            Some((100, 200))
        );
    }

    #[test]
    fn parse_incremental_large_slots() {
        assert_eq!(
            parse_incremental_slots("incremental-snapshot-389727635-389731639-GtyN2j3M.tar.zst"),
            Some((389727635, 389731639))
        );
    }

    #[test]
    fn parse_incremental_not_matching() {
        assert_eq!(parse_incremental_slots("snapshot-100-abcdef.tar.zst"), None);
        assert_eq!(parse_incremental_slots("random-file.txt"), None);
    }
}
