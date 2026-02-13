//! Merged provisioner — combines binary download/verify (from link) with
//! validator install/configure/systemd (from operator).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pillar_shared::proto::ProvisionCommand;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

// ---------------------------------------------------------------------------
// Download + verify (from link/provisioner.rs)
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

/// Download and stage a binary to /tmp/pillar-staging/binary.
/// Returns the path to the staged binary on success.
pub async fn download_and_stage(download_url: &str, sha256: &str) -> Result<PathBuf, String> {
    let staging_dir = std::path::Path::new("/tmp/pillar-staging");
    tokio::fs::create_dir_all(staging_dir)
        .await
        .map_err(|e| format!("create staging dir: {e}"))?;
    let staged = staging_dir.join("binary");

    let timeout = Duration::from_secs(3600);
    download_and_verify(download_url, &staged, sha256, timeout).await?;

    Ok(staged)
}

// ---------------------------------------------------------------------------
// Provisioning types and config (from operator/provisioner.rs)
// ---------------------------------------------------------------------------

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
    pub service_name: &'static str,
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

                let yellowstone_path = "/etc/pillar/yellowstone-grpc.json";
                if config.yellowstone_grpc
                    && !config
                        .geyser_plugin_configs
                        .iter()
                        .any(|p| p == yellowstone_path)
                {
                    args.push(format!("--geyser-plugin-config {yellowstone_path}"));
                }

                for path in &config.geyser_plugin_configs {
                    args.push(format!("--geyser-plugin-config {path}"));
                }

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

// ---------------------------------------------------------------------------
// Systemd helpers
// ---------------------------------------------------------------------------

pub async fn write_unit_file(service_name: &str, content: &str) -> Result<(), String> {
    let path = format!("/etc/systemd/system/{service_name}.service");
    let mut child = tokio::process::Command::new("sudo")
        .args(["tee", &path])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("sudo tee {path}: {e}"))?;

    use tokio::io::AsyncWriteExt as _;
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

pub async fn write_client_configs(
    installer: &ClientInstaller,
    config: &ProvisionConfig,
) -> Result<(), String> {
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
        _ => {}
    }

    if config.yellowstone_grpc {
        let grpc_config = yellowstone_grpc_config();
        tokio::fs::write("/etc/pillar/yellowstone-grpc.json", &grpc_config)
            .await
            .map_err(|e| format!("write yellowstone-grpc.json: {e}"))?;
        tracing::info!("wrote /etc/pillar/yellowstone-grpc.json");
    }

    let _ = installer;
    Ok(())
}

// ---------------------------------------------------------------------------
// Agent config update
// ---------------------------------------------------------------------------

fn reference_rpc_for_cluster(cluster: &str) -> Vec<String> {
    match cluster {
        "devnet" => vec!["https://api.devnet.solana.com".to_string()],
        "testnet" => vec!["https://api.testnet.solana.com".to_string()],
        _ => vec!["https://api.mainnet-beta.solana.com".to_string()],
    }
}

/// Update the agent config file to match the provisioned client/cluster.
pub async fn update_agent_config(
    config: &ProvisionConfig,
    installer: &ClientInstaller,
) -> Result<(), String> {
    let config_path = std::env::var("PILLAR_AGENT_CONFIG")
        .unwrap_or_else(|_| "/etc/pillar/agent.yaml".to_string());

    let yaml_str = tokio::fs::read_to_string(&config_path)
        .await
        .map_err(|e| format!("read {config_path}: {e}"))?;

    let mut doc: serde_yaml::Value =
        serde_yaml::from_str(&yaml_str).map_err(|e| format!("parse {config_path}: {e}"))?;

    let map = doc
        .as_mapping_mut()
        .ok_or_else(|| "agent config is not a YAML mapping".to_string())?;

    map.insert(
        serde_yaml::Value::String("client".to_string()),
        serde_yaml::Value::String(config.client.as_str().to_string()),
    );

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

    let lifecycle = map
        .entry(serde_yaml::Value::String("lifecycle".to_string()))
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    if let Some(lc_map) = lifecycle.as_mapping_mut() {
        lc_map.insert(
            serde_yaml::Value::String("service_name".to_string()),
            serde_yaml::Value::String(installer.service_name.to_string()),
        );
    }

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
        "updated agent config"
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

    let _ = stop_service(installer.service_name).await;
    install_binary(staged_binary, &installer.binary_path).await?;
    write_client_configs(&installer, &config).await?;

    let unit_content = installer.systemd_unit(&config);
    write_unit_file(installer.service_name, &unit_content).await?;
    daemon_reload().await?;
    enable_and_start(installer.service_name).await?;

    if let Err(e) = update_agent_config(&config, &installer).await {
        tracing::warn!(error = %e, "failed to update agent config (restart may use stale config)");
    }

    tracing::info!(
        client = config.client.as_str(),
        version = %config.version,
        "provision complete — agent will exit for config reload"
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

    let service_name = match cmd.binary_name.as_str() {
        "agave-validator" => "solana-validator",
        "jito-validator" => "jito-validator",
        "fdctl" => "firedancer",
        other => other,
    };

    stop_service(service_name).await?;
    install_binary(staged_binary, &dest).await?;
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
