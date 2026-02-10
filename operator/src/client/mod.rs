use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

/// Which validator client software is running on this node.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Display, EnumString,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ClientKind {
    #[default]
    Agave,
    Jito,
    Firedancer,
    Frankendancer,
    Dummy,
}

/// Concrete validator client — holds service name and binary path for the configured client kind.
pub struct ValidatorClient {
    service_name: &'static str,
    binary_path: PathBuf,
}

impl ValidatorClient {
    pub fn from_kind(kind: ClientKind) -> Self {
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
            ClientKind::Dummy => Self {
                service_name: "dummy-validator",
                binary_path: PathBuf::from("/dev/null"),
            },
        }
    }

    pub fn service_name(&self) -> &str {
        self.service_name
    }

    pub fn binary_path(&self) -> &PathBuf {
        &self.binary_path
    }
}
