# Pillar Architecture

Solana node operations platform. Three components, two deployment units, one data type flowing through the entire system.

## Components

- **Operator** (`pillar-operator`) — runs on each node, manages the validator lifecycle (health checks, restarts, snapshot recovery). No HTTP server, no external communication.
- **Link** (`pillar-link`) — runs alongside operator on each node, owns all external communication (HTTP endpoints, gRPC to controller). Reads operator state file, enriches with system/process metrics.
- **Controller** (`pillar-controller`) — centralized management plane. Receives metrics from all Link instances via gRPC, stores in SQLite, serves web UI, provides Grafana-compatible scrape endpoint.

## Architecture Quanta

An *architecture quantum* is the smallest independently deployable unit with high functional cohesion that includes all structural elements required for it to function.

Pillar has **two quanta**:

### Quantum 1: Node Agent (operator + link)

These two binaries are co-deployed on every node and cannot function independently:

- **Operator** produces state but has no way to expose it — no HTTP server, no gRPC, no sockets. It writes to a file and that's it.
- **Link** consumes that file but produces no node-health data of its own — it's purely a relay and enrichment layer.

Neither is useful alone. They form a single deployment quantum. You ship them together, upgrade them together, and a node is only "online" when both are running.

### Quantum 2: Controller

The controller is independently deployable. It runs on a completely different machine (often a laptop or a management server), has its own persistence (SQLite), its own UI (embedded SPA), and its own config. It can start, stop, and restart without affecting any running node agents — nodes just see a temporary gRPC disconnect and retry with backoff.

### The quantum boundary is the gRPC wire

The two quanta are connected only by the gRPC service defined in `pillar.proto`. This is the single contract between deployment units. Within each quantum, coupling is much tighter (shared crate, shared files on disk).

## Static Coupling (Compile-Time)

Static coupling is what the compiler enforces — shared types, shared crates, shared proto definitions.

```
                    ┌──────────────┐
                    │ pillar-shared │
                    │  (library)   │
                    └──────┬───────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
        ┌─────▼──┐   ┌────▼───┐  ┌────▼──────┐
        │operator│   │  link  │  │controller │
        └────────┘   └────────┘  └───────────┘
```

All three binaries depend on `pillar-shared`. No binary depends on any other binary. The dependency graph is a clean star topology.

### What `pillar-shared` provides

| Export | Used by | Purpose |
|--------|---------|---------|
| `proto::NodeStatus` | all three | The single data type flowing through the entire system |
| `proto::ControllerCommand` | link, controller | Command envelope (restart, recover, provision, upgrade, stop) |
| `proto::ProvisionCommand` | all three | 23-field provisioning spec |
| `proto::LogBatch` / `LogEntry` | link, controller | Structured log streaming |
| `write_state()` / `read_state()` | operator, link | Binary proto file I/O |
| `PendingCommand` enum | operator, link | JSON-serialized IPC between link and operator |
| `PENDING_COMMAND_PATH` | operator, link | Hardcoded path constant (`/var/run/pillar/pending-command.json`) |

### The `extern_path` trick

Both link and controller compile `pillar.proto` through `tonic-prost-build` for their gRPC stubs, but use `extern_path(".pillar", "pillar_shared::proto")` so the generated code references the shared crate's types instead of generating duplicates. This means `proto::NodeStatus` is a single Rust type everywhere, not three independently generated copies. This is a deliberate static coupling choice that eliminates serialization boundaries within each quantum.

### Static coupling strength by pair

| Pair | Coupling | Mechanism |
|------|----------|-----------|
| operator ↔ link | **High** | Shared `NodeStatus` struct (32 fields), shared `PendingCommand` enum, shared file paths, shared `read_state`/`write_state` functions |
| link ↔ controller | **Medium** | Shared proto types via `extern_path`, but communicate only through gRPC (wire boundary) |
| operator ↔ controller | **None** | Zero direct dependency. They share types through `pillar-shared` but never reference each other |

## Dynamic Coupling (Runtime)

Dynamic coupling is how components communicate at runtime — the protocols, the synchronicity, the failure domains.

### Channel 1: Operator → Link (file-based, async, one-way)

```
Operator                          Link
   │                               │
   │  write_state(NodeStatus)      │
   │  ─── /var/run/pillar/ ───►    │  read_state() every 5s
   │  operator-state.bin           │
   │  (atomic: write tmp+rename)   │  enrich with cpu/mem/disk/net
   │                               │  store in SharedState
```

- **Protocol**: Binary protobuf file on local filesystem
- **Synchronicity**: Fully asynchronous. Operator writes every 20s, link polls every 5s. They never wait for each other.
- **Failure mode**: If operator dies, link reads stale data (detectable via `updated_at_unix_secs`). If link dies, operator doesn't notice or care.
- **Coupling strength**: **Data coupling** — they share only a data structure (NodeStatus) through a file. No function calls, no shared memory, no sockets.

### Channel 2: Link → Operator (file-based, async, one-way)

```
Link                              Operator
   │                               │
   │  write PendingCommand JSON    │
   │  ─── /var/run/pillar/ ───►    │  process_pending_command()
   │  pending-command.json         │  every reconcile tick (20s)
   │                               │  read + delete atomically
```

- **Protocol**: JSON file on local filesystem (tagged enum via serde)
- **Synchronicity**: Fire-and-forget. Link writes the file and moves on. Operator picks it up on next tick.
- **Failure mode**: If operator is down, the command file sits there until it restarts. At-most-once delivery (delete before processing).
- **Coupling strength**: **Data coupling** — the `PendingCommand` enum is the sole contract.

### Channel 3: Link → Controller (gRPC, bidirectional, persistent)

```
Link                                Controller
   │                                    │
   │──── RegisterNode ─────────────────►│  upsert_node() in SQLite
   │                                    │
   │──── ReportStatus (every 10s) ─────►│  update NodeRegistry + SQLite
   │                                    │
   │◄─── CommandStream (server-stream)──│  mpsc channel per node
   │                                    │
   │──── PushLogs (client-stream) ─────►│  insert into logs table
   │                                    │  broadcast to SSE subscribers
   │──── ReportUpgradeStatus ──────────►│  log success/failure
```

- **Protocol**: gRPC over HTTP/2 (tonic), defined by `PillarController` service in `pillar.proto`
- **Synchronicity**: Mixed. `ReportStatus` is request-response (synchronous). `CommandStream` is a long-lived server-stream (asynchronous push). `PushLogs` is client-streaming (asynchronous).
- **Failure mode**: Link retries with exponential backoff (1s → 60s max). Controller tracks nodes as "offline" after 60s of silence. Commands are lost if the stream drops (no persistent queue).
- **Coupling strength**: **Contract coupling** — they agree on the proto service definition but know nothing about each other's internals.

### Channel 4: Controller → Nodes (command push through gRPC)

The full command path from UI to node:

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
Link handle_command()
    │  match on Command variant
    ▼
write PendingCommand JSON file
    │
    ▼
Operator process_pending_command()
    │  read + delete + execute
    ▼
systemctl restart solana-validator
```

Six hops from click to action. Each hop is a different coupling mechanism:

1. HTTP JSON (browser → controller)
2. In-memory mpsc channel (API handler → registry)
3. gRPC stream (registry → link)
4. File I/O (link → operator)
5. Subprocess exec (operator → systemd)
6. Systemd signal (systemd → validator process)

## Full Coupling Map

```
    ┌─────────────────── QUANTUM 1: Node Agent ───────────────────┐
    │                                                              │
    │  ┌──────────┐   binary proto file   ┌──────────┐           │
    │  │ Operator  │ ───────────────────► │   Link   │           │
    │  │          │ ◄─────────────────── │          │           │
    │  │          │   JSON command file   │          │           │
    │  └──────────┘                       └─────┬────┘           │
    │       │                                    │                │
    │       │ systemctl                          │                │
    │       ▼                                    │                │
    │  ┌──────────┐                              │                │
    │  │ systemd  │                              │                │
    │  │(validator)│                              │                │
    │  └──────────┘                              │                │
    └────────────────────────────────────────────┼────────────────┘
                                                 │
                                          gRPC (5 RPCs)
                                       proto contract only
                                                 │
    ┌─────────────── QUANTUM 2: Controller ──────┼────────────────┐
    │                                            │                │
    │  ┌──────────┐   in-memory channels  ┌─────▼────┐           │
    │  │  HTTP/   │ ◄───────────────────► │  gRPC    │           │
    │  │  Web UI  │   (mpsc, broadcast)   │  Server  │           │
    │  └──────────┘                       └──────────┘           │
    │       │                                    │                │
    │       │ SQL queries                        │                │
    │       ▼                                    ▼                │
    │  ┌─────────────────────────────────────────────┐           │
    │  │              SQLite                          │           │
    │  │  nodes │ status_history │ logs │ alerts      │           │
    │  └─────────────────────────────────────────────┘           │
    └─────────────────────────────────────────────────────────────┘
```

### Coupling summary at each boundary

| Boundary | Static | Dynamic | Coupling Type |
|----------|--------|---------|---------------|
| Operator ↔ Link | High (shared crate, shared types, shared file paths) | Low (async file I/O, no coordination) | **Data coupling** via filesystem |
| Link ↔ Controller | Medium (shared proto types via extern_path) | Medium (persistent gRPC, bidirectional streams) | **Contract coupling** via proto service |
| Operator ↔ Controller | None (no direct reference) | None (no direct communication) | **Zero coupling** — link is the bridge |
| Within Controller | N/A | High (in-memory channels, shared DashMap, shared Db) | **Content coupling** (monolith internals) |

The key architectural property: **static coupling is highest within each quantum** (operator and link share `pillar-shared` intimately), while **dynamic coupling is loosest across the quantum boundary** (gRPC with retry/backoff, no shared state). Tight cohesion within a deployment unit, loose coupling between them.
