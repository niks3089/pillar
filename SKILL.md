# Pillar Operational Runbooks

## Dev Environment Quick Reference

| Resource | URL / Path |
|----------|-----------|
| Controller UI | `http://202.8.11.101:8080` |
| Agent HTTP | `http://202.8.11.101:9090` (health, status, metrics) |
| Node ID | `mainnet-validator-1` |
| Cluster | **devnet** |
| Validator | Agave v3.1.8 |
| SSH | `ssh ubuntu@202.8.11.101` |
| Validator service | `solana-validator.service` (runs as `sol`) |
| Agent service | `pillar-agent.service` (runs as `sol`) |
| Controller service | `pillar-controller.service` (runs as `pillar`) |
| rpc-operator service | `rpc-operator.service` (runs as `sol`, disabled) |

### Current State (2026-02-20)

- **Cluster**: switched from testnet → **devnet**
- **pillar-controller**: running, HTTP `:8080`, gRPC `:50051`
- **pillar-agent**: running, HTTP `:9090`, connected to controller, **bootstrap-aware recovery** deployed
- **solana-validator**: running on devnet, downloading snapshot (~55GB, ~17 MB/s)
- **rpc-operator**: stopped + disabled
- Agent correctly detects validator is bootstrapping and skips destructive recovery
- Bootstrap/download INFO lines flowing to controller UI

### Devnet Configuration

**Validator systemd unit** (`/etc/systemd/system/solana-validator.service`):
- Entrypoints: `entrypoint.devnet.solana.com:8001`, `entrypoint2.devnet.solana.com:8001`, `entrypoint3.devnet.solana.com:8001`
- Known validators: `dv1ZAGvdsz5hHLwWXsVnM94hWf1pjbKVau1QVkaMJ92`, `dv2eQHeP4RFrJZ6UeiZWoc3XTtmtZCUKxxCApCDcRNV`, `dv4ACNkpYPcE3aKmYDqZm9G5EB3J4MRoeE7WNDRBVJB`, `dv3qDFk1DTF36Z62bNvrCXe9sKATA6xvVy6A798xxAS`
- Genesis hash: `EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG`
- Shred version: 29062
- Same flags as testnet: `--no-voting --full-rpc-api --limit-ledger-size --private-rpc`

**Agent config** (`/etc/pillar/agent.yaml`):
- `network.cluster: devnet`
- `network.reference_rpc_urls: [https://api.devnet.solana.com]`
- `lifecycle.max_startup_wait_secs: 3600` (1 hour, was 600)
- `lifecycle.max_catchup_wait_secs: 7200` (2 hours, was 1800)

## Bug Fix: Crash Loop During Bootstrap (2026-02-20)

### Root Cause

The agent's reconciler only used JSON-RPC (`getSlot`) to determine validator state. During bootstrap (downloading snapshots, searching for peers, loading ledger), the validator's RPC endpoint doesn't respond. The agent interpreted this as `state=Off` and triggered destructive recovery:

1. Health check calls `getSlot` on `127.0.0.1:8899` → connection refused (validator bootstrapping)
2. After 3 consecutive failures (60s with 20s interval) → transitions to `Off`
3. `attempt_recovery()` fires: stops validator → wipes ledger → tries snapshot download → restarts
4. This kills the in-progress bootstrap/download
5. Validator starts over from scratch
6. After 3 such cycles in 1 hour → "crash loop detected, backing off"

**The agent was creating the crash loop, not the validator.**

### How rpc-operator Avoids This

rpc-operator uses the **admin RPC socket** (`admin_rpc.start_progress()`) which returns the actual validator start progress:
- `DownloadingSnapshot { slot, rpc_addr }` — actively downloading
- `SearchingForRpcService` — looking for peers with snapshots
- `LoadingLedger` — loading accounts from snapshot
- `WaitingForSupermajority` — network halt (timer paused)
- `Running` — ready for JSON-RPC health checks

This lets it distinguish "process not running" from "process running but not RPC-ready yet".

### Fix Applied

**`agent/src/reconcile.rs` — `attempt_recovery()`**: Added `systemctl is-active` check before any recovery action. If the service is running, the validator is bootstrapping — skip recovery and let it run.

```rust
match self.service_manager.is_active().await {
    Ok(true) => {
        tracing::info!(
            state_duration_secs = self.state_entered_at.elapsed().as_secs(),
            "validator service is running — skipping recovery (bootstrap in progress)"
        );
        return;
    }
    Ok(false) => {
        tracing::info!("validator service is not active, proceeding with recovery");
    }
    Err(e) => {
        tracing::warn!(error = %e, "failed to check service status, proceeding with recovery");
    }
}
```

**`agent/src/config.rs` — Default timeouts increased**:
- `max_startup_wait_secs`: 600 → **3600** (1 hour, matches rpc-operator)
- `max_catchup_wait_secs`: 1800 → **7200** (2 hours)

**`agent/src/lifecycle.rs`**: Removed `#[allow(dead_code)]` from `is_active()` — now used by the reconciler.

### Verification

After deploying the fix, agent logs show:
```
validator service is running — skipping recovery (bootstrap in progress) state_duration_secs=20
validator service is running — skipping recovery (bootstrap in progress) state_duration_secs=40
validator service is running — skipping recovery (bootstrap in progress) state_duration_secs=60
...
```

No crash loops, no false restarts. The validator downloads its snapshot undisturbed.

### Timeout Comparison

| Parameter | pillar-agent (before) | pillar-agent (after) | rpc-operator |
|-----------|----------------------|---------------------|-------------|
| Startup timeout | 600s (10 min) | **3600s (1 hr)** | 3600s (1 hr) |
| Catchup timeout | 1800s (30 min) | **7200s (2 hr)** | 14400s (4 hr) |
| Health check interval | 20s | 20s | 10s |
| Crash threshold | 3 restarts/hr | 3 restarts/hr | N/A (no crash loop concept) |
| Bootstrap detection | None (RPC only) | **systemctl is-active** | Admin RPC socket |

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
| **Validator control** | Active — auto-recovery when service is stopped, timeout-based restarts | Active — directly runs `sudo systemctl stop/start sol.service` |
| **Bootstrap detection** | `systemctl is-active` check — skips recovery if service running | Admin RPC socket — sees `DownloadingSnapshot`, `SearchingForRpcService`, etc. |
| **Snapshot handling** | Parses download progress from journald logs → Prometheus metrics | Integrated download via WDT (Helius internal) + snapfinder.py fallback |
| **Recovery** | Automatic: Off + service stopped → wipe → download → restart. Also from controller UI. | Automatic: detect off → stop service → wipe → download → restart |
| **Config** | YAML file (`/etc/pillar/agent.yaml`) | Environment variables only (no config file) |
| **Metrics** | Prometheus (pull via `/metrics`) | Statsd (UDP push to `127.0.0.1:7998`) |
| **Alerts** | None (TODO in controller) | Email via SendGrid on state transitions |
| **Health checks** | JSON-RPC (`getSlot`) + slot comparison + systemctl is-active | Admin RPC socket + JSON-RPC + slot comparison |
| **Geyser plugins** | None | Health monitor + reload (Yellowstone gRPC) |
| **RocksDB backup** | None | Scheduled S3/R2 backup (zstd compressed) |
| **Log streaming** | journald → gRPC → controller → SQLite + SSE → UI | None (logs via journald only) |
| **Provisioning** | Script-based via controller (template rendering) | N/A (assumes validator already installed) |
| **UI** | Web UI (fleet overview, node detail, logs, provisioning) | None (health endpoint at `:7999/status`) |
| **Binary** | `/usr/local/bin/pillar-agent` (14 MB) | `/usr/local/bin/rpc_operator` (~200 MB, includes Solana SDK) |
| **Service** | `pillar-agent.service` | `rpc-operator.service` |
| **Solana SDK** | None (raw JSON-RPC via reqwest) | Direct dep on `solana-rpc-client`, `agave-validator`, `solana-core` v2.3.3 |

### State Machine Comparison

**pillar-agent states**: `Off` → `StartingUp` → `Behind` → `Healthy` (+ `Recovering`)
- Determined by JSON-RPC health check every 20s
- `Off` = RPC unreachable AND systemd service not active (after fix)
- `Off` with service active = bootstrap in progress, skip recovery
- `StartingUp` = RPC responds but no reference slot yet
- `Behind` = slot gap > threshold (default 100 slots)
- `Healthy` = within threshold + (voting if validator role)
- Startup timeout: 1 hour → triggers recovery
- Catchup timeout: 2 hours → triggers recovery
- Crash loop = 3+ restarts in 1 hour window

**rpc-operator states**: `Off` → `CleaningAccountsAndLedger` → `DownloadingSnapshot` → `StartingUp` → `Behind` → `Healthy`
- Determined by admin RPC + JSON-RPC every 10s (configurable)
- `Off` triggers active intervention (stop → wipe → download → restart)
- Startup timeout: 1 hour → forces restart
- Catchup timeout: 4 hours → forces restart
- Checks supermajority status to avoid restarting during network halts

### What rpc-operator Does That Pillar Agent Doesn't (Yet)

1. **Admin RPC socket** — uses validator admin socket for startup progress tracking (DownloadingSnapshot, SearchingForRpcService, LoadingLedger, WaitingForSupermajority)
2. **Integrated snapshot download** — WDT (Warp Data Transfer) from Helius internal pool + snapfinder.py for public snapshots
3. **Geyser plugin management** — monitors Yellowstone gRPC plugin health, reloads if stuck
4. **RocksDB backup** — scheduled backups to S3/R2 with zstd compression
5. **Email alerts** — SendGrid notifications on Down/CaughtUp events
6. **Snapshot serving** — rate-limited TCP server for distributing snapshots to other nodes (ports 10003/10004)
7. **Supermajority check** — pauses restart timer during `WaitingForSupermajority` (network halt)
8. **Maintenance mode** — reads a file to skip operator actions during manual maintenance

## rpc-operator Test Results (Testnet, 2026-02-20)

### Test 3: rpc-operator Monitors Running Validator

Tested switching from pillar-agent to rpc-operator while the validator was actively downloading a testnet snapshot.

**Procedure:**
1. Stopped pillar-agent, started rpc-operator
2. Validator was at 67% download progress (~3.4GB of ~5.1GB)

**Results:**
- rpc-operator correctly detected `StartingUp { progress: DownloadingSnapshot { slot: 389727635, rpc_addr: 69.67.150.133:8899 } }`
- Showed `seconds_until_restart: 3600` countdown (1 hour)
- Did NOT interfere with the active download
- After download completed (5.1GB in 486s), validator tried incremental snapshot → 404
- Validator restarted, entered "No snapshots available" loop
- rpc-operator detected `SearchingForRpcService`, continued countdown
- rpc-operator would have force-restarted after 1 hour timeout, but since it can't download snapshots itself (no snapshot-finder.py), this would just loop

**Key finding:** Full snapshot downloaded successfully but incremental snapshot 404'd:
```
✨ Downloaded snapshot-389727635-...tar.zst (5118371425 bytes) in 486s
HTTP status client error (404 Not Found) for url (...incremental-snapshot-389727635-389731639...tar.zst)
Failed to download a snapshot archive for slot 389731639
Excluding akicJSdNFWszP2Le38t1NtVeywXtvoxdiGciaELwZHz as a future RPC candidate
```

**Conclusion:** rpc-operator's admin RPC socket gives much better visibility into validator state, but its active recovery features (snapshot download) require Helius production infrastructure. On the dev box, rpc-operator is effectively a passive monitor with a restart timer — similar to pillar-agent's new behavior.

### Previous Test Results (2026-02-20, Testnet)

**Pillar Agent (Test 1) — SUCCESS:**
- Agent detected `state=off`, `restart_count=1`, `crash_looping=false`
- Bootstrap INFO lines flow to controller UI
- Prometheus snapshot download metrics exposed
- Agent passively monitors; validator handles its own bootstrap

**rpc-operator (Test 2) — PANICKED:**
- Detected validator off → stopped `sol.service` → `CleaningAccountsAndLedger` → `DownloadingSnapshot`
- **Panicked** at `snapshot_service.rs:251`: `snapshot-finder.py` not available
- Requires Helius production infrastructure (WDT + snapfinder)

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

### Dev Box Quirks

1. **Service name**: rpc-operator hardcodes `sol.service`; dev box uses `solana-validator.service`. Symlink exists:
   ```bash
   /etc/systemd/system/sol.service → /etc/systemd/system/solana-validator.service
   ```

2. **Missing snapshot-finder.py**: rpc-operator panics at `snapshot_service.rs:251` when `SNAPSHOT_SOURCE=external` because `snapshot-finder.py` isn't installed.

3. **Directory ownership**: rpc-operator runs `rm -rf` directly (no sudo) on data dirs. The `sol` user must own `/mnt/accounts`, `/mnt/ledger`, `/mnt/snapshots`.

4. **Ports**: rpc-operator health check on `:7999`, statsd on `:7998` (UDP), snapshot server on `:10003/:10004` (if enabled).

## Bootstrap / Snapshot Download Loop

### Symptoms

When a validator starts fresh (or after a snapshot wipe), it must download a snapshot from peers before it can participate in the cluster. During this phase:

- Agent reports `state=Off` (RPC not responding) but now detects service is running → skips recovery
- Validator progresses through bootstrap: discovering peers, downloading snapshots, loading state
- Devnet snapshots are ~55GB, testnet ~5GB, mainnet 50-100+ GB

### Incremental Snapshot 404 Problem (Testnet)

Observed on testnet: full snapshot downloads successfully but incremental snapshot 404s, causing the validator to blacklist the peer and restart:
```
✨ Downloaded snapshot-389727635-...tar.zst (5118371425 bytes) in 486s
HTTP status client error (404 Not Found) for url (...incremental-snapshot...tar.zst)
Failed to download a snapshot archive for slot 389731639
Excluding akicJSdNFWszP2Le38t1NtVeywXtvoxdiGciaELwZHz as a future RPC candidate
```
After blacklisting, "No snapshots available" loop until more peers appear. This is a testnet peer availability issue, not a pillar bug.

### How Each Tool Handles Bootstrap

**pillar-agent (after fix)**:
- Detects `state=Off` via RPC, but checks `systemctl is-active` before recovery
- If service running → logs "bootstrap in progress", skips recovery
- If service stopped → attempts recovery (wipe + download + restart)
- Startup timeout (1 hour) and catchup timeout (2 hours) as safety nets
- Parses journald for download progress → Prometheus metrics
- Forwards bootstrap/download INFO lines to controller UI

**rpc-operator (active intervention)**:
- Detects validator state via admin RPC socket (DownloadingSnapshot, SearchingForRpcService, etc.)
- If Off + OPERATOR_SETUP_SOLANA=true → stops service → wipes → downloads → restarts
- Startup timeout: 1 hour. Catchup timeout: 4 hours.
- Pauses timer during WaitingForSupermajority (network halt)
- Requires snapshot-finder.py or WDT for downloads (panics without)

### Diagnosis

```bash
# See download progress
sudo journalctl -u solana-validator -f --no-pager | grep -i "download\|snapshot\|bootstrap"

# Check for 404 / blacklist errors
sudo journalctl -u solana-validator --since "1 hour ago" --no-pager | grep -i "404\|blacklist\|stale"

# Check how many times the validator has restarted
systemctl show solana-validator --property=NRestarts

# Check agent behavior (should show "skipping recovery" during bootstrap)
sudo journalctl -u pillar-agent --since "5 min ago" --no-pager | grep -E "skip|recovery|crash|restart"
```

### Fix: Stale Snapshot / Repeated 404s

```bash
# 1. Stop the validator
sudo systemctl stop solana-validator

# 2. Wipe snapshots and ledger
sudo rm -rf /mnt/snapshots/*
sudo rm -rf /mnt/ledger/*

# 3. Start the validator — it will re-download from scratch
sudo systemctl start solana-validator
```

## Devnet Setup (2026-02-20)

### What Was Done

1. Stopped rpc-operator and validator (was on testnet)
2. Wiped all data dirs (`/mnt/snapshots/*`, `/mnt/ledger/*`, `/mnt/accounts/*`)
3. Updated validator systemd unit: testnet entrypoints/genesis → devnet
4. Updated agent config: `cluster: devnet`, `reference_rpc_urls: [https://api.devnet.solana.com]`
5. Updated agent timeouts: startup 10m→1hr, catchup 30m→2hr
6. Built and deployed fixed agent binary
7. Started validator + agent on devnet
8. Validator found devnet peers (shred version 29062) and began downloading snapshot at ~17 MB/s

### Pending TODO

- [ ] **Wait for devnet snapshot download to complete** (~55GB at 17 MB/s ≈ ~55 min)
- [ ] **Verify state transitions**: Off → StartingUp → Behind → Healthy
- [ ] **Verify Prometheus metrics** during download: `pillar_snapshot_download_bytes`, speed, total
- [ ] **Verify controller UI** shows devnet logs and status
- [ ] **Test recovery when service actually stops**: kill validator, verify agent detects service stopped and triggers real recovery
- [ ] **Consider admin RPC socket** — add Solana admin RPC support to pillar-agent for better bootstrap visibility (like rpc-operator), without adding Solana SDK dependency
- [ ] **Update controller cluster-defaults** — add devnet entrypoints/known-validators/reference-rpc to `/api/cluster-defaults/devnet`
- [ ] **Rename node ID** — `mainnet-validator-1` is misleading now that we're on devnet

## Testing: Agent vs rpc-operator Validator Bring-Up

### Important: Never run both simultaneously

pillar-agent and rpc-operator both manage the validator. Running both causes conflicting restart/recovery actions. Always stop one before starting the other:

```bash
# Switch to rpc-operator
sudo systemctl stop pillar-agent && sudo systemctl start rpc-operator

# Switch to pillar-agent
sudo systemctl stop rpc-operator && sudo systemctl start pillar-agent
```
