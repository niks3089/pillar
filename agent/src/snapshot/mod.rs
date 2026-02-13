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

/// Parse slot number from a snapshot filename.
/// Solana snapshots follow the pattern: snapshot-<slot>-<hash>.tar.zst
pub fn parse_slot_from_filename(filename: &str) -> Option<u64> {
    let name = filename.strip_prefix("snapshot-")?;
    let slot_str = name.split('-').next()?;
    slot_str.parse().ok()
}

/// Scan a directory for snapshot files and return the highest slot found.
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
        if let Some(slot) = parse_slot_from_filename(&name) {
            highest = Some(highest.map_or(slot, |h: u64| h.max(slot)));
        }
    }

    Ok(highest)
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
    fn parse_incremental_ignored() {
        assert_eq!(
            parse_slot_from_filename("incremental-snapshot-100-200-abcdef.tar.zst"),
            None
        );
    }
}
