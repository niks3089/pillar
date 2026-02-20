# Pillar Operational Runbooks

## Dev Environment Quick Reference

| Resource | URL / Path |
|----------|-----------|
| Controller UI | `http://202.8.11.101:8080` |
| Agent HTTP | `http://202.8.11.101:9090` (health, status, metrics) |
| Node ID | `mainnet-validator-1` |
| Cluster | testnet |
| Validator | Agave v3.1.8 |
| SSH | `ssh ubuntu@202.8.11.101` |
| Validator service | `solana-validator.service` (runs as `sol`) |
| Agent service | `pillar-agent.service` (runs as `sol`) |
| Controller service | `pillar-controller.service` (runs as `pillar`) |
| rpc-operator service | `rpc-operator.service` (runs as `sol`, disabled) |

### Current State (2026-02-20)

- **pillar-controller**: running, HTTP `:8080`, gRPC `:50051`
- **pillar-agent**: running, HTTP `:9090`, connected to controller
- **solana-validator**: running, bootstrap loop ("No snapshots available" — searching for peers)
- **rpc-operator**: stopped + disabled (requires snapshot-finder.py to function)
- Bootstrap INFO lines flowing from validator → agent → controller → UI logs tab
- Snapshot download Prometheus metrics at zero (no active download)

## Pillar Agent vs rpc-operator

### What They Are

**pillar-agent** is the Pillar project's node agent (`/Users/nikhil/helius/pillar/agent/`). It runs on each validator node, monitors health via JSON-RPC, collects system metrics, streams journald logs, and pushes everything to a centralized controller via gRPC. The controller provides a web UI, REST API, Prometheus `/metrics`, and can send commands (restart, provision, recover) back to the agent via gRPC.

**rpc-operator** is Helius's production validator lifecycle manager (`/Users/nikhil/helius/monorepo/rust-services/rpc-operator/`). It runs standalone on each node with no central controller. It directly controls the validator via `sudo systemctl`, automatically downloads snapshots when the validator is off, manages Geyser plugins, backs up RocksDB to S3/R2, and sends email alerts. It depends on Helius-internal infrastructure (WDT snapshot transfer, snapshot-finder.py, statsd).

Pillar is designed to eventually replace rpc-operator with a more observable, centrally-managed approach.

### Architecture Comparison

| Aspect | pillar-agent | rpc-operator |
|--------|-------------|-------------|
| **Source code** | `/Users/nikhil/helius/pillar/agent/` | `/Users/nikhil/helius/monorepo/rust-services/rpc-operator/` |
| **Dependencies** | `pillar/shared/` (proto types) | `monorepo/rust-services/snapshot-service/` (path dep) + Solana SDK v2.3.3 |
| **Architecture** | Agent + centralized Controller | Standalone per-node operator |
| **Validator control** | Passive — reports state; controller sends commands via gRPC | Active — directly runs `sudo systemctl stop/start sol.service` |
| **Snapshot handling** | Parses download progress from journald logs → Prometheus metrics | Integrated download via WDT (Helius internal) + snapfinder.py fallback |
| **Recovery** | Manual from controller UI (POST /api/nodes/:id/recover) | Automatic: detect off → stop service → wipe → download → restart |
| **Config** | YAML file (`/etc/pillar/agent.yaml`) | Environment variables only (no config file) |
| **Metrics** | Prometheus (pull via `/metrics`) | Statsd (UDP push to `127.0.0.1:7998`) |
| **Alerts** | None (TODO in controller) | Email via SendGrid on state transitions |
| **Health checks** | JSON-RPC (`getHealth`, `getSlot`) + slot comparison | Admin RPC socket + JSON-RPC + slot comparison |
| **Geyser plugins** | None | Health monitor + reload (Yellowstone gRPC) |
| **RocksDB backup** | None | Scheduled S3/R2 backup (zstd compressed) |
| **Log streaming** | journald → gRPC → controller → SQLite + SSE → UI | None (logs via journald only) |
| **Provisioning** | Script-based via controller (template rendering) | N/A (assumes validator already installed) |
| **UI** | Web UI (fleet overview, node detail, logs, provisioning) | None (health endpoint at `:7999/status`) |
| **Binary** | `/usr/local/bin/pillar-agent` (14 MB) | `/usr/local/bin/rpc_operator` (~200 MB, includes Solana SDK) |
| **Service** | `pillar-agent.service` | `rpc-operator.service` |
| **Service user** | `sol` | `sol` (production uses `solana`) |
| **Solana SDK** | None (raw JSON-RPC via reqwest) | Direct dep on `solana-rpc-client`, `agave-validator`, `solana-core` v2.3.3 |

### State Machine Comparison

**pillar-agent states**: `Off` → `StartingUp` → `Behind` → `Healthy` (+ `Recovering`)
- Determined by JSON-RPC health check every 20s
- `Off` = validator process not running or RPC unreachable
- `StartingUp` = RPC responds but no reference slot yet
- `Behind` = slot gap > threshold (default 50 slots)
- `Healthy` = within threshold + (voting if validator role)
- Crash loop = 3+ restarts in 1 hour window

**rpc-operator states**: `Off` → `CleaningAccountsAndLedger` → `DownloadingSnapshot` → `StartingUp` → `Behind` → `Healthy`
- Determined by admin RPC + JSON-RPC every 10s (configurable)
- `Off` triggers active intervention (stop → wipe → download → restart)
- Timeout-based recovery: restart if startup > 1hr or behind > 4hr
- Checks supermajority status to avoid restarting during network halts

### What rpc-operator Does That Pillar Agent Doesn't (Yet)

1. **Active snapshot recovery** — when validator is off, automatically wipes stale data and downloads fresh snapshots
2. **Integrated snapshot download** — WDT (Warp Data Transfer) from Helius internal pool + snapfinder.py for public snapshots
3. **Geyser plugin management** — monitors Yellowstone gRPC plugin health, reloads if stuck
4. **RocksDB backup** — scheduled backups to S3/R2 with zstd compression
5. **Email alerts** — SendGrid notifications on Down/CaughtUp events
6. **Admin RPC** — uses validator admin socket for startup progress tracking
7. **Snapshot serving** — rate-limited TCP server for distributing snapshots to other nodes (ports 10003/10004)
8. **Timeout-based restart** — if startup takes >1hr or catch-up takes >4hr, forces restart
9. **Maintenance mode** — reads a file to skip operator actions during manual maintenance

## rpc-operator: Paths, Build & Deploy

### Source Code Paths

```
Local (macOS):
  /Users/nikhil/helius/monorepo/rust-services/rpc-operator/     # rpc-operator source
  /Users/nikhil/helius/monorepo/rust-services/snapshot-service/  # required sibling dependency

Dev box (202.8.11.101):
  /tmp/rpc-operator-build/rpc-operator/                          # synced source
  /tmp/rpc-operator-build/snapshot-service/                      # synced dependency
  /tmp/rpc-operator-build/rpc-operator/target/release/rpc_operator  # built binary
  /usr/local/bin/rpc_operator                                    # deployed binary
  /etc/systemd/system/rpc-operator.service                       # systemd unit
```

### Build & Deploy (from macOS to dev box)

```bash
# 1. Sync source to dev box (both rpc-operator and snapshot-service)
rsync -az --exclude target --exclude node_modules --exclude .git \
  /Users/nikhil/helius/monorepo/rust-services/rpc-operator/ \
  ubuntu@202.8.11.101:/tmp/rpc-operator-build/rpc-operator/

rsync -az --exclude target --exclude node_modules --exclude .git \
  /Users/nikhil/helius/monorepo/rust-services/snapshot-service/ \
  ubuntu@202.8.11.101:/tmp/rpc-operator-build/snapshot-service/

# 2. Build on dev box (~2 min first build, heavy Solana SDK deps)
ssh ubuntu@202.8.11.101 "cd /tmp/rpc-operator-build/rpc-operator && \
  export PATH=/home/ubuntu/.cargo/bin:\$PATH && \
  cargo build --release"

# 3. Deploy binary
ssh ubuntu@202.8.11.101 "sudo systemctl stop rpc-operator 2>/dev/null; \
  sudo cp /tmp/rpc-operator-build/rpc-operator/target/release/rpc_operator /usr/local/bin/rpc_operator"

# 4. Start (only if pillar-agent is stopped — never run both!)
ssh ubuntu@202.8.11.101 "sudo systemctl stop pillar-agent && \
  sudo systemctl start rpc-operator"
```

### Build Notes

- **Rust edition 2024**: rpc-operator uses edition 2024, requires Rust 1.88+ (auto-installed via rust-toolchain.toml)
- **Huge dependency tree**: Solana SDK pulls ~500 crates; first build downloads ~200MB of deps
- **snapshot-service path dep**: `Cargo.toml` has `snapshot_service = { path = "../snapshot-service" }` — the sibling crate must be synced alongside
- **Build warnings**: ~32 warnings (deprecated snapshot types, dead code) — these are in the rpc-operator/snapshot-service code, not ours
- **Cross-compile**: Same as pillar — build on the dev box (Linux x86_64), not on macOS

### Systemd Service Configuration

Service file: `/etc/systemd/system/rpc-operator.service`

```ini
[Unit]
Description=RPC Operator
After=network-online.target

[Service]
Type=simple
User=sol
ExecStart=/usr/local/bin/rpc_operator
Environment=RPC_REFERENCES=https://api.testnet.solana.com
Environment=LEDGER_PATH=/mnt/ledger
Environment=ACCOUNTS_PATH=/mnt/accounts
Environment=SNAPSHOT_PATH=/mnt/snapshots
Environment=NETWORK=testnet
Environment=OPERATOR_SETUP_SOLANA=true
Environment=MAX_SLOTS_BEHIND_HEALTHY=25
Environment=RUN_INTERVAL_SECONDS=10
Environment=SNAPSHOT_SOURCE=external
Environment=WIPE_ACCOUNTS_AND_LEDGER=true
Environment=USE_LOCAL_SNAPSHOTS=false
Environment=BACKUP_ROCKSDB=false
Environment=UPLOAD_SNAPSHOTS=disabled
Environment=GEYSER_PLUGIN_TYPE=NONE
Environment=RUST_LOG=info
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

All rpc-operator config is via env vars (no YAML file). Key variables:

| Variable | Value | Description |
|----------|-------|-------------|
| `RPC_REFERENCES` | `https://api.testnet.solana.com` | Reference RPCs for slot comparison (CSV) |
| `LEDGER_PATH` | `/mnt/ledger` | Validator ledger directory |
| `ACCOUNTS_PATH` | `/mnt/accounts` | Validator accounts directory |
| `SNAPSHOT_PATH` | `/mnt/snapshots` | Validator snapshots directory |
| `NETWORK` | `testnet` | Cluster name |
| `OPERATOR_SETUP_SOLANA` | `true` | Enable auto-restart when validator is off |
| `SNAPSHOT_SOURCE` | `external` | `external` = snapfinder.py, `internal` = Helius WDT pool |
| `RUN_INTERVAL_SECONDS` | `10` | Health check interval |
| `MAX_SLOTS_BEHIND_HEALTHY` | `25` | Max slots behind before "Behind" state |
| `GEYSER_PLUGIN_TYPE` | `NONE` | `YELLOWSTONE_GRPC` or `NONE` |
| `BACKUP_ROCKSDB` | `false` | Disable RocksDB S3 backup |

### Dev Box Quirks

1. **Service name**: rpc-operator hardcodes `sol.service`; dev box uses `solana-validator.service`. Symlink exists:
   ```bash
   /etc/systemd/system/sol.service → /etc/systemd/system/solana-validator.service
   ```

2. **Missing snapshot-finder.py**: rpc-operator panics at `snapshot_service.rs:251` when `SNAPSHOT_SOURCE=external` because `snapshot-finder.py` isn't installed. To fix, either install snapfinder or use `USE_LOCAL_SNAPSHOTS=true` (skips download, assumes snapshot already present).

3. **Directory ownership**: rpc-operator runs `rm -rf` directly (no sudo) on data dirs. The `sol` user must own `/mnt/accounts`, `/mnt/ledger`, `/mnt/snapshots` and be able to recreate them.

4. **Ports**: rpc-operator health check on `:7999`, statsd on `:7998` (UDP), snapshot server on `:10003/:10004` (if enabled).

## Bootstrap / Snapshot Download Loop

### Symptoms

When a validator starts fresh (or after a snapshot wipe), it must download a snapshot from peers before it can participate in the cluster. During this phase:

- Agent reports `state=Off` because the validator process keeps restarting
- Crash loop detection triggers (3+ restarts/hour) — UI shows "crash loop detected"
- In reality, the validator is progressing through bootstrap: discovering peers, downloading snapshots, and attempting to start from the downloaded state

The typical bootstrap cycle looks like:

1. Validator starts, searches for RPC peers with snapshots
2. Begins downloading a snapshot (can be 50-100+ GB on mainnet)
3. If download completes, validator loads the snapshot and starts catching up
4. If download fails (peer disconnects, stale snapshot 404, blacklisted), validator exits and systemd restarts it
5. Repeat until a valid snapshot is fully downloaded and loaded

### "No snapshots available" Loop

The validator finds peers via gossip but none serve usable snapshots. Logs show:
```
Searching for an RPC service with shred version 27350 (Retrying: No snapshots available)...
Total 195 RPC nodes found. 1 known, 0 blacklisted
```

This can happen when:
- Testnet peers aren't serving snapshots at the moment
- The node has connectivity issues to snapshot-serving peers
- Known validators are unreachable

The validator will keep retrying until it finds a snapshot to download. This is **not** a download loop — it's stuck at peer discovery. Unlike a stale-snapshot 404 loop, wiping ledger/snapshots won't help here since the dirs are already empty.

### How Each Tool Handles Bootstrap

**pillar-agent (passive monitoring)**:
- Detects `state=Off`, reports to controller
- Parses journald for download progress → exposes via Prometheus metrics
- Forwards bootstrap/download INFO lines to controller (bypasses `validator_min_level: warn`)
- Does NOT intervene — the validator bootstraps on its own via gossip/peers
- Recovery must be triggered manually from controller UI

**rpc-operator (active intervention)**:
- Detects validator is off → stops `sol.service` immediately
- Checks snapshot freshness: if stale, wipes accounts + ledger
- Downloads snapshot via snapfinder.py or WDT (requires Helius infra)
- Restarts `sol.service` after download
- If startup takes >1hr, forces another restart cycle
- Does NOT rely on validator's native gossip-based bootstrap

### Diagnosis

Check journald for snapshot/download/bootstrap activity:

```bash
# See download progress
sudo journalctl -u solana-validator -f --no-pager | grep -i "download\|snapshot\|bootstrap"

# Check for 404 / blacklist errors
sudo journalctl -u solana-validator --since "1 hour ago" --no-pager | grep -i "404\|blacklist\|stale"

# Check how many times the validator has restarted
systemctl show solana-validator --property=NRestarts

# Check if "No snapshots available" loop
sudo journalctl -u solana-validator --since "5 min ago" --no-pager | grep "No snapshots available"
```

Example healthy download progress lines:

```
Downloading 52428800000 bytes from 10.0.0.5:8899...
downloaded 548684968 bytes 10.4% 13474726.0 bytes/s
downloaded 1097369936 bytes 20.9% 14523891.0 bytes/s
...
Downloaded 52428800000 bytes in 3845s
```

### Pillar Observability

When the agent detects snapshot download progress in journald logs, it exposes:

- **Prometheus metrics**: `pillar_snapshot_download_bytes`, `pillar_snapshot_download_total_bytes`, `pillar_snapshot_download_speed_bps`
- **UI logs**: Bootstrap/download/RPC-search INFO lines pass through the log filter even when `validator_min_level: warn`
- **Grafana**: Node Detail dashboard has a "Snapshot Download" row with progress gauge, speed chart, and bytes downloaded

### Fix: Stale Snapshot / Repeated 404s

If the validator is stuck in a loop where it keeps downloading stale snapshots that fail validation:

```bash
# 1. Stop the validator
sudo systemctl stop solana-validator

# 2. Wipe snapshots and ledger (accounts are rebuilt from snapshot)
sudo rm -rf /mnt/snapshots/*
sudo rm -rf /mnt/ledger/*

# 3. Start the validator — it will re-download from scratch
sudo systemctl start solana-validator
```

### Verification

After restarting:

1. Watch logs for download progress: `sudo journalctl -u solana-validator -f`
2. Check Pillar UI logs tab — download lines should appear
3. Check Prometheus: `curl localhost:9090/metrics | grep snapshot_download`
4. Once download completes, validator loads snapshot and starts catching up — `state` transitions from `Off` → `StartingUp` → `Behind` → `Healthy`

## Testing: Agent vs rpc-operator Validator Bring-Up

### Important: Never run both simultaneously

pillar-agent and rpc-operator both manage the validator. Running both causes conflicting restart/recovery actions. Always stop one before starting the other:

```bash
# Switch to rpc-operator
sudo systemctl stop pillar-agent && sudo systemctl start rpc-operator

# Switch to pillar-agent
sudo systemctl stop rpc-operator && sudo systemctl start pillar-agent
```

### Test 1: Pillar Agent Brings Up Validator

```bash
# 1. Stop rpc-operator if running
sudo systemctl stop rpc-operator 2>/dev/null

# 2. Stop validator and wipe state for clean test
sudo systemctl stop solana-validator
sudo rm -rf /mnt/snapshots/* /mnt/ledger/*

# 3. Start agent + controller (agent detects validator is off, reports to controller)
sudo systemctl start pillar-controller
sudo systemctl start pillar-agent

# 4. Start validator — agent monitors bootstrap via journald
sudo systemctl start solana-validator

# 5. Observe in Pillar UI:
#    - http://202.8.11.101:8080 → node detail → logs tab
#    - Bootstrap/download INFO lines should appear
#    - Prometheus: curl http://202.8.11.101:9090/metrics | grep snapshot_download
#    - State transitions: Off → (downloading) → StartingUp → Behind → Healthy
```

### Test 2: rpc-operator Brings Up Validator

```bash
# 1. Stop pillar-agent to avoid conflicts
sudo systemctl stop pillar-agent

# 2. Stop validator and wipe state
sudo systemctl stop solana-validator
sudo rm -rf /mnt/snapshots/* /mnt/ledger/*

# 3. Recreate dirs (rpc-operator expects them to exist)
sudo mkdir -p /mnt/snapshots/remote /mnt/snapshots/snapshots /mnt/ledger /mnt/accounts/run /mnt/accounts/snapshot
sudo chown -R sol:sol /mnt/snapshots /mnt/ledger /mnt/accounts

# 4. Start rpc-operator (it will detect validator is Off and take action)
sudo systemctl start rpc-operator

# 5. Watch rpc-operator logs:
sudo journalctl -u rpc-operator -f

# NOTE: On dev box, rpc-operator will panic because snapshot-finder.py is not installed.
# This test requires the full Helius snapshot infrastructure to work.
```

### Test Results (2026-02-20)

**Pillar Agent (Test 1) — SUCCESS:**
- Agent detected `state=off`, `restart_count=1`, `crash_looping=false`
- Bootstrap INFO lines (`Searching for an RPC service`, `No snapshots available`) flow to controller UI
- Prometheus metrics: `pillar_snapshot_download_bytes=0` (expected — peer discovery, no active download)
- Controller `/metrics` endpoint exposes all 3 snapshot download gauges per node
- Agent continues passively monitoring; validator handles its own bootstrap

**rpc-operator (Test 2) — PARTIAL (infra missing):**
- Detected validator is off after ~8s (correct)
- Stopped `sol.service` (via symlink to `solana-validator.service`)
- Attempted recovery: `CleaningAccountsAndLedger` → `DownloadingSnapshot`
- **Panicked** at `snapshot_service.rs:251`: `snapshot-finder.py` not available
- Conclusion: rpc-operator requires Helius production snapshot infrastructure (WDT + snapfinder)

**Key Takeaway**: pillar-agent works out of the box on any node with just journald access. rpc-operator requires Helius-specific infrastructure (snapshot-finder.py, WDT) and direct Solana SDK integration.
