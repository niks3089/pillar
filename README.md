# Pillar

Solana node operations platform with 2 components:

- **Agent** (`pillar-agent`) — runs on each node, **supervises the validator process** (health checks, restarts, snapshot recovery) and handles all external communication (HTTP endpoints, gRPC to controller, Prometheus metrics, log streaming).
- **Controller** (`pillar-controller`) — centralized management plane with web UI. **One controller manages many nodes**: it receives status/metrics from every agent and pushes lifecycle commands.

## Architecture

On each node, the agent supervises a running validator — it drives the validator's
lifecycle, reads its health over JSON-RPC, and tails its logs:

```
┌──────────────────────────── one node ─────────────────────────────┐
│                                                                    │
│   ┌──────────────────────────────────────────────────────────┐    │
│   │  Validator  (agave │ jito │ firedancer)  — systemd service │    │
│   │  state: running / catching-up / behind / stopped           │    │
│   └───────────▲───────────────────────────────┬───────────────┘    │
│               │ manage:                        │ JSON-RPC:          │
│               │ start / stop / restart         │ health, slot,      │
│               │ provision / recover            │ version, voting    │
│               │                                ▼                    │
│   ┌──────────────────────────── Agent ───────────────────────────┐ │
│   │  reconcile loop · state machine · crash-loop detection         │ │
│   │  snapshot download + recovery · sysinfo metrics · journald tail│ │
│   │  HTTP :9090  →  /health  /status  /version  /metrics           │ │
│   └────────────────────────────────────────────────────────────────┘│
└───────────────────────────────────┬────────────────────────────────┘
                                     │ gRPC (mTLS)
                                     ▼
```

Each agent connects out to the one controller:

```
Agent                                     Controller  (one, manages every node)
   │──── RegisterNode ──────────────────────►│  store in SQLite
   │──── ReportStatus (every 10s) ──────────►│  update NodeRegistry + SQLite
   │◄─── CommandStream (server-stream) ──────│  provision / upgrade / restart /
   │                                         │  recover / stop
   │──── PushLogs (client-stream) ──────────►│  logs table
   │                                         │  serve web UI + /metrics + Grafana
```

A single `NodeStatus` type flows agent → controller → SQLite → web UI + `/metrics`.

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

The controller binary requires GLIBC 2.39 — install on **Ubuntu 24.04 (Noble) or newer**.

### Controller

```bash
curl -sSL https://github.com/niks3089/pillar/releases/latest/download/install-controller.sh \
  | sudo bash -s -- --external-url https://<controller-ip>:50051
```

Installs the controller, Prometheus, and Grafana (dashboards provisioned automatically).
Default login is `admin` / `admin` — change it before any real use.

### Node (agent)

The controller issues the exact command (with token) at `GET /api/onboard-command`:

```bash
curl -sSL https://github.com/niks3089/pillar/releases/latest/download/install-node.sh \
  | sudo bash -s -- --controller https://<controller-ip>:50051 --token <token> --http-url http://<controller-ip>:8080
```

## Grant submission

See [`docs/GRANT.md`](docs/GRANT.md) for the grant-submission overview, the live demo
deployment, and the multi-client roadmap.

## Configuration

- Agent: `PILLAR_AGENT_CONFIG` env var or `agent.yaml`
- Controller: `PILLAR_CONTROLLER_CONFIG` env var or `controller-config.yaml`
