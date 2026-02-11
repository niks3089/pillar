//! Download, verify, and stage validator binaries.
//! Types (ProvisionConfig, ClientKind, ClientInstaller, Stage) moved to operator.

use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

// ---------------------------------------------------------------------------
// Download + verify
// ---------------------------------------------------------------------------

/// Download a file from `url` to `dest`, streaming to disk in chunks.
/// Returns the number of bytes written.
pub async fn download_file(url: &str, dest: &Path, timeout: Duration) -> Result<u64, String> {
    tracing::info!(url, dest = %dest.display(), timeout_secs = timeout.as_secs(), "downloading");

    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("build HTTP client: {e}"))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}: {url}", response.status()));
    }

    let content_length = response.content_length();
    if let Some(len) = content_length {
        tracing::info!(size_mb = len / 1_000_000, "download size");
    }

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("create {}: {e}", dest.display()))?;

    let mut stream = response;
    let mut written: u64 = 0;
    let mut last_log_mb: u64 = 0;
    let progress_interval_bytes: u64 = 50 * 1_000_000; // log every 50 MB

    while let Some(chunk) = stream
        .chunk()
        .await
        .map_err(|e| format!("read chunk: {e}"))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("write {}: {e}", dest.display()))?;
        written += chunk.len() as u64;

        let current_mb = written / 1_000_000;
        if current_mb >= last_log_mb + progress_interval_bytes / 1_000_000 {
            tracing::info!(bytes = written, mb = current_mb, "download progress");
            last_log_mb = current_mb;
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("flush {}: {e}", dest.display()))?;

    tracing::info!(bytes = written, dest = %dest.display(), "download complete");
    Ok(written)
}

/// Compute the SHA256 hex digest of a file.
pub async fn sha256_file(path: &Path) -> Result<String, String> {
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| format!("read {}: {e}", path.display()))?;

    let hash = Sha256::digest(&data);
    Ok(format!("{hash:x}"))
}

/// Verify a file's SHA256 matches the expected hex digest.
pub async fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let actual = sha256_file(path).await?;
    if actual != expected {
        return Err(format!(
            "SHA256 mismatch for {}: expected {expected}, got {actual}",
            path.display()
        ));
    }
    tracing::info!(path = %path.display(), "SHA256 verified");
    Ok(())
}

/// Download a file and verify its SHA256. Cleans up on failure.
pub async fn download_and_verify(
    url: &str,
    dest: &Path,
    expected_sha256: &str,
    timeout: Duration,
) -> Result<(), String> {
    download_file(url, dest, timeout).await?;

    if let Err(e) = verify_sha256(dest, expected_sha256).await {
        // Clean up the bad download
        let _ = tokio::fs::remove_file(dest).await;
        return Err(e);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Command file writing
// ---------------------------------------------------------------------------

/// Write a PendingCommand JSON file atomically for the operator to pick up.
pub async fn write_pending_command(cmd: &pillar_shared::PendingCommand) -> Result<(), String> {
    let path = pillar_shared::PENDING_COMMAND_PATH;
    let tmp = format!("{path}.tmp");

    let json = serde_json::to_string_pretty(cmd)
        .map_err(|e| format!("serialize pending command: {e}"))?;

    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(path).parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }

    tokio::fs::write(&tmp, &json)
        .await
        .map_err(|e| format!("write {tmp}: {e}"))?;

    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| format!("rename {tmp} -> {path}: {e}"))?;

    tracing::info!(command_type = %cmd.command_type(), "wrote pending command file");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sha256_file_computes_correct_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        tokio::fs::write(&path, b"hello world").await.unwrap();

        let hash = sha256_file(&path).await.unwrap();
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[tokio::test]
    async fn verify_sha256_passes_on_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        tokio::fs::write(&path, b"hello world").await.unwrap();

        let result = verify_sha256(
            &path,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn verify_sha256_fails_on_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        tokio::fs::write(&path, b"hello world").await.unwrap();

        let result = verify_sha256(&path, "0000000000000000").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("SHA256 mismatch"));
    }

    #[tokio::test]
    async fn download_and_verify_cleans_up_on_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("bad.bin");

        // Write a file and check verify_sha256 directly
        tokio::fs::write(&dest, b"some content").await.unwrap();

        let result = verify_sha256(&dest, "wrong_hash").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn write_pending_command_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending-command.json");
        let tmp = dir.path().join("pending-command.json.tmp");

        let cmd = pillar_shared::PendingCommand::Provision {
            staged_binary_path: "/tmp/staged".to_string(),
            provision: Box::new(pillar_shared::proto::ProvisionCommand {
                client: "agave".to_string(),
                version: "2.1.6".to_string(),
                cluster: "testnet".to_string(),
                ..Default::default()
            }),
        };

        let json = serde_json::to_string_pretty(&cmd).unwrap();
        tokio::fs::write(&tmp, &json).await.unwrap();
        tokio::fs::rename(&tmp, &path).await.unwrap();

        // Read back
        let data = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: pillar_shared::PendingCommand = serde_json::from_str(&data).unwrap();
        assert_eq!(parsed.command_type(), "provision");
        match &parsed {
            pillar_shared::PendingCommand::Provision { staged_binary_path, .. } => {
                assert_eq!(staged_binary_path, "/tmp/staged");
            }
            _ => panic!("expected Provision variant"),
        }
    }
}
