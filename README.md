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

The controller binary requires GLIBC 2.39 — install on **Ubuntu 24.04 (Noble) or newer**.

### Controller

```bash
curl -sSL https://janus-meter.s3.eu-north-1.amazonaws.com/pillar/latest/install-controller.sh \
  | sudo bash -s -- --external-url http://<controller-ip>:50051
```

Installs the controller, Prometheus, and Grafana (dashboards provisioned automatically).
Default login is `admin` / `admin` — change it before any real use.

### Node (agent)

The controller issues the exact command (with token) at `GET /api/onboard-command`:

```bash
curl -sSL https://janus-meter.s3.eu-north-1.amazonaws.com/pillar/latest/install-node.sh \
  | sudo bash -s -- --controller http://<controller-ip>:50051 --token <token> --http-url http://<controller-ip>:8080
```

## Grant submission

See [`docs/GRANT.md`](docs/GRANT.md) for the grant-submission overview, the live demo
deployment, and the multi-client roadmap.

## Configuration

- Agent: `PILLAR_AGENT_CONFIG` env var or `agent.yaml`
- Controller: `PILLAR_CONTROLLER_CONFIG` env var or `controller-config.yaml`
