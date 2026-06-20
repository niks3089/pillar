# Pillar Architecture

Solana node operations platform. Two components, two deployment units, one data type flowing through the entire system.

## Components

- **Agent** (`pillar-agent`) — runs on each node. Single binary that manages the validator lifecycle (health checks, restarts, snapshot recovery) and handles all external communication (HTTP endpoints, gRPC to controller, Prometheus metrics, log streaming to controller). Enriches health state with system/process metrics before reporting.
- **Controller** (`pillar-controller`) — centralized management plane. Receives metrics from all agents via gRPC, stores in SQLite, serves web UI, provides Grafana-compatible scrape endpoint.

## Architecture Quanta

An *architecture quantum* is the smallest independently deployable unit with high functional cohesion that includes all structural elements required for it to function.

Pillar has **two quanta**, each a single binary:

### Quantum 1: Agent

One binary per node. Runs the reconcile loop (health checks, state machine, crash detection, recovery), serves HTTP endpoints for load balancers, pushes status to the controller via gRPC, receives commands via gRPC server-streaming, and streams logs from journald.

Internally, these responsibilities run as independent tokio tasks in a single runtime:

- **Reconciler** — health checks, state machine, provisioning, crash loop detection
- **Metrics updater** — refreshes sysinfo, enriches NodeStatus with CPU/mem/disk/net
- **gRPC client** — registers with controller, pushes status, receives commands
- **HTTP server** — `/health`, `/status`, `/version`, `/metrics`
- **Log collector** — tails journald, streams batches to controller

Commands from the controller flow directly into the reconcile loop via an in-process `mpsc` channel — no file IPC, no serialization.

### Quantum 2: Controller

Independently deployable. Runs on a different machine (often a laptop or management server), has its own persistence (SQLite), its own UI (embedded SPA), and its own config. Can start, stop, and restart without affecting any running agents — nodes just see a temporary gRPC disconnect and retry with backoff.

### The quantum boundary is the gRPC wire

The two quanta are connected only by the gRPC service defined in `pillar.proto`. This is the single contract between deployment units.

## Static Coupling (Compile-Time)

Static coupling is what the compiler enforces — shared types, shared crates, shared proto definitions.

```
                    ┌──────────────┐
                    │ pillar-shared │
                    │  (library)   │
                    └──────┬───────┘
                           │
                    ┌──────┴──────┐
                    │             │
              ┌─────▼──┐   ┌────▼──────┐
              │ agent  │   │controller │
              └────────┘   └───────────┘
```

Both binaries depend on `pillar-shared`. Neither depends on the other.

### What `pillar-shared` provides

| Export | Used by | Purpose |
|--------|---------|---------|
| `proto::NodeStatus` | both | The single data type flowing through the entire system |
| `proto::ControllerCommand` | both | Command envelope (restart, recover, provision, upgrade, stop) |
| `proto::ProvisionCommand` | both | Provisioning spec (client, version, cluster, paths, flags, addons) |
| `proto::LogBatch` / `LogEntry` | both | Structured log streaming |
| `types::NodeState` / `NodeHealth` | agent | Internal health check result types |

### The `extern_path` trick

Both agent and controller compile `pillar.proto` through `tonic-prost-build` for their gRPC stubs, but use `extern_path(".pillar", "pillar_shared::proto")` so the generated code references the shared crate's types instead of generating duplicates. This means `proto::NodeStatus` is a single Rust type everywhere — not two independently generated copies.

### Static coupling strength

| Pair | Coupling | Mechanism |
|------|----------|-----------|
| agent ↔ controller | **Medium** | Shared proto types via `extern_path`, but communicate only through gRPC (wire boundary) |
| Within agent | **High** | Shared `NodeStatus`, direct in-memory channels, same tokio runtime |
| Within controller | **High** | In-memory channels (mpsc, broadcast), shared DashMap, shared Db handle |

## Dynamic Coupling (Runtime)

Dynamic coupling is how components communicate at runtime — the protocols, the synchronicity, the failure domains.

### Internal: Agent tasks (in-process, channels)

```
                        ┌─────────────────┐
                        │   Reconciler    │
                  ┌────►│  (health check, │◄───── cmd_rx (mpsc)
                  │     │  state machine) │
                  │     └────────┬────────┘
                  │              │ writes NodeStatus
                  │              ▼
                  │     ┌─────────────────┐
                  │     │ SharedStatus    │ Arc<RwLock<NodeStatus>>
                  │     └───┬─────┬───────┘
                  │         │     │
                  │         │     ▼
                  │         │  ┌──────────────┐
                  │         │  │Metrics       │ refreshes sysinfo,
                  │         │  │Updater       │ enriches NodeStatus
                  │         │  └──────────────┘
                  │         │
                  │         ▼
                  │     ┌──────────────┐
                  │     │ HTTP Server  │  /health, /status, /metrics
                  │     └──────────────┘
                  │
                  │     ┌──────────────┐
                  └─────│ gRPC Client  │  ReportStatus, CommandStream
                        │              │  cmd_tx ──► reconciler
                        └──────────────┘
```

- **Protocol**: In-memory `Arc<RwLock<NodeStatus>>` for state sharing, `tokio::sync::mpsc` for commands
- **Synchronicity**: All tasks run concurrently. Reconciler writes status, other tasks read it. Commands flow instantly via channel.
- **Failure mode**: If one task panics, other tasks continue. The reconcile loop is the critical path — if it dies, health checks stop.

### External: Agent → Controller (gRPC, bidirectional, persistent)

```
Agent                                   Controller
   │                                        │
   │──── RegisterNode ─────────────────────►│  upsert_node() in SQLite
   │                                        │
   │──── ReportStatus (every 10s) ─────────►│  update NodeRegistry + SQLite
   │                                        │
   │◄─── CommandStream (server-stream) ────│  mpsc channel per node
   │                                        │
   │──── PushLogs (client-stream) ─────────►│  insert into logs table
   │                                        │  broadcast to SSE subscribers
   │──── ReportUpgradeStatus ──────────────►│  log success/failure
```

- **Protocol**: gRPC over HTTP/2 (tonic), defined by `PillarController` service in `pillar.proto`
- **Synchronicity**: Mixed. `ReportStatus` is request-response. `CommandStream` is a long-lived server-stream. `PushLogs` is client-streaming.
- **Failure mode**: Agent retries with exponential backoff (1s → 60s max). Controller tracks nodes as "offline" after 60s of silence. Commands are lost if the stream drops (no persistent queue).
- **Coupling strength**: **Contract coupling** — they agree on the proto service definition but know nothing about each other's internals.

### Command path: Controller → Node

```
User clicks "Restart"
    │
    ▼
HTTP API (POST /api/nodes/:id/restart)
    │
    ▼
NodeRegistry.send_command()
    │  mpsc::Sender<ControllerCommand>
    ▼
CommandStream gRPC (server-streaming)
    │
    ▼
Agent gRPC handler
    │  cmd_tx.send(AgentCommand::Restart)
    ▼
Reconciler cmd_rx.recv()
    │  execute immediately
    ▼
systemctl restart solana-validator
```

Four hops from click to action:

1. HTTP JSON (browser → controller)
2. In-memory mpsc channel → gRPC stream (controller → agent)
3. In-memory mpsc channel (gRPC task → reconciler)
4. Subprocess exec (reconciler → systemd → validator)

## Full Coupling Map

```
    ┌──────────────────── QUANTUM 1: Agent ───────────────────────┐
    │                                                              │
    │  ┌──────────────────────────────────────────────┐           │
    │  │              pillar-agent                     │           │
    │  │                                              │           │
    │  │  ┌────────────┐  Arc<RwLock>  ┌───────────┐ │           │
    │  │  │ Reconciler │ ────────────► │  HTTP     │ │           │
    │  │  │            │               │  /health  │ │           │
    │  │  │            │  Arc<RwLock>  ┌┤  /metrics │ │           │
    │  │  │            │ ────────────►│└───────────┘ │           │
    │  │  │            │              │┌───────────┐ │           │
    │  │  │            │◄── mpsc ─────││  gRPC     │ │           │
    │  │  └──────┬─────┘              ││  Client   ├─┼───────┐   │
    │  │         │ systemctl          │└───────────┘ │       │   │
    │  │         ▼                    │┌───────────┐ │       │   │
    │  │    ┌──────────┐              ││  Log      ├─┼───┐   │   │
    │  │    │ systemd  │              ││ Collector │ │   │   │   │
    │  │    │(validator)│              │└───────────┘ │   │   │   │
    │  │    └──────────┘              └──────────────┘   │   │   │
    │  └──────────────────────────────────────────────┘   │   │   │
    └─────────────────────────────────────────────────┼───┼───┘   │
                                                      │   │       │
                                          gRPC (5 RPCs)   │       │
                                                      │   │       │
    ┌─────────────── QUANTUM 2: Controller ───────────┼───┼───────┘
    │                                                 │   │
    │  ┌──────────┐   in-memory channels  ┌───────────▼───▼──┐
    │  │  HTTP/   │ ◄───────────────────► │  gRPC Server     │
    │  │  Web UI  │   (mpsc, broadcast)   └──────────────────┘
    │  └──────────┘                              │
    │       │                                    │
    │       │ SQL queries                        │
    │       ▼                                    ▼
    │  ┌─────────────────────────────────────────────┐
    │  │              SQLite                          │
    │  │  nodes │ status_history │ logs │ alerts      │
    │  └─────────────────────────────────────────────┘
    └─────────────────────────────────────────────────────────────┘
```

### Coupling summary

| Boundary | Static | Dynamic | Coupling Type |
|----------|--------|---------|---------------|
| Agent ↔ Controller | Medium (shared proto types via extern_path) | Medium (persistent gRPC, bidirectional streams) | **Contract coupling** via proto service |
| Within Agent | High (same crate, shared Arc/channels) | High (in-memory channels, shared state) | **Content coupling** (single process) |
| Within Controller | High (shared DashMap, shared Db) | High (in-memory channels) | **Content coupling** (single process) |

Each quantum is a monolith internally (tight cohesion), connected to the other only by the gRPC wire (loose coupling). One data type (`NodeStatus`) flows from the reconcile loop → shared state → gRPC → SQLite → web UI.

## Why One Binary Per Node

The agent was originally two processes (operator + link) communicating via files on disk. They were merged because:

- **They were one deployment quantum** — always shipped together, upgraded together, couldn't function without each other
- **File IPC added complexity for no gain** — binary proto state file + JSON command file + inotify watcher, all to bridge two processes on the same box
- **Duplicate boilerplate** — two configs, two systemd units, two tokio runtimes, two signal handlers, two loggers
- **Latency** — commands had to round-trip through files (up to 20s polling) instead of flowing directly via in-memory channel

The merge eliminated the IPC layer entirely. Commands now go from gRPC handler → mpsc channel → reconcile loop in microseconds. State is just an `Arc<RwLock<NodeStatus>>`, not a file.
