use std::collections::HashMap;

/// Simple `{{placeholder}}` template renderer.
/// Replaces all occurrences of `{{key}}` with the corresponding value from `vars`.
pub fn render(template: &str, vars: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{{{key}}}}}"), value);
    }
    result
}

#[allow(dead_code)]
pub mod scripts {
    pub const PROVISION_AGAVE: &str = include_str!("../scripts/provision-agave.sh.tmpl");
    pub const PROVISION_JITO: &str = include_str!("../scripts/provision-jito.sh.tmpl");
    pub const PROVISION_FIREDANCER: &str = include_str!("../scripts/provision-firedancer.sh.tmpl");
    pub const PROVISION_FRANKENDANCER: &str =
        include_str!("../scripts/provision-frankendancer.sh.tmpl");
    pub const UPGRADE_VALIDATOR: &str = include_str!("../scripts/upgrade-validator.sh.tmpl");
    pub const UPGRADE_AGENT: &str = include_str!("../scripts/upgrade-agent.sh.tmpl");
    pub const RECOVER: &str = include_str!("../scripts/recover.sh.tmpl");
    pub const RESTART: &str = include_str!("../scripts/restart.sh.tmpl");
    pub const STOP: &str = include_str!("../scripts/stop.sh.tmpl");
}

pub fn provision_template(client: &str) -> Result<&'static str, String> {
    match client {
        "agave" => Ok(scripts::PROVISION_AGAVE),
        "jito" => Ok(scripts::PROVISION_JITO),
        "firedancer" => Ok(scripts::PROVISION_FIREDANCER),
        "frankendancer" => Ok(scripts::PROVISION_FRANKENDANCER),
        other => Err(format!("unknown client: {other}")),
    }
}

/// Map a validator client name to its systemd service name.
pub fn service_name_for_client(client: &str) -> &'static str {
    match client {
        "agave" => "solana-validator",
        "jito" => "jito-validator",
        "firedancer" => "firedancer",
        "frankendancer" => "frankendancer",
        _ => "solana-validator",
    }
}

/// Map a validator client name to its binary path.
pub fn binary_path_for_client(client: &str) -> &'static str {
    match client {
        "agave" => "/usr/local/bin/agave-validator",
        "jito" => "/usr/local/bin/jito-validator",
        "firedancer" => "/usr/local/bin/fdctl",
        "frankendancer" => "/usr/local/bin/fdctl",
        _ => "/usr/local/bin/agave-validator",
    }
}

/// Map a binary_name to its service name (for upgrades).
pub fn service_name_for_binary(binary_name: &str) -> &str {
    match binary_name {
        "agave-validator" => "solana-validator",
        "jito-validator" => "jito-validator",
        "fdctl" => "firedancer",
        other => other,
    }
}

/// Map cluster to reference RPC URLs for agent config.
pub fn reference_rpc_for_cluster(cluster: &str) -> &'static str {
    match cluster {
        "devnet" => "https://api.devnet.solana.com",
        "testnet" => "https://api.testnet.solana.com",
        _ => "https://api.mainnet-beta.solana.com",
    }
}

/// Build the ExecStart line for Agave/Jito systemd unit.
#[allow(clippy::too_many_arguments)]
pub fn build_exec_start(
    binary_path: &str,
    identity: &str,
    vote_account: &str,
    ledger_path: &str,
    snapshot_path: &str,
    accounts_path: &str,
    rpc_port: u32,
    gossip_port: u32,
    dynamic_port_range: &str,
    entrypoints: &[String],
    known_validators: &[String],
    jito_mev: bool,
    jito_block_engine_url: &str,
    yellowstone_grpc: bool,
    geyser_plugin_configs: &[String],
    validator_flags: &HashMap<String, String>,
    extra_args: &[String],
) -> String {
    let mut args = vec![
        binary_path.to_string(),
        format!("--identity {identity}"),
        format!("--ledger {ledger_path}"),
        format!("--snapshots {snapshot_path}"),
        format!("--accounts {accounts_path}"),
        format!("--rpc-port {rpc_port}"),
        format!("--gossip-port {gossip_port}"),
        format!("--dynamic-port-range {dynamic_port_range}"),
    ];

    if !vote_account.is_empty() && !validator_flags.contains_key("no-voting") {
        args.push(format!("--vote-account {vote_account}"));
    }

    for ep in entrypoints {
        args.push(format!("--entrypoint {ep}"));
    }

    for kv in known_validators {
        args.push(format!("--known-validator {kv}"));
    }

    if !known_validators.is_empty() {
        args.push("--only-known-rpc".to_string());
        args.push("--no-genesis-fetch".to_string());
    }

    if jito_mev {
        args.push(format!("--block-engine-url {jito_block_engine_url}"));
        if !validator_flags.contains_key("tip-payment-program-pubkey") {
            args.push("--tip-payment-program-pubkey T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt".to_string());
        }
        if !validator_flags.contains_key("tip-distribution-program-pubkey") {
            args.push("--tip-distribution-program-pubkey 4R3gSG8BpU4t19KYj8CfnBtxhxJBjKHHaBnQ4SYnHNDn".to_string());
        }
        if !validator_flags.contains_key("commission-bps") {
            args.push("--commission-bps 800".to_string());
        }
    }

    let yellowstone_path = "/etc/pillar/yellowstone-grpc.json";
    if yellowstone_grpc && !geyser_plugin_configs.iter().any(|p| p == yellowstone_path) {
        args.push(format!("--geyser-plugin-config {yellowstone_path}"));
    }

    for path in geyser_plugin_configs {
        args.push(format!("--geyser-plugin-config {path}"));
    }

    let mut flag_keys: Vec<&String> = validator_flags.keys().collect();
    flag_keys.sort();
    for key in flag_keys {
        let value = &validator_flags[key];
        if value.is_empty() {
            args.push(format!("--{key}"));
        } else {
            args.push(format!("--{key} {value}"));
        }
    }

    for arg in extra_args {
        args.push(arg.clone());
    }

    args.join(" \\\n  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_replaces_placeholders() {
        let template = "hello {{name}}, you are {{age}} years old";
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "world".to_string());
        vars.insert("age".to_string(), "42".to_string());
        assert_eq!(render(template, &vars), "hello world, you are 42 years old");
    }

    #[test]
    fn render_leaves_unknown_placeholders() {
        let template = "{{known}} and {{unknown}}";
        let mut vars = HashMap::new();
        vars.insert("known".to_string(), "yes".to_string());
        assert_eq!(render(template, &vars), "yes and {{unknown}}");
    }

    #[test]
    fn provision_template_valid_clients() {
        assert!(provision_template("agave").is_ok());
        assert!(provision_template("jito").is_ok());
        assert!(provision_template("firedancer").is_ok());
        assert!(provision_template("frankendancer").is_ok());
    }

    #[test]
    fn provision_template_unknown_client() {
        assert!(provision_template("unknown").is_err());
    }

    #[test]
    fn service_name_mapping() {
        assert_eq!(service_name_for_client("agave"), "solana-validator");
        assert_eq!(service_name_for_client("jito"), "jito-validator");
        assert_eq!(service_name_for_client("firedancer"), "firedancer");
    }

    #[test]
    fn exec_start_basic() {
        let exec = build_exec_start(
            "/usr/local/bin/agave-validator",
            "/home/sol/validator-keypair.json",
            "/home/sol/vote-account-keypair.json",
            "/mnt/ledger",
            "/mnt/snapshots",
            "/mnt/accounts",
            8899,
            8001,
            "8000-8020",
            &["entrypoint.mainnet-beta.solana.com:8001".to_string()],
            &["7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2".to_string()],
            false,
            "",
            false,
            &[],
            &HashMap::new(),
            &[],
        );
        assert!(exec.contains("--identity /home/sol/validator-keypair.json"));
        assert!(exec.contains("--entrypoint entrypoint.mainnet-beta.solana.com:8001"));
        assert!(exec.contains("--known-validator"));
        assert!(exec.contains("--only-known-rpc"));
        assert!(exec.contains("--gossip-port 8001"));
    }
}
