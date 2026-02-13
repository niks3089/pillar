# Pillar

Solana node operations platform with 2 components:

- **Agent** (`pillar-agent`) — runs on each node, manages the validator lifecycle (health checks, restarts, snapshot recovery) and handles all external communication (HTTP endpoints, gRPC to controller, Prometheus metrics, log streaming)
- **Controller** (`pillar-controller`) — centralized management plane with web UI, receives metrics from all agents, pushes commands

## Architecture

```
Agent                                   Controller
   │                                        │
   │  reconcile loop (health, state)        │
   │  enrich with system metrics            │
   │                                        │
   │──── RegisterNode ─────────────────────►│  store in SQLite
   │──── ReportStatus (every 10s) ─────────►│  update NodeRegistry + SQLite
   │◄─── CommandStream (server-stream) ─────│  push commands (restart, etc.)
   │──── PushLogs (client-stream) ─────────►│  store in logs table
   │                                        │  serve web UI + /metrics
```

## Building

```bash
cargo build --release
```

## Testing

```bash
cargo test
cargo clippy -- -D warnings
```

## Installation

### Controller

```bash
curl -sSL https://github.com/niks3089/pillar/releases/latest/download/install-controller.sh | bash
```

### Node (agent)

```bash
curl -sSL https://get.pillar.sh | bash -s -- --controller <controller-endpoint>
```

## Configuration

- Agent: `PILLAR_AGENT_CONFIG` env var or `agent.yaml`
- Controller: `PILLAR_CONTROLLER_CONFIG` env var or `controller-config.yaml`
