# Jito Validator

## Overview

Jito is a fork of the Agave validator maintained by the [Jito Foundation](https://www.jito.wtf/) that adds MEV (Maximal Extractable Value) capabilities to the Solana validator. It integrates with the Jito Block Engine to receive transaction bundles -- groups of up to 5 transactions that execute atomically -- enabling validators to earn additional revenue through MEV tips on top of standard staking rewards.

- **Maintainer:** Jito Foundation
- **Repository:** [github.com/jito-foundation/jito-solana](https://github.com/jito-foundation/jito-solana)
- **Language:** Rust (fork of Agave)
- **License:** Apache 2.0
- **Status:** Production-ready; widely deployed on mainnet-beta for MEV-enabled validation

Jito is a superset of Agave. All standard Agave CLI flags and behavior apply. This document focuses on the Jito-specific additions; refer to `Agave.md` for the base validator configuration.

---

## Hardware Requirements

Jito has the same hardware requirements as Agave since it is a direct fork with additional MEV logic.

### Minimum

| Component | Specification |
|-----------|--------------|
| **CPU** | 12 cores / 24 threads, 2.8 GHz+ base clock |
| **CPU Features** | SHA extensions, AVX2 |
| **RAM** | 256 GB |
| **Storage** | 2 TB NVMe PCIe Gen3 x4, high TBW |
| **Network** | 1 Gbps symmetric |
| **OS** | Ubuntu 20.04+ or similar Linux |

### Recommended

| Component | Specification |
|-----------|--------------|
| **CPU** | 16 cores / 32 threads, 3.0 GHz+ |
| **RAM** | 512 GB with ECC |
| **Storage** | Separate NVMe for accounts, ledger, and snapshots |
| **Network** | 10 Gbps symmetric |

### Additional: Block Engine Latency

For optimal MEV performance, the round-trip latency to the Block Engine should be:

| Latency | Quality |
|---------|---------|
| < 50 ms | Ideal |
| < 100 ms | Acceptable |
| > 100 ms | May miss bundles |

Choose a Block Engine region close to your validator's data center.

---

## Building / Installing

### Method 1: Prebuilt Binaries from GitHub (Recommended)

Download from [Jito-Solana Releases](https://github.com/jito-foundation/jito-solana/releases):

```bash
# Example for a specific version
wget https://github.com/jito-foundation/jito-solana/releases/download/v2.1.6-jito/jito-solana-release-x86_64-unknown-linux-gnu.tar.bz2
tar xjf jito-solana-release-x86_64-unknown-linux-gnu.tar.bz2
sudo install -m 755 jito-solana-release/bin/jito-validator /usr/local/bin/jito-validator
```

### Method 2: Build from Source

```bash
# Prerequisites (same as Agave)
sudo apt install -y build-essential pkg-config libudev-dev llvm libclang-dev protobuf-compiler

# Clone and build
git clone https://github.com/jito-foundation/jito-solana.git
cd jito-solana
git checkout v2.1.6-jito  # Or the latest release tag
CARGO_BUILD_JOBS=8 scripts/cargo-install-all.sh --validator-only .
```

### Versioning

Jito versions track the upstream Agave version with a `-jito` suffix:

```
v2.1.6-jito    -> Based on Agave v2.1.6
v2.0.15-jito   -> Based on Agave v2.0.15
```

Jito releases typically follow Agave releases within a few days to a week.

---

## Initializing

Initialization is identical to Agave. See the [Agave Initializing section](./Agave.md#initializing) for:

- Creating the `sol` user
- System tuning (sysctl, ulimits)
- CPU governor
- Keypair generation
- Data directory creation
- Vote account creation

All sysctl, ulimit, disk layout, and user setup steps are the same.

---

## Configuring

Jito uses CLI flags just like Agave. All Agave flags are supported, plus additional MEV-specific flags.

### MEV-Specific Flags

| Flag | Description | Required |
|------|-------------|----------|
| `--block-engine-url <URL>` | Block Engine endpoint for receiving bundles | Yes (for MEV) |
| `--shred-receiver-address <IP:PORT>` | Address for low-latency shred forwarding | Recommended |
| `--tip-payment-program-pubkey <PUBKEY>` | On-chain tip payment program | Yes (for MEV) |
| `--tip-distribution-program-pubkey <PUBKEY>` | On-chain tip distribution program | Yes (for MEV) |
| `--merkle-root-upload-authority <PUBKEY>` | Authority for merkle root uploads | Yes (for MEV) |
| `--commission-bps <BPS>` | MEV commission in basis points (100ths of a percent) | Yes (for MEV) |

### On-Chain Program Addresses

#### Mainnet-beta

| Program | Pubkey |
|---------|--------|
| **Tip Payment** | `T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt` |
| **Tip Distribution** | `4R3gSG8BpU4t19KYj8CfnBtxhxJBjKHHaBnQ4SYnHNDn` |
| **Merkle Root Upload Authority** | `GZctHpWXmsZC1YHACTGGcHhYxjdRqQvTpYkb9LMvxDIb` |

#### Testnet

| Program | Pubkey |
|---------|--------|
| **Tip Payment** | `GJHtFqM9agxPmkeKjHny6qiRKrXZALvvFGiKf11QE7hy` |
| **Tip Distribution** | `DzvGET57TAgEDxvm3ERUM4GNcsAJdqjDLCne9sdfY4wf` |
| **Merkle Root Upload Authority** | `7T4inmPmtNBX3MhLwJ9hFsSMnGJYYkKioVABSNTWVRuS` |

### Block Engine URLs (Mainnet Regional)

| Region | Block Engine URL | Shred Receiver Address |
|--------|------------------|------------------------|
| Amsterdam | `https://amsterdam.mainnet.block-engine.jito.wtf` | `74.118.140.240:1002` |
| Dublin | `https://dublin.mainnet.block-engine.jito.wtf` | `64.130.61.8:1002` |
| Frankfurt | `https://frankfurt.mainnet.block-engine.jito.wtf` | `64.130.50.14:1002` |
| London | `https://london.mainnet.block-engine.jito.wtf` | `142.91.127.175:1002` |
| New York | `https://ny.mainnet.block-engine.jito.wtf` | `141.98.216.96:1002` |
| Salt Lake City | `https://slc.mainnet.block-engine.jito.wtf` | `64.130.53.8:1002` |
| Singapore | `https://singapore.mainnet.block-engine.jito.wtf` | `202.8.11.224:1002` |
| Tokyo | `https://tokyo.mainnet.block-engine.jito.wtf` | `202.8.9.160:1002` |

### Block Engine URLs (Testnet)

| Region | Block Engine URL |
|--------|------------------|
| Dallas | `https://dallas.testnet.block-engine.jito.wtf` |
| New York | `https://ny.testnet.block-engine.jito.wtf` |

### Example: Mainnet Consensus Validator with MEV

```bash
jito-validator \
  --identity /home/sol/validator-keypair.json \
  --vote-account /home/sol/vote-account-keypair.json \
  --ledger /mnt/ledger \
  --snapshots /mnt/snapshots \
  --accounts /mnt/accounts \
  --rpc-port 8899 \
  --gossip-port 8001 \
  --dynamic-port-range 8000-8020 \
  --entrypoint entrypoint.mainnet-beta.solana.com:8001 \
  --entrypoint entrypoint2.mainnet-beta.solana.com:8001 \
  --known-validator 7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2 \
  --only-known-rpc \
  --no-genesis-fetch \
  --limit-ledger-size \
  --tip-payment-program-pubkey T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt \
  --tip-distribution-program-pubkey 4R3gSG8BpU4t19KYj8CfnBtxhxJBjKHHaBnQ4SYnHNDn \
  --merkle-root-upload-authority GZctHpWXmsZC1YHACTGGcHhYxjdRqQvTpYkb9LMvxDIb \
  --commission-bps 800 \
  --block-engine-url https://ny.mainnet.block-engine.jito.wtf \
  --shred-receiver-address 141.98.216.96:1002 \
  --log /home/sol/jito-validator.log
```

### Example: Testnet Validator with MEV

```bash
jito-validator \
  --identity /home/sol/validator-keypair.json \
  --vote-account /home/sol/vote-account-keypair.json \
  --ledger /mnt/ledger \
  --snapshots /mnt/snapshots \
  --accounts /mnt/accounts \
  --rpc-port 8899 \
  --gossip-port 8001 \
  --dynamic-port-range 8000-8020 \
  --entrypoint entrypoint.testnet.solana.com:8001 \
  --entrypoint entrypoint2.testnet.solana.com:8001 \
  --known-validator 5D1fNXzvv5NjV1ysLjirC4WY92RNsVH18vjmcszZd8on \
  --only-known-rpc \
  --no-genesis-fetch \
  --limit-ledger-size \
  --tip-payment-program-pubkey GJHtFqM9agxPmkeKjHny6qiRKrXZALvvFGiKf11QE7hy \
  --tip-distribution-program-pubkey DzvGET57TAgEDxvm3ERUM4GNcsAJdqjDLCne9sdfY4wf \
  --merkle-root-upload-authority 7T4inmPmtNBX3MhLwJ9hFsSMnGJYYkKioVABSNTWVRuS \
  --commission-bps 800 \
  --block-engine-url https://ny.testnet.block-engine.jito.wtf \
  --log /home/sol/jito-validator.log
```

### Runtime Reconfiguration (No Restart Required)

Jito supports changing the Block Engine URL and shred receiver address at runtime via the admin RPC socket:

```bash
# Change block engine URL
jito-validator -l /mnt/ledger set-block-engine-config \
  --block-engine-url https://ny.mainnet.block-engine.jito.wtf

# Change shred receiver address
jito-validator -l /mnt/ledger set-shred-receiver-address \
  141.98.216.96:1002
```

### Systemd Unit File

```ini
[Unit]
Description=Jito Validator
After=network.target
StartLimitIntervalSec=0

[Service]
Type=simple
User=sol
ExecStart=/usr/local/bin/jito-validator \
  --identity /home/sol/validator-keypair.json \
  --vote-account /home/sol/vote-account-keypair.json \
  --ledger /mnt/ledger \
  --snapshots /mnt/snapshots \
  --accounts /mnt/accounts \
  --tip-payment-program-pubkey T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt \
  --tip-distribution-program-pubkey 4R3gSG8BpU4t19KYj8CfnBtxhxJBjKHHaBnQ4SYnHNDn \
  --merkle-root-upload-authority GZctHpWXmsZC1YHACTGGcHhYxjdRqQvTpYkb9LMvxDIb \
  --commission-bps 800 \
  --block-engine-url https://ny.mainnet.block-engine.jito.wtf \
  --shred-receiver-address 141.98.216.96:1002 \
  --limit-ledger-size
Restart=on-failure
RestartSec=1
LimitNOFILE=1000000
LogRateLimitIntervalSec=0

[Install]
WantedBy=multi-user.target
```

---

## Tuning

### Base Tuning

All Agave tuning applies to Jito. See the [Agave Tuning section](./Agave.md#tuning) for:

- Disk layout (separate NVMe for ledger, accounts, snapshots)
- RAM disk for accounts
- Network tuning (sysctl buffer sizes)
- CPU governor
- Firewall ports

### MEV-Specific Tuning

**Block Engine Latency:** The most important MEV-specific tuning factor is network latency to the Block Engine. Tips are time-sensitive; bundles arriving late may not be included in blocks.

- Choose the Block Engine region closest to your data center
- Ensure stable, low-latency networking to the Block Engine endpoint
- Monitor the `block_engine_stage-stats` metric for connection health

**Commission BPS:** The `--commission-bps` flag controls the percentage of MEV tips the validator keeps vs distributing to stakers:

| BPS Value | Validator Keep | Staker Receive |
|-----------|---------------|----------------|
| 0 | 0% | 100% |
| 500 | 5% | 95% |
| 800 | 8% | 92% |
| 1000 | 10% | 90% |

Higher commission attracts less stake; lower commission attracts more. Most validators run 800-1000 bps.

---

## Monitoring

### Standard Monitoring (Same as Agave)

```bash
# Check gossip
solana gossip | grep <IDENTITY_PUBKEY>

# Check catchup
solana catchup <IDENTITY_PUBKEY>

# Check voting and stake
solana validators | grep <IDENTITY_PUBKEY>

# Monitor via admin socket
jito-validator -l /mnt/ledger monitor
```

### Jito-Specific Metrics

Jito exposes additional InfluxDB metrics beyond standard Agave:

| Metric | Description |
|--------|-------------|
| `block_engine_stage-stats` | Block Engine connection status, bundles received/processed |
| `bundle_stage-stats` | Bundle execution statistics, tips earned |

These metrics are reported to the standard `SOLANA_METRICS_CONFIG` endpoint.

### Tip Monitoring

**Tip Floor API** (current minimum tip to be competitive):

```bash
curl -s https://bundles.jito.wtf/api/v1/bundles/tip_floor | jq
```

**Jito Tip Dashboard:** [jito-labs.metabaseapp.com](https://jito-labs.metabaseapp.com/public/dashboard/016d4d60-e168-4a8f-93c7-4cd5ec6c7c8d)

### Verifying MEV is Working

1. Check Block Engine connectivity in logs:
   ```bash
   grep -i "block.engine" /home/sol/jito-validator.log | tail -10
   ```

2. Verify bundles are being received:
   ```bash
   grep -i "bundle" /home/sol/jito-validator.log | tail -10
   ```

3. Check tip earnings via the Jito tip dashboard or on-chain tip distribution accounts.

---

## Pillar Integration

Pillar provisions and manages the Jito validator through the following configuration:

### Service Details

| Property | Value |
|----------|-------|
| **Service name** | `jito-validator` |
| **Binary path** | `/usr/local/bin/jito-validator` |
| **Config method** | CLI flags (no config file) |
| **Systemd unit** | `/etc/systemd/system/jito-validator.service` |
| **Runs as** | `sol` user |

### How Pillar Provisions Jito

1. User fills out the "Setup Validator" form in the controller UI with client = Jito and enables the "Jito MEV" checkbox (which reveals the Block Engine URL input)
2. Controller sends `ProvisionCommand` to the agent via gRPC `CommandStream`
3. Agent downloads the binary from the provided `download_url`, verifies SHA256
4. Agent installs the binary to `/usr/local/bin/jito-validator`
5. Agent generates a systemd unit with all CLI flags in the `ExecStart` line, including MEV flags

### MEV Flag Generation

When `jito_mev` is enabled and the client is Jito, Pillar automatically adds these flags to the `ExecStart`:

- `--block-engine-url <jito_block_engine_url>` (from the UI input)
- `--tip-payment-program-pubkey T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt` (mainnet default)
- `--tip-distribution-program-pubkey 4R3gSG8BpU4t19KYj8CfnBtxhxJBjKHHaBnQ4SYnHNDn` (mainnet default)
- `--commission-bps 800` (default 8%)

Each of these defaults can be overridden via `validator_flags` in the provision command. For example, setting `tip-payment-program-pubkey` in the validator flags map will use that value instead of the default.

### ExecStart Generation

The Jito `ExecStart` is built identically to Agave (all standard flags) plus the MEV flags above. Yellowstone gRPC is also supported via `--geyser-plugin-config`.

### Upgrades

Binary-only upgrade: stop `jito-validator`, replace `/usr/local/bin/jito-validator`, restart. Configuration and systemd unit are not changed during upgrades.

---

## Sources

- [Jito MEV Documentation](https://jito-foundation.gitbook.io/mev)
- [Jito CLI Arguments](https://jito-foundation.gitbook.io/mev/jito-solana/command-line-arguments)
- [Jito On-Chain Addresses](https://jito-foundation.gitbook.io/mev/mev-payment-and-distribution/on-chain-addresses)
- [Jito Building the Software](https://jito-foundation.gitbook.io/mev/jito-solana/building-the-software)
- [Jito Checking Correct Operation](https://jito-foundation.gitbook.io/mev/jito-solana/checking-correct-operation)
- [Jito-Solana GitHub Releases](https://github.com/jito-foundation/jito-solana/releases)
- [Jito Block Engine API](https://docs.jito.wtf/lowlatencytxnsend/#api)
- [Jito Tip Dashboard](https://jito-labs.metabaseapp.com/public/dashboard/016d4d60-e168-4a8f-93c7-4cd5ec6c7c8d)
- [Jito FAQs](https://jito-foundation.gitbook.io/mev/jito-solana/faqs)
