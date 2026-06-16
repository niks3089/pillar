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

/// Map cluster to its genesis hash. Firedancer requires an explicit
/// `expected_genesis_hash` in its config (it does not accept "auto").
pub fn genesis_hash_for_cluster(cluster: &str) -> &'static str {
    match cluster {
        "devnet" => "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG",
        "testnet" => "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY",
        _ => "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d",
    }
}

/// Jito MEV defaults for a cluster.
///
/// Sources (verified against jito-foundation/jito-programs `declare_id!` and the Jito
/// command-line-arguments docs): the tip-payment and tip-distribution programs are
/// deployed at *different* addresses on mainnet vs testnet, so these MUST be
/// cluster-aware — applying the mainnet addresses on testnet produces a validator that
/// cannot find the tip programs.
pub struct JitoDefaults {
    pub block_engine_url: &'static str,
    pub tip_payment_program: &'static str,
    pub tip_distribution_program: &'static str,
}

pub fn jito_defaults_for_cluster(cluster: &str) -> JitoDefaults {
    match cluster {
        "testnet" => JitoDefaults {
            block_engine_url: "https://testnet.block-engine.jito.wtf",
            tip_payment_program: "GJHtFqM9agxPmkeKjHny6qiRKrXZALvvFGiKf11QE7hy",
            tip_distribution_program: "DzvGET57TAgEDxvm3ERUM4GNcsAJdqjDLCne9sdfY4wf",
        },
        // mainnet / mainnet-beta / anything else: Jito does not run on devnet, so the
        // mainnet programs are the only safe default for non-testnet clusters.
        _ => JitoDefaults {
            block_engine_url: "https://mainnet.block-engine.jito.wtf",
            tip_payment_program: "T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt",
            tip_distribution_program: "4R3gSG8BpU4t19KYj8CfnbtRpnT8gtk4dvTHxVRwc2r7",
        },
    }
}

/// Jito MEV runtime configuration for the validator ExecStart line.
/// Empty string fields fall back to the cluster default (block engine) or are omitted
/// (relayer, shred receiver — both optional; many operators run relayer-less).
#[derive(Default)]
pub struct JitoConfig {
    pub enabled: bool,
    pub block_engine_url: String,
    pub relayer_url: String,
    pub shred_receiver_addr: String,
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
    cluster: &str,
    jito: &JitoConfig,
    yellowstone_grpc: bool,
    geyser_plugin_configs: &[String],
    validator_flags: &HashMap<String, String>,
    extra_args: &[String],
    no_port_check: bool,
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

    if jito.enabled {
        let defaults = jito_defaults_for_cluster(cluster);
        let block_engine_url = if jito.block_engine_url.is_empty() {
            defaults.block_engine_url
        } else {
            jito.block_engine_url.as_str()
        };
        args.push(format!("--block-engine-url {block_engine_url}"));

        // Relayer and shred receiver are optional (relayer-less is common).
        if !jito.relayer_url.is_empty() {
            args.push(format!("--relayer-url {}", jito.relayer_url));
        }
        if !jito.shred_receiver_addr.is_empty() {
            args.push(format!("--shred-receiver-address {}", jito.shred_receiver_addr));
        }

        if !validator_flags.contains_key("tip-payment-program-pubkey") {
            args.push(format!(
                "--tip-payment-program-pubkey {}",
                defaults.tip_payment_program
            ));
        }
        if !validator_flags.contains_key("tip-distribution-program-pubkey") {
            args.push(format!(
                "--tip-distribution-program-pubkey {}",
                defaults.tip_distribution_program
            ));
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

    if no_port_check && !validator_flags.contains_key("no-port-check") {
        args.push("--no-port-check".to_string());
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
            "mainnet",
            &JitoConfig::default(),
            false,
            &[],
            &HashMap::new(),
            &[],
            false,
        );
        assert!(exec.contains("--identity /home/sol/validator-keypair.json"));
        assert!(exec.contains("--entrypoint entrypoint.mainnet-beta.solana.com:8001"));
        assert!(exec.contains("--known-validator"));
        assert!(exec.contains("--only-known-rpc"));
        assert!(exec.contains("--gossip-port 8001"));
        // Jito disabled by default — no MEV flags.
        assert!(!exec.contains("--block-engine-url"));
        assert!(!exec.contains("--tip-payment-program-pubkey"));
    }

    /// Helper for the Jito exec_start tests.
    fn jito_exec(cluster: &str, jito: &JitoConfig, flags: &HashMap<String, String>) -> String {
        build_exec_start(
            "/usr/local/bin/jito-validator",
            "/home/sol/identity.json",
            "/home/sol/vote.json",
            "/mnt/ledger",
            "/mnt/snapshots",
            "/mnt/accounts",
            8899,
            8001,
            "8000-8030",
            &["entrypoint.testnet.solana.com:8001".to_string()],
            &[],
            cluster,
            jito,
            false,
            &[],
            flags,
            &[],
            false,
        )
    }

    #[test]
    fn jito_mainnet_uses_mainnet_tip_programs_and_default_block_engine() {
        let jito = JitoConfig {
            enabled: true,
            ..Default::default()
        };
        let exec = jito_exec("mainnet", &jito, &HashMap::new());
        // Block engine falls back to the cluster default when not supplied.
        assert!(exec.contains("--block-engine-url https://mainnet.block-engine.jito.wtf"));
        assert!(exec
            .contains("--tip-payment-program-pubkey T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt"));
        assert!(exec.contains(
            "--tip-distribution-program-pubkey 4R3gSG8BpU4t19KYj8CfnbtRpnT8gtk4dvTHxVRwc2r7"
        ));
        assert!(exec.contains("--commission-bps 800"));
        // Optional flags omitted when not configured.
        assert!(!exec.contains("--relayer-url"));
        assert!(!exec.contains("--shred-receiver-address"));
    }

    #[test]
    fn jito_testnet_uses_testnet_tip_programs() {
        let jito = JitoConfig {
            enabled: true,
            ..Default::default()
        };
        let exec = jito_exec("testnet", &jito, &HashMap::new());
        assert!(exec.contains("--block-engine-url https://testnet.block-engine.jito.wtf"));
        assert!(exec
            .contains("--tip-payment-program-pubkey GJHtFqM9agxPmkeKjHny6qiRKrXZALvvFGiKf11QE7hy"));
        assert!(exec.contains(
            "--tip-distribution-program-pubkey DzvGET57TAgEDxvm3ERUM4GNcsAJdqjDLCne9sdfY4wf"
        ));
        // Crucially: never the mainnet programs on testnet.
        assert!(!exec.contains("T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt"));
        assert!(!exec.contains("4R3gSG8BpU4t19KYj8CfnbtRpnT8gtk4dvTHxVRwc2r7"));
    }

    #[test]
    fn jito_relayer_and_shred_receiver_included_when_set() {
        let jito = JitoConfig {
            enabled: true,
            block_engine_url: "https://frankfurt.mainnet.block-engine.jito.wtf".to_string(),
            relayer_url: "http://frankfurt.mainnet.relayer.jito.wtf:8100".to_string(),
            shred_receiver_addr: "64.130.50.14:1002".to_string(),
        };
        let exec = jito_exec("mainnet", &jito, &HashMap::new());
        // Operator-supplied block engine overrides the default.
        assert!(exec.contains("--block-engine-url https://frankfurt.mainnet.block-engine.jito.wtf"));
        assert!(exec.contains("--relayer-url http://frankfurt.mainnet.relayer.jito.wtf:8100"));
        assert!(exec.contains("--shred-receiver-address 64.130.50.14:1002"));
    }

    #[test]
    fn jito_explicit_flags_override_defaults() {
        let jito = JitoConfig {
            enabled: true,
            ..Default::default()
        };
        let mut flags = HashMap::new();
        flags.insert(
            "tip-payment-program-pubkey".to_string(),
            "CustomTipPayment1111111111111111111111111111".to_string(),
        );
        flags.insert("commission-bps".to_string(), "1000".to_string());
        let exec = jito_exec("mainnet", &jito, &flags);
        assert!(exec.contains("--tip-payment-program-pubkey CustomTipPayment1111111111111111111111111111"));
        assert!(exec.contains("--commission-bps 1000"));
        // The default commission/tip-payment must not also be emitted.
        assert!(!exec.contains("--commission-bps 800"));
        assert!(!exec.contains("T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt"));
    }

    #[test]
    fn no_port_check_appended_only_when_requested() {
        let base = |npc: bool| {
            build_exec_start(
                "/usr/local/bin/agave-validator",
                "/id.json",
                "/vote.json",
                "/l",
                "/s",
                "/a",
                8899,
                8001,
                "8000-8030",
                &[],
                &[],
                "testnet",
                &JitoConfig::default(),
                false,
                &[],
                &HashMap::new(),
                &[],
                npc,
            )
        };
        assert!(base(true).contains("--no-port-check"));
        assert!(!base(false).contains("--no-port-check"));
    }
}
