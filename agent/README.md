# pillar-agent

The node-resident agent that manages the Solana validator lifecycle and handles all external communication. Single binary per node — runs health checks, manages restarts and snapshot recovery, serves HTTP endpoints for load balancers, pushes status to the controller via gRPC, receives commands, collects system metrics, and streams logs.

## How It Works

The agent runs five concurrent tasks in a single runtime:

1. **Reconciler** — health checks the local validator every 20s via JSON-RPC, maintains the state machine, handles crash loop detection, snapshot recovery, and processes commands from the controller
2. **Metrics updater** — refreshes sysinfo every 5s, enriches `NodeStatus` with CPU/mem/disk/net/process metrics
3. **gRPC client** — registers with the controller, pushes enriched `NodeStatus` every 10s, receives commands via server-streaming `CommandStream`
4. **HTTP server** — serves `/health`, `/status`, `/version`, `/metrics` for load balancers and debugging
5. **Log collector** — tails journald for validator and agent systemd units, streams log batches to the controller

Commands from the controller flow directly into the reconcile loop via an in-process `mpsc` channel — no file IPC, no serialization overhead.

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

Transitions are debounced: the node must report `Off` for `consecutive_off_threshold` consecutive checks (default 3) before the agent considers it truly down.

## Modules

```
agent/src/
├── main.rs              # bootstrap: load config → create tasks → run
├── reconcile.rs         # Reconciler: health checks, state machine, command execution
├── command.rs           # AgentCommand enum (gRPC → reconciler channel type)
├── config.rs            # AgentConfig with all nested config structs + validation
├── error.rs             # PillarError enum (thiserror), PillarResult alias
├── event.rs             # OperatorEvent + EventKind for structured logging
├── role.rs              # NodeRole: Rpc | Validator | Grpc
├── grpc.rs              # ControllerLink: gRPC client (register, report, commands, logs)
├── http.rs              # axum router: GET /health, /status, /version, /metrics
├── metrics.rs           # Prometheus registry: reads all metrics from NodeStatus
├── metrics_updater.rs   # async loop: refreshes sysinfo, enriches NodeStatus
├── agent_health.rs      # atomic counters for agent self-health (controller latency, etc.)
├── system_info.rs       # sysinfo wrapper: CPU, memory, disk, network, per-process stats
├── log_collector.rs     # tails journald, buffers + streams log batches to controller
├── provisioner.rs       # validator install, systemd unit generation, binary upgrade
├── client/
│   └── mod.rs           # ValidatorClient + ClientKind enum (Agave, Jito, Firedancer, etc.)
├── health/
│   ├── mod.rs           # SlotHealthChecker: JSON-RPC health checks, create_health_checker()
│   └── rpc_client.rs    # raw JSON-RPC client (no Solana SDK)
├── lifecycle/
│   └── mod.rs           # SystemdManager: start/stop/restart via sudo systemctl
└── snapshot/
    ├── mod.rs           # helpers: parse_slot_from_filename, scan_snapshot_dir
    ├── download_tcp.rs  # TcpSnapshotManager: full + incremental download
    ├── staleness.rs     # is_stale() pure function
    └── recovery.rs      # SnapshotRecovery: stop → wipe → download → restart
```

## Configuration

Loaded from `PILLAR_AGENT_CONFIG` env var or `agent.yaml` (YAML, via figment):

```yaml
role: rpc                            # rpc | validator | grpc
client: agave                        # agave | jito | firedancer | frankendancer | dummy
http_listen: 0.0.0.0:9090
sysinfo_refresh_interval_secs: 5     # how often to refresh system metrics

controller:
  endpoint: http://10.0.0.1:50051    # gRPC endpoint
  node_id: my-node-1
  report_interval_secs: 10           # how often to push status

network:
  cluster: testnet
  reference_rpc_urls:
    - https://api.testnet.solana.com

lifecycle:
  service_name: solana-validator
  max_startup_wait_secs: 600         # 10 min before timeout
  max_catchup_wait_secs: 1800        # 30 min before timeout
  crash_window_secs: 3600            # sliding window for crash detection
  crash_threshold: 3                 # crashes in window before giving up

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
  consecutive_off_threshold: 3       # consecutive Off checks before acting

paths:
  ledger_path: /mnt/ledger
  snapshot_path: /mnt/snapshots

log_collector:
  enabled: true
  units:
    - solana-validator.service
    - pillar-agent.service
  buffer_size: 100                   # max entries per batch
  flush_interval_ms: 1000            # max time before flushing

# Optional: write state to a binary file for external debugging tools
debug_state_file: ""
```

Config is validated at startup — the agent refuses to start with dangerous misconfigurations (zero timeouts, empty RPC URLs, invalid paths).

## Internal Data Flow

```
Reconciler                SharedStatus                 gRPC Client
    │                    (Arc<RwLock>)                      │
    │  write NodeStatus ──────►                            │
    │                         │◄── read ──── ReportStatus ─┤
    │                         │                             │
    │◄── AgentCommand ── mpsc ──── CommandStream ──────────┤
    │                         │                             │
    │                         │◄── read ──── HTTP Server    │
    │                         │     /health /status /metrics│
    │                                                       │
Metrics Updater                                             │
    │  refresh sysinfo                                      │
    │  enrich NodeStatus ──►                                │
```

Commands flow from controller via gRPC `CommandStream` → `mpsc` channel → reconcile loop. Status flows from reconcile loop → `Arc<RwLock<NodeStatus>>` → gRPC `ReportStatus` + HTTP endpoints.

## Health Checking

Health checks use raw JSON-RPC via reqwest — no Solana SDK dependency.

| Node Role   | RPC Methods Used                          | Healthy When                                 |
|-------------|-------------------------------------------|----------------------------------------------|
| `rpc`       | `getSlot` (local + reference)             | slots_behind ≤ threshold                     |
| `grpc`      | `getSlot` (local + reference)             | slots_behind ≤ threshold                     |
| `validator` | `getSlot` + `getVoteAccounts` (reference) | slots_behind ≤ threshold AND actively voting |

## Commands

Commands arrive from the controller via gRPC and are dispatched to the reconcile loop:

| Command     | Action                                                                 |
|-------------|------------------------------------------------------------------------|
| `Restart`   | Restart the validator via systemd                                      |
| `Recover`   | Force snapshot recovery (stop → wipe → download → restart)             |
| `Stop`      | Stop the validator (no automatic restart)                              |
| `Provision` | Download validator binary, generate systemd unit, start service        |
| `Upgrade`   | Download new binary, verify SHA256, atomic swap, restart               |

## HTTP Endpoints

| Endpoint       | Response |
|----------------|----------|
| `GET /health`  | 200 if `healthy == true`, 503 otherwise |
| `GET /status`  | Full enriched NodeStatus as JSON |
| `GET /version` | `{"service": "pillar-agent", "version": "..."}` |
| `GET /metrics` | Prometheus text format (all metrics from enriched NodeStatus) |

## Building

```bash
cargo build -p pillar-agent
cargo build -p pillar-agent --release   # with LTO
```

## Running

```bash
PILLAR_AGENT_CONFIG=agent.yaml cargo run -p pillar-agent
```

Logs are structured via `tracing` with an env filter (`RUST_LOG`).
