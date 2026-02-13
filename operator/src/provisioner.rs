//! Validator provisioner — handles installing and configuring a validator
//! binary on the node, triggered by a PendingCommand from Link.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pillar_shared::proto::ProvisionCommand;

/// Parsed, validated provision config derived from the proto command.
#[derive(Debug, Clone)]
pub struct ProvisionConfig {
    pub client: ClientKind,
    pub version: String,
    #[allow(dead_code)]
    pub cluster: String,
    #[allow(dead_code)]
    pub node_type: String,
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
    pub gossip_port: u32,
    /// Client-specific CLI flags: "flag-name" -> "value" (empty for bare flags).
    /// Rendered as `--{key}` or `--{key} {value}` for Agave/Jito.
    pub validator_flags: HashMap<String, String>,
    pub geyser_plugin_configs: Vec<String>,
    pub environment_vars: HashMap<String, String>,
    pub extra_args: Vec<String>,
    pub restart_sec: u32,
    pub log_rate_limit_disable: bool,
    pub start_limit_disable: bool,
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
            node_type: cmd.node_type.clone(),
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
            gossip_port: if cmd.gossip_port == 0 { 8001 } else { cmd.gossip_port },
            validator_flags: cmd.validator_flags.clone(),
            geyser_plugin_configs: cmd.geyser_plugin_configs.clone(),
            environment_vars: cmd.environment_vars.clone(),
            extra_args: cmd.extra_args.clone(),
            restart_sec: if cmd.restart_sec == 0 { 1 } else { cmd.restart_sec },
            log_rate_limit_disable: cmd.log_rate_limit_disable,
            start_limit_disable: cmd.start_limit_disable,
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
                format!(
                    "{} run --config /etc/pillar/validator.toml",
                    self.binary_path.display()
                )
            }
            _ => {
                // Agave / Jito: CLI flags
                let mut args = vec![
                    self.binary_path.display().to_string(),
                    format!("--identity {}", config.identity_keypair_path.display()),
                    format!("--ledger {}", config.ledger_path.display()),
                    format!("--snapshots {}", config.snapshot_path.display()),
                    format!("--accounts {}", config.accounts_path.display()),
                    format!("--rpc-port {}", config.rpc_port),
                    format!("--gossip-port {}", config.gossip_port),
                    format!("--dynamic-port-range {}", config.dynamic_port_range),
                ];

                // Vote account: only if path set and "no-voting" isn't in flags
                if !config.vote_account_keypair_path.as_os_str().is_empty()
                    && !config.validator_flags.contains_key("no-voting")
                {
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

                // Jito MEV: block-engine-url from explicit field, pubkeys/commission from
                // validator_flags with hardcoded defaults as safety net.
                if config.jito_mev && config.client == ClientKind::Jito {
                    args.push(format!(
                        "--block-engine-url {}",
                        config.jito_block_engine_url
                    ));
                    if !config
                        .validator_flags
                        .contains_key("tip-payment-program-pubkey")
                    {
                        args.push("--tip-payment-program-pubkey T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt".to_string());
                    }
                    if !config
                        .validator_flags
                        .contains_key("tip-distribution-program-pubkey")
                    {
                        args.push("--tip-distribution-program-pubkey 4R3gSG8BpU4t19KYj8CfnBtxhxJBjKHHaBnQ4SYnHNDn".to_string());
                    }
                    if !config.validator_flags.contains_key("commission-bps") {
                        args.push("--commission-bps 800".to_string());
                    }
                }

                // Yellowstone gRPC — auto-add if not already in geyser_plugin_configs
                let yellowstone_path = "/etc/pillar/yellowstone-grpc.json";
                if config.yellowstone_grpc
                    && !config
                        .geyser_plugin_configs
                        .iter()
                        .any(|p| p == yellowstone_path)
                {
                    args.push(format!("--geyser-plugin-config {yellowstone_path}"));
                }

                // Generic geyser plugin configs
                for path in &config.geyser_plugin_configs {
                    args.push(format!("--geyser-plugin-config {path}"));
                }

                // All client-specific flags from the map (sorted for deterministic output)
                let mut flag_keys: Vec<&String> = config.validator_flags.keys().collect();
                flag_keys.sort();
                for key in flag_keys {
                    let value = &config.validator_flags[key];
                    if value.is_empty() {
                        args.push(format!("--{key}"));
                    } else {
                        args.push(format!("--{key} {value}"));
                    }
                }

                // Extra args (raw pass-through, appended last)
                for arg in &config.extra_args {
                    args.push(arg.clone());
                }

                args.join(" \\\n  ")
            }
        }
    }

    /// Generate the systemd unit file content.
    pub fn systemd_unit(&self, config: &ProvisionConfig) -> String {
        let exec_start = self.exec_start(config);
        let description = format!("Solana Validator ({})", config.client.as_str());

        let mut unit_section = format!(
            "[Unit]\n\
             Description={description}\n\
             After=network.target\n"
        );
        if config.start_limit_disable {
            unit_section.push_str("StartLimitIntervalSec=0\n");
        }

        let mut service_section = format!(
            "\n[Service]\n\
             Type=simple\n\
             User=sol\n\
             ExecStart={exec_start}\n\
             Restart=on-failure\n\
             RestartSec={restart_sec}\n\
             LimitNOFILE=1000000\n",
            restart_sec = config.restart_sec,
        );
        if config.log_rate_limit_disable {
            service_section.push_str("LogRateLimitIntervalSec=0\n");
        }
        // Environment variables
        let mut env_keys: Vec<&String> = config.environment_vars.keys().collect();
        env_keys.sort();
        for key in env_keys {
            let val = &config.environment_vars[key];
            service_section.push_str(&format!("Environment={key}={val}\n"));
        }

        let install_section = "\n[Install]\nWantedBy=multi-user.target\n";

        format!("{unit_section}{service_section}{install_section}")
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

/// Write a systemd unit file to /etc/systemd/system/ via sudo tee.
pub async fn write_unit_file(service_name: &str, content: &str) -> Result<(), String> {
    let path = format!("/etc/systemd/system/{service_name}.service");
    let mut child = tokio::process::Command::new("sudo")
        .args(["tee", &path])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("sudo tee {path}: {e}"))?;

    use tokio::io::AsyncWriteExt;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(content.as_bytes())
            .await
            .map_err(|e| format!("write to sudo tee: {e}"))?;
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("sudo tee wait: {e}"))?;
    if !status.success() {
        return Err(format!("sudo tee {path} failed with {status}"));
    }

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

/// Install a binary from `src` to `dest` using sudo install for proper permissions.
pub async fn install_binary(src: &Path, dest: &Path) -> Result<(), String> {
    let output = tokio::process::Command::new("sudo")
        .args([
            "install",
            "-m",
            "755",
            &src.display().to_string(),
            &dest.display().to_string(),
        ])
        .output()
        .await
        .map_err(|e| format!("sudo install {}: {e}", dest.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "sudo install {} failed: {stderr}",
            dest.display()
        ));
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
pub fn process_pending_command() -> Result<Option<pillar_shared::PendingCommand>, String> {
    let path = std::path::Path::new(pillar_shared::PENDING_COMMAND_PATH);
    if !path.exists() {
        return Ok(None);
    }

    let data =
        std::fs::read_to_string(path).map_err(|e| format!("read pending command: {e}"))?;

    // Delete immediately to avoid re-processing on crash
    if let Err(e) = std::fs::remove_file(path) {
        tracing::warn!(error = %e, "failed to delete pending command file");
    }

    let cmd: pillar_shared::PendingCommand =
        serde_json::from_str(&data).map_err(|e| format!("parse pending command: {e}"))?;

    tracing::info!(command_type = %cmd.command_type(), "read pending command");
    Ok(Some(cmd))
}

// ---------------------------------------------------------------------------
// Operator config update
// ---------------------------------------------------------------------------

/// Cluster-specific reference RPC URLs.
fn reference_rpc_for_cluster(cluster: &str) -> Vec<String> {
    match cluster {
        "devnet" => vec!["https://api.devnet.solana.com".to_string()],
        "testnet" => vec!["https://api.testnet.solana.com".to_string()],
        _ => vec!["https://api.mainnet-beta.solana.com".to_string()],
    }
}

/// Update the operator config file to match the provisioned client/cluster.
/// This ensures the operator uses the correct health checker, service name,
/// and reference RPCs after a restart.
pub async fn update_operator_config(
    config: &ProvisionConfig,
    installer: &ClientInstaller,
) -> Result<(), String> {
    let config_path = std::env::var("PILLAR_CONFIG")
        .unwrap_or_else(|_| "/etc/pillar/operator.yaml".to_string());

    // Read existing config
    let yaml_str = tokio::fs::read_to_string(&config_path)
        .await
        .map_err(|e| format!("read {config_path}: {e}"))?;

    // Parse as serde_yaml::Value so we preserve unknown fields
    let mut doc: serde_yaml::Value =
        serde_yaml::from_str(&yaml_str).map_err(|e| format!("parse {config_path}: {e}"))?;

    let map = doc
        .as_mapping_mut()
        .ok_or_else(|| "operator config is not a YAML mapping".to_string())?;

    // Update client
    map.insert(
        serde_yaml::Value::String("client".to_string()),
        serde_yaml::Value::String(config.client.as_str().to_string()),
    );

    // Update network.cluster and network.reference_rpc_urls
    let network = map
        .entry(serde_yaml::Value::String("network".to_string()))
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    if let Some(net_map) = network.as_mapping_mut() {
        net_map.insert(
            serde_yaml::Value::String("cluster".to_string()),
            serde_yaml::Value::String(config.cluster.clone()),
        );
        let rpcs: Vec<serde_yaml::Value> = reference_rpc_for_cluster(&config.cluster)
            .into_iter()
            .map(serde_yaml::Value::String)
            .collect();
        net_map.insert(
            serde_yaml::Value::String("reference_rpc_urls".to_string()),
            serde_yaml::Value::Sequence(rpcs),
        );
    }

    // Update lifecycle.service_name
    let lifecycle = map
        .entry(serde_yaml::Value::String("lifecycle".to_string()))
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    if let Some(lc_map) = lifecycle.as_mapping_mut() {
        lc_map.insert(
            serde_yaml::Value::String("service_name".to_string()),
            serde_yaml::Value::String(installer.service_name.to_string()),
        );
    }

    // Write back
    let new_yaml =
        serde_yaml::to_string(&doc).map_err(|e| format!("serialize config: {e}"))?;
    tokio::fs::write(&config_path, &new_yaml)
        .await
        .map_err(|e| format!("write {config_path}: {e}"))?;

    tracing::info!(
        config_path,
        client = config.client.as_str(),
        cluster = %config.cluster,
        service_name = installer.service_name,
        "updated operator config"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Orchestrators
// ---------------------------------------------------------------------------

/// Execute a full provision sequence: install binary, write configs, start service.
pub async fn provision(cmd: &ProvisionCommand, staged_binary: &Path) -> Result<(), String> {
    let config = ProvisionConfig::from_command(cmd)?;
    let installer = ClientInstaller::for_client(config.client);

    // 1. Stop existing service (ignore error — may not be running)
    let _ = stop_service(installer.service_name).await;

    // 2. Install binary
    install_binary(staged_binary, &installer.binary_path).await?;

    // 3. Write client-specific configs
    write_client_configs(&installer, &config).await?;

    // 4. Write systemd unit
    let unit_content = installer.systemd_unit(&config);
    write_unit_file(installer.service_name, &unit_content).await?;

    // 5. Reload systemd
    daemon_reload().await?;

    // 6. Enable and start
    enable_and_start(installer.service_name).await?;

    // 7. Update operator config to match provisioned client/cluster
    if let Err(e) = update_operator_config(&config, &installer).await {
        tracing::warn!(error = %e, "failed to update operator config (operator restart may use stale config)");
    }

    tracing::info!(
        client = config.client.as_str(),
        version = %config.version,
        "provision complete — operator will exit for config reload"
    );
    Ok(())
}

/// Execute a binary upgrade: stop, swap binary, restart.
pub async fn upgrade(
    cmd: &pillar_shared::proto::UpgradeCommand,
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
    install_binary(staged_binary, &dest).await?;

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
            ..Default::default()
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
        assert!(exec.contains("--gossip-port 8001"));
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

    #[test]
    fn exec_start_jito_with_mev_defaults() {
        let mut cmd = sample_command();
        cmd.client = "jito".to_string();
        cmd.jito_mev = true;
        cmd.jito_block_engine_url = "https://block-engine.example.com".to_string();
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        assert!(exec.contains("--block-engine-url https://block-engine.example.com"));
        assert!(exec.contains("--tip-payment-program-pubkey T1pyya"));
        assert!(exec.contains("--tip-distribution-program-pubkey 4R3gSG"));
        assert!(exec.contains("--commission-bps 800"));
    }

    #[test]
    fn exec_start_jito_custom_pubkeys_via_flags() {
        let mut cmd = sample_command();
        cmd.client = "jito".to_string();
        cmd.jito_mev = true;
        cmd.jito_block_engine_url = "https://block-engine.example.com".to_string();
        // Override via validator_flags
        cmd.validator_flags.insert(
            "tip-payment-program-pubkey".to_string(),
            "CustomTipPayment111111111111111111111111111".to_string(),
        );
        cmd.validator_flags.insert(
            "tip-distribution-program-pubkey".to_string(),
            "CustomTipDistribution1111111111111111111111".to_string(),
        );
        cmd.validator_flags
            .insert("commission-bps".to_string(), "1000".to_string());
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        // Should NOT contain the defaults since flags map overrides
        assert!(!exec.contains("T1pyya"));
        assert!(!exec.contains("4R3gSG"));
        assert!(!exec.contains("--commission-bps 800"));
        // Should contain the custom values from the map
        assert!(exec.contains("--commission-bps 1000"));
        assert!(exec.contains("--tip-payment-program-pubkey CustomTipPayment"));
        assert!(exec.contains("--tip-distribution-program-pubkey CustomTipDistribution"));
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

    #[tokio::test]
    async fn install_binary_permissions_and_move() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src-binary");
        let dest = dir.path().join("dest-binary");
        std::fs::write(&src, b"#!/bin/sh\necho hi").unwrap();

        match install_binary(&src, &dest).await {
            Ok(()) => {
                assert!(dest.exists());
                let content = std::fs::read(&dest).unwrap();
                assert_eq!(content, b"#!/bin/sh\necho hi");
            }
            Err(e) if e.contains("sudo") || e.contains("Permission") => {
                // Expected in environments without sudo
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn process_pending_command_no_file() {
        let result = process_pending_command();
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn process_pending_command_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending-command.json");

        let cmd = pillar_shared::PendingCommand::Provision {
            staged_binary_path: "/tmp/staged-binary".to_string(),
            provision: Box::new(sample_command()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        std::fs::write(&path, &json).unwrap();

        let data = std::fs::read_to_string(&path).unwrap();
        let parsed: pillar_shared::PendingCommand = serde_json::from_str(&data).unwrap();
        assert_eq!(parsed.command_type(), "provision");
        match parsed {
            pillar_shared::PendingCommand::Provision { provision, .. } => {
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

    // --- validator_flags map tests ---

    #[test]
    fn validator_flags_bare_and_valued() {
        let mut cmd = sample_command();
        cmd.validator_flags
            .insert("no-voting".to_string(), String::new());
        cmd.validator_flags
            .insert("limit-ledger-size".to_string(), "50000000".to_string());
        cmd.validator_flags
            .insert("rpc-bind-address".to_string(), "0.0.0.0".to_string());
        cmd.vote_account_keypair_path = String::new();
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        assert!(exec.contains("--no-voting"));
        assert!(exec.contains("--limit-ledger-size 50000000"));
        assert!(exec.contains("--rpc-bind-address 0.0.0.0"));
        // no-voting in flags should suppress --vote-account
        assert!(!exec.contains("--vote-account"));
    }

    #[test]
    fn validator_flags_rpc_preset() {
        let mut cmd = sample_command();
        cmd.vote_account_keypair_path = String::new();
        // Simulate what the UI sends for RPC node type
        for flag in [
            "no-voting",
            "private-rpc",
            "full-rpc-api",
            "enable-rpc-transaction-history",
            "no-port-check",
            "no-skip-initial-accounts-db-clean",
            "wal-recovery-mode",
            "limit-ledger-size",
        ] {
            cmd.validator_flags.insert(flag.to_string(), String::new());
        }
        cmd.validator_flags.insert(
            "rpc-bind-address".to_string(),
            "0.0.0.0".to_string(),
        );
        cmd.validator_flags.insert(
            "expected-genesis-hash".to_string(),
            "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d".to_string(),
        );
        // Override wal-recovery-mode with value
        cmd.validator_flags.insert(
            "wal-recovery-mode".to_string(),
            "skip_any_corrupted_record".to_string(),
        );
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        assert!(exec.contains("--no-voting"));
        assert!(exec.contains("--private-rpc"));
        assert!(exec.contains("--full-rpc-api"));
        assert!(exec.contains("--enable-rpc-transaction-history"));
        assert!(exec.contains("--no-port-check"));
        assert!(exec.contains("--expected-genesis-hash 5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"));
        assert!(exec.contains("--wal-recovery-mode skip_any_corrupted_record"));
        assert!(!exec.contains("--vote-account"));
    }

    #[test]
    fn validator_flags_sorted_deterministic() {
        let mut cmd = sample_command();
        cmd.validator_flags
            .insert("zzz-flag".to_string(), String::new());
        cmd.validator_flags
            .insert("aaa-flag".to_string(), "value".to_string());
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        let aaa_pos = exec.find("--aaa-flag").unwrap();
        let zzz_pos = exec.find("--zzz-flag").unwrap();
        assert!(aaa_pos < zzz_pos, "flags should be sorted alphabetically");
    }

    #[test]
    fn extra_args_appended_after_flags() {
        let mut cmd = sample_command();
        cmd.validator_flags
            .insert("private-rpc".to_string(), String::new());
        cmd.extra_args = vec!["--custom-escape-hatch".to_string()];
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        let flags_pos = exec.find("--private-rpc").unwrap();
        let extra_pos = exec.find("--custom-escape-hatch").unwrap();
        assert!(extra_pos > flags_pos, "extra_args should come after flags");
    }

    #[test]
    fn systemd_unit_environment_vars() {
        let mut cmd = sample_command();
        cmd.environment_vars.insert(
            "SOLANA_METRICS_CONFIG".to_string(),
            "host=https://metrics.solana.com:8086,db=mainnet-beta".to_string(),
        );
        cmd.restart_sec = 1;
        cmd.log_rate_limit_disable = true;
        cmd.start_limit_disable = true;
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let unit = installer.systemd_unit(&config);
        assert!(unit.contains("Environment=SOLANA_METRICS_CONFIG="));
        assert!(unit.contains("RestartSec=1"));
        assert!(unit.contains("LogRateLimitIntervalSec=0"));
        assert!(unit.contains("StartLimitIntervalSec=0"));
    }

    #[test]
    fn geyser_plugin_configs() {
        let mut cmd = sample_command();
        cmd.geyser_plugin_configs = vec!["/etc/pillar/custom-geyser.json".to_string()];
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        let installer = ClientInstaller::for_client(config.client);
        let exec = installer.exec_start(&config);
        assert!(exec.contains("--geyser-plugin-config /etc/pillar/custom-geyser.json"));
    }

    #[test]
    fn defaults_gossip_port_and_restart_sec() {
        let cmd = sample_command();
        let config = ProvisionConfig::from_command(&cmd).unwrap();
        assert_eq!(config.gossip_port, 8001);
        assert_eq!(config.restart_sec, 1);
    }
}
