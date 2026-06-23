use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpStream;

use crate::error::{PillarError, PillarResult};

use super::{scan_snapshot_dir, scan_snapshot_slots};

/// RAII guard that resets the downloading flag on drop, even if a panic occurs.
struct DownloadGuard<'a> {
    flag: &'a AtomicBool,
}

impl<'a> DownloadGuard<'a> {
    fn new(flag: &'a AtomicBool) -> Self {
        flag.store(true, Ordering::SeqCst);
        Self { flag }
    }
}

impl Drop for DownloadGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

/// Pure-Rust TCP snapshot download.
///
/// Connects to a snapshot server and reads a simple wire protocol:
///   [8 bytes: file_size (u64 LE)] [4 bytes: name_len (u32 LE)] [name] [file data]
///
pub struct TcpSnapshotManager {
    server_hostname: String,
    snapshot_dir: PathBuf,
    full_snapshot_port: u16,
    incremental_snapshot_port: u16,
    min_speed_mbs: f64,
    downloading: AtomicBool,
}

impl TcpSnapshotManager {
    pub fn new(server_hostname: String, snapshot_dir: PathBuf) -> Self {
        Self {
            server_hostname,
            snapshot_dir,
            full_snapshot_port: 10003,
            incremental_snapshot_port: 10004,
            min_speed_mbs: 500.0,
            downloading: AtomicBool::new(false),
        }
    }

    #[allow(dead_code)]
    pub fn with_ports(mut self, full: u16, incremental: u16) -> Self {
        self.full_snapshot_port = full;
        self.incremental_snapshot_port = incremental;
        self
    }

    #[allow(dead_code)]
    pub fn with_min_speed_mbs(mut self, mbs: f64) -> Self {
        self.min_speed_mbs = mbs;
        self
    }

    pub async fn download_snapshot(&self) -> PillarResult<()> {
        if self.server_hostname.is_empty() {
            return Err(PillarError::Snapshot(
                "no TCP snapshot server hostname configured".to_string(),
            ));
        }

        let _guard = DownloadGuard::new(&self.downloading);
        self.do_download().await
    }

    pub async fn highest_local_slot(&self) -> PillarResult<Option<u64>> {
        scan_snapshot_dir(&self.snapshot_dir).await
    }

    #[allow(dead_code)]
    pub fn is_downloading(&self) -> bool {
        self.downloading.load(Ordering::SeqCst)
    }

    async fn do_download(&self) -> PillarResult<()> {
        // Full snapshot is required
        receive_file(
            &self.server_hostname,
            self.full_snapshot_port,
            &self.snapshot_dir,
            self.min_speed_mbs,
            "full",
        )
        .await?;

        // Incremental is best-effort — warn on failure but don't abort
        if let Err(e) = receive_file(
            &self.server_hostname,
            self.incremental_snapshot_port,
            &self.snapshot_dir,
            self.min_speed_mbs,
            "incremental",
        )
        .await
        {
            tracing::warn!(error = %e, "incremental download failed, continuing with full only");
        }

        // Verify compatibility: incremental base_slot must match full snapshot slot
        self.check_snapshot_compatibility().await;

        Ok(())
    }

    /// Verify that the incremental snapshot's base slot matches the full snapshot slot.
    /// If they don't match, delete the incremental (validator would reject it anyway).
    async fn check_snapshot_compatibility(&self) {
        let (full_slot, incr_slots) = match scan_snapshot_slots(&self.snapshot_dir).await {
            Ok(slots) => slots,
            Err(e) => {
                tracing::warn!(error = %e, "failed to scan snapshots for compatibility check");
                return;
            }
        };

        let (Some(full), Some((incr_base, incr_end))) = (full_slot, incr_slots) else {
            return; // Nothing to check if either is missing
        };

        if incr_base != full {
            tracing::warn!(
                full_slot = full,
                incremental_base = incr_base,
                incremental_end = incr_end,
                "incremental snapshot base slot does not match full snapshot — deleting incremental"
            );
            // Find and delete the incompatible incremental file
            if let Ok(mut entries) = tokio::fs::read_dir(&self.snapshot_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("incremental-snapshot-") {
                        if let Err(e) = tokio::fs::remove_file(entry.path()).await {
                            tracing::warn!(error = %e, file = %name, "failed to delete incompatible incremental");
                        } else {
                            tracing::info!(file = %name, "deleted incompatible incremental snapshot");
                        }
                    }
                }
            }
        } else {
            tracing::info!(
                full_slot = full,
                incremental_end = incr_end,
                "snapshot compatibility check passed"
            );
        }
    }
}

/// Connect to a TCP snapshot server, read the header, stream the file, and
/// validate transfer speed.
async fn receive_file(
    hostname: &str,
    port: u16,
    dest_dir: &std::path::Path,
    min_speed_mbs: f64,
    label: &str,
) -> PillarResult<()> {
    let addr = format!("{hostname}:{port}");
    tracing::info!(addr = %addr, kind = label, "connecting to snapshot server");

    let mut stream = TcpStream::connect(&addr)
        .await
        .map_err(|e| PillarError::Snapshot(format!("connect to {addr}: {e}")))?;

    stream.set_nodelay(true).ok();

    // Read header: file_size (u64 LE) + filename_len (u32 LE) + filename
    let file_size = read_u64_le(&mut stream).await?;
    let filename = sanitize_snapshot_filename(&read_length_prefixed_string(&mut stream).await?)?;

    tracing::info!(filename = %filename, file_size, kind = label, "receiving snapshot");

    let dest = dest_dir.join(&filename);
    let file = tokio::fs::File::create(&dest)
        .await
        .map_err(|e| PillarError::Snapshot(format!("create {}: {e}", dest.display())))?;
    let mut writer = BufWriter::with_capacity(512 * 1024, file);

    let mut buf = vec![0u8; 512 * 1024]; // 512 KB read buffer
    let mut downloaded: u64 = 0;
    let start = Instant::now();
    let mut last_speed_check = start;

    loop {
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| PillarError::Snapshot(format!("read error: {e}")))?;

        if n == 0 {
            break;
        }

        writer
            .write_all(&buf[..n])
            .await
            .map_err(|e| PillarError::Snapshot(format!("write error: {e}")))?;

        downloaded += n as u64;

        // Check speed every 10 seconds after we have some data
        if last_speed_check.elapsed().as_secs() >= 10 && downloaded > 0 {
            let elapsed = start.elapsed().as_secs_f64();
            let speed_mbs = (downloaded as f64 / elapsed) / 1_000_000.0;
            let pct = if file_size > 0 {
                (downloaded as f64 / file_size as f64) * 100.0
            } else {
                0.0
            };

            tracing::debug!(
                kind = label,
                speed_mbs = format!("{speed_mbs:.0}"),
                progress_pct = format!("{pct:.1}"),
                "transfer progress"
            );

            // Only enforce minimum speed after 5% to let the connection warm up
            if pct > 5.0 && speed_mbs < min_speed_mbs {
                return Err(PillarError::Snapshot(format!(
                    "transfer too slow: {speed_mbs:.0} MB/s (min {min_speed_mbs:.0}) at {pct:.1}%"
                )));
            }

            last_speed_check = Instant::now();
        }
    }

    writer
        .flush()
        .await
        .map_err(|e| PillarError::Snapshot(format!("flush error: {e}")))?;

    let elapsed = start.elapsed().as_secs_f64();
    let speed_mbs = if elapsed > 0.0 {
        (downloaded as f64 / elapsed) / 1_000_000.0
    } else {
        0.0
    };

    tracing::info!(
        path = %dest.display(),
        downloaded,
        elapsed_secs = format!("{elapsed:.1}"),
        speed_mbs = format!("{speed_mbs:.0}"),
        kind = label,
        "TCP download complete"
    );

    Ok(())
}

async fn read_u64_le(stream: &mut TcpStream) -> PillarResult<u64> {
    let mut buf = [0u8; 8];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| PillarError::Snapshot(format!("read file size: {e}")))?;
    Ok(u64::from_le_bytes(buf))
}

/// Accept only a bare snapshot file name so a crafted value can't traverse out of
/// the destination directory (absolute paths / `..`).
fn sanitize_snapshot_filename(raw: &str) -> PillarResult<String> {
    let name = std::path::Path::new(raw)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| *n == raw)
        .filter(|n| n.starts_with("snapshot-") || n.starts_with("incremental-snapshot-"));
    match name {
        Some(n) => Ok(n.to_string()),
        None => Err(PillarError::Snapshot(format!(
            "rejected snapshot filename: {raw:?}"
        ))),
    }
}

async fn read_length_prefixed_string(stream: &mut TcpStream) -> PillarResult<String> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| PillarError::Snapshot(format!("read string length: {e}")))?;
    let len = u32::from_le_bytes(len_buf) as usize;

    const MAX_FILENAME_LEN: usize = 1024;
    if len > MAX_FILENAME_LEN {
        return Err(PillarError::Snapshot(format!(
            "filename length {len} exceeds maximum {MAX_FILENAME_LEN}"
        )));
    }

    let mut buf = vec![0u8; len];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| PillarError::Snapshot(format!("read string data: {e}")))?;

    String::from_utf8(buf).map_err(|e| PillarError::Snapshot(format!("invalid utf-8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::sanitize_snapshot_filename;

    #[test]
    fn accepts_plain_snapshot_names() {
        assert_eq!(
            sanitize_snapshot_filename("snapshot-100-abc.tar.zst").unwrap(),
            "snapshot-100-abc.tar.zst"
        );
        assert!(sanitize_snapshot_filename("incremental-snapshot-1-2-x.tar.zst").is_ok());
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        assert!(sanitize_snapshot_filename("../../etc/cron.d/x").is_err());
        assert!(sanitize_snapshot_filename("/etc/passwd").is_err());
        assert!(sanitize_snapshot_filename("snap/snapshot-1.tar.zst").is_err());
        assert!(sanitize_snapshot_filename("evil.tar.zst").is_err());
    }
}
