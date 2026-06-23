use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rcgen::{
    CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType,
};

/// Paths to generated certificate files.
pub struct CertPaths {
    pub ca_cert: PathBuf,
    pub ca_key: PathBuf,
    pub server_cert: PathBuf,
    pub server_key: PathBuf,
}

/// Ensure TLS certificates exist in `certs_dir`. If any are missing, generate
/// a CA + server cert using a self-signed CA.
///
/// `external_url` is parsed to extract the hostname for the server certificate SAN.
pub fn ensure_certs(certs_dir: &str, external_url: &str) -> Result<CertPaths> {
    let dir = Path::new(certs_dir);
    fs::create_dir_all(dir).context(format!("creating certs directory {certs_dir}"))?;

    let paths = CertPaths {
        ca_cert: dir.join("ca.pem"),
        ca_key: dir.join("ca-key.pem"),
        server_cert: dir.join("server.pem"),
        server_key: dir.join("server-key.pem"),
    };

    // If all files exist, skip generation.
    if paths.ca_cert.exists()
        && paths.ca_key.exists()
        && paths.server_cert.exists()
        && paths.server_key.exists()
    {
        tracing::info!(dir = certs_dir, "TLS certificates already exist, skipping generation");
        return Ok(paths);
    }

    tracing::info!(dir = certs_dir, "generating TLS certificates");

    // --- CA ---
    let ca_key_pair = KeyPair::generate().context("generating CA key pair")?;
    let mut ca_params =
        CertificateParams::new(Vec::<String>::new()).context("creating CA params")?;
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Pillar CA");
    ca_params
        .distinguished_name
        .push(DnType::OrganizationName, "Pillar");
    ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_cert = ca_params
        .self_signed(&ca_key_pair)
        .context("self-signing CA certificate")?;

    fs::write(&paths.ca_cert, ca_cert.pem()).context("writing ca.pem")?;
    fs::write(&paths.ca_key, ca_key_pair.serialize_pem()).context("writing ca-key.pem")?;

    // --- Server cert ---
    let server_key_pair = KeyPair::generate().context("generating server key pair")?;
    let san_entries = build_server_sans(external_url);
    let mut server_params =
        CertificateParams::new(Vec::<String>::new()).context("creating server params")?;
    server_params
        .distinguished_name
        .push(DnType::CommonName, "Pillar Controller");
    server_params.subject_alt_names = san_entries;
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server_params
        .signed_by(&server_key_pair, &ca_cert, &ca_key_pair)
        .context("signing server certificate")?;

    fs::write(&paths.server_cert, server_cert.pem()).context("writing server.pem")?;
    fs::write(&paths.server_key, server_key_pair.serialize_pem())
        .context("writing server-key.pem")?;

    tracing::info!(dir = certs_dir, "TLS certificates generated successfully");
    Ok(paths)
}

/// Issue a client certificate (CN = node_id) signed by the CA in `certs_dir`, for
/// mTLS node authentication. Returns (cert_pem, key_pem).
pub fn issue_client_cert(certs_dir: &str, node_id: &str) -> Result<(String, String)> {
    let dir = Path::new(certs_dir);
    let ca_cert_pem = fs::read_to_string(dir.join("ca.pem")).context("reading ca.pem")?;
    let ca_key_pem = fs::read_to_string(dir.join("ca-key.pem")).context("reading ca-key.pem")?;

    let ca_key = KeyPair::from_pem(&ca_key_pem).context("loading CA key")?;
    let ca_cert = CertificateParams::from_ca_cert_pem(&ca_cert_pem)
        .context("loading CA cert")?
        .self_signed(&ca_key)
        .context("reconstructing CA cert")?;

    let client_key = KeyPair::generate().context("generating client key")?;
    let mut params =
        CertificateParams::new(Vec::<String>::new()).context("creating client params")?;
    params.distinguished_name.push(DnType::CommonName, node_id);
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let cert = params
        .signed_by(&client_key, &ca_cert, &ca_key)
        .context("signing client certificate")?;

    Ok((cert.pem(), client_key.serialize_pem()))
}

/// Generate a random 32-byte hex token for agent authentication.
pub fn generate_token() -> String {
    use std::fmt::Write;
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).expect("getrandom failed");
    let mut hex = String::with_capacity(64);
    for b in &buf {
        write!(hex, "{b:02x}").unwrap();
    }
    hex
}

/// Build SAN entries for the server certificate from the external_url.
/// Always includes localhost and 127.0.0.1.
fn build_server_sans(external_url: &str) -> Vec<SanType> {
    let mut sans = vec![
        SanType::DnsName("localhost".try_into().expect("localhost is valid")),
        SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
    ];

    if let Some(host) = extract_host(external_url) {
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if !ip.is_loopback() {
                sans.push(SanType::IpAddress(ip));
            }
        } else if host != "localhost" {
            if let Ok(dns) = host.try_into() {
                sans.push(SanType::DnsName(dns));
            }
        }
    }

    sans
}

fn extract_host(url: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let host_port = without_scheme.split('/').next()?;
    if host_port.is_empty() {
        return None;
    }
    if host_port.starts_with('[') {
        let end = host_port.find(']')?;
        Some(host_port[1..end].to_string())
    } else {
        Some(
            host_port
                .rsplit_once(':')
                .map(|(h, _)| h)
                .unwrap_or(host_port)
                .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_host_from_url() {
        assert_eq!(
            extract_host("http://1.2.3.4:50051"),
            Some("1.2.3.4".to_string())
        );
        assert_eq!(
            extract_host("https://host.example.com:50051"),
            Some("host.example.com".to_string())
        );
        assert_eq!(
            extract_host("http://localhost:8080"),
            Some("localhost".to_string())
        );
        assert_eq!(extract_host(""), None);
    }

    #[test]
    fn generate_certs_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ensure_certs(dir.path().to_str().unwrap(), "http://10.0.0.1:50051").unwrap();
        assert!(paths.ca_cert.exists());
        assert!(paths.ca_key.exists());
        assert!(paths.server_cert.exists());
        assert!(paths.server_key.exists());

        // Running again should skip generation
        let paths2 =
            ensure_certs(dir.path().to_str().unwrap(), "http://10.0.0.1:50051").unwrap();
        assert!(paths2.ca_cert.exists());
    }

    #[test]
    fn issued_client_cert_has_node_id_cn() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().to_str().unwrap();
        ensure_certs(d, "http://10.0.0.1:50051").unwrap();

        let (cert_pem, key_pem) = issue_client_cert(d, "mainnet-validator-1").unwrap();
        assert!(key_pem.contains("BEGIN PRIVATE KEY"));

        let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()).unwrap();
        assert_eq!(
            crate::grpc_server::leaf_common_name(&pem.contents).as_deref(),
            Some("mainnet-validator-1")
        );
    }

    #[test]
    fn token_is_64_hex_chars() {
        let token = generate_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
