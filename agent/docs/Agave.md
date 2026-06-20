# Agave Validator

## Overview

Agave is the reference Solana validator client, maintained by [Anza](https://www.anza.xyz/) (formerly Solana Labs). It is the most widely deployed validator on the Solana network and is the baseline implementation against which all other clients are compared. Agave handles consensus, block production, RPC serving, ledger management, and gossip networking.

- **Maintainer:** Anza
- **Repository:** [github.com/anza-xyz/agave](https://github.com/anza-xyz/agave)
- **Language:** Rust
- **License:** Apache 2.0
- **Status:** Production-ready; the default client for most Solana validators and RPC nodes

Previously known as `solana-validator` (from the `solana-labs/solana` repo), the project was renamed to Agave after the Anza spinoff. The binary is now `agave-validator` but the systemd service is still commonly named `solana-validator`.

---

## Hardware Requirements

### Minimum

| Component | Specification |
|-----------|--------------|
| **CPU** | 12 cores / 24 threads, 2.8 GHz+ base clock |
| **CPU Features** | SHA extensions, AVX2 (AVX-512f helpful but not required) |
| **CPU Architecture** | AMD Zen 3+ or Intel Ice Lake+ recommended |
| **RAM** | 256 GB |
| **Storage** | 2 TB NVMe PCIe Gen3 x4, high TBW rating |
| **Network** | 1 Gbps symmetric |
| **OS** | Ubuntu 20.04+ or similar Linux |

### Recommended

| Component | Specification |
|-----------|--------------|
| **CPU** | 16 cores / 32 threads, 3.0 GHz+ base clock |
| **RAM** | 512 GB with ECC |
| **Storage** | Separate NVMe disks for accounts (1 TB+), ledger (1 TB+), and snapshots (500 GB+) |
| **Network** | 10 Gbps symmetric |

### RPC Nodes (full indexed)

RPC nodes serving the full API with transaction history need more resources:

| Component | Specification |
|-----------|--------------|
| **CPU** | 16 cores / 32 threads |
| **RAM** | 512 GB+ |
| **Storage** | Larger ledger disk if retaining transaction history |

For up-to-date hardware recommendations, see [solanahcl.org](https://solanahcl.org/).

---

## Building / Installing

### Method 1: Solana Install Tool (Recommended for operators)

The official install script downloads prebuilt binaries:

```bash
sh -c "$(curl -sSfL https://release.anza.xyz/v2.1.6/install)"
```

You can use a specific version tag or a channel:

```bash
# Specific version
sh -c "$(curl -sSfL https://release.anza.xyz/v2.1.6/install)"

# Latest stable
sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"

# Beta channel
sh -c "$(curl -sSfL https://release.anza.xyz/beta/install)"
```

After installation, add to PATH:

```bash
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
```

### Method 2: Prebuilt Binaries from GitHub

Download directly from [GitHub Releases](https://github.com/anza-xyz/agave/releases):

```bash
# Linux x86_64
wget https://github.com/anza-xyz/agave/releases/download/v2.1.6/solana-release-x86_64-unknown-linux-gnu.tar.bz2
tar xjf solana-release-x86_64-unknown-linux-gnu.tar.bz2
export PATH="$PWD/solana-release/bin:$PATH"
```

### Method 3: Build from Source

Prerequisites:

```bash
# Ubuntu/Debian
sudo apt install -y build-essential pkg-config libudev-dev llvm libclang-dev protobuf-compiler

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Build:

```bash
git clone https://github.com/anza-xyz/agave.git
cd agave
git checkout v2.1.6
./scripts/cargo-install-all.sh --validator-only .
export PATH="$PWD/bin:$PATH"
```

Building from source requires approximately 32 GB of available memory.

### Versioning

Agave uses semantic versioning: `vMAJOR.MINOR.PATCH` (e.g., `v2.1.6`).

- **Major:** Breaking changes to consensus or major API shifts
- **Minor:** Feature additions, performance improvements
- **Patch:** Bug fixes, security patches

---

## Initializing

### Create the `sol` User

The validator should run as a dedicated unprivileged user:

```bash
sudo useradd -m -s /bin/bash sol
sudo mkdir -p /home/sol
sudo chown sol:sol /home/sol
```

### System Tuning (sysctl)

Create `/etc/sysctl.d/21-agave-validator.conf`:

```ini
# Network buffer sizes (128 MB)
net.core.rmem_default = 134217728
net.core.rmem_max = 134217728
net.core.wmem_default = 134217728
net.core.wmem_max = 134217728

# Accounts database requires many memory-mapped files
vm.max_map_count = 1000000

# Accounts database requires many open file handles
fs.nr_open = 1000000
```

Apply:

```bash
sudo sysctl --system
```

### File Descriptor Limits

Create `/etc/security/limits.d/90-solana-nofiles.conf`:

```
sol - nofile 1000000
sol - memlock 2000000
```

Or add to the systemd unit directly:

```ini
[Service]
LimitNOFILE=1000000
LimitMEMLOCK=2000000000
```

### CPU Governor

Set to `performance` for consistent clock speeds:

```bash
echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor
```

To persist across reboots, use `cpufrequtils` or a systemd service.

### Generate Keypairs

```bash
# As the sol user
su - sol

# Validator identity
solana-keygen new -o ~/validator-keypair.json

# Vote account
solana-keygen new -o ~/vote-account-keypair.json

# Authorized withdrawer (store securely offline)
solana-keygen new -o ~/authorized-withdrawer-keypair.json
```

### Create Data Directories

```bash
sudo mkdir -p /mnt/ledger /mnt/snapshots /mnt/accounts
sudo chown sol:sol /mnt/ledger /mnt/snapshots /mnt/accounts
```

### Create a Vote Account (on-chain)

```bash
solana create-vote-account \
  ~/vote-account-keypair.json \
  ~/validator-keypair.json \
  ~/authorized-withdrawer-keypair.json \
  --commission 10
```

---

## Configuring

Agave is configured entirely through CLI flags passed to the `agave-validator` binary. There is no config file; all settings are flags on the ExecStart line of the systemd unit.

### Essential Flags

| Flag | Description | Default |
|------|-------------|---------|
| `--identity <PATH>` | Validator identity keypair | Required |
| `--vote-account <PATH_OR_PUBKEY>` | Vote account keypair or pubkey | Required for voting |
| `--ledger <PATH>` | Ledger directory | Required |
| `--accounts <PATH>` | Accounts directory | Inside ledger dir |
| `--snapshots <PATH>` | Snapshot storage directory | Inside ledger dir |
| `--rpc-port <PORT>` | JSON-RPC HTTP port | 8899 |
| `--gossip-port <PORT>` | Gossip protocol port | 8001 |
| `--dynamic-port-range <MIN-MAX>` | Range for dynamic ports (TPU, repair, etc.) | 8000-10000 |
| `--entrypoint <HOST:PORT>` | Gossip entrypoint (repeat for multiple) | Required |
| `--known-validator <PUBKEY>` | Trusted validator for snapshot download (repeat) | Recommended |
| `--expected-genesis-hash <HASH>` | Reject if genesis hash doesn't match | Recommended |

### Cluster-Specific Entrypoints

**Mainnet-beta:**
```
--entrypoint entrypoint.mainnet-beta.solana.com:8001
--entrypoint entrypoint2.mainnet-beta.solana.com:8001
--entrypoint entrypoint3.mainnet-beta.solana.com:8001
--entrypoint entrypoint4.mainnet-beta.solana.com:8001
--entrypoint entrypoint5.mainnet-beta.solana.com:8001
```

**Testnet:**
```
--entrypoint entrypoint.testnet.solana.com:8001
--entrypoint entrypoint2.testnet.solana.com:8001
--entrypoint entrypoint3.testnet.solana.com:8001
```

**Devnet:**
```
--entrypoint entrypoint.devnet.solana.com:8001
```

### Security & Networking Flags

| Flag | Description |
|------|-------------|
| `--only-known-rpc` | Only download snapshots from known validators |
| `--no-genesis-fetch` | Don't fetch genesis from the cluster (use with `--known-validator`) |
| `--private-rpc` | Don't publish RPC port in gossip |
| `--no-voting` | Disable consensus voting (for RPC nodes) |
| `--limit-ledger-size` | Prune old ledger data to save disk space |
| `--limit-ledger-size <SHREDS>` | Limit to N shreds (default: 200M) |
| `--full-rpc-api` | Enable all RPC methods including non-default ones |
| `--no-port-check` | Skip entrypoint port reachability check |

### Logging & Diagnostics

| Flag | Description |
|------|-------------|
| `--log <PATH>` | Log file path (default: stderr) |
| `--log-messages-bytes-limit <N>` | Max log message size in bytes |
| `--wal-recovery-mode skip_any_corrupted_record` | Recover from WAL corruption |

### Environment Variables

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Log level filter (e.g., `solana=info,solana_metrics=warn`) |
| `SOLANA_METRICS_CONFIG` | InfluxDB metrics endpoint for Solana dashboard |

### Example: Testnet Consensus Validator

```bash
agave-validator \
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
  --entrypoint entrypoint3.testnet.solana.com:8001 \
  --known-validator 5D1fNXzvv5NjV1ysLjirC4WY92RNsVH18vjmcszZd8on \
  --known-validator dDzy5SR3AXdYWVqbDEkVFdvSPCtS9ihF5kJkHCtXoFs \
  --only-known-rpc \
  --no-genesis-fetch \
  --expected-genesis-hash 4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY \
  --limit-ledger-size \
  --wal-recovery-mode skip_any_corrupted_record \
  --log /home/sol/agave-validator.log
```

### Example: Mainnet RPC Node (non-voting)

```bash
agave-validator \
  --identity /home/sol/validator-keypair.json \
  --no-voting \
  --ledger /mnt/ledger \
  --snapshots /mnt/snapshots \
  --accounts /mnt/accounts \
  --rpc-port 8899 \
  --full-rpc-api \
  --private-rpc \
  --dynamic-port-range 8000-8020 \
  --entrypoint entrypoint.mainnet-beta.solana.com:8001 \
  --entrypoint entrypoint2.mainnet-beta.solana.com:8001 \
  --known-validator 7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2 \
  --known-validator GdnSyH3YtwcxFvQrVVJMm1JhTS4QVX7MFsX56uJLUfiZ \
  --only-known-rpc \
  --no-genesis-fetch \
  --limit-ledger-size \
  --enable-rpc-transaction-history \
  --enable-extended-tx-metadata-storage \
  --log /home/sol/agave-validator.log
```

### Systemd Unit File

```ini
[Unit]
Description=Agave Validator
After=network.target
StartLimitIntervalSec=0

[Service]
Type=simple
User=sol
ExecStart=/usr/local/bin/agave-validator \
  --identity /home/sol/validator-keypair.json \
  --vote-account /home/sol/vote-account-keypair.json \
  --ledger /mnt/ledger \
  --snapshots /mnt/snapshots \
  --accounts /mnt/accounts \
  --limit-ledger-size
Restart=on-failure
RestartSec=1
LimitNOFILE=1000000
LogRateLimitIntervalSec=0

[Install]
WantedBy=multi-user.target
```

### Geyser Plugins (Yellowstone gRPC)

To enable Yellowstone gRPC for streaming account and transaction data:

```bash
--geyser-plugin-config /etc/pillar/yellowstone-grpc.json
```

Multiple `--geyser-plugin-config` flags can be specified for multiple plugins.

---

## Tuning

### Disk Layout

For best performance, use separate NVMe drives:

| Mount Point | Purpose | Recommended Size |
|-------------|---------|-----------------|
| `/mnt/ledger` | Ledger (blockstore) | 1 TB+ |
| `/mnt/accounts` | Accounts database | 1 TB+ |
| `/mnt/snapshots` | Snapshot archives | 500 GB+ |

Use `ext4` or `xfs` with `noatime` mount option:

```
/dev/nvme0n1p1 /mnt/ledger ext4 defaults,noatime 0 0
/dev/nvme1n1p1 /mnt/accounts ext4 defaults,noatime 0 0
```

### RAM Disk for Accounts (Optional)

Using tmpfs for the accounts database reduces SSD wear and improves IOPS:

```bash
# /etc/fstab
tmpfs /mnt/accounts tmpfs rw,size=300G,user=sol 0 0
```

Requires enough swap to prevent OOM:

```bash
sudo fallocate -l 250G /swapfile
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile
```

### Network Tuning

Ensure the sysctl values from the Initializing section are applied. For 10 Gbps networks, also consider:

```ini
net.ipv4.tcp_rmem = 4096 87380 134217728
net.ipv4.tcp_wmem = 4096 87380 134217728
```

### Firewall Ports

| Port | Protocol | Purpose |
|------|----------|---------|
| 8000-8020 | TCP+UDP | Dynamic port range (TPU, repair, etc.) |
| 8899 | TCP | JSON-RPC |
| 8900 | TCP | RPC WebSocket |
| 8001 | UDP | Gossip |

---

## Monitoring

### Solana CLI Tools

```bash
# Check if validator has joined gossip
solana gossip | grep <IDENTITY_PUBKEY>

# Check slot catchup status
solana catchup <IDENTITY_PUBKEY>
# Or for localhost:
solana catchup --our-localhost

# Check voting status
solana validators | grep <IDENTITY_PUBKEY>

# Check block production
solana block-production | grep <IDENTITY_PUBKEY>

# Check balance
solana balance <IDENTITY_PUBKEY>

# Check epoch info
solana epoch-info

# Check leader schedule
solana leader-schedule | grep <IDENTITY_PUBKEY>
```

### RPC Health Endpoints

```bash
# Basic health check (200 = healthy, 503 = behind)
curl http://localhost:8899/health

# Get current slot
curl -s http://localhost:8899 -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getSlot"}'

# Get health status
curl -s http://localhost:8899 -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}'

# Get vote accounts
curl -s http://localhost:8899 -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getVoteAccounts"}'
```

### Agave Watchtower

`agave-watchtower` is a monitoring tool that can automatically restart the validator or send notifications:

```bash
agave-watchtower \
  --validator-identity <PUBKEY> \
  --monitor-active-stake \
  --notify-on-transactions
```

### Metrics Dashboard

Agave can report metrics to the Solana InfluxDB dashboard:

```bash
# Testnet
export SOLANA_METRICS_CONFIG="host=https://metrics.solana.com:8086,db=tds,u=testnet_write,p=c4fa841aa918bf8274e3e2a44d77568d9861b3ea"

# Mainnet
export SOLANA_METRICS_CONFIG="host=https://metrics.solana.com:8086,db=mainnet-beta,u=mainnet-beta_write,p=password"
```

Dashboard: [metrics.solana.com:3000](https://metrics.solana.com:3000/d/monitor/cluster-telemetry)

### Log Analysis

```bash
# Tail the log
tail -f /home/sol/agave-validator.log

# Search for errors
grep -i error /home/sol/agave-validator.log | tail -20

# Check slot progress
grep "slot.*processed" /home/sol/agave-validator.log | tail -5
```

---

## Pillar Integration

Pillar provisions and manages the Agave validator through the following configuration:

### Service Details

| Property | Value |
|----------|-------|
| **Service name** | `solana-validator` |
| **Binary path** | `/usr/local/bin/agave-validator` |
| **Config method** | CLI flags (no config file) |
| **Systemd unit** | `/etc/systemd/system/solana-validator.service` |
| **Runs as** | `sol` user |

### How Pillar Provisions Agave

1. User fills out the "Setup Validator" form in the controller UI (client = Agave, version, cluster, paths, keypairs, entrypoints, known validators, addons)
2. Controller sends `ProvisionCommand` to the agent via gRPC `CommandStream`
3. Agent downloads the binary from the provided `download_url`, verifies SHA256
4. Agent installs the binary to `/usr/local/bin/agave-validator`
5. Agent generates a systemd unit with all CLI flags in the `ExecStart` line
6. If Yellowstone gRPC is enabled, writes `/etc/pillar/yellowstone-grpc.json` and adds `--geyser-plugin-config` flag
7. Agent runs `systemctl daemon-reload` then `systemctl enable --now solana-validator`
8. Agent updates its own config (`/etc/pillar/agent.yaml`) with client, cluster, and service name

### ExecStart Generation

Pillar builds the Agave `ExecStart` line from the `ProvisionConfig`:

- Base flags: `--identity`, `--ledger`, `--snapshots`, `--accounts`, `--rpc-port`, `--gossip-port`, `--dynamic-port-range`
- `--vote-account` added unless `no-voting` is in `validator_flags`
- `--entrypoint` for each entrypoint
- `--known-validator` for each known validator; if any are present, also adds `--only-known-rpc` and `--no-genesis-fetch`
- Custom flags from `validator_flags` map (e.g., `limit-ledger-size`, `private-rpc`, `full-rpc-api`)
- `extra_args` appended at the end

### Upgrades

Binary-only upgrade: stop `solana-validator`, replace `/usr/local/bin/agave-validator`, restart. The systemd unit and configuration are not changed during upgrades.

---

## Sources

- [Anza Validator Setup Guide](https://docs.anza.xyz/operations/setup-a-validator)
- [Anza Hardware Requirements](https://docs.anza.xyz/operations/requirements)
- [Solana CLI Installation](https://docs.anza.xyz/cli/install)
- [Agave GitHub Releases](https://github.com/anza-xyz/agave/releases)
- [Validator Start Guide](https://docs.solanalabs.com/operations/guides/validator-start)
- [Validator Monitoring Guide](https://docs.solanalabs.com/operations/guides/validator-monitor)
- [RPC Node Setup](https://docs.solanalabs.com/operations/setup-an-rpc-node)
- [Solana Hardware Compatibility List](https://solanahcl.org/)
- [Solana Metrics Dashboard](https://metrics.solana.com:3000/d/monitor/cluster-telemetry)
