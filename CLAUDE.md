# Pillar

Solana node operations platform with 3 components:

- **Operator** (`pillar-operator`) — runs on each node, manages the validator lifecycle (health checks, restarts, snapshot recovery). No HTTP server, no external communication. Writes state to a binary proto file for Link to read.
- **Link** (`pillar-link`) — runs alongside operator on each node, owns all external communication (HTTP endpoints, gRPC to controller). Reads operator state file, enriches with system/process metrics, and exposes via HTTP + Prometheus metrics + gRPC.
- **Controller** (`pillar-controller`) — centralized management plane. Receives metrics from all Link instances via gRPC, stores in SQLite, serves web UI, provides Grafana-compatible scrape endpoint, fires alerts, and pushes binary upgrades to nodes.

Deployment: **operator + link + controller** (no standalone mode). Link always connects to a controller. Link keeps all 4 HTTP endpoints (/health, /status, /metrics, /version) for local debugging and load balancers, but the controller is the primary visibility layer for the fleet.

## End-to-End Onboarding Flow

### Step 1: Install the Controller

Single command pulls a prebuilt binary from the open-source GitHub release:

```bash
curl -sSL https://github.com/niks3089/pillar/releases/latest/download/install-controller.sh | bash
```

The script:
1. Downloads the `pillar-controller` binary for the current OS/arch from GitHub Releases
2. Detects network reachability — is this machine directly reachable from the internet?
3. If behind NAT/firewall (Mac, home server, corporate network): sets up a **Cloudflare Tunnel** (free tier, no account required for quick start) to expose the gRPC (50051) and HTTP (8080) ports via a stable public URL
4. If on a public IP: uses the public IP directly
5. Starts the controller, prints:
   ```
   Pillar Controller running!
     UI:  https://pillar-abc123.cfargotunnel.com  (or http://<public-ip>:8080)
     Add nodes with:
       curl -sSL https://get.pillar.sh | bash -s -- --controller https://pillar-abc123.cfargotunnel.com:50051
   ```

The controller install script supports macOS (launchd) and Linux (systemd). macOS is a first-class target since operators often run the controller on their laptop for small fleets or development.

### Step 2: Open the UI, Onboard Nodes

The controller UI always shows the **"Add Node"** panel with the pre-generated onboard command. This command is always visible — it's the primary way to grow the fleet:

```
UI: Add a Node
  Run this on any Linux machine to join it to your fleet:
  ┌──────────────────────────────────────────────────────────────────┐
  │ curl -sSL https://get.pillar.sh | bash -s -- \                  │
  │   --controller https://pillar-abc123.cfargotunnel.com:50051     │
  └──────────────────────────────────────────────────────────────────┘
  [Copy to clipboard]
```

The node install script (`install-node.sh`):
1. Downloads prebuilt `pillar-operator` + `pillar-link` binaries from GitHub Releases
2. Runs preflight checks (Linux, CPU, RAM, disk, systemd) with cluster-aware thresholds
3. Creates `sol` user, applies sysctl tuning, installs Solana CLI, generates validator keypairs
4. Configures link to connect to the provided controller endpoint
5. Sets up sudoers so `sol` can manage validator systemd services via `sudo systemctl`
6. Starts services, node registers with controller via `RegisterNode` RPC
7. Controller UI shows the node as "registered"

### Step 3: Node Appears in UI with Lifecycle States

Once a node connects, the UI shows it progressing through states:

| UI State | Meaning |
|----------|---------|
| `registered` | Node installed pillar, connected to controller, no validator yet |
| `provisioning` | Validator install in progress (triggered from UI) |
| `starting_up` | Validator process started, waiting for first health check |
| `behind` | Validator running but catching up to the chain tip |
| `healthy` | Fully synced, serving traffic (green) |
| `recovering` | Snapshot recovery in progress (operator triggered) |
| `unhealthy` | Health checks failing, validator may be stuck |
| `offline` | Node stopped reporting (link unreachable for >60s) |

### Step 4: Install Validator from the UI

The UI provides a **"Setup Validator"** panel on every node detail page. The user configures:

- **Client**: Agave, Jito, Firedancer, Frankendancer (select)
- **Cluster**: mainnet-beta, testnet, devnet (select — auto-fills entrypoints + known validators)
- **Version**: text input (e.g. "2.1.6")
- **Paths**: ledger (`/mnt/ledger`), snapshots (`/mnt/snapshots`), accounts (`/mnt/accounts`) — pre-filled defaults
- **Identity keypair path** (default: `/home/sol/validator-keypair.json`)
- **Vote account keypair path**
- **Entrypoints**: textarea, auto-filled per cluster
- **Known validators**: textarea, auto-filled per cluster (empty for devnet — having known validators adds `--only-known-rpc` / `--no-genesis-fetch` which breaks devnet initial sync)
- **RPC Port**: text input (default 8899)
- **Dynamic Port Range**: text input (default 8000-8020)
- **Download URL** + **SHA256**: for the validator binary
- **Addons** (checkboxes):
  - Jito MEV (reveals block engine URL input)
  - Yellowstone gRPC

Clicking "Install Validator" sends a `ProvisionCommand` via `POST /api/nodes/:id/provision`:
1. Controller wraps the request in `ControllerCommand::Provision(ProvisionCommand{...})`
2. Controller pushes command to link via the `CommandStream` gRPC
3. Link receives the command, stages the binary, writes `PendingCommand` for operator
4. Operator picks up the pending command, downloads validator, configures systemd, starts the service
5. UI shows state progression: `provisioning` → `starting_up` → `behind` → `healthy`

### Connectivity: Controller Behind NAT/Firewall

The fundamental problem: Link initiates gRPC connections **outbound** to the controller. If the controller is behind NAT, nodes can't reach it.

**Solution: Cloudflare Tunnel (default for NAT/firewall)**

The `install-controller.sh` script:
1. Checks if the machine has a routable public IP
2. If not (private IP like 10.x, 172.16-31.x, 192.168.x, or Mac): installs `cloudflared`
3. Creates a tunnel exposing ports 50051 (gRPC) and 8080 (HTTP UI) on a stable `*.cfargotunnel.com` URL
4. Stores tunnel credentials in `/etc/pillar/tunnel.json` for persistence across restarts
5. The tunnel URL becomes the `--controller` endpoint used by all nodes

**Alternative connectivity options** (user can override):
- **Public IP**: `--controller-endpoint http://<public-ip>:50051` (no tunnel needed)
- **VPN/Tailscale**: if all machines are on the same mesh, use the Tailscale IP
- **SSH reverse tunnel**: `--tunnel ssh --relay <relay-host>` for air-gapped environments
- **Custom domain**: user configures DNS + TLS termination themselves, passes the URL

The onboarding command embedded in the UI always reflects the correct reachable endpoint, regardless of which connectivity method was chosen during controller install.

## Architecture Decisions

- **Fully independent** — no external path deps
- **No Solana SDK deps** — raw JSON-RPC via reqwest for health checks
- **Prometheus metrics** (open standard, not Datadog/StatsD)
- **figment** for YAML config, **thiserror** for errors
- **Operator has no HTTP server** — Link is responsible for all external-facing endpoints
- **Single `NodeStatus` proto type** — flows from operator → binary state file → link → controller → SQLite → web UI + /metrics + alerts. Operator writes node-health fields, Link enriches with system/process metrics before pushing to controller.
- **State sharing via atomic binary proto file** — operator encodes `NodeStatus` with prost, writes via temp + rename; link polls and decodes
- **Controller always required** — Link must always connect to a controller (no feature gate, no standalone mode)
- **extern_path for shared proto types** — link's and controller's build.rs use `extern_path` so gRPC stubs reference `shared::proto::*` directly, avoiding duplicate message types
- **All services run as `sol` user** — the same Anza-convention user that runs the validator. Operator manages systemd services via `sudo systemctl` with a sudoers rule (`/etc/sudoers.d/sol-systemctl`). No separate `pillar` user.
- Author: Nikhil Acharya

## Data Flow

```
Operator                          Link                           Controller
   |                               |                                |
   |  build NodeStatus             |                                |
   |  (node health fields)         |                                |
   |                               |                                |
   |  encode proto binary -------> read_state() decode              |
   |  write to state file          |                                |
   |                               |  enrich with system metrics    |
   |                               |  (cpu, mem, disk, net, procs)  |
   |                               |                                |
   |                               |  RegisterNode on connect ----> |  store in SQLite
   |                               |  push enriched NodeStatus ---> |  store in status_history
   |                               |  stream logs (PushLogs) -----> |  store in logs table
   |                               |  serve /metrics, /status, etc  |
   |                               |                                |  evaluate alert rules
   |                               |  <--- ControllerCommand ----   |  (restart, recover, upgrade)
   |                               |                                |
   |                               |                                |  serve web UI + /metrics
   |                               |                                |  (Grafana scrape endpoint)
   |                               |                                |  serve logs in UI per node
```

## Crate Structure

```
pillar/
  Cargo.toml              # workspace: operator, shared, link, controller (edition 2021, LTO release)
  config.yaml             # operator config (role, network, paths)
  link-config.yaml        # link config (state_path, http_listen, controller)
  shared/                 # library crate — proto types + state reader/writer shared between operator and link
    build.rs              # prost-build proto compilation with serde derives
    proto/
      pillar.proto        # NodeStatus, gRPC service (ReportStatus, CommandStream, RegisterNode, ReportUpgradeStatus, PushLogs)
    src/
      lib.rs              # exports proto module + read_state/write_state binary helpers
      types.rs            # NodeState, NodeHealth, SlotInfo (internal operator types)
  operator/               # binary crate — the runtime agent on each node
    src/
      main.rs             # bootstrap: config -> services -> run operator loop + signal handling
      operator.rs         # Operator struct, reconciliation loop + state machine, builds NodeStatus proto
      config.rs           # OperatorConfig (nested: NetworkConfig, LifecycleConfig, SnapshotConfig, HealthConfig, PathConfig)
      error.rs            # PillarError enum (thiserror), PillarResult alias
      role.rs             # NodeRole enum { Rpc, Validator, Grpc }
      event.rs            # OperatorEvent struct, EventKind enum (state transitions, restarts, crashes)
      state.rs            # re-exports NodeStatus + write_state from shared
      health/
        mod.rs            # HealthChecker trait + create_health_checker() factory
        types.rs          # re-exports NodeHealth, NodeState, SlotInfo from shared
        rpc_client.rs     # raw JSON-RPC client (getSlot, getHealth, getVoteAccounts)
        rpc_health.rs     # RpcHealthChecker — slot comparison for RPC/gRPC nodes
        validator_health.rs # ValidatorHealthChecker — slot + voting checks for validators
      client/
        mod.rs            # ValidatorClient trait + ClientKind enum + create_client() factory
        agave.rs          # AgaveClient (production-ready, service: solana-validator)
        jito.rs           # JitoClient (stub, service: jito-validator) — TODO: MEV extensions
        firedancer.rs     # FiredancerClient (stub, service: firedancer) — TODO: fdctl management
        frankendancer.rs  # FrankendancerClient (stub) — TODO: fdctl + agave hybrid
        dummy.rs          # DummyClient for testing/mocking
      lifecycle/
        mod.rs            # SystemdManager (start, stop, restart, is_active) — uses `sudo systemctl` for service management
      snapshot/
        mod.rs            # SnapshotManager trait + DownloadMethod enum + factory + helpers
        download_tcp.rs   # TcpSnapshotManager (full + incremental, speed monitoring)
        staleness.rs      # is_stale() pure function
        recovery.rs       # SnapshotRecovery: stop -> wipe ledger -> download -> restart
  link/                   # binary crate — external communication sidecar
    build.rs              # tonic-prost proto compilation with extern_path (client stubs)
    src/
      main.rs             # bootstrap: state reader, metrics updater, gRPC controller link, HTTP server
      config.rs           # LinkConfig (state_path, poll_interval, http_listen, controller — required)
      error.rs            # LinkError enum, LinkResult alias
      grpc.rs             # ControllerLink — pushes enriched NodeStatus, handles CommandStream (restart, recover, upgrade)
      http.rs             # axum router: GET /health, /status, /version, /metrics
      metrics.rs          # Prometheus registry: reads all metrics from enriched NodeStatus
      metrics_updater.rs  # async loop: refreshes sysinfo, enriches NodeStatus with system/process metrics
      state_reader.rs     # polls operator-state.bin into SharedState (Arc<RwLock<Option<NodeStatus>>>)
      system_info.rs      # sysinfo wrapper: CPU, memory, disk, network, per-process stats
      log_collector.rs    # tails journald for validator/operator/link services, streams log batches to controller via PushLogs RPC
  scripts/
    install-node.sh       # idempotent installer for operator + link on a Linux node
```

## Controller Crate

```
controller/
  Cargo.toml              # rusqlite (bundled), tonic, axum, rust-embed, tower-http, tokio-stream
  build.rs                # tonic-prost-build (server=true, client=false, extern_path)
  controller-config.yaml  # default config file
  web/                    # React + Vite SPA (build to dist/, embedded via rust-embed)
    dist/                 # built web assets (embedded at compile time)
    src/                  # React source (main.tsx, App.tsx, api.ts, pages/)
  dashboards/             # Grafana dashboards + Prometheus scrape config
    grafana/              # fleet-overview.json, node-detail.json
    prometheus/           # scrape.yml
  src/
    main.rs               # config → SQLite → gRPC server → HTTP server → retention pruner
    config.rs             # ControllerConfig (listen addrs, db_path, retention_days, external_url)
    error.rs              # ControllerError enum (thiserror)
    db.rs                 # SQLite schema, CRUD, retention pruning (rusqlite + spawn_blocking)
    grpc_server.rs        # PillarController impl (5 RPCs: ReportStatus, CommandStream, RegisterNode, ReportUpgradeStatus, PushLogs)
    node_registry.rs      # in-memory node tracking (status, command channels, log broadcast)
    api.rs                # axum JSON API: /api/overview, /api/nodes, /api/onboard-command, commands (restart, recover, provision), logs, SSE
    web.rs                # axum static file serving via rust-embed (SPA fallback)
    metrics_endpoint.rs   # Prometheus /metrics with per-node labels for Grafana
```

### Controller Web UI Screens

**1. Fleet Overview (landing page)**
- Node count by state (healthy/behind/offline/etc.) as summary cards
- Table of all nodes: node_id, state (color-coded), client, version, slots_behind, uptime, last_seen
- Click any row → node detail page
- Always-visible "Add Node" panel with the onboard command (copy to clipboard)

**2. Node Detail**
- Current state badge + state history timeline
- Live metrics: slots_behind, CPU, memory, disk, network
- Actions: Restart, Recover, Upgrade, Remove
- Config: current operator.yaml + link.yaml (read-only view)
- **Logs tab** — live-streaming log viewer for all services on this node:
  - Filter by service: validator, operator, link (toggle each on/off)
  - Filter by level: info, warn, error, debug
  - Search/grep within logs
  - Auto-scroll (live tail mode) with pause button
  - Logs stream from controller via SSE (`/api/nodes/:id/logs/stream`)
  - Historical logs paginated via `/api/nodes/:id/logs`

**3. Setup Validator (per-node, embedded in Node Detail page)**
- Client select: Agave / Jito / Firedancer / Frankendancer
- Cluster select: mainnet-beta / testnet / devnet (auto-fills entrypoints + known validators; devnet has empty known validators)
- Version text input
- Paths: ledger_path, snapshot_path, accounts_path (pre-filled defaults)
- Identity keypair path, vote account keypair path
- RPC Port (default 8899), Dynamic Port Range (default 8000-8020)
- Entrypoints textarea, known validators textarea
- Download URL + SHA256 text inputs
- Addons: Jito MEV checkbox (reveals block engine URL), Yellowstone gRPC checkbox
- "Install Validator" button → sends `ProvisionCommand` via `POST /api/nodes/:id/provision`

**4. Alerts**
- Active alerts table: node, rule, message, fired_at
- Alert history with resolution timestamps
- Config editor for alert rules

**5. Upgrades**
- Upload binary artifact (drag-and-drop)
- Available artifacts table (name, version, sha256, uploaded_at)
- Bulk upgrade: select nodes → choose artifact → roll out (one-by-one or parallel)
- Upgrade history per node

**6. Grafana (embedded)**
- Embedded Grafana iframe or link to external Grafana instance
- Auto-provisioned dashboards: fleet overview, per-node detail
- Controller serves as Prometheus scrape target for Grafana

### Controller JSON API

```
GET  /api/onboard-command            returns the node onboard command (with correct controller URL)
GET  /api/overview                   fleet summary (counts by state, total nodes)
GET  /api/nodes                      list all nodes with latest status + lifecycle state
GET  /api/nodes/:id                  single node detail + recent history
GET  /api/nodes/:id/history          paginated status history
POST /api/nodes/:id/restart          send RestartCommand
POST /api/nodes/:id/recover          send RecoverCommand
POST /api/nodes/:id/upgrade          trigger binary upgrade
POST /api/nodes/:id/provision        install validator (client, version, cluster, addons)
GET  /api/nodes/:id/logs             paginated logs (?service=validator&level=error&since=...&limit=100)
GET  /api/nodes/:id/logs/stream      SSE stream of live logs (filtered by query params)
DELETE /api/nodes/:id                remove node from fleet
GET  /api/cluster-defaults/:cluster  entrypoints, known_validators, reference_rpc for a cluster
GET  /api/alerts                     list alerts (active + resolved)
POST /api/alerts/:id/resolve         resolve alert
POST /api/artifacts                  upload binary artifact (pillar binaries or validator binaries)
GET  /api/artifacts                  list available artifacts
GET  /api/artifacts/:name/:version   download artifact (used by link during upgrades)
GET  /api/versions/:client           list available versions for a client (fetched from GitHub Releases)
GET  /metrics                        Prometheus scrape (all nodes, labeled)
```

### Controller SQLite Schema

```sql
CREATE TABLE nodes (
    node_id TEXT PRIMARY KEY,
    lifecycle_state TEXT NOT NULL DEFAULT 'registered',  -- registered|provisioning|starting_up|behind|healthy|recovering|unhealthy|offline
    role TEXT,
    client TEXT,
    client_version TEXT,
    cluster TEXT,
    hostname TEXT,
    architecture TEXT,
    os TEXT,
    operator_version TEXT,
    link_version TEXT,
    last_seen_at INTEGER,
    registered_at INTEGER,
    -- provisioning config (set when validator installed via UI)
    provision_config_json TEXT   -- {client, version, cluster, addons: {jito_mev, yellowstone}, paths, identity}
);

CREATE TABLE status_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id TEXT NOT NULL,
    status_json TEXT NOT NULL,
    received_at INTEGER NOT NULL
);
CREATE INDEX idx_status_history_node_time ON status_history(node_id, received_at);

CREATE TABLE alerts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id TEXT NOT NULL,
    rule_name TEXT NOT NULL,
    message TEXT,
    fired_at INTEGER NOT NULL,
    resolved_at INTEGER,
    notified BOOLEAN DEFAULT FALSE
);

CREATE TABLE logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id TEXT NOT NULL,
    service TEXT NOT NULL,           -- 'validator', 'operator', 'link'
    level TEXT NOT NULL,             -- 'info', 'warn', 'error', 'debug'
    message TEXT NOT NULL,
    unit TEXT,                       -- systemd unit name
    timestamp_ms INTEGER NOT NULL    -- original timestamp from node
);
CREATE INDEX idx_logs_node_time ON logs(node_id, timestamp_ms);
CREATE INDEX idx_logs_node_service ON logs(node_id, service, timestamp_ms);

CREATE TABLE artifacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,         -- 'pillar-operator', 'pillar-link', 'agave-validator', etc.
    version TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    size_bytes INTEGER,
    uploaded_at INTEGER NOT NULL,
    UNIQUE(name, version)
);

CREATE TABLE upgrade_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id TEXT NOT NULL,
    binary_name TEXT NOT NULL,
    from_version TEXT,
    to_version TEXT NOT NULL,
    initiated_at INTEGER NOT NULL,
    completed_at INTEGER,
    success BOOLEAN,
    error_message TEXT
);
```

### Alert Rules (config-driven)

```yaml
alerts:
  - name: node_unhealthy
    condition: "healthy == false for 5m"
    action: log
  - name: crash_looping
    condition: "crash_looping == true"
    action: webhook
    webhook_url: "https://hooks.slack.com/..."
  - name: disk_full
    condition: "disk_used_pct > 90"
    action: webhook
    webhook_url: "https://hooks.slack.com/..."
```

Alert deduplication: alerts fire on state transition (healthy→unhealthy), not on every report. Alerts auto-resolve when the condition clears.

### Upgrade Mechanism

1. Upload binary to controller (`POST /api/artifacts`)
2. Trigger upgrade from UI/API (`POST /api/nodes/:id/upgrade`)
3. Controller sends `UpgradeCommand` via `CommandStream` gRPC
4. Link downloads binary from controller (`GET /api/artifacts/:name/:version`)
5. Link verifies SHA256, atomic-swaps binary on disk, restarts service via systemd
6. Link reports success/failure back via `ReportUpgradeStatus` RPC

**Self-upgrade for Link:** swap binary on disk → exit cleanly → systemd `Restart=always` restarts with new binary.

### Validator Provisioning Flow (from UI)

1. User fills out the "Setup Validator" form on the Node Detail page (client, version, cluster, paths, keypairs, entrypoints, known_validators, addons)
2. UI sends `POST /api/nodes/:id/provision` with a `ProvisionRequest` JSON body
3. Controller maps the request to a `ProvisionCommand` proto, wraps in `ControllerCommand::Provision(...)`, sends via `registry.send_command()`
4. Link receives the command on its `CommandStream` gRPC, downloads the validator binary (from `download_url`), verifies SHA256
5. Link writes a `PendingCommand` JSON file (`/var/run/pillar/pending-command.json`) with `command_type: "provision"` and the full `ProvisionCommand`
6. Operator picks up the pending command, installs the binary, writes systemd unit, starts the validator
7. Node state progresses: `registered` → `provisioning` → `starting_up` → `behind` → `healthy`
8. UI shows live progress via SSE log stream + polling

### Log Streaming

Logs from all services on each node (validator, operator, link) are streamed to the controller and viewable in the UI.

**Flow:**
1. Link's `log_collector` tails journald for configured systemd units (`solana-validator.service`, `operator.service`, `link.service`)
2. Log entries are parsed into structured `LogEntry` messages (service, timestamp, level, message, unit)
3. Link buffers entries and pushes batches to controller via the `PushLogs` client-streaming gRPC RPC
4. Controller writes log batches to the `logs` SQLite table
5. Controller prunes old logs based on `retention_days` config (same pruner as status_history)
6. UI shows logs on the Node Detail page with live tail via SSE

**Link log collector design:**
- Reads from journald via `journalctl -f -u <unit> --output=json` subprocess per unit (simple, no extra crate deps)
- Parses JSON output: extracts `MESSAGE`, `PRIORITY`, `__REALTIME_TIMESTAMP`, `_SYSTEMD_UNIT`
- Maps journald priority (0-7) to level string: 0-3 → error, 4 → warn, 5-6 → info, 7 → debug
- Buffers up to 100 entries or 1 second (whichever comes first) before flushing a `LogBatch`
- If controller is unreachable, drops log batches (logs are best-effort, not durably queued — journald is the source of truth on-node)
- Configurable in link config: `log_collector.units` (list of systemd units to tail), `log_collector.buffer_size`, `log_collector.enabled` (default true)

**Controller log serving:**
- `GET /api/nodes/:id/logs` — paginated historical query with filters (service, level, since, until, limit, offset, search text)
- `GET /api/nodes/:id/logs/stream` — SSE endpoint, controller pushes new log entries as they arrive. Filters via query params. Frontend connects on page load, auto-reconnects on disconnect.
- Retention pruning runs on the same schedule as status_history pruning

### Grafana Integration

Controller exposes `/metrics` in Prometheus text format with per-node labels:
```
pillar_node_healthy{node_id="node-1",role="rpc",cluster="mainnet"} 1
pillar_node_slots_behind{node_id="node-1"} 5
```

Grafana scrapes the controller's `/metrics` endpoint. One controller = one scrape target for the entire fleet.

### Future: `dashboards/` Folder

```
dashboards/
  grafana/
    provisioning.json     # Grafana dashboard provisioning
    fleet-overview.json   # fleet summary dashboard
    node-detail.json      # single-node detail dashboard
  prometheus/
    scrape.yml            # Prometheus scrape config for controller
  alerts/
    rules.yml             # Prometheus alerting rules (optional, controller has built-in alerts)
```

## Key Types

- **`proto::NodeStatus`** (shared) — single flat proto type flowing operator → state file → link → controller. Contains node health, slot info, restart/crash state, role/client/cluster/version metadata, system metrics (cpu, mem, disk, net), per-process metrics (validator, operator, link), and upgrade/version tracking fields (operator_version, link_version, pending_upgrade, hostname).
- **`NodeState`** enum — `Off | StartingUp | Behind | Healthy | Recovering` (internal operator enum, serialized as string in proto)
- **`NodeHealth`** — state + slot_info + slots_behind (internal operator health check result)
- **`ClientKind`** enum — `Agave | Jito | Firedancer | Frankendancer | Dummy`
- **`DownloadMethod`** enum — `Tcp` (only active transport, extensible for WDT/HTTP/S3)
- **`NodeRole`** enum — `Rpc | Validator | Grpc`

## Operator Reconciliation Loop

The core loop in `operator.rs` runs on a configurable interval:
1. Health check (treats errors as `Off`; requires `consecutive_off_threshold` consecutive Off checks before transitioning)
2. State transition handling (logs events, updates timestamps)
3. Build `NodeStatus` proto and write to binary state file (atomic write)
4. Timeout checks (max startup wait, max catchup wait)
5. Attempt recovery if `Off` (checks crash threshold, triggers snapshot recovery)

Recovery sequence: stop validator -> wipe ledger -> download snapshot -> restart validator.

## Link Enrichment

The metrics updater in Link enriches the `NodeStatus` read from disk with:
- System metrics: CPU usage, memory (used/total), disk (used/total), network (rx/tx bytes)
- Process metrics: validator (CPU, memory), operator (CPU, memory), link (CPU, memory)

This enriched `NodeStatus` is the single source of truth for HTTP endpoints, Prometheus metrics, and gRPC pushes to controller.

## Link HTTP Endpoints

| Endpoint | Response |
|----------|----------|
| `GET /health` | 200 if `healthy == true`, 503 otherwise |
| `GET /status` | Full enriched NodeStatus as JSON (proto types have serde derives), or 503 if unavailable |
| `GET /version` | `{"service": "link", "version": "..."}` |
| `GET /metrics` | Prometheus text format (all metrics from enriched NodeStatus) |

These endpoints remain available even when the controller is unreachable — Link keeps collecting and serving data locally.

## Prometheus Metrics

Node metrics: `pillar_node_state`, `pillar_node_slots_behind`, `pillar_node_local_slot`, `pillar_node_reference_slot`, `pillar_node_healthy`, `pillar_node_restarts_total`, `pillar_node_crash_looping`, `pillar_health_check_duration_seconds`, `pillar_node_info{role,client,cluster,version}`

System metrics: `pillar_system_cpu_usage_percent`, `pillar_system_memory_*`, `pillar_system_disk_*`, `pillar_system_network_*`

Process metrics (labeled by process: validator/operator/link): `pillar_process_cpu_percent`, `pillar_process_memory_bytes`

Link health: `pillar_link_state_file_age_seconds`, `pillar_link_state_read_errors_total`

## Configuration

Operator config (`PILLAR_CONFIG` env var or `config.yaml`):
- `role`: rpc/validator/grpc
- `client`: agave (default) / jito / firedancer / frankendancer / dummy
- `state_path`: default `/var/run/pillar/operator-state.bin`
- `network`: cluster, reference_rpc_urls
- `lifecycle`: service_name, max_startup_wait_secs (600), max_catchup_wait_secs (1800), crash_window_secs (3600), crash_threshold (3)
- `snapshot`: download_method, server_hostname, staleness_threshold_slots (1000), download_timeout_secs (3600)
- `health`: check_interval_secs (20), slots_behind_threshold (100), rpc_timeout_secs (10), local_rpc_url, consecutive_off_threshold (3)
- `paths`: ledger_path (/mnt/ledger), snapshot_path (/mnt/snapshots)

Link config (`PILLAR_LINK_CONFIG` env var or `link-config.yaml`):
- `state_path`: path to operator state binary proto file
- `poll_interval_secs`: 5
- `http_listen`: 0.0.0.0:9090
- `controller` (required): endpoint, node_id, report_interval_secs (10)

Controller config (planned, `PILLAR_CONTROLLER_CONFIG` env var):
- `grpc_listen`: 0.0.0.0:50051
- `http_listen`: 0.0.0.0:8080
- `db_path`: /var/lib/pillar/controller.db
- `retention_days`: 30 (applies to status_history and logs tables)
- `external_url`: the public URL nodes use to reach this controller (auto-set by install script, used in onboard command)
- `tunnel`: tunnel config if behind NAT (`type`: cloudflare/none, `credentials_path`)
- `github_repo`: niks3089/pillar (for fetching release artifacts)
- `alerts`: list of alert rules (name, condition, action, webhook_url)

Link log collector config (in `link-config.yaml` under `log_collector`):
- `enabled`: true (default) — set false to disable log streaming
- `units`: list of systemd units to tail (default: `["solana-validator.service", "operator.service", "link.service", "controller.service"]`)
- `buffer_size`: 100 (max entries per batch before flush)
- `flush_interval_ms`: 1000 (max time before flushing a partial batch)

## Installation

### Controller Install (run once, on any machine — Mac or Linux)

```bash
curl -sSL https://github.com/niks3089/pillar/releases/latest/download/install-controller.sh | bash
```

Phases:
1. **Detect OS** — macOS (launchd) or Linux (systemd)
2. **Download** — prebuilt `pillar-controller` binary from GitHub Releases for current OS/arch
3. **Network detection** — check if machine has a routable public IP
4. **Tunnel setup** (if behind NAT) — install `cloudflared`, create tunnel, expose gRPC + HTTP
5. **Config** — write controller config with external_url (tunnel URL or public IP)
6. **Start** — launch controller, print UI URL and node onboard command

### Node Install (run on each validator machine — Linux only)

The command is provided by the controller UI. It always includes the controller endpoint:

```bash
curl -sSL https://get.pillar.sh | bash -s -- --controller https://pillar-abc123.cfargotunnel.com:50051
```

Or from local source during development:

```bash
sudo ./scripts/install-node.sh --binaries-dir /path/to/binaries --controller-endpoint http://10.0.0.1:50051
sudo ./scripts/install-node.sh --binaries-dir /path/to/binaries --controller-endpoint http://10.0.0.1:50051 --cluster testnet --node-id my-node-1
sudo ./scripts/install-node.sh --binaries-dir /path/to/binaries --controller-endpoint http://10.0.0.1:50051 --cluster devnet --solana-version stable
```

Phases:
1. **Preflight** — Linux, x86_64/aarch64, systemd, /proc
2. **System assessment** — cluster-aware CPU/RAM/disk thresholds (hard fail: <4 cores or <8GB RAM), AVX2/SHA feature checks, network bandwidth hint, firewall port reminder
3. **Sol user setup** — create `sol` user with `/home/sol`, own data dirs, sudoers for `sudo systemctl`, sysctl tuning (rmem/wmem 128MB, max_map_count 1M), nofile limits (1M)
4. **Solana CLI** — install via `release.anza.xyz` as sol user (skip if already installed), add to PATH
5. **Keypairs** — generate `validator-keypair.json`, `vote-account-keypair.json`, `authorized-withdrawer-keypair.json` in `/home/sol/` (skip existing)
6. **Install** — binaries to /usr/local/bin
7. **Config** — /etc/pillar/operator.yaml and /etc/pillar/link.yaml (cluster-aware reference RPC defaults)
8. **Systemd** — creates service units running as `sol` user, enables on boot, starts services
9. **Register** — link starts, connects to controller, sends RegisterNode RPC; controller UI shows node as "registered"

Post-install: open controller UI → select the new node → "Setup Validator" to install and configure the validator from the UI.

## Design Patterns

- **Traits for extensibility**: `ServiceManager`, `SnapshotManager`, `ValidatorClient`, `HealthChecker` are all traits with concrete impls. Add new impls without changing consumers.
- **Enum + factory pattern**: `create_client()`, `create_health_checker()`, `create_snapshot_manager()` dispatch by enum variant. Adding a new variant = one enum entry + one file + one match arm.
- **Config-driven**: everything configurable via YAML with serde defaults. No scattered env vars.
- **Single proto type everywhere**: `NodeStatus` is the only data type flowing between components. Operator writes it, Link enriches it, controller receives it, Prometheus reads it.
- **Atomic state sharing**: operator writes binary proto via temp-file + rename, link polls and reads. Safe for concurrent access on small files.
- **Snapshots are client-agnostic**: same `snapshot-<slot>-<hash>.tar.zst` format regardless of validator client.
- **Separation of concerns**: operator manages the node, link handles all external communication and metric enrichment, controller provides fleet visibility and management.

## Dev Environment
- Single Ubuntu dev box: `139.84.215.43`
  - Controller: HTTP `:8080`, gRPC `:50051`
  - Operator + Link running on the same box
- Cluster: **testnet only** (box is small)
- Reference RPC: `https://api.testnet.solana.com`

## Current Status

**Production-ready**:
- Operator core loop + state machine (Agave client)
- Health checking (RPC and Validator modes) with consecutive-off debounce
- Sliding-window crash loop detection (`crash_window_secs` + `crash_threshold`)
- Config validation at startup (both operator and link)
- Snapshot download via TCP + recovery orchestration
- Systemd lifecycle management with retry
- Link HTTP server (health, status, version, metrics)
- Prometheus metrics collection (node + system + process via enriched NodeStatus)
- Binary proto state file reader/writer
- System/process metrics enrichment in Link
- gRPC controller push with enriched NodeStatus (always-on, retries with backoff)
- Idempotent node installer script (`scripts/install-node.sh`) — creates sol user, sudoers, sysctl tuning, Solana CLI, keypairs, cluster-aware system checks
- Controller gRPC server — all 5 RPCs (ReportStatus, CommandStream, RegisterNode, ReportUpgradeStatus, PushLogs)
- Controller SQLite — schema, node registry, status_history, logs with retention pruning
- Controller web UI — vanilla JS SPA embedded via rust-embed + React source (fleet overview, node detail with logs, Setup Validator provisioning panel)
- Controller JSON API — /api/overview, /api/nodes, /api/nodes/:id, history, logs, logs/stream (SSE), restart, recover, provision, onboard-command, DELETE
- Controller Prometheus `/metrics` endpoint — per-node labels for Grafana scraping
- Controller node lifecycle state machine — registered → starting_up → behind → healthy → recovering → offline
- Controller config — grpc_listen, http_listen, db_path, retention_days, external_url
- Grafana dashboards — fleet overview + node detail JSON provisioning
- Prometheus scrape config for controller endpoint

**Incomplete / TODO**:

_Controller features (deferred):_
- [ ] Controller alert engine — condition eval, webhook/log actions, dedup on transition
- [ ] Controller artifact storage — upload, serve, SHA256 verification

_Install scripts:_
- [ ] `scripts/install-controller.sh` — single-command installer (download binary from GitHub Releases, detect NAT, setup Cloudflare Tunnel if needed, start controller, print UI URL + onboard command)
- [x] `scripts/install-node.sh` — sol user, sudoers, sysctl, Solana CLI, keypairs, cluster-aware system assessment, `--solana-version` / `--cluster` flags
- [ ] Update `scripts/install-node.sh` — download prebuilt binaries from GitHub Releases instead of `--binaries-dir`; add gRPC connectivity check during install
- [ ] `https://get.pillar.sh` — redirect to latest install-node.sh from GitHub Releases

_Connectivity:_
- [ ] Cloudflare Tunnel integration in controller — auto-detect NAT, install cloudflared, create persistent tunnel, expose gRPC + HTTP
- [ ] Tunnel health monitoring — controller detects if tunnel goes down, logs warning

_Validator provisioning from UI:_
- [x] Provision HTTP API — `POST /api/nodes/:id/provision` sends `ProvisionCommand` via `CommandStream` gRPC
- [x] Provision UI — "Setup Validator" panel on Node Detail page (client, cluster, version, paths, keypairs, entrypoints, known validators, download URL, SHA256, Jito MEV, Yellowstone gRPC)
- [x] Proto `ProvisionCommand` — 17 fields (client, version, cluster, paths, keypairs, entrypoints, known_validators, download_url, sha256, jito_mev, jito_block_engine_url, yellowstone_grpc, rpc_port, dynamic_port_range)
- [x] Controller `build.rs` extern_path for `ProvisionCommand` — avoids duplicate generated struct
- [ ] Version fetcher — query GitHub Releases API for Agave/Jito/Firedancer versions (dropdown instead of text input)

_Log streaming (link side):_
- [x] Link `log_collector.rs` — tail journald for validator/operator/link units, parse JSON output, buffer into LogBatch messages
- [x] Link log_collector config — `log_collector.units`, `log_collector.buffer_size`, `log_collector.enabled`
- [x] Link PushLogs gRPC client — stream log batches to controller, drop on disconnect (best-effort)

_Node agent improvements:_
- [x] gRPC controller commands — restart/recover write PendingCommand for operator; upgrade downloads + writes PendingCommand
- [x] IPC mechanism between Link and Operator for controller commands (PendingCommand tagged enum)
- [x] Upgrade handler in Link — download binary, verify SHA256, atomic swap, restart via systemd
- [x] Upgrade HTTP endpoint — `POST /api/nodes/:id/upgrade` on controller
- [ ] Self-upgrade for Link — swap binary, exit, let systemd restart with new version
- [ ] update_config handler — write config and signal reload

_Validator clients:_
- [ ] Jito client — stub only, needs MEV extensions and tip-distribution config
- [ ] Firedancer client — stub only, needs fdctl process management and TOML config
- [ ] Frankendancer client — stub only, needs hybrid fdctl + agave handling

## Conventions

- `cargo clippy -- -D warnings` must be clean
- `#![allow(dead_code)]` on modules with types not yet wired into main — remove as code matures
- No code copied from rpc-operator — reference for protocol compatibility only, implementations are fresh
- Shared types live in `pillar-shared` crate; operator and link both depend on it
- Proto types generated via prost with serde derives for JSON serialization
- Tests included inline in modules (health determination, metrics, HTTP endpoints, state reader, staleness)
