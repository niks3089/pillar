# Pillar

Solana node operations platform with 3 components:

- **Operator** (`pillar-operator`) — runs on each node, manages the validator lifecycle (health checks, restarts, snapshot recovery)
- **Link** (`pillar-link`) — runs alongside operator on each node, owns all external communication (HTTP endpoints, gRPC to controller)
- **Controller** (`pillar-controller`) — centralized management plane with web UI, receives metrics from all nodes, pushes commands

## Architecture

```
Operator                    Link                        Controller
   |                          |                             |
   |  write state file -----> |  read + enrich              |
   |                          |  push via gRPC -----------> |  store in SQLite
   |                          |  <--- commands ------------ |  serve web UI
   |  <-- pending command --- |                             |
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

### Node (operator + link)

```bash
curl -sSL https://get.pillar.sh | bash -s -- --controller <controller-endpoint>
```

## Configuration

- Operator: `PILLAR_CONFIG` env var or `config.yaml`
- Link: `PILLAR_LINK_CONFIG` env var or `link-config.yaml`
- Controller: `PILLAR_CONTROLLER_CONFIG` env var or `controller-config.yaml`
