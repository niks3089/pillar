//! Masks secret values in scripts and script output before they are shown in
//! the UI, stored in the controller DB, or written to logs.
//!
//! Commands stay readable — only the values are replaced — so an operator can
//! still see exactly what ran: `Environment=FOO_API_KEY=***`,
//! `https://rpc.example.com/?api-key=***`.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;

pub const MASK: &str = "***";

/// `NAME=value` where NAME looks secret (env vars, unit `Environment=` lines, exports).
static SECRET_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b([A-Z0-9_]*(?:TOKEN|SECRET|PASSWORD|PASSWD|API_?KEY|AUTHORIZATION|CREDENTIAL|PRIVATE_?KEY)[A-Z0-9_]*=)("[^"]*"|'[^']*'|[^\s"'&]+)"#,
    )
    .expect("valid regex")
});

/// URL query parameters that carry credentials (`?api-key=…`, `&token=…`).
static SECRET_QUERY_PARAM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)([?&](?:api[-_]?key|token|access_token|auth|secret|key)=)[^&\s"'#]+"#)
        .expect("valid regex")
});

/// CLI flags whose next argument is a secret (`--token abc`, `--password=abc`).
static SECRET_FLAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(--(?:auth-)?(?:token|password|secret|api-key)(?:=|\s+))("[^"]*"|'[^']*'|[^\s"']+)"#,
    )
    .expect("valid regex")
});

/// InfluxDB password inside `SOLANA_METRICS_CONFIG=host=…,db=…,u=…,p=<password>`.
static METRICS_PASSWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(SOLANA_METRICS_CONFIG=["']?[^\s"']*?\bp=)[^,\s"']+"#).expect("valid regex")
});

/// URL userinfo: `https://user:pass@host`.
static URL_USERINFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"([a-z][a-z0-9+.-]*://[^/\s:@'"]+:)[^@\s/'"]+@"#).expect("valid regex")
});

pub fn redact(text: &str) -> Cow<'_, str> {
    let mut out = Cow::Borrowed(text);
    for (re, replacement) in [
        (&*METRICS_PASSWORD, "${1}***"),
        (&*SECRET_ASSIGNMENT, "${1}***"),
        (&*SECRET_QUERY_PARAM, "${1}***"),
        (&*SECRET_FLAG, "${1}***"),
        (&*URL_USERINFO, "${1}***@"),
    ] {
        if let Cow::Owned(s) = re.replace_all(&out, replacement) {
            out = Cow::Owned(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn masks_secret_env_assignments_but_keeps_names() {
        assert_eq!(
            redact("Environment=HELIUS_API_KEY=abc123\nEnvironment=RUST_LOG=info"),
            "Environment=HELIUS_API_KEY=***\nEnvironment=RUST_LOG=info"
        );
        assert_eq!(
            redact("export SOLANA_METRICS_PASSWORD='p w'"),
            "export SOLANA_METRICS_PASSWORD=***"
        );
        assert_eq!(redact("AUTH_TOKEN=\"x y\" run"), "AUTH_TOKEN=*** run");
        // Public keys are not secrets.
        assert_eq!(redact("AUTHORIZED_VOTER=7Np4"), "AUTHORIZED_VOTER=7Np4");
    }

    #[test]
    fn masks_credentials_in_urls() {
        assert_eq!(
            redact("- https://mainnet.helius-rpc.com/?api-key=deadbeef"),
            "- https://mainnet.helius-rpc.com/?api-key=***"
        );
        assert_eq!(
            redact("http://h/x?a=1&token=t0k&b=2"),
            "http://h/x?a=1&token=***&b=2"
        );
        assert_eq!(
            redact("https://user:hunter2@rpc.example.com/"),
            "https://user:***@rpc.example.com/"
        );
    }

    #[test]
    fn masks_metrics_password_but_keeps_host_and_db() {
        assert_eq!(
            redact("Environment=SOLANA_METRICS_CONFIG=host=https://metrics.solana.com:8086,db=tds,u=testnet_write,p=c4fa84"),
            "Environment=SOLANA_METRICS_CONFIG=host=https://metrics.solana.com:8086,db=tds,u=testnet_write,p=***"
        );
    }

    #[test]
    fn masks_secret_flags() {
        assert_eq!(
            redact("install --token abc --cluster devnet"),
            "install --token *** --cluster devnet"
        );
        assert_eq!(redact("x --password=hunter2"), "x --password=***");
    }

    #[test]
    fn leaves_ordinary_commands_alone() {
        let s = "sudo systemctl stop surfpool\nrm -f /home/sol/.pillar/surfpool.hold\necho \"$sha  file\" | sha256sum -c";
        assert_eq!(redact(s), s);
        assert_eq!(
            redact("--known-validator 7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2"),
            "--known-validator 7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2"
        );
    }
}
