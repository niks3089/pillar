# Pillar

Solana node operations platform with 2 components:

- **Agent** (`pillar-agent`) — single binary on each node. Manages the validator lifecycle (health checks, restarts, snapshot recovery), collects system/process metrics, serves HTTP endpoints, streams logs, and connects to the controller via gRPC.
- **Controller** (`pillar-controller`) — centralized management plane. Receives metrics from all agents via gRPC, stores in SQLite, serves web UI, provides Grafana-compatible scrape endpoint, fires alerts, and pushes binary upgrades to nodes.

Deployment: **agent + controller**. Agent always connects to a controller.

## Architecture Decisions

- **Fully independent** from Helius shared crates (no `helius` path dep)
- **No Solana SDK deps** — raw JSON-RPC via reqwest for health checks
- **Prometheus metrics** (open standard)
- **figment** for YAML config, **thiserror** for errors
- **Single agent binary** — reconciler, gRPC, HTTP, metrics, log collector run as concurrent async tasks sharing `NodeStatus` via `Arc<RwLock>`
- **Single `NodeStatus` proto type** — flows agent → controller → SQLite → web UI + /metrics
- **Controller always required** — no standalone agent mode
- **extern_path for shared proto types** — agent and controller build.rs use `extern_path` so gRPC stubs reference `pillar_shared::proto::*`
- **All services run as `sol` user** — sudoers at `/etc/sudoers.d/sol-pillar` grants passwordless access to systemctl, install, tee, sed, mkdir, cp, find
- **Script-based provisioning** — controller renders shell script templates, sends via gRPC, agent executes as `sol` user (not sudo bash). Scripts use `sudo` for individual privileged commands.
- **SSE log streaming** — `SseLogEntry` wrapper in api.rs maps proto `timestamp_unix_ms` → frontend `timestamp_ms`
- Author: Nikhil Acharya

## Crate Structure

```
pillar/
  Cargo.toml              # workspace: agent, shared, controller (edition 2021, LTO release)
  shared/                 # library crate — proto types + state reader/writer
    build.rs              # prost-build proto compilation with serde derives
    proto/pillar.proto    # NodeStatus, gRPC service (ReportStatus, CommandStream, RegisterNode, ReportScriptResult, PushLogs)
    src/lib.rs            # exports proto module
  agent/                  # binary crate
    src/
      main.rs             # bootstrap: config → spawn reconciler + gRPC + HTTP + metrics + logs
      config.rs           # AgentConfig
      reconcile.rs        # health check loop + state machine + command handler
      script_executor.rs  # runs provision/upgrade scripts as `bash` (not sudo bash)
      grpc.rs             # gRPC client (RegisterNode, ReportStatus, CommandStream, PushLogs)
      http.rs             # axum: GET /health, /status, /version, /metrics
      metrics.rs          # Prometheus registry
      metrics_updater.rs  # enriches NodeStatus with sysinfo
      log_collector.rs    # tails journald → PushLogs gRPC (requires systemd-journal group)
      health/             # HealthChecker trait: rpc_health.rs, validator_health.rs
      client/             # ValidatorClient trait: agave.rs, jito.rs, firedancer.rs, frankendancer.rs
      lifecycle/mod.rs    # SystemdManager — uses `sudo systemctl`
      snapshot/           # SnapshotManager trait, TCP download, recovery
  controller/
    scripts/              # Shell script templates (provision-agave.sh.tmpl, etc.)
    src/
      main.rs             # config → SQLite → gRPC server → HTTP server → retention pruner
      db.rs               # SQLite schema, CRUD, retention pruning
      grpc_server.rs      # PillarController impl (5 RPCs), includes stderr in script failure logs
      api.rs              # axum JSON API, SseLogEntry wrapper for SSE
      templates.rs        # include_str! for script templates, render() with {{placeholder}}
      node_registry.rs    # in-memory node tracking, command channels, log broadcast
      web.rs              # rust-embed SPA serving
      metrics_endpoint.rs # Prometheus /metrics with per-node labels
    web/                  # React + Vite SPA (build to dist/, embedded via rust-embed)
    dashboards/           # Grafana JSON + Prometheus scrape config
  scripts/
    install-node.sh       # idempotent installer for pillar-agent
```

## Controller JSON API

```
GET  /api/onboard-command            node onboard command (with controller URL)
GET  /api/overview                   fleet summary
GET  /api/nodes                      all nodes with latest status
GET  /api/nodes/:id                  node detail
GET  /api/nodes/:id/history          paginated status history
GET  /api/nodes/:id/logs             paginated logs (service, level, since, limit)
GET  /api/nodes/:id/logs/stream      SSE live log stream
POST /api/nodes/:id/restart          restart validator
POST /api/nodes/:id/recover          snapshot recovery
POST /api/nodes/:id/provision        install validator (client, version, cluster, etc.)
POST /api/nodes/:id/upgrade          binary upgrade
POST /api/nodes/:id/stop             stop validator
POST /api/nodes/:id/cancel           cancel provisioning
DELETE /api/nodes/:id                remove node
GET  /api/cluster-defaults/:cluster  entrypoints, known_validators, reference_rpc
GET  /metrics                        Prometheus scrape (all nodes, labeled)
```

## Provision Flow

1. User fills "Setup Validator" form in UI (client, version, cluster, paths, flags, etc.)
2. `POST /api/nodes/:id/provision` → controller renders script template → sends via `CommandStream` gRPC
3. Agent script_executor runs script as `bash` (sol user), individual commands use `sudo`
4. Script: checks if binary exists at correct version → downloads if needed → writes systemd unit → starts service → updates agent config → restarts agent
5. Agave v3.x: validator binary NOT in release tarballs — must be pre-installed or built from source
6. Agave v2.x: `agave-validator` IS in the `solana-release-*.tar.bz2` tarball
7. Default dynamic port range: `8000-8030` (v3.x requires minimum 25 ports)

## Dev Environment

- Dev box: `202.8.11.101` (SSH user: `ubuntu`, 32 cores, 123GB RAM)
  - Controller: HTTP `:8080`, gRPC `:50051`
  - Agent on same box, connected to controller
  - Cluster: **testnet**, currently running Agave v3.1.8
- Reference RPC: `https://api.testnet.solana.com`

## Building & Deploying

Dev host is macOS (aarch64). Dev box is Linux x86_64. Cross-compiling fails for libsqlite3-sys. Build on the dev box.

```bash
# 1. Build frontend (from Mac)
cd controller/web && npm run build

# 2. Sync source to dev box
rsync -az --exclude target --exclude node_modules --exclude .git . ubuntu@202.8.11.101:/tmp/pillar-build/

# 3. Build on dev box
ssh ubuntu@202.8.11.101 "cd /tmp/pillar-build && export PATH=/home/ubuntu/.cargo/bin:\$PATH && cargo build --release -p pillar-controller"

# 4. Deploy controller (NOTE: systemd runs /usr/local/bin/controller, NOT pillar-controller)
ssh ubuntu@202.8.11.101 "sudo systemctl stop pillar-controller && sudo cp /tmp/pillar-build/target/release/controller /usr/local/bin/controller && sudo systemctl start pillar-controller"

# 5. Deploy agent
ssh ubuntu@202.8.11.101 "cd /tmp/pillar-build && export PATH=/home/ubuntu/.cargo/bin:\$PATH && cargo build --release -p pillar-agent"
ssh ubuntu@202.8.11.101 "sudo systemctl stop pillar-agent && sudo cp /tmp/pillar-build/target/release/agent /usr/local/bin/pillar-agent && sudo systemctl start pillar-agent"
```

**Important**: `include_str!` template changes may not trigger recompilation. Force it:
```bash
rm -rf target/release/.fingerprint/pillar-controller-* target/release/deps/pillar_controller-* target/release/controller
```

### Service files
- Controller: `/etc/systemd/system/pillar-controller.service` (runs as `pillar` user, binary: `/usr/local/bin/controller`)
- Agent: `/etc/systemd/system/pillar-agent.service` (runs as `sol` user, binary: `/usr/local/bin/pillar-agent`)
- Controller config: `/etc/pillar/controller.yaml`
- Agent config: `/etc/pillar/agent.yaml`

## Current Status

**Working end-to-end**:
- Agent: reconciler, health checks, crash detection, metrics, HTTP endpoints, gRPC push, log collector, script executor
- Controller: gRPC server (5 RPCs), SQLite persistence, web UI (fleet overview + node detail + logs + provisioning), JSON API, Prometheus /metrics, SSE log streaming
- Provisioning: Agave v3.1.8 deployed on testnet via UI, script-based flow with tarball/binary/pre-installed detection
- Logs: journald → agent → controller → SQLite + SSE → UI (requires sol in systemd-journal group)
- Install script: `scripts/install-node.sh` with sol user, sudoers, sysctl, Solana CLI, keypairs

**TODO**:
- [ ] Controller alert engine — condition eval, webhook/log actions, dedup on transition
- [ ] Controller artifact storage — upload, serve, SHA256 verification
- [ ] `scripts/install-controller.sh` — single-command installer with NAT detection + Cloudflare Tunnel
- [ ] Update `scripts/install-node.sh` — download prebuilt binary from GitHub Releases
- [ ] Version fetcher — GitHub Releases API for Agave/Jito/Firedancer version dropdown
- [ ] Agent self-upgrade — swap binary, exit, systemd restarts with new version
- [ ] Jito/Firedancer/Frankendancer client impls (stubs only)

## Conventions

- `cargo clippy -- -D warnings` must be clean
- Shared types in `pillar-shared`; proto types via prost with serde derives
- Tests inline in modules
- Script templates in `controller/scripts/` use `{{placeholder}}` syntax, compiled in via `include_str!`
