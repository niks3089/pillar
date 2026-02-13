# pillar-controller

The centralized management plane for the Pillar fleet. Receives status from all agents via gRPC, stores in SQLite, serves a web UI, provides a Prometheus-compatible `/metrics` endpoint for Grafana, and pushes commands (restart, recover, provision, upgrade) to nodes.

## How It Works

The controller runs four concurrent services:

1. **gRPC server** (`:50051`) — implements the `PillarController` service: `RegisterNode`, `ReportStatus`, `CommandStream`, `PushLogs`, `ReportUpgradeStatus`
2. **HTTP server** (`:8080`) — serves the JSON API, the embedded React web UI, and the Prometheus `/metrics` endpoint
3. **Retention pruner** — runs hourly, deletes status history and logs older than `retention_days`
4. **Node registry** — in-memory state (DashMap) tracking connected nodes, their latest status, command channels, and log broadcast channels

## Modules

```
controller/src/
├── main.rs              # bootstrap: config → SQLite → gRPC + HTTP servers → pruner
├── config.rs            # ControllerConfig (listen addrs, db_path, retention_days, external_url)
├── db.rs                # SQLite schema, CRUD, retention pruning (rusqlite + spawn_blocking)
├── grpc_server.rs       # PillarController impl (5 RPCs)
├── node_registry.rs     # in-memory node tracking (status, command channels, log broadcast)
├── api.rs               # axum JSON API: overview, nodes, commands, logs, SSE, provision
├── web.rs               # axum static file serving via rust-embed (SPA fallback)
└── metrics_endpoint.rs  # Prometheus /metrics with per-node labels for Grafana
```

```
controller/web/            # React + Vite SPA (built to dist/, embedded via rust-embed)
├── src/
│   ├── main.tsx
│   ├── App.tsx
│   ├── api.ts
│   └── pages/
│       ├── Overview.tsx   # fleet overview: node table, state counts, add-node panel
│       └── NodeDetail.tsx # per-node: status, metrics, logs, actions, setup validator
└── dist/                  # built assets (embedded at compile time)
```

```
controller/dashboards/     # Grafana provisioning
└── grafana/
    ├── fleet-overview.json
    └── node-detail.json
```

## Configuration

Loaded from `PILLAR_CONTROLLER_CONFIG` env var or `controller-config.yaml` (YAML, via figment):

```yaml
grpc_listen: 0.0.0.0:50051
http_listen: 0.0.0.0:8080
db_path: /var/lib/pillar/controller.db
retention_days: 30
external_url: ""   # public URL for onboard command (auto-set by install script)
```

## JSON API

```
GET    /api/onboard-command          node onboard command (with correct controller URL)
GET    /api/overview                 fleet summary (counts by state, total nodes)
GET    /api/nodes                    list all nodes with latest status + lifecycle state
GET    /api/nodes/:id                single node detail + recent history
GET    /api/nodes/:id/history        paginated status history
POST   /api/nodes/:id/restart        send RestartCommand
POST   /api/nodes/:id/recover        send RecoverCommand
POST   /api/nodes/:id/provision      install validator (client, version, cluster, addons)
POST   /api/nodes/:id/stop           stop validator (no automatic restart)
POST   /api/nodes/:id/cancel         cancel in-progress deployment
POST   /api/nodes/:id/upgrade        trigger binary upgrade
GET    /api/nodes/:id/logs           paginated logs (?service=&level=&since=&limit=)
GET    /api/nodes/:id/logs/stream    SSE stream of live logs
DELETE /api/nodes/:id                remove node from fleet
GET    /api/cluster-defaults/:cluster entrypoints + known_validators for a cluster
GET    /metrics                      Prometheus scrape (all nodes, labeled)
```

## gRPC Service

Defined in `shared/proto/pillar.proto`:

| RPC | Direction | Description |
|-----|-----------|-------------|
| `RegisterNode` | agent → controller | Register on connect, upsert in SQLite |
| `ReportStatus` | agent → controller | Push enriched NodeStatus every 10s |
| `CommandStream` | controller → agent | Server-stream of commands (restart, recover, provision, etc.) |
| `PushLogs` | agent → controller | Client-stream of log batches from journald |
| `ReportUpgradeStatus` | agent → controller | Report upgrade success/failure |

## SQLite Schema

```sql
nodes                -- node_id, lifecycle_state, role, client, cluster, last_seen_at, ...
status_history       -- node_id, status_blob (proto binary), received_at (pruned by retention_days)
logs                 -- node_id, service, level, message, timestamp_ms (pruned by retention_days)
alerts               -- node_id, rule_name, message, fired_at, resolved_at
artifacts            -- name, version, sha256, size_bytes, uploaded_at
upgrade_history      -- node_id, binary_name, from/to version, success, error
```

## Node Lifecycle States

| State | Meaning |
|-------|---------|
| `registered` | Node installed agent, connected, no validator yet |
| `provisioning` | Validator install in progress (triggered from UI) |
| `starting_up` | Validator started, waiting for first health check |
| `behind` | Validator running but catching up to chain tip |
| `healthy` | Fully synced, serving traffic |
| `recovering` | Snapshot recovery in progress |
| `stopped` | Validator explicitly stopped via API |
| `unhealthy` | Health checks failing |
| `offline` | Node stopped reporting (unreachable for >60s) |

## Web UI Screens

1. **Fleet Overview** — node count by state, table of all nodes (click → detail), always-visible "Add Node" panel with onboard command
2. **Node Detail** — state badge, live metrics, actions (restart, recover, stop, cancel), logs tab with live tail via SSE, "Setup Validator" provisioning form

## Prometheus / Grafana

The controller exposes `/metrics` in Prometheus text format with per-node labels:

```
pillar_node_healthy{node_id="node-1",role="rpc",cluster="mainnet"} 1
pillar_node_slots_behind{node_id="node-1"} 5
pillar_system_cpu_usage_percent{node_id="node-1"} 42.5
```

One controller = one scrape target for the entire fleet. Grafana dashboards are provided in `dashboards/grafana/`.

## Building

```bash
cargo build -p pillar-controller
cargo build -p pillar-controller --release   # with LTO
```

### Rebuilding the Web UI

```bash
cd controller/web
npm install
npm run build    # outputs to dist/, embedded at compile time via rust-embed
```

## Running

```bash
PILLAR_CONTROLLER_CONFIG=controller-config.yaml cargo run -p pillar-controller
```

The controller supports macOS (for development / small fleets) and Linux. Logs are structured via `tracing` with an env filter (`RUST_LOG`).
