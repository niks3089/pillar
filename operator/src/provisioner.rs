//! Validator provisioner — handles installing and configuring a validator
//! binary on the node, triggered by a PendingCommand from Link.

use std::path::{Path, PathBuf};

use shared::proto::ProvisionCommand;

/// Parsed, validated provision config derived from the proto command.
#[derive(Debug, Clone)]
pub struct ProvisionConfig {
    pub client: ClientKind,
    pub version: String,
    #[allow(dead_code)]
    pub cluster: String,
    pub identity_keypair_path: PathBuf,
    pub vote_account_keypair_path: PathBuf,
    pub ledger_path: PathBuf,
    pub snapshot_path: PathBuf,
    pub accounts_path: PathBuf,
    pub entrypoints: Vec<String>,
    pub known_validators: Vec<String>,
    #[allow(dead_code)]
    pub download_url: String,
    #[allow(dead_code)]
    pub sha256: String,
    pub jito_mev: bool,
    pub jito_block_engine_url: String,
    pub yellowstone_grpc: bool,
    pub rpc_port: u32,
    pub dynamic_port_range: String,
}

impl ProvisionConfig {
    /// Parse and validate a ProvisionCommand into a ProvisionConfig.
    pub fn from_command(cmd: &ProvisionCommand) -> Result<Self, String> {
        let client = ClientKind::parse(&cmd.client)?;

        if cmd.version.is_empty() {
            return Err("version is required".to_string());
        }
        if cmd.cluster.is_empty() {
            return Err("cluster is required".to_string());
        }
        if cmd.download_url.is_empty() {
            return Err("download_url is required".to_string());
        }
        if cmd.sha256.is_empty() {
            return Err("sha256 is required".to_string());
        }
        if cmd.identity_keypair_path.is_empty() {
            return Err("identity_keypair_path is required".to_string());
        }

        Ok(Self {
            client,
            version: cmd.version.clone(),
            cluster: cmd.cluster.clone(),
            identity_keypair_path: PathBuf::from(&cmd.identity_keypair_path),
            vote_account_keypair_path: PathBuf::from(&cmd.vote_account_keypair_path),
            ledger_path: PathBuf::from(&cmd.ledger_path),
            snapshot_path: PathBuf::from(&cmd.snapshot_path),
            accounts_path: PathBuf::from(&cmd.accounts_path),
            entrypoints: cmd.entrypoints.clone(),
            known_validators: cmd.known_validators.clone(),
            download_url: cmd.download_url.clone(),
            sha256: cmd.sha256.clone(),
            jito_mev: cmd.jito_mev,
            jito_block_engine_url: cmd.jito_block_engine_url.clone(),
            yellowstone_grpc: cmd.yellowstone_grpc,
            rpc_port: if cmd.rpc_port == 0 { 8899 } else { cmd.rpc_port },
            dynamic_port_range: if cmd.dynamic_port_range.is_empty() {
                "8000-8020".to_string()
            } else {
                cmd.dynamic_port_range.clone()
            },
        })
    }
}

/// Supported validator clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Agave,
    Jito,
    Firedancer,
    Frankendancer,
}

impl ClientKind {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "agave" => Ok(Self::Agave),
            "jito" => Ok(Self::Jito),
            "firedancer" => Ok(Self::Firedancer),
            "frankendancer" => Ok(Self::Frankendancer),
            other => Err(format!("unknown client: {other}")),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Agave => "agave",
            Self::Jito => "jito",
            Self::Firedancer => "firedancer",
            Self::Frankendancer => "frankendancer",
        }
    }
}

/// Client-specific installation details.
pub struct ClientInstaller {
    /// The systemd service name for this validator.
    pub service_name: &'static str,
    /// Where to install the binary.
    pub binary_path: PathBuf,
}

impl ClientInstaller {
    pub fn for_client(kind: ClientKind) -> Self {
        match kind {
            ClientKind::Agave => Self {
                service_name: "solana-validator",
                binary_path: PathBuf::from("/usr/local/bin/agave-validator"),
            },
            ClientKind::Jito => Self {
                service_name: "jito-validator",
                binary_path: PathBuf::from("/usr/local/bin/jito-validator"),
            },
            ClientKind::Firedancer => Self {
                service_name: "firedancer",
                binary_path: PathBuf::from("/usr/local/bin/fdctl"),
            },
            ClientKind::Frankendancer => Self {
                service_name: "frankendancer",
                binary_path: PathBuf::from("/usr/local/bin/fdctl"),
            },
        }
    }

    /// Build the ExecStart line for the systemd unit.
    pub fn exec_start(&self, config: &ProvisionConfig) -> String {
        match config.client {
            ClientKind::Firedancer | ClientKind::Frankendancer => {
                format!("{} run --config /etc/pillar/validator.toml", self.binary_path.display())
            }
            _ => {
                // agave / jito: CLI flags
                let mut args = vec![
                    self.binary_path.display().to_string(),
                    format!("--identity {}", config.identity_keypair_path.display()),
                    format!("--ledger {}", config.ledger_path.display()),
                    format!("--snapshots {}", config.snapshot_path.display()),
                    format!("--accounts {}", config.accounts_path.display()),
                    format!("--rpc-port {}", config.rpc_port),
                    format!("--dynamic-port-range {}", config.dynamic_port_range),
                ];

                if !config.vote_account_keypair_path.as_os_str().is_empty() {
                    args.push(format!(
                        "--vote-account {}",
                        config.vote_account_keypair_path.display()
                    ));
                }

                for ep in &config.entrypoints {
                    args.push(format!("--entrypoint {ep}"));
                }

                for kv in &config.known_validators {
                    args.push(format!("--known-validator {kv}"));
                }

                if !config.known_validators.is_empty() {
                    args.push("--only-known-rpc".to_string());
                    args.push("--no-genesis-fetch".to_string());
                }

                // Jito MEV flags
                if config.jito_mev && config.client == ClientKind::Jito {
                    args.push(format!(
                        "--block-engine-url {}",
                        config.jito_block_engine_url
                    ));
                    args.push("--tip-payment-program-pubkey T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt".to_string());
                    args.push("--tip-distribution-program-pubkey 4R3gSG8BpU4t19KYj8CfnBtxhxJBjKHHaBnQ4SYnHNDn".to_string());
                    args.push("--commission-bps 800".to_string());
                }

                // Yellowstone gRPC geyser plugin
                if config.yellowstone_grpc {
                    args.push("--geyser-plugin-config /etc/pillar/yellowstone-grpc.json".to_string());
                }

                args.push("--expected-genesis-hash auto".to_string());
                args.push("--wal-recovery-mode skip_any_corrupted_record".to_string());
                args.push("--limit-ledger-size".to_string());

                args.join(" \\\n  ")
            }
        }
    }

    /// Generate the systemd unit file content.
    pub fn systemd_unit(&self, config: &ProvisionConfig) -> String {
        let exec_start = self.exec_start(config);
        let description = format!("Solana Validator ({})", config.client.as_str());

        format!(
            "[Unit]\n\
             Description={description}\n\
             After=network.target\n\
             \n\
             [Service]\n\
             Type=simple\n\
             User=sol\n\
             ExecStart={exec_start}\n\
             Restart=on-failure\n\
             RestartSec=10\n\
             LimitNOFILE=1000000\n\
             \n\
             [Install]\n\
             WantedBy=multi-user.target\n"
        )
    }
}

/// Provisioning progress stages, reported back to the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Stage {
    Downloading,
    Verifying,
    Installing,
    ConfiguringSystemd,
    Starting,
    Complete,
    Failed,
}

#[allow(dead_code)]
impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Downloading => "downloading",
            Self::Verifying => "verifying",
            Self::Installing => "installing",
            Self::ConfiguringSystemd => "configuring_systemd",
            Self::Starting => "starting",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }
}

// ---------------------------------------------------------------------------
// Systemd helpers
// ---------------------------------------------------------------------------

/// Write a systemd unit file to /etc/systemd/system/.
pub async fn write_unit_file(service_name: &str, content: &str) -> Result<(), String> {
    let path = format!("/etc/systemd/system/{service_name}.service");
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("write unit file {path}: {e}"))?;
    tracing::info!(path, "wrote systemd unit file");
    Ok(())
}

/// Run `sudo systemctl daemon-reload`.
pub async fn daemon_reload() -> Result<(), String> {
    let output = tokio::process::Command::new("sudo")
        .args(["systemctl", "daemon-reload"])
        .output()
        .await
        .map_err(|e| format!("sudo systemctl daemon-reload: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("daemon-reload failed: {stderr}"));
    }
    Ok(())
}

/// Enable and start a systemd service.
pub async fn enable_and_start(service_name: &str) -> Result<(), String> {
    let output = tokio::process::Command::new("sudo")
        .args(["systemctl", "enable", "--now", service_name])
        .output()
        .await
        .map_err(|e| format!("sudo systemctl enable --now {service_name}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("enable_and_start {service_name} failed: {stderr}"));
    }
    tracing::info!(service_name, "enabled and started");
    Ok(())
}

/// Stop a systemd service. Returns Ok even if the service is not running.
pub async fn stop_service(service_name: &str) -> Result<(), String> {
    let output = tokio::process::Command::new("sudo")
        .args(["systemctl", "stop", service_name])
        .output()
        .await
        .map_err(|e| format!("sudo systemctl stop {service_name}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(service_name, stderr = %stderr, "stop_service returned non-zero (may not be running)");
    }
    Ok(())
}

/// Restart a systemd service.
pub async fn restart_service(service_name: &str) -> Result<(), String> {
    let output = tokio::process::Command::new("sudo")
        .args(["systemctl", "restart", service_name])
        .output()
        .await
        .map_err(|e| format!("sudo systemctl restart {service_name}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("restart {service_name} failed: {stderr}"));
    }
    tracing::info!(service_name, "restarted");
    Ok(())
}

// ---------------------------------------------------------------------------
// Binary installation (sync, uses std::fs)
// ---------------------------------------------------------------------------

/// Install a binary from `src` to `dest`: chmod 0755, then rename (or copy+delete).
pub fn install_binary(src: &Path, dest: &Path) -> Result<(), String> {
    // Ensure parent directory exists
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }

    // Set executable permissions (0755)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(src, perms)
            .map_err(|e| format!("chmod {}: {e}", src.display()))?;
    }

    // Atomic move (rename within same filesystem, or copy+delete across)
    if std::fs::rename(src, dest).is_err() {
        std::fs::copy(src, dest)
            .map_err(|e| format!("copy {} -> {}: {e}", src.display(), dest.display()))?;
        std::fs::remove_file(src)
            .map_err(|e| format!("remove {}: {e}", src.display()))?;
    }

    tracing::info!(path = %dest.display(), "binary installed");
    Ok(())
}

// ---------------------------------------------------------------------------
// Client config writing
// ---------------------------------------------------------------------------

/// Generate Firedancer TOML config content.
pub fn firedancer_toml(config: &ProvisionConfig) -> String {
    format!(
        "[layout]\n\
         affinity = \"auto\"\n\
         \n\
         [consensus]\n\
         identity_path = \"{identity}\"\n\
         vote_account_path = \"{vote}\"\n\
         expected_genesis_hash = \"auto\"\n\
         \n\
         [ledger]\n\
         path = \"{ledger}\"\n\
         accounts_path = \"{accounts}\"\n\
         limit_size = true\n\
         \n\
         [gossip]\n\
         entrypoints = [{entrypoints}]\n",
        identity = config.identity_keypair_path.display(),
        vote = config.vote_account_keypair_path.display(),
        ledger = config.ledger_path.display(),
        accounts = config.accounts_path.display(),
        entrypoints = config
            .entrypoints
            .iter()
            .map(|e| format!("\"{e}\""))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// Generate default Yellowstone gRPC geyser plugin config JSON.
pub fn yellowstone_grpc_config() -> String {
    serde_json::json!({
        "libpath": "/usr/local/lib/libyellowstone_grpc_geyser.so",
        "log": {
            "level": "info"
        },
        "grpc": {
            "address": "0.0.0.0:10000",
            "max_decoding_message_size": "4_194_304"
        }
    })
    .to_string()
}

/// Write client-specific config files to disk.
pub async fn write_client_configs(
    installer: &ClientInstaller,
    config: &ProvisionConfig,
) -> Result<(), String> {
    // Ensure config directory exists
    tokio::fs::create_dir_all("/etc/pillar")
        .await
        .map_err(|e| format!("create /etc/pillar: {e}"))?;

    match config.client {
        ClientKind::Firedancer | ClientKind::Frankendancer => {
            let toml = firedancer_toml(config);
            tokio::fs::write("/etc/pillar/validator.toml", &toml)
                .await
                .map_err(|e| format!("write validator.toml: {e}"))?;
            tracing::info!("wrote /etc/pillar/validator.toml");
        }
        _ => {
            // Agave/Jito: flags in ExecStart, no separate config file
        }
    }

    if config.yellowstone_grpc {
        let grpc_config = yellowstone_grpc_config();
        tokio::fs::write("/etc/pillar/yellowstone-grpc.json", &grpc_config)
            .await
            .map_err(|e| format!("write yellowstone-grpc.json: {e}"))?;
        tracing::info!("wrote /etc/pillar/yellowstone-grpc.json");
    }

    let _ = installer; // used for pattern matching above, suppress warning
    Ok(())
}

// ---------------------------------------------------------------------------
// Command file processing
// ---------------------------------------------------------------------------

/// Check for a pending command file, read it, delete it, and return the parsed command.
/// Returns `Ok(None)` if no command file exists.
pub fn process_pending_command() -> Result<Option<shared::PendingCommand>, String> {
    let path = std::path::Path::new(shared::PENDING_COMMAND_PATH);
    if !path.exists() {
        return Ok(None);
    }

    let data = std::fs::read_to_string(path)
        .map_err(|e| format!("read pending command: {e}"))?;

    // Delete immediately to avoid re-processing on crash
    if let Err(e) = std::fs::remove_file(path) {
        tracing::warn!(error = %e, "failed to delete pending command file");
    }

    let cmd: shared::PendingCommand =
        serde_json::from_str(&data).map_err(|e| format!("parse pending command: {e}"))?;

    tracing::info!(command_type = %cmd.command_type(), "read pending command");
    Ok(Some(cmd))
}

// ---------------------------------------------------------------------------
// Orchestrators
// ---------------------------------------------------------------------------

/// Execute a full provision sequence: install binary, write configs, start service.
pub async fn provision(
    cmd: &ProvisionCommand,
    staged_binary: &Path,
) -> Result<(), String> {
    let config = ProvisionConfig::from_command(cmd)?;
    let installer = ClientInstaller::for_client(config.client);

    // 1. Stop existing service (ignore error — may not be running)
    let _ = stop_service(installer.service_name).await;

    // 2. Install binary
    install_binary(staged_binary, &installer.binary_path)?;

    // 3. Write client-specific configs
    write_client_configs(&installer, &config).await?;

    // 4. Write systemd unit
    let unit_content = installer.systemd_unit(&config);
    write_unit_file(installer.service_name, &unit_content).await?;

    // 5. Reload systemd
    daemon_reload().await?;

    // 6. Enable and start
    enable_and_start(installer.service_name).await?;

    tracing::info!(
        client = config.client.as_str(),
        version = %config.version,
        "provision complete"
    );
    Ok(())
}

/// Execute a binary upgrade: stop, swap binary, restart.
pub async fn upgrade(
    cmd: &shared::proto::UpgradeCommand,
    staged_binary: &Path,
) -> Result<(), String> {
    if cmd.binary_name.is_empty() {
        return Err("binary_name is required for upgrade".to_string());
    }

    let dest = PathBuf::from(format!("/usr/local/bin/{}", cmd.binary_name));

    // Determine service name from binary name
    let service_name = match cmd.binary_name.as_str() {
        "agave-validator" => "solana-validator",
        "jito-validator" => "jito-validator",
        "fdctl" => "firedancer",
        other => other,
    };

    // 1. Stop
    stop_service(service_name).await?;

    // 2. Install binary
    install_binary(staged_binary, &dest)?;

    // 3. Restart
    restart_service(service_name).await?;

    tracing::info!(
        binary = %cmd.binary_name,
        version = %cmd.version,
        "upgrade complete"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_command() -> ProvisionCommand {
        ProvisionCommand {
            client: "agave".to_string(),
            version: "2.1.6".to_string(),
            cluster: "mainnet-beta".to_string(),
            identity_keypair_path: "/home/sol/validator-keypair.json".to_string(),
            vote_account_keypair_path: "/home/sol/vote-account-keypair.json".to_string(),
            ledger_path: "/mnt/ledger".to_string(),
            snapshot_path: "/mnt/snapshots".to_string(),
            accounts_path: "/mnt/accounts".to_string(),
            entrypoints: vec![
                "entrypoint.mainnet-beta.solana.com:8001".to_string(),
                "entrypoint2.mainnet-beta.solana.com:8001".to_string(),
            ],
            known_validators: vec![
                "7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2".to_string(),
            ],
            download_url: "https://github.com/anza-xyz/agave/releases/download/v2.1.6/solana-release-x86_64-unknown-linux-gnu.tar.bz2".to_string(),
            sha256: "abc123".to_string(),
            jito_mev: false,
            jito_block_engine_url: String::new(),
            yellowstone_grpc: false,
            rpc_port: 0,
            dynamic_port_range: String::new(),
        }
    }

    #[test]
    fn parse_valid_command() {
        let config = ProvisionConfig::from_command(&sample_command()).unwrap();
        assert_eq!(config.client, ClientKind::Agave);
        assert_eq!(config.version, "2.1.6");
        assert_eq!(config.cluster, "mainnet-beta");
        assert_eq!(config.entrypoints.len(), 2);
        assert_eq!(config.known_validators.len(), 1);
    }

    #[test]
    fn parse_unknown_client_fails() {
        let mut cmd = sample_command();
        cmd.client = "unknown".to_string();
        assert!(ProvisionConfig::from_command(&cmd).is_err());
    }

    #[test]
    fn parse_missing_version_fails() {
        let mut cmd = sample_command();
        cmd.version = String::new();
        assert!(ProvisionConfig::from_command(&cmd).is_err());
    }

    #[test]
    fn exec_start_contains_identity() {
        let config = ProvisionConfig::from_command(&sample_command()).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        assert!(exec.contains("--identity /home/sol/validator-keypair.json"));
        assert!(exec.contains("--entrypoint entrypoint.mainnet-beta.solana.com:8001"));
        assert!(exec.contains("--known-validator"));
        assert!(exec.contains("--only-known-rpc"));
    }

    #[test]
    fn exec_start_firedancer_uses_toml() {
        let mut cmd = sample_command();
        cmd.client = "firedancer".to_string();
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        assert!(exec.contains("fdctl run --config"));
    }

    #[test]
    fn systemd_unit_has_required_sections() {
        let config = ProvisionConfig::from_command(&sample_command()).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let unit = installer.systemd_unit(&config);
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("LimitNOFILE=1000000"));
        assert!(unit.contains("Restart=on-failure"));
    }

    #[test]
    fn client_kind_roundtrip() {
        for name in ["agave", "jito", "firedancer", "frankendancer"] {
            let kind = ClientKind::parse(name).unwrap();
            assert_eq!(kind.as_str(), name);
        }
    }

    // --- Chunk 2 tests: addon flags ---

    #[test]
    fn exec_start_jito_with_mev_flags() {
        let mut cmd = sample_command();
        cmd.client = "jito".to_string();
        cmd.jito_mev = true;
        cmd.jito_block_engine_url = "https://block-engine.example.com".to_string();
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        assert!(exec.contains("--block-engine-url https://block-engine.example.com"));
        assert!(exec.contains("--tip-payment-program-pubkey"));
        assert!(exec.contains("--tip-distribution-program-pubkey"));
        assert!(exec.contains("--commission-bps"));
    }

    #[test]
    fn exec_start_jito_without_mev() {
        let mut cmd = sample_command();
        cmd.client = "jito".to_string();
        cmd.jito_mev = false;
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        assert!(!exec.contains("--block-engine-url"));
    }

    #[test]
    fn exec_start_with_yellowstone() {
        let mut cmd = sample_command();
        cmd.yellowstone_grpc = true;
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        assert!(exec.contains("--geyser-plugin-config /etc/pillar/yellowstone-grpc.json"));
    }

    #[test]
    fn firedancer_toml_has_identity() {
        let config = ProvisionConfig::from_command(&sample_command()).unwrap();
        let toml = firedancer_toml(&config);
        assert!(toml.contains("/home/sol/validator-keypair.json"));
        assert!(toml.contains("[consensus]"));
        assert!(toml.contains("[ledger]"));
        assert!(toml.contains("[gossip]"));
    }

    #[test]
    fn yellowstone_grpc_config_valid_json() {
        let json_str = yellowstone_grpc_config();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.get("grpc").is_some());
        assert!(parsed.get("libpath").is_some());
    }

    // --- Chunk 3 tests: install_binary ---

    #[test]
    fn install_binary_permissions_and_move() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src-binary");
        let dest = dir.path().join("subdir/dest-binary");
        std::fs::write(&src, b"#!/bin/sh\necho hi").unwrap();

        install_binary(&src, &dest).unwrap();

        // Source should be gone
        assert!(!src.exists());
        // Dest should exist
        assert!(dest.exists());
        // Content preserved
        let content = std::fs::read(&dest).unwrap();
        assert_eq!(content, b"#!/bin/sh\necho hi");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&dest).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o755);
        }
    }

    // --- Chunk 5 tests: command file processing ---

    #[test]
    fn process_pending_command_no_file() {
        // PENDING_COMMAND_PATH doesn't exist in test env
        let result = process_pending_command();
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn process_pending_command_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending-command.json");

        let cmd = shared::PendingCommand::Provision {
            staged_binary_path: "/tmp/staged-binary".to_string(),
            provision: Box::new(sample_command()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        std::fs::write(&path, &json).unwrap();

        // Read it back manually (can't override PENDING_COMMAND_PATH const)
        let data = std::fs::read_to_string(&path).unwrap();
        let parsed: shared::PendingCommand = serde_json::from_str(&data).unwrap();
        assert_eq!(parsed.command_type(), "provision");
        match parsed {
            shared::PendingCommand::Provision { provision, .. } => {
                assert_eq!(provision.client, "agave");
            }
            _ => panic!("expected Provision variant"),
        }
    }

    #[test]
    fn exec_start_devnet_no_known_validators() {
        let mut cmd = sample_command();
        cmd.cluster = "devnet".to_string();
        cmd.known_validators = vec![];
        cmd.entrypoints = vec!["entrypoint.devnet.solana.com:8001".to_string()];
        // rpc_port=0 and empty dynamic_port_range should get defaults
        cmd.rpc_port = 0;
        cmd.dynamic_port_range = String::new();
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        assert!(exec.contains("--rpc-port 8899"));
        assert!(exec.contains("--dynamic-port-range 8000-8020"));
        assert!(!exec.contains("--only-known-rpc"));
        assert!(!exec.contains("--no-genesis-fetch"));
    }

    #[test]
    fn provision_validates_command() {
        let mut cmd = sample_command();
        cmd.version = String::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(provision(&cmd, std::path::Path::new("/tmp/fake")));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("version is required"));
    }
}
