# pillar-operator

The node-resident agent that manages the Solana validator lifecycle. Operator runs on each node, performing health checks, managing restarts, handling snapshot recovery, and processing provisioning commands. It has no HTTP server and no external communication — all external-facing functionality is handled by [Link](../link/).

## How It Works

Operator runs a reconciliation loop on a configurable interval (default 20s). Each tick:

1. **Health check** — queries the local validator's JSON-RPC (`getSlot`, `getHealth`, `getVoteAccounts`) and compares against reference RPC endpoints to determine node state
2. **State transition** — updates the internal state machine, logs events, tracks timestamps
3. **Publish state** — encodes `NodeStatus` as a binary protobuf file (`operator-state.bin`) for Link to read
4. **Timeout enforcement** — triggers recovery if startup or catchup exceeds configured limits
5. **Recovery** — if the node is `Off` and crash-looping, stops the validator, wipes the ledger, downloads a fresh snapshot, and restarts

## State Machine

```
                ┌──────────────┐
                │     Off      │ ◄── health check errors / service down
                └──────┬───────┘
                       │ service starts
                       ▼
                ┌──────────────┐
                │  StartingUp  │ ◄── waiting for first successful health check
                └──────┬───────┘
                       │ slot data available, slots_behind > threshold
                       ▼
                ┌──────────────┐
                │    Behind    │ ◄── catching up to chain tip
                └──────┬───────┘
                       │ slots_behind ≤ threshold (+ voting for validators)
                       ▼
                ┌──────────────┐
                │   Healthy    │ ◄── fully synced, serving traffic
                └──────────────┘

                ┌──────────────┐
                │  Recovering  │ ◄── snapshot recovery in progress
                └──────────────┘
```

Transitions are debounced: the node must report `Off` for `consecutive_off_threshold` consecutive checks (default 3) before the operator considers it truly down.

## Modules

```
operator/src/
├── main.rs            # bootstrap: load config → create services → run loop
├── operator.rs        # Operator struct, reconciliation loop, state machine
├── config.rs          # OperatorConfig with nested config structs + validation
├── error.rs           # PillarError enum (thiserror), PillarResult alias
├── event.rs           # OperatorEvent + EventKind for structured logging
├── role.rs            # NodeRole: Rpc | Validator | Grpc
├── state.rs           # re-exports NodeStatus + write_state from shared
├── provisioner.rs     # handles PendingCommand (provision, upgrade, restart, recover, stop)
├── health/
│   ├── mod.rs         # HealthChecker trait + create_health_checker() factory
│   ├── types.rs       # NodeHealth, NodeState, SlotInfo (re-exported from shared)
│   ├── rpc_client.rs  # raw JSON-RPC client (no Solana SDK)
│   ├── rpc_health.rs  # RpcHealthChecker — slot comparison for RPC/gRPC nodes
│   └── validator_health.rs  # ValidatorHealthChecker — slot + voting checks
├── client/
│   └── mod.rs         # ValidatorClient + ClientKind enum (Agave, Jito, Firedancer, etc.)
├── lifecycle/
│   └── mod.rs         # SystemdManager — start/stop/restart via sudo systemctl
└── snapshot/
    ├── mod.rs         # helpers: parse_slot_from_filename, scan_snapshot_dir
    ├── download_tcp.rs  # TcpSnapshotManager — full + incremental download
    ├── staleness.rs   # is_stale() pure function
    └── recovery.rs    # SnapshotRecovery — stop → wipe → download → restart
```

## Configuration

Loaded from `PILLAR_CONFIG` env var or `config.yaml` (YAML, via figment):

```yaml
role: rpc                          # rpc | validator | grpc
client: agave                      # agave | jito | firedancer | frankendancer | dummy
state_path: /var/run/pillar/operator-state.bin

network:
  cluster: testnet
  reference_rpc_urls:
    - https://api.testnet.solana.com

lifecycle:
  service_name: solana-validator
  max_startup_wait_secs: 600       # 10 min before timeout
  max_catchup_wait_secs: 1800      # 30 min before timeout
  crash_window_secs: 3600          # sliding window for crash detection
  crash_threshold: 3               # crashes in window before recovery

snapshot:
  download_method: tcp
  server_hostname: snapshots.example.com
  staleness_threshold_slots: 1000
  download_timeout_secs: 3600

health:
  check_interval_secs: 20
  slots_behind_threshold: 100
  rpc_timeout_secs: 10
  local_rpc_url: http://127.0.0.1:8899
  consecutive_off_threshold: 3     # consecutive Off checks before acting

paths:
  ledger_path: /mnt/ledger
  snapshot_path: /mnt/snapshots
```

Config is validated at startup — the operator refuses to start with dangerous misconfigurations (zero timeouts, empty RPC URLs, invalid paths).

## Health Checking

Health checks use raw JSON-RPC via reqwest — no Solana SDK dependency.

| Node Role   | RPC Methods Used                          | Healthy When                                    |
|-------------|-------------------------------------------|-------------------------------------------------|
| `rpc`       | `getSlot` (local + reference)             | slots_behind ≤ threshold                        |
| `grpc`      | `getSlot` (local + reference)             | slots_behind ≤ threshold                        |
| `validator` | `getSlot` + `getVoteAccounts` (reference) | slots_behind ≤ threshold AND actively voting     |

Any RPC error is treated as `Off`. The operator requires `consecutive_off_threshold` consecutive failures before transitioning to `Off`, preventing transient RPC hiccups from triggering unnecessary restarts.

## Crash Loop Detection

The operator tracks restart timestamps in a sliding window (`crash_window_secs`, default 1 hour). If the number of restarts in the window exceeds `crash_threshold` (default 3), the operator declares a crash loop and triggers snapshot recovery instead of another simple restart.

## Snapshot Recovery

Recovery flow:

1. Stop the validator service via systemd
2. Wipe the ledger directory
3. Download a fresh snapshot via TCP (supports full + incremental)
4. Restart the validator service

Snapshots are client-agnostic — the same `snapshot-<slot>-<hash>.tar.zst` format works regardless of validator client.

## Provisioning

Operator processes `PendingCommand` files written by Link at `/var/run/pillar/pending-command.json`. Supported commands:

| Command     | Action                                                                 |
|-------------|------------------------------------------------------------------------|
| `Provision` | Download validator binary, generate systemd unit, start service        |
| `Upgrade`   | Download new binary, verify SHA256, atomic swap, restart               |
| `Restart`   | Restart the validator via systemd                                      |
| `Recover`   | Force snapshot recovery (stop → wipe → download → restart)             |
| `Stop`      | Stop the validator (no automatic restart)                              |

After provisioning, the operator updates its own `config.yaml` with the new client, cluster, and path settings.

## Validator Clients

| Client          | Service Name         | Binary Path                      | Status         |
|-----------------|----------------------|----------------------------------|----------------|
| Agave           | `solana-validator`   | `/usr/local/bin/agave-validator`  | Production     |
| Jito            | `jito-validator`     | `/usr/local/bin/jito-validator`   | Stub           |
| Firedancer      | `firedancer`         | `/usr/local/bin/fdctl`            | Stub           |
| Frankendancer   | `frankendancer`      | `/usr/local/bin/fdctl`            | Stub           |
| Dummy           | `dummy-validator`    | `/dev/null`                       | Testing        |

## Systemd Integration

All services run as the `sol` user. The operator manages the validator service via `sudo systemctl` with a sudoers rule at `/etc/sudoers.d/sol-systemctl`. Operations include start, stop, restart, and status checks, all with exponential backoff retry (3 attempts, 500ms base delay).

## State File

The operator writes a binary protobuf-encoded `NodeStatus` to disk (default `/var/run/pillar/operator-state.bin`) using atomic temp-file + rename. Link polls this file, enriches it with system/process metrics, and pushes it to the controller. This is the sole communication channel between operator and link — no IPC, no sockets, just a file.

## Building

```bash
cargo build -p pillar-operator
cargo build -p pillar-operator --release   # with LTO
```

## Running

```bash
# With default config path
PILLAR_CONFIG=config.yaml cargo run -p pillar-operator

# Or set the env var
export PILLAR_CONFIG=/etc/pillar/operator.yaml
operator
```

The operator logs structured JSON via `tracing` with an env filter (`RUST_LOG`).
