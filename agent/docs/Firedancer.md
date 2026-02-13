# Firedancer Validator

## Overview

Firedancer is a completely independent, ground-up reimplementation of the Solana validator written in C by [Jump Crypto](https://jumpcrypto.com/) (now maintained by the Firedancer team). Unlike Frankendancer (the hybrid), full Firedancer aims to replace **all** Agave components -- including replay, gossip, and repair -- with high-performance C implementations using kernel-bypass networking (AF_XDP), huge pages, and CPU core pinning.

- **Maintainer:** Firedancer team (Jump Crypto)
- **Repository:** [github.com/firedancer-io/firedancer](https://github.com/firedancer-io/firedancer)
- **Language:** C (core), Rust (Agave components in Frankendancer mode)
- **License:** Apache 2.0
- **Status:** Full Firedancer is **not yet production-ready** for mainnet consensus. It is in heavy development. For production use, see [Frankendancer](./Frankendancer.md).

Full Firedancer is the long-term goal: a validator that can run entirely without the Agave codebase, achieving significantly higher throughput and lower latency.

---

## Hardware Requirements

Firedancer's hardware requirements are currently the same as Frankendancer (which depends on Agave). As Firedancer matures and removes Agave dependencies, requirements may decrease.

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
| **CPU** | 32 cores, 3.0 GHz+ with AVX-512 support |
| **RAM** | 512 GB with ECC |
| **Storage** | Separate NVMe for accounts and ledger |
| **Network** | 1 Gbps+ symmetric |
| **NIC** | Intel X540 (ixgbe), Intel X710 (i40e), or Intel E800 (ice) for optimal XDP support |

### NIC Compatibility

Firedancer uses AF_XDP for kernel-bypass networking. While any ethernet NIC works, these drivers are well-tested:

| Driver | NIC Series | Notes |
|--------|-----------|-------|
| `ixgbe` | Intel X540 | Widely tested |
| `i40e` | Intel X710 series | Widely tested |
| `ice` | Intel E800 series | Widely tested |
| `mlx5` | Mellanox/NVIDIA ConnectX | Tested with driver mode XDP |

---

## Building / Installing

Firedancer does not produce prebuilt binaries. You must build from source.

### Prerequisites

- **GCC** version 8.5+ (only 11, 12, 13 are officially supported/tested)
- **rustup** (for Agave components in Frankendancer mode)
- **clang**, **git**, **make**
- Linux kernel 4.18+

### Clone and Install Dependencies

```bash
git clone --recurse-submodules https://github.com/firedancer-io/firedancer.git
cd firedancer
git checkout v0.811.30108  # Or the latest release tag
```

Install system packages and compile library dependencies:

```bash
./deps.sh
```

This installs system packages via your distro's package manager and compiles library dependencies into `./opt`.

### Build

```bash
make -j fdctl solana
```

This builds:

- `fdctl` -- the single Firedancer binary (start, stop, configure, monitor)
- `solana` -- the Solana CLI for convenience (RPC commands like `solana transfer`)

Building requires approximately 32 GB of available memory.

The compiled binaries are placed in `./build/native/gcc/bin/`.

### Cross-Architecture Builds

Firedancer auto-detects the build machine's CPU features and enables architecture-specific optimizations. Binaries built on one machine may not run on another with different CPU features.

To target a specific architecture:

```bash
MACHINE=linux_gcc_x86_64 make -j fdctl solana
```

Available targets are under the `config/` directory.

### Versioning

Full Firedancer does not have independent release tags yet. The Frankendancer release tags (`v0.xxx.yyyyy`) are used. See [Frankendancer Versioning](./Frankendancer.md#versioning) for details.

### Updating

```bash
git fetch --tags
git checkout v0.811.30108  # New version
git submodule update
make -j fdctl solana
```

---

## Initializing

Firedancer requires explicit system initialization before running. The `fdctl configure` command automates this.

### Full Initialization

```bash
sudo ./build/native/gcc/bin/fdctl configure init all --config ~/config.toml
```

This runs all initialization stages. Each stage can also be run individually.

### Initialization Stages

#### hugetlbfs

Reserves huge (2 MiB) and gigantic (1 GiB) pages from the Linux kernel. Almost all Firedancer memory is allocated from these pages for performance.

```bash
sudo fdctl configure init hugetlbfs --config ~/config.toml
```

Output:

```
NOTICE  hugetlbfs ... configuring
NOTICE  RUN: `mkdir -p /mnt/.fd/.huge`
NOTICE  RUN: `mount -t hugetlbfs none /mnt/.fd/.huge -o pagesize=2097152,min_size=228589568`
NOTICE  RUN: `mkdir -p /mnt/.fd/.gigantic`
NOTICE  RUN: `mount -t hugetlbfs none /mnt/.fd/.gigantic -o pagesize=1073741824,min_size=27917287424`
```

This must be run immediately after boot, before memory becomes fragmented.

Configuration in TOML:

```toml
[hugetlbfs]
    mount_path = "/mnt/.fd"
    max_page_size = "gigantic"          # "huge" for VMs/cloud
    gigantic_page_threshold_mib = 128
```

#### sysctl

Sets required kernel parameters:

```bash
sudo fdctl configure init sysctl --config ~/config.toml
```

| Parameter | Minimum | Required | Purpose |
|-----------|---------|----------|---------|
| `vm.max_map_count` | 1000000 | Yes | Agave accounts DB file mapping |
| `fs.file-max` | 1024000 | Yes | Agave accounts DB file handles |
| `fs.nr_open` | 1024000 | Yes | Agave accounts DB file handles |
| `net.ipv4.conf.lo.rp_filter` | 2 | Yes | Loopback QUIC response routing |
| `net.ipv4.conf.lo.accept_local` | 1 | Yes | Loopback QUIC response routing |
| `net.core.bpf_jit_enable` | 1 | No | BPF JIT for faster XDP |
| `kernel.numa_balancing` | 0 | No | Firedancer manages NUMA itself |

#### hyperthreads

Checks that critical tiles (`pack`, `poh`) don't share CPU cores with hyperthreaded siblings:

```bash
sudo fdctl configure init hyperthreads --config ~/config.toml
```

If hyperthreading is active, warnings are printed:

```
WARNING  pack cpu 5 has hyperthread pair cpu 29 which should be offline.
WARNING  poh cpu 9 has hyperthread pair cpu 33 which should be offline.
```

For optimal performance, offline the hyperthread siblings of `pack` and `poh` tiles.

#### ethtool-channels

Configures NIC receive-side scaling (RSS) queues to match the number of `net` tiles:

```bash
sudo fdctl configure init ethtool-channels --config ~/config.toml
```

Three modes controlled by `[net.xdp.rss_queue_mode]`:

- **simple** (default): Reduces total queue count to match net tile count. Works everywhere but can impact non-Firedancer traffic.
- **dedicated**: Reserves dedicated queues for Firedancer using ntuple rules. Better performance, may not work with all NICs.
- **auto**: Tries dedicated, falls back to simple.

#### ethtool-offloads

Disables NIC features incompatible with XDP:

```bash
sudo fdctl configure init ethtool-offloads --config ~/config.toml
```

Disables `generic-receive-offload` and `tx-gre-segmentation` on the network interface.

#### ethtool-loopback

Disables `tx-udp-segmentation` on the loopback device (required for Agave-to-Firedancer loopback communication):

```bash
sudo fdctl configure init ethtool-loopback --config ~/config.toml
```

#### bonding

Adjusts bonding driver timeouts for XDP compatibility (only on bonded interfaces):

```bash
sudo fdctl configure init bonding --config ~/config.toml
```

### Checking Configuration

To verify all stages are properly configured without making changes:

```bash
fdctl configure check all --config ~/config.toml
```

### Reversing Configuration

```bash
sudo fdctl configure fini hugetlbfs --config ~/config.toml
```

---

## Configuring

Firedancer is configured via a TOML file. Only values you want to override need to be specified; all options have defaults.

### Running

```bash
sudo fdctl run --config ~/config.toml
```

Or set the `FIREDANCER_CONFIG_TOML` environment variable.

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

### Configuration Reference

#### `[layout]` -- Tile Configuration

Controls how many of each tile type to run and which CPU cores they occupy.

```toml
[layout]
    affinity = "auto"           # CPU core assignment (or explicit: "1-18")
    agave_affinity = "auto"     # Cores for Agave subprocess (Frankendancer only)
    net_tile_count = 1          # Network I/O tiles
    quic_tile_count = 1         # QUIC protocol tiles
    verify_tile_count = 6       # Signature verification tiles
    bank_tile_count = 4         # Transaction execution tiles
    shred_tile_count = 1        # Block distribution tiles
    resolh_tile_count = 1       # Address lookup table resolver tiles
    blocklist_cores = "0h"      # Cores to exclude from auto layout
```

The `affinity` string format:
- Single core: `"5"`
- Range: `"1-18"`
- Range with stride (skip hyperthreads): `"0-10/2"`
- Floating (OS-scheduled): `"f5"` (5 floating tiles)
- Mixed: `"f1,0-1,2-4/2,f1"`

#### `[consensus]` -- Identity and Voting

```toml
[consensus]
    identity_path = "/home/firedancer/validator-keypair.json"
    vote_account_path = "/home/firedancer/vote-keypair.json"
    expected_genesis_hash = ""                  # Reject if mismatch
    known_validators = []                       # Trusted snapshot sources
    snapshot_fetch = true                       # Fetch snapshot from cluster
    genesis_fetch = true                        # Fetch genesis from cluster
    poh_speed_test = true                       # Check PoH performance at boot
    wait_for_vote_to_start_leader = true        # Prevent double-signing
    os_network_limits_test = true               # Check network speed at boot
```

#### `[ledger]` -- Storage Paths

```toml
[ledger]
    path = ""                          # Default: ~/. firedancer/fd1/ledger
    accounts_path = ""                 # Default: ledger/accounts
    accounts_hash_cache_path = ""      # Default: ledger/accounts_hash_cache
    limit_size = 200_000_000           # Max shreds in root slots
    account_indexes = []               # Optional: "program-id", "spl-token-owner", "spl-token-mint"
```

#### `[gossip]` -- Network Discovery

```toml
[gossip]
    entrypoints = [
        "entrypoint.testnet.solana.com:8001",
    ]
    port = 8001
    port_check = true           # Verify entrypoints can reach us
```

#### `[rpc]` -- JSON-RPC Server

```toml
[rpc]
    port = 0                    # 0 = disabled; set to 8899 to enable
    full_api = false            # Enable all RPC methods
    private = false             # Don't publish in gossip
    bind_address = ""           # Default: 127.0.0.1 if private, else 0.0.0.0
    transaction_history = false
    extended_tx_metadata_storage = false
    only_known = true           # Only use RPC of known validators
```

#### `[snapshots]` -- Snapshot Configuration

```toml
[snapshots]
    enabled = true
    incremental_snapshots = true
    full_snapshot_interval_slots = 25000
    incremental_snapshot_interval_slots = 100
    maximum_full_snapshots_to_retain = 2
    maximum_incremental_snapshots_to_retain = 4
    minimum_snapshot_download_speed = 10485760  # 10 MB/s
    path = ""                   # Default: ledger path
```

#### `[log]` -- Logging

```toml
[log]
    path = ""                   # Default: /tmp with unique name; "-" for stdout
    colorize = "auto"           # "auto", "true", "false"
    level_logfile = "INFO"
    level_stderr = "NOTICE"
    level_flush = "WARNING"
```

Log levels (lowest to highest): DEBUG, INFO, NOTICE, WARNING, ERR, CRIT, ALERT, EMERG.

#### `[net]` -- Networking

```toml
[net]
    provider = "xdp"            # "xdp" (recommended) or "socket" (fallback)
    interface = ""              # Auto-detected if empty
    bind_address = ""           # IPv4 bind address

    [net.xdp]
        xdp_mode = "skb"       # "skb" (safe), "drv" (faster), "default" (auto)
        xdp_zero_copy = false
        xdp_rx_queue_size = 32768
        xdp_tx_queue_size = 32768
        flush_timeout_micros = 20
        rss_queue_mode = "simple"   # "simple", "dedicated", "auto"
```

#### `[tiles.*]` -- Per-Tile Configuration

```toml
[tiles.quic]
    regular_transaction_listen_port = 9001
    quic_transaction_listen_port = 9007
    max_concurrent_connections = 131072
    idle_timeout_millis = 10000
    retry = true

[tiles.shred]
    shred_listen_port = 8003

[tiles.metric]
    prometheus_listen_address = "127.0.0.1"
    prometheus_listen_port = 7999

[tiles.gui]
    enabled = true
    gui_listen_address = "127.0.0.1"
    gui_listen_port = 80
```

#### `[tiles.bundle]` -- Jito Bundle Integration

```toml
[tiles.bundle]
    enabled = false
    url = ""                                # Block Engine URL
    tls_domain_name = ""
    tip_distribution_program_addr = ""
    tip_payment_program_addr = ""
    tip_distribution_authority = ""
    commission_bps = 0
    keepalive_interval_millis = 5000
    tls_cert_verify = true
```

#### `[hugetlbfs]` -- Huge Pages

```toml
[hugetlbfs]
    mount_path = "/mnt/.fd"
    max_page_size = "gigantic"      # "gigantic" (1 GiB) or "huge" (2 MiB for VMs)
    gigantic_page_threshold_mib = 128
```

### Permissions

Firedancer requires root (or specific capabilities) to start because of AF_XDP kernel-bypass networking. After boot, it drops privileges to the configured `user`.

Required capabilities (if not running as root):

- `CAP_NET_RAW` -- bind raw socket for XDP
- `CAP_SYS_ADMIN` -- BPF operations, user namespace sandboxing
- `CAP_SETUID` / `CAP_SETGID` -- switch to unprivileged user

The `user` specified in the TOML should be minimally privileged and should not have sudo access.

---

## Tuning

### Tile Count Optimization

The primary tuning lever in Firedancer is adjusting tile counts and their CPU core assignments.

#### Performance per Tile (Intel Ice Lake reference)

| Tile | Default | Throughput per Tile | Primary Bottleneck |
|------|---------|--------------------|--------------------|
| `net` | 1 | > 1M TPS | Rarely a bottleneck |
| `quic` | 1 | > 1M TPS | Rarely a bottleneck |
| `verify` | 4-6 | 20-40K TPS | **Primary bottleneck** -- add more tiles |
| `bank` | 4 | 20-40K TPS (diminishing returns) | Limited by Agave runtime locking |
| `shred` | 1 | Cluster-size dependent | 1 is enough for mainnet |

#### Tuning Strategy

1. Start with default tile counts
2. Use `fdctl monitor` to identify saturated tiles (look for `% finish` near 100%)
3. Increase the tile count for saturated tiles
4. Ensure total tile count fits within available CPU cores

#### Schedule Strategy

The `pack` tile supports three scheduling strategies:

```toml
[tiles.pack]
    schedule_strategy = "balanced"  # "perf", "balanced", "revenue"
```

| Strategy | Block Fullness | Revenue | Best For |
|----------|---------------|---------|----------|
| **perf** | 100% consistently | Lower | Network health |
| **balanced** (default) | Near 100% | Good | Most validators |
| **revenue** | Variable (often unfull) | Highest MEV | Revenue-focused (deprecated) |

### Benchmarking

Firedancer includes a built-in benchmarking tool:

```bash
fddev bench --config ~/bench-config.toml
```

Example tuned benchmark config for a 32-core AMD EPYC 7513:

```toml
[ledger]
    path = "/dev/shm/{name}/ledger"

[layout]
    affinity = "14-57,f1"
    agave_affinity = "58-63"
    verify_tile_count = 30
    bank_tile_count = 6
    shred_tile_count = 1

[development.genesis]
    fund_initial_accounts = 32768

[development.bench]
    benchg_tile_count = 12
    benchs_tile_count = 2
    affinity = "f1,0-13"
    larger_max_cost_per_block = true
    larger_shred_limits_per_block = true
```

### Hyperthread Management

For tiles that must run serially (`pack`, `poh`), offline their hyperthread siblings:

```bash
# Find hyperthread pairs
cat /sys/devices/system/cpu/cpu5/topology/thread_siblings_list
# Output: 5,29

# Offline the sibling
echo 0 | sudo tee /sys/devices/system/cpu/cpu29/online
```

---

## Monitoring

### fdctl monitor (Live Dashboard)

Real-time tile performance monitoring:

```bash
fdctl monitor --config ~/config.toml
```

Output:

```
    tile |     pid |      stale | heart |        sig | in backp |  % hkeep |  % backp |   % wait |  % finish
---------+---------+------------+-------+------------+----------+----------+----------+----------+-----------
     net | 1108973 |          - |     - |  run( run) |   -(  -) |   40.118 |    0.000 |   59.882 |     0.000
    quic | 1108975 |          - |     - |  run( run) |   -(  -) |    0.325 |    0.000 |   99.675 |     0.000
  verify | 1108978 |          - |     - |  run( run) |   -(  -) |    0.496 |    0.000 |   99.504 |     0.000
```

Key columns:

| Column | Meaning |
|--------|---------|
| `% finish` | Time spent doing useful work (100% = saturated) |
| `% wait` | Time spent idle, waiting for work |
| `% backp` | Time in back-pressure (downstream is full) |
| `in backp` | Whether currently in back-pressure |
| `backp cnt` | Total back-pressure events |

Also shows inter-tile link statistics:

```
             link |  tot TPS |  ovrnp cnt |  ovrnr cnt
------------------+----------+------------+------------
    quic->verify  |    17.2K |     9(+1)  |     0(+0)
```

- `ovrnp cnt` -- producer overrun (verify too slow, dropping transactions)
- `ovrnr cnt` -- reader overrun

### Prometheus Metrics

Firedancer exposes Prometheus-compatible metrics:

```bash
curl http://localhost:7999/metrics
```

Configured in TOML:

```toml
[tiles.metric]
    prometheus_listen_address = "127.0.0.1"
    prometheus_listen_port = 7999
```

### GUI Dashboard

Firedancer has a built-in web GUI:

```toml
[tiles.gui]
    enabled = true
    gui_listen_address = "127.0.0.1"
    gui_listen_port = 80
```

Access at `http://localhost:80` in a browser.

### Agave CLI Tools

Since Firedancer (in Frankendancer mode) runs Agave internally, standard Solana CLI tools work:

```bash
# Check gossip presence
solana -ut gossip

# Check catchup
solana -ut catchup --our-localhost

# Check voting
solana -ut validators

# Check block production
solana -ut block-production
```

### Process Tree

Firedancer runs each tile in a separate process for security isolation:

```bash
pstree <PID> -as
```

If any tile process dies, all others are brought down with it.

### AF_XDP Caveat

Packets sent and received via AF_XDP do not appear in standard tools like `tcpdump`. Use the `fdctl monitor` tool or Prometheus metrics for network diagnostics.

---

## Pillar Integration

Pillar provisions and manages the Firedancer validator through the following configuration:

### Service Details

| Property | Value |
|----------|-------|
| **Service name** | `firedancer` |
| **Binary path** | `/usr/local/bin/fdctl` |
| **Config method** | TOML file (`/etc/pillar/validator.toml`) |
| **Systemd unit** | `/etc/systemd/system/firedancer.service` |
| **Runs as** | `sol` user |

### How Pillar Provisions Firedancer

1. User fills out the "Setup Validator" form in the controller UI (client = Firedancer)
2. Controller sends `ProvisionCommand` to the agent via gRPC `CommandStream`
3. Agent downloads the `fdctl` binary from the provided `download_url`, verifies SHA256
4. Agent installs the binary to `/usr/local/bin/fdctl`
5. Agent generates a minimal TOML config at `/etc/pillar/validator.toml`
6. Agent writes the systemd unit with `ExecStart=/usr/local/bin/fdctl run --config /etc/pillar/validator.toml`
7. Agent runs `systemctl daemon-reload` then `systemctl enable --now firedancer`

### Generated TOML

Pillar currently generates a minimal TOML with:

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

Additional configuration (RPC port, snapshot path, known validators, tile counts, etc.) is not yet included in the generated TOML. Operators may need to manually edit `/etc/pillar/validator.toml` for advanced settings.

### ExecStart

```
/usr/local/bin/fdctl run --config /etc/pillar/validator.toml
```

No CLI flags beyond the config file path. All configuration is in the TOML.

### Upgrades

Binary-only upgrade: stop `firedancer`, replace `/usr/local/bin/fdctl`, restart. The TOML config is not changed during upgrades.

### Initialization Note

Pillar does not currently run `fdctl configure init all` during provisioning. The operator must ensure hugetlbfs, sysctl, and ethtool configuration is performed before starting Firedancer. This can be done manually or via the install script.

---

## Sources

- [Firedancer Getting Started](https://docs.firedancer.io/guide/getting-started.html)
- [Firedancer Configuring](https://docs.firedancer.io/guide/configuring.html)
- [Firedancer Initializing](https://docs.firedancer.io/guide/initializing.html)
- [Firedancer Performance Tuning](https://docs.firedancer.io/guide/tuning.html)
- [Firedancer Monitoring](https://docs.firedancer.io/guide/monitoring.html)
- [Firedancer Troubleshooting](https://docs.firedancer.io/guide/troubleshooting.html)
- [Firedancer GitHub Repository](https://github.com/firedancer-io/firedancer)
- [Firedancer default.toml Configuration Reference](https://docs.firedancer.io/guide/configuring.html#options)
