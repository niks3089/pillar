# Frankendancer Validator

## Overview

Frankendancer is a hybrid Solana validator that combines high-performance Firedancer components with the production-proven Agave runtime. It replaces the Agave networking stack and block production pipeline with Firedancer's C implementation (AF_XDP networking, QUIC, signature verification, transaction scheduling, shredding) while keeping Agave for replay, gossip, repair, and RPC.

- **Maintainer:** Firedancer team (Jump Crypto)
- **Repository:** [github.com/firedancer-io/firedancer](https://github.com/firedancer-io/firedancer) (same repo as Firedancer)
- **Language:** C (Firedancer tiles) + Rust (Agave subprocess)
- **Binary:** `fdctl` (same binary as Firedancer)
- **License:** Apache 2.0
- **Status:** **Production-ready.** The recommended way to run Firedancer technology today. Deployed on mainnet-beta by multiple validators.

### Architecture

Frankendancer runs as a process tree with two major components:

1. **Firedancer tiles** -- separate processes for networking, QUIC, signature verification, deduplication, packing, shredding, and other high-performance tasks. Each tile is pinned to a dedicated CPU core.
2. **Agave subprocess** -- runs replay, gossip, repair, RPC, and other consensus-critical components not yet reimplemented in Firedancer. The Agave threads share a set of CPU cores specified by `agave_affinity`.

```
fdctl run
 |
 +-- net:0           (Firedancer C tile, dedicated core)
 +-- quic:0          (Firedancer C tile, dedicated core)
 +-- verify:0..N     (Firedancer C tiles, dedicated cores)
 +-- dedup:0         (Firedancer C tile, dedicated core)
 +-- resolv:0        (Firedancer C tile, dedicated core)
 +-- pack:0          (Firedancer C tile, dedicated core)
 +-- bank:0..N       (Firedancer C tiles, dedicated cores)
 +-- poh:0           (Firedancer C tile, dedicated core)
 +-- shred:0..N      (Firedancer C tiles, dedicated cores)
 +-- store:0         (Firedancer C tile, dedicated core)
 +-- sign:0          (Firedancer C tile, dedicated core)
 +-- metric:0        (Firedancer C tile, dedicated core)
 +-- gui:0           (Firedancer C tile, optional)
 +-- plugin:0        (Firedancer C tile, optional)
 +-- diag:0          (Firedancer C tile, dedicated core)
 +-- run-agave       (Agave subprocess, shares agave_affinity cores)
      +-- 35+ threads (replay, gossip, repair, RPC, etc.)
```

If any process in the tree dies, all others are terminated.

### Data Flow (Leader Pipeline)

```
Network -> net -> quic -> verify -> dedup -> resolv -> pack -> bank -> poh -> shred -> store
                                                                                |
                                                                           Network (out)
```

Firedancer tiles handle the entire leader pipeline from network ingress through block distribution. The Agave subprocess handles replay (executing and confirming blocks from other leaders), gossip protocol, and repair requests.

### Blockstore Compatibility

The Firedancer blockstore in the ledger directory is compatible with Agave. You can switch between Frankendancer and Agave while keeping the same ledger directory.

---

## Hardware Requirements

### Minimum

| Component | Specification |
|-----------|--------------|
| **CPU** | 24 cores, 2.8 GHz+ base clock |
| **CPU Features** | AVX2 required; AVX-512 strongly recommended |
| **RAM** | 256 GB |
| **Storage** | 2 TB NVMe PCIe Gen3, high TBW |
| **Network** | 1 Gbps symmetric |
| **OS** | Linux only (kernel 4.18+: Ubuntu 20.04+, Fedora 29+, Debian 11+, RHEL 8+) |

### Recommended

| Component | Specification |
|-----------|--------------|
| **CPU** | 32 cores, 3.0 GHz+ with AVX-512 |
| **RAM** | 512 GB with ECC |
| **Storage** | Separate NVMe for accounts and ledger |
| **Network** | 1 Gbps+ symmetric |
| **NIC** | Intel X540 (ixgbe), X710 (i40e), or E800 (ice) for optimal XDP |

The hardware requirements include the overhead of the Agave subprocess. As more Agave components are replaced by Firedancer, requirements may decrease.

---

## Building / Installing

Frankendancer is built from the same source and produces the same binary (`fdctl`) as Firedancer.

### Prerequisites

- **GCC** 8.5+ (11, 12, 13 officially tested)
- **rustup** (required for the Agave components)
- **clang**, **git**, **make**
- Linux kernel 4.18+

### Build

```bash
git clone --recurse-submodules https://github.com/firedancer-io/firedancer.git
cd firedancer
git checkout v0.811.30108  # Latest Frankendancer release

# Install dependencies
./deps.sh

# Build
make -j fdctl solana
```

Binaries are placed in `./build/native/gcc/bin/`.

Building requires approximately 32 GB of available memory.

### Versioning

Frankendancer uses a three-component version: `v0.xxx.yyyyy`

| Component | Meaning | Example |
|-----------|---------|---------|
| **Major** | Always `0` until full Firedancer. Full Firedancer will be `1.x` | `0` |
| **Minor** | Increments by 100 for new releases, by 1 for patches | `811` (8th release, patch 11) |
| **Patch** | Encodes the Agave version: `v1.17.14` -> `11714`, `v2.1.6` -> `20106` | `30108` (Agave v3.1.8) |

```
================= main branch (bleeding edge, do not use) =================
   \                             \
    \ v0.100.11814                \ v0.200.11901
     \                             \
      \ v0.100.11815                \ v0.201.11902
       \
        \ v0.101.11815
```

### Updating

```bash
git fetch --tags
git checkout v0.811.30108  # New version tag
git submodule update
make -j fdctl solana
```

---

## Initializing

Frankendancer requires the same initialization as full Firedancer. See [Firedancer Initializing](./Firedancer.md#initializing) for full details.

### Quick Reference

```bash
# Run all initialization stages
sudo fdctl configure init all --config ~/config.toml

# Or individual stages:
sudo fdctl configure init hugetlbfs --config ~/config.toml     # Huge pages (run immediately after boot)
sudo fdctl configure init sysctl --config ~/config.toml         # Kernel parameters
sudo fdctl configure init hyperthreads --config ~/config.toml   # Check hyperthread pairs
sudo fdctl configure init ethtool-channels --config ~/config.toml
sudo fdctl configure init ethtool-offloads --config ~/config.toml
sudo fdctl configure init ethtool-loopback --config ~/config.toml
sudo fdctl configure init bonding --config ~/config.toml        # Only for bonded NICs
```

### Key Differences from Agave Initialization

| Step | Agave | Frankendancer |
|------|-------|---------------|
| sysctl tuning | Manual `/etc/sysctl.d/` file | `fdctl configure init sysctl` (automated) |
| Huge pages | Not needed | Required (`fdctl configure init hugetlbfs`) |
| NIC configuration | Not needed | Required for XDP (`ethtool-channels`, `ethtool-offloads`) |
| Hyperthread check | Not needed | Recommended (`fdctl configure init hyperthreads`) |
| File descriptor limits | Manual systemd/PAM config | Same, plus automated sysctl |
| CPU governor | Manual | Manual (same as Agave) |
| Run at boot | Optional | **Required** -- `fdctl configure init all` must run after every reboot |

---

## Configuring

Frankendancer uses the exact same TOML configuration format as Firedancer. See [Firedancer Configuring](./Firedancer.md#configuring) for the complete reference.

### Key Configuration for Frankendancer

The most important Frankendancer-specific configuration is the `agave_affinity` -- the CPU cores allocated to the Agave subprocess:

```toml
[layout]
    affinity = "1-18"           # Firedancer tiles
    agave_affinity = "19-31"    # Agave subprocess threads
```

These should **not overlap**. Firedancer tiles expect exclusive core access; sharing cores with Agave threads causes context switching and performance degradation.

### Minimal Testnet Configuration

```toml
user = "firedancer"

[gossip]
    entrypoints = [
        "entrypoint.testnet.solana.com:8001",
        "entrypoint2.testnet.solana.com:8001",
        "entrypoint3.testnet.solana.com:8001",
    ]

[consensus]
    identity_path = "/home/firedancer/validator-keypair.json"
    vote_account_path = "/home/firedancer/vote-keypair.json"
    known_validators = [
        "5D1fNXzvv5NjV1ysLjirC4WY92RNsVH18vjmcszZd8on",
        "dDzy5SR3AXdYWVqbDEkVFdvSPCtS9ihF5kJkHCtXoFs",
        "Ft5fbkqNa76vnsjYNwjDZUXoTWpP7VYm3mtsaQckQADN",
        "eoKpUABi59aT4rR9HGS3LcMecfut9x7zJyodWWP43YQ",
        "9QxCLckBiJc783jnMvXZubK4wH86Eqqvashtrwvcsgkv",
    ]

[rpc]
    port = 8899
    full_api = true
    private = true

[reporting]
    solana_metrics_config = "host=https://metrics.solana.com:8086,db=tds,u=testnet_write,p=c4fa841aa918bf8274e3e2a44d77568d9861b3ea"
```

### Mainnet Configuration Example

```toml
user = "firedancer"

[layout]
    affinity = "1-22"
    agave_affinity = "23-31"
    verify_tile_count = 8
    bank_tile_count = 4

[gossip]
    entrypoints = [
        "entrypoint.mainnet-beta.solana.com:8001",
        "entrypoint2.mainnet-beta.solana.com:8001",
        "entrypoint3.mainnet-beta.solana.com:8001",
        "entrypoint4.mainnet-beta.solana.com:8001",
        "entrypoint5.mainnet-beta.solana.com:8001",
    ]

[consensus]
    identity_path = "/home/firedancer/validator-keypair.json"
    vote_account_path = "/home/firedancer/vote-keypair.json"
    known_validators = [
        "7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2",
        "GdnSyH3YtwcxFvQrVVJMm1JhTS4QVX7MFsX56uJLUfiZ",
        "DE1bawNcRJB9rVm3buyMVfr8mBEoyyu73NBovf2oXJsJ",
        "CakcnaRDHka2gXyfbEd2d3xsvkJkqsLw2akB3zsN1D2S",
    ]

[ledger]
    path = "/mnt/ledger"
    accounts_path = "/mnt/accounts"

[snapshots]
    path = "/mnt/snapshots"

[rpc]
    port = 8899
    full_api = true
    private = true

[tiles.gui]
    enabled = true
```

### Bundle Support (Jito MEV)

Frankendancer has native Jito bundle support via the `[tiles.bundle]` configuration:

```toml
[tiles.bundle]
    enabled = true
    url = "https://ny.mainnet.block-engine.jito.wtf"
    tip_distribution_program_addr = "4R3gSG8BpU4t19KYj8CfnBtxhxJBjKHHaBnQ4SYnHNDn"
    tip_payment_program_addr = "T1pyyaTNZsKv2WcRAB8oVnk93mLJw2XzjtVYqCsaHqt"
    tip_distribution_authority = "GZctHpWXmsZC1YHACTGGcHhYxjdRqQvTpYkb9LMvxDIb"
    commission_bps = 800
```

This is a native tile rather than CLI flags (unlike Jito-Solana's approach), providing better integration with the Firedancer pipeline.

### Running

```bash
# Initialize system (run after every boot)
sudo fdctl configure init all --config ~/config.toml

# Start the validator
sudo fdctl run --config ~/config.toml
```

---

## Tuning

### Core Allocation Strategy

The most important tuning decision in Frankendancer is how to split CPU cores between Firedancer tiles and the Agave subprocess.

**General guidelines for a 32-core machine:**

```toml
[layout]
    # Firedancer tiles: ~20 cores
    affinity = "1-20"
    # Agave subprocess: ~11 cores
    agave_affinity = "21-31"
    # Leave core 0 for the OS
```

**Tile budget:**

| Tiles | Count | Cores Used |
|-------|-------|-----------|
| net | 1 | 1 |
| quic | 1 | 1 |
| verify | 6 | 6 |
| dedup | 1 | 1 |
| resolv | 1 | 1 |
| pack | 1 | 1 |
| bank | 4 | 4 |
| poh | 1 | 1 |
| shred | 1 | 1 |
| store | 1 | 1 |
| sign | 1 | 1 |
| metric | 1 | 1 |
| gui + plugin + diag | 3 | 3 (can float) |
| **Total Firedancer** | | **~24** |
| **Agave subprocess** | | **remaining cores** |

### Agave Subprocess Tuning

The Agave subprocess has its own internal threading, including the unified scheduler:

```toml
[layout]
    # Threads for the replay stage unified scheduler
    # Default: agave_cores - 4 (if agave_cores >= 8)
    agave_unified_scheduler_handler_threads = 0  # 0 = auto
```

More threads can help during catchup. If the validator keeps falling behind, increase `agave_affinity` core count and/or this thread count.

### Verify Tile Count

Signature verification is typically the bottleneck. Each verify tile handles 20-40K TPS. Scale up until verify tiles are no longer at 100% utilization (check with `fdctl monitor`):

```toml
[layout]
    verify_tile_count = 8  # Increase from default 6 if needed
```

### Schedule Strategy

```toml
[tiles.pack]
    schedule_strategy = "balanced"  # Default, recommended for most
```

See [Firedancer Tuning](./Firedancer.md#tuning) for the full comparison of `perf`, `balanced`, and `revenue` strategies.

### Shred Tile Count

One shred tile is sufficient for mainnet. Testnet may require 2 due to different cluster dynamics:

```toml
[layout]
    shred_tile_count = 1  # 2 for testnet if needed
```

### Disk Tuning

Same as Agave -- separate NVMe drives for accounts and ledger, `noatime` mount option, ext4 or xfs.

### Network Tuning

For optimal XDP performance:

1. Use a supported NIC (Intel ixgbe, i40e, or ice)
2. Consider `xdp_mode = "drv"` for driver-mode XDP (faster than generic `"skb"`)
3. Enable `xdp_zero_copy = true` if supported by your NIC driver
4. Use `rss_queue_mode = "dedicated"` if your NIC supports it

```toml
[net]
    provider = "xdp"
    [net.xdp]
        xdp_mode = "drv"           # Faster, requires driver support
        xdp_zero_copy = true       # DMA directly into tile memory
        rss_queue_mode = "dedicated"
```

### Benchmarking

Use the built-in benchmarking tool to test throughput:

```bash
./build/native/gcc/bin/fddev bench --config ~/bench-config.toml
```

Monitor tile saturation during benchmarking with `fdctl monitor` to find bottlenecks.

---

## Monitoring

### fdctl monitor

Real-time tile performance dashboard:

```bash
fdctl monitor --config ~/config.toml
```

This shows per-tile CPU utilization, back-pressure, and inter-tile link statistics. See [Firedancer Monitoring](./Firedancer.md#monitoring) for output format details.

### Prometheus Metrics

```bash
curl http://localhost:7999/metrics
```

Configure the endpoint:

```toml
[tiles.metric]
    prometheus_listen_address = "127.0.0.1"
    prometheus_listen_port = 7999
```

### GUI

```toml
[tiles.gui]
    enabled = true
    gui_listen_address = "127.0.0.1"
    gui_listen_port = 80
```

Access at `http://localhost:80`.

### Agave CLI Tools

All standard Solana CLI commands work because the Agave subprocess is running:

```bash
# Gossip
solana gossip | grep <IDENTITY>

# Catchup
solana catchup <IDENTITY>
# or
solana catchup --our-localhost

# Validators list
solana validators | grep <IDENTITY>

# Block production
solana block-production | grep <IDENTITY>

# Admin monitor (via Agave admin socket)
agave-validator --ledger /mnt/ledger monitor
```

### Solana Metrics Dashboard

```toml
[reporting]
    solana_metrics_config = "host=https://metrics.solana.com:8086,db=mainnet-beta,u=mainnet-beta_write,p=password"
```

### Log Files

Frankendancer maintains two logs:

1. **Permanent log** -- written to a file, detailed (default: `/tmp/` with unique name)
2. **Ephemeral log** -- written to stderr, abbreviated for quick inspection

Configure in TOML:

```toml
[log]
    path = "/home/firedancer/fdctl.log"
    level_logfile = "INFO"
    level_stderr = "NOTICE"
    level_flush = "WARNING"
```

Log rotation: Firedancer does not support SIGUSR1/SIGUSR2 rotation. Use `logrotate` with `copytruncate`.

### Health Checks

The Agave subprocess exposes the standard RPC health endpoint:

```bash
# Basic health (200 = healthy, 503 = behind)
curl http://localhost:8899/health

# Current slot
curl -s http://localhost:8899 -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getSlot"}'
```

---

## Pillar Integration

Pillar provisions and manages the Frankendancer validator through the following configuration:

### Service Details

| Property | Value |
|----------|-------|
| **Service name** | `frankendancer` |
| **Binary path** | `/usr/local/bin/fdctl` |
| **Config method** | TOML file (`/etc/pillar/validator.toml`) |
| **Systemd unit** | `/etc/systemd/system/frankendancer.service` |
| **Runs as** | `sol` user |

### How Pillar Provisions Frankendancer

The provisioning flow is identical to Firedancer:

1. User fills out the "Setup Validator" form in the controller UI (client = Frankendancer)
2. Controller sends `ProvisionCommand` to the agent via gRPC `CommandStream`
3. Agent downloads the `fdctl` binary from the provided `download_url`, verifies SHA256
4. Agent installs the binary to `/usr/local/bin/fdctl`
5. Agent generates a minimal TOML config at `/etc/pillar/validator.toml` (same content as Firedancer)
6. Agent writes the systemd unit with `ExecStart=/usr/local/bin/fdctl run --config /etc/pillar/validator.toml`
7. Agent runs `systemctl daemon-reload` then `systemctl enable --now frankendancer`

### Differences from Firedancer in Pillar

| Property | Firedancer | Frankendancer |
|----------|------------|---------------|
| Service name | `firedancer` | `frankendancer` |
| Binary path | `/usr/local/bin/fdctl` | `/usr/local/bin/fdctl` (same) |
| Config file | `/etc/pillar/validator.toml` | `/etc/pillar/validator.toml` (same) |
| Generated TOML | Minimal (layout, consensus, ledger, gossip) | Identical |
| ExecStart | `fdctl run --config ...` | `fdctl run --config ...` (same) |

The only difference in Pillar is the systemd service name (`frankendancer` vs `firedancer`). The binary, config format, and TOML content are identical.

### Generated TOML

```toml
[layout]
affinity = "auto"

[consensus]
identity_path = "/home/sol/validator-keypair.json"
vote_account_path = "/home/sol/vote-account-keypair.json"
expected_genesis_hash = "auto"

[ledger]
path = "/mnt/ledger"
accounts_path = "/mnt/accounts"
limit_size = true

[gossip]
entrypoints = ["entrypoint.testnet.solana.com:8001"]
```

For advanced settings (tile counts, agave_affinity, RPC, snapshots, bundle config), operators should manually edit `/etc/pillar/validator.toml`.

### Upgrades

Binary-only upgrade: stop `frankendancer`, replace `/usr/local/bin/fdctl`, restart. Note that the upgrade command maps `fdctl` binary name to the `firedancer` service -- operators upgrading Frankendancer should verify service name handling.

### Initialization Note

Pillar does not currently run `fdctl configure init all` during provisioning. The operator must ensure hugetlbfs, sysctl, and NIC configuration are performed before starting Frankendancer. This is a manual step or should be included in the node install script.

---

## Migrating from Agave

### Steps

1. **Stop the Agave validator:**
   ```bash
   sudo systemctl stop solana-validator
   ```

2. **Install Frankendancer:**
   ```bash
   # Build or download fdctl
   sudo install -m 755 fdctl /usr/local/bin/fdctl
   ```

3. **Create TOML config** from your existing Agave CLI flags. Map flags to TOML sections:

   | Agave Flag | TOML Equivalent |
   |-----------|----------------|
   | `--identity <path>` | `[consensus] identity_path = "<path>"` |
   | `--vote-account <path>` | `[consensus] vote_account_path = "<path>"` |
   | `--ledger <path>` | `[ledger] path = "<path>"` |
   | `--accounts <path>` | `[ledger] accounts_path = "<path>"` |
   | `--rpc-port <port>` | `[rpc] port = <port>` |
   | `--entrypoint <host:port>` | `[gossip] entrypoints = ["<host:port>"]` |
   | `--known-validator <pubkey>` | `[consensus] known_validators = ["<pubkey>"]` |
   | `--limit-ledger-size` | `[ledger] limit_size = 200_000_000` |
   | `--full-rpc-api` | `[rpc] full_api = true` |
   | `--private-rpc` | `[rpc] private = true` |
   | `--dynamic-port-range <range>` | `dynamic_port_range = "<range>"` |

4. **Initialize the system:**
   ```bash
   sudo fdctl configure init all --config ~/config.toml
   ```

5. **Start Frankendancer:**
   ```bash
   sudo fdctl run --config ~/config.toml
   ```

The ledger directory is compatible between Agave and Frankendancer. No data migration is needed.

---

## Sources

- [Firedancer Getting Started (Frankendancer)](https://docs.firedancer.io/guide/getting-started.html)
- [Firedancer Configuring](https://docs.firedancer.io/guide/configuring.html)
- [Firedancer Initializing](https://docs.firedancer.io/guide/initializing.html)
- [Firedancer Performance Tuning](https://docs.firedancer.io/guide/tuning.html)
- [Firedancer Monitoring](https://docs.firedancer.io/guide/monitoring.html)
- [Firedancer FAQ](https://docs.firedancer.io/guide/faq.html)
- [Firedancer GitHub Repository](https://github.com/firedancer-io/firedancer)
- [Firedancer default.toml Reference](https://docs.firedancer.io/guide/configuring.html#options)
