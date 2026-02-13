pub mod types;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/pillar.rs"));
}

use std::path::Path;

use prost::Message;

/// Read operator state from a binary proto file on disk.
///
/// Returns `Ok(None)` if the file does not exist (operator hasn't written yet).
/// The file is small and written atomically by operator (write-tmp + rename),
/// so a sync read is safe and avoids async overhead.
pub fn read_state(path: &Path) -> Result<Option<proto::NodeStatus>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let status =
                proto::NodeStatus::decode(bytes.as_slice()).map_err(|e| format!("decode error: {e}"))?;
            Ok(Some(status))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read error: {e}")),
    }
}

/// Write operator state atomically to a binary proto file.
/// Uses write-to-temp + rename to avoid partial reads.
pub fn write_state(status: &proto::NodeStatus, path: &Path) -> Result<(), String> {
    let bytes = status.encode_to_vec();
    let tmp = path.with_extension("tmp");

    std::fs::write(&tmp, &bytes).map_err(|e| format!("write error: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename error: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn sample_status() -> proto::NodeStatus {
        proto::NodeStatus {
            state: "healthy".to_string(),
            local_slot: 1000,
            reference_slot: 1005,
            slots_behind: 5,
            healthy: true,
            restart_count: 2,
            crash_looping: false,
            health_check_duration_secs: 0.5,
            version: "0.1.0".to_string(),
            role: "rpc".to_string(),
            client: "agave".to_string(),
            cluster: "mainnet".to_string(),
            updated_at_unix_secs: chrono::Utc::now().timestamp(),
            state_duration_secs: 120,
            validator_process: "agave-validator".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn read_state_missing_file() {
        let result = read_state(Path::new("/tmp/pillar-test-nonexistent.bin"));
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn read_state_valid_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let status = sample_status();
        write_state(&status, tmp.path()).unwrap();

        let result = read_state(tmp.path()).unwrap().unwrap();
        assert_eq!(result.state, "healthy");
        assert_eq!(result.restart_count, 2);
        assert_eq!(result.cluster, "mainnet");
        assert_eq!(result.validator_process, "agave-validator");
    }

    #[test]
    fn read_state_malformed_data() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        // Write invalid protobuf (a field tag with wiretype 7 which is invalid)
        tmp.write_all(&[0xFF, 0xFF, 0xFF]).unwrap();

        let result = read_state(tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("decode error"));
    }

    #[test]
    fn state_roundtrip() {
        let status = sample_status();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.bin");
        write_state(&status, &path).unwrap();

        let decoded = read_state(&path).unwrap().unwrap();
        assert_eq!(decoded.state, status.state);
        assert_eq!(decoded.restart_count, status.restart_count);
        assert_eq!(decoded.validator_process, "agave-validator");
        assert_eq!(decoded.local_slot, 1000);
        assert_eq!(decoded.reference_slot, 1005);
    }

    #[test]
    fn node_state_display() {
        assert_eq!(types::NodeState::Off.to_string(), "off");
        assert_eq!(types::NodeState::StartingUp.to_string(), "starting_up");
        assert_eq!(types::NodeState::Behind.to_string(), "behind");
        assert_eq!(types::NodeState::Healthy.to_string(), "healthy");
        assert_eq!(types::NodeState::Recovering.to_string(), "recovering");
    }

    #[test]
    fn node_state_as_str() {
        assert_eq!(types::NodeState::Off.as_str(), "off");
        assert_eq!(types::NodeState::Healthy.as_str(), "healthy");
    }
}
